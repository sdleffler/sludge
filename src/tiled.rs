use {
    anyhow::*,
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    serde::{de::DeserializeOwned, Deserialize, Serialize},
    std::{
        marker::PhantomData,
        ops,
        path::{Path, PathBuf},
    },
};

use crate::{
    ecs::*,
    filesystem::Filesystem,
    graphics::{
        DrawableGraph, DrawableNodeId, Graphics, InstanceParam, Mesh, Sprite, SpriteBatch,
        SpriteId, Texture,
    },
    loader::{Inspect, Key, Load, Loaded, Storage},
    math::*,
    tiled::xml_parser::LayerData,
    Atom, Resources, SharedResources,
};

mod xml_parser;

fn deserialize_properties<T: DeserializeOwned>(properties: &xml_parser::Properties) -> Result<T> {
    use xml_parser::PropertyValue::*;

    let ron_map = properties
        .iter()
        .map(|(k, v)| {
            let key = k.to_owned();
            let value = match v {
                BoolValue(b) => ron::Value::Bool(*b),
                FloatValue(f) => {
                    ron::Value::Number(ron::Number::Float(ron::value::Float::new(*f as f64)))
                }
                IntValue(i) => ron::Value::Number(ron::Number::Integer(*i as i64)),
                ColorValue(_) => bail!("Color property values not yet supported!"),
                StringValue(s) => ron::Value::String(s.to_owned()),
            };

            Ok((ron::Value::String(key), value))
        })
        .collect::<Result<ron::Map>>()?;

    ron::Value::Map(ron_map).into_rust().map_err(Error::from)
}

pub trait TileProperties: Send + Sync + Clone + 'static {
    fn is_solid(&self) -> bool {
        false
    }
}

impl TileProperties for ron::Value {
    fn is_solid(&self) -> bool {
        #[derive(Deserialize)]
        struct Extract {
            #[serde(default)]
            solid: bool,
        }

        self.clone()
            .into_rust::<Extract>()
            .map(|ext| ext.solid)
            .unwrap_or_default()
    }
}

pub trait LayerProperties: Send + Sync + Clone + 'static {
    fn is_solid(&self) -> bool {
        false
    }

    fn index(&self) -> i32 {
        -1
    }
}

impl LayerProperties for ron::Value {
    fn is_solid(&self) -> bool {
        #[derive(Deserialize)]
        struct Extract {
            #[serde(default)]
            solid: bool,
        }

        self.clone()
            .into_rust::<Extract>()
            .map(|ext| ext.solid)
            .unwrap_or_default()
    }

    fn index(&self) -> i32 {
        #[derive(Deserialize)]
        struct Extract {
            #[serde(default)]
            index: i32,
        }

        self.clone()
            .into_rust::<Extract>()
            .map(|ext| ext.index)
            .unwrap_or(-1)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Frame {
    pub local_id: u32,
    pub duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "TileProps: DeserializeOwned"))]
pub struct TileData<TileProps = ron::Value> {
    pub tile_type: Option<String>,
    pub local_id: u32,
    pub frames: Option<Vec<Frame>>,

    #[serde(bound(deserialize = "TileProps: DeserializeOwned"))]
    pub properties: TileProps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "TileProps: DeserializeOwned"))]
pub struct TileSheet<TileProps = ron::Value> {
    name: String,

    first_global_id: u32,
    tile_count: u32,

    /// Path to the image file.
    source: PathBuf,

    sheet_width: u32,
    sheet_height: u32,

    tile_width: u32,
    tile_height: u32,

    margin: u32,
    spacing: u32,

    /// Mapping local IDs to tile data.
    #[serde(bound(deserialize = "TileProps: DeserializeOwned"))]
    tile_data: HashMap<u32, TileData<TileProps>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TileSheetRegion {
    pub global_id: u32,
    pub local_id: u32,

    pub bounds: Box2<u32>,
    pub uv: Box2<f32>,
}

impl<TileProps> TileSheet<TileProps> {
    fn from_tiled(tiled: &xml_parser::Tileset) -> Result<Self>
    where
        TileProps: DeserializeOwned,
    {
        ensure!(
            tiled.images.len() == 1,
            "tileset must have exactly one image"
        );
        let image = &tiled.images[0];

        let tile_data = tiled
            .tiles
            .iter()
            .map(|tile| {
                let tile_data = TileData {
                    tile_type: tile.tile_type.clone(),
                    local_id: tile.id,
                    frames: tile.animation.as_ref().map(|frames| {
                        frames
                            .iter()
                            .map(|frame| Frame {
                                local_id: frame.tile_id,
                                duration: frame.duration,
                            })
                            .collect()
                    }),
                    properties: deserialize_properties(&tile.properties)?,
                };

                Ok((tile.id, tile_data))
            })
            .collect::<Result<_>>()?;

        Ok(TileSheet {
            name: tiled.name.clone(),

            first_global_id: tiled.first_gid,

            source: image.source.clone(),

            sheet_width: image.width as u32,
            sheet_height: image.height as u32,

            tile_width: tiled.tile_width,
            tile_height: tiled.tile_height,

            spacing: tiled.spacing,
            margin: tiled.margin,

            tile_data,

            tile_count: tiled.tilecount.expect("TODO"),
        })
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.sheet_width, self.sheet_height)
    }

    pub fn get_region_from_local_id(&self, local_id: u32) -> TileSheetRegion {
        assert!(local_id < self.tile_count, "local id out of bounds");

        let origin = na::Point2::origin() + na::Vector2::repeat(self.margin);
        let extents = na::Vector2::new(self.tile_width, self.tile_height);
        let stride = na::Vector2::repeat(self.spacing) + extents;
        let columns = (self.sheet_width - self.margin) / (self.tile_width + self.spacing);
        let coord = na::Vector2::new(local_id % columns, local_id / columns);
        let corner = origin + stride.component_mul(&coord);
        let sheet_dims = Vector2::new(self.sheet_width as f32, self.sheet_height as f32);
        let uv_corner =
            Point2::from(na::convert::<_, Vector2<f32>>(corner.coords).component_div(&sheet_dims));
        let uv_extents = na::convert::<_, Vector2<f32>>(extents).component_div(&sheet_dims);

        TileSheetRegion {
            global_id: self.first_global_id + local_id,
            local_id: local_id,
            bounds: Box2::from_extents(corner, extents),
            uv: Box2::from_extents(uv_corner, uv_extents),
        }
    }

    pub fn get_region_from_global_id(&self, gid: u32) -> TileSheetRegion {
        assert!(gid >= self.first_global_id && gid <= self.last_global_id());
        self.get_region_from_local_id(gid - self.first_global_id)
    }

    pub fn iter_regions(&self) -> impl Iterator<Item = TileSheetRegion> + '_ {
        (0..self.tile_count).map(move |local_id| self.get_region_from_local_id(local_id))
    }

    pub fn iter_tile_data(&self) -> impl Iterator<Item = (u32, &TileData<TileProps>)> + '_ {
        self.tile_data
            .iter()
            .map(move |(local_id, tile)| (local_id + self.first_global_id, tile))
    }

    pub fn first_global_id(&self) -> u32 {
        self.first_global_id
    }

    pub fn last_global_id(&self) -> u32 {
        assert!(self.tile_count > 0, "tilesheet has no tiles");
        self.first_global_id + self.tile_count - 1
    }

    pub fn is_tile_animated_by_local_id(&self, local_id: u32) -> bool {
        self.tile_data
            .get(&local_id)
            .map(|data| data.frames.is_some())
            .unwrap_or_default()
    }

    pub fn is_tile_animated_by_global_id(&self, gid: u32) -> bool {
        assert!(gid >= self.first_global_id && gid <= self.last_global_id());
        self.is_tile_animated_by_local_id(gid - self.first_global_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u32>,
}

impl Chunk {
    pub fn tiles(&self) -> impl Iterator<Item = ((i32, i32), u32)> + '_ {
        let (w, x, y) = (self.w, self.x, self.y);
        self.data
            .iter()
            .copied()
            .enumerate()
            .map(move |(i, n)| ((i as u32 % w, i as u32 / w), n))
            .map(move |((i, j), n)| ((i as i32 + x, j as i32 + y), n))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "LayerProps: DeserializeOwned"))]
pub struct TileLayer<LayerProps = ron::Value> {
    pub name: Option<String>,
    pub opacity: f32,
    pub visible: bool,
    pub chunks: HashMap<(i32, i32), Chunk>,

    #[serde(bound(deserialize = "LayerProps: DeserializeOwned"))]
    pub properties: LayerProps,
}

impl Default for TileLayer {
    fn default() -> Self {
        Self {
            name: None,
            opacity: 1.0,
            visible: true,
            chunks: HashMap::new(),
            properties: ron::Value::Map(ron::Map::default()),
        }
    }
}

impl<L> TileLayer<L> {
    pub fn chunks(&self) -> impl Iterator<Item = ((i32, i32), &Chunk)> + '_ {
        self.chunks.iter().map(|(&k, v)| (k, v))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "LayerProps: DeserializeOwned"))]
#[non_exhaustive]
pub enum Layer<LayerProps = ron::Value> {
    #[serde(bound(deserialize = "LayerProps: DeserializeOwned"))]
    TileLayer(TileLayer<LayerProps>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "LayerProps: DeserializeOwned, TileProps: DeserializeOwned"))]
pub struct TiledMap<LayerProps = ron::Value, TileProps = ron::Value> {
    source: PathBuf,

    width: u32,
    height: u32,

    tile_width: u32,
    tile_height: u32,

    tile_sheets: Vec<TileSheet<TileProps>>,
    layers: Vec<Layer<LayerProps>>,
}

impl<'a, L: Component, T: Component> SmartComponent<ScContext<'a>> for TiledMap<L, T> {}

impl<LayerProps, TileProps> TiledMap<LayerProps, TileProps> {
    pub fn from_tiled(path: &Path, tiled: &xml_parser::Map) -> Result<Self>
    where
        LayerProps: DeserializeOwned,
        TileProps: DeserializeOwned,
    {
        let tile_sheets = tiled
            .tilesets
            .iter()
            .map(|ts| TileSheet::from_tiled(ts))
            .collect::<Result<_>>()?;

        let mut layers = tiled
            .layers
            .iter()
            .map(|layer| {
                let mut chunks = HashMap::new();

                match &layer.tiles {
                    LayerData::Finite(data) => {
                        chunks.insert(
                            (0, 0),
                            Chunk {
                                x: 0,
                                y: 0,
                                w: tiled.width,
                                h: tiled.height,
                                data: data.iter().flatten().map(|lt| lt.gid).collect(),
                            },
                        );
                    }
                    LayerData::Infinite(tiled_chunks) => {
                        for (&(x, y), tiled_chunk) in tiled_chunks.iter() {
                            chunks.insert(
                                (x, y),
                                Chunk {
                                    x: tiled_chunk.x,
                                    y: tiled_chunk.y,
                                    w: tiled_chunk.width,
                                    h: tiled_chunk.height,
                                    data: tiled_chunk
                                        .tiles
                                        .iter()
                                        .flatten()
                                        .map(|lt| lt.gid)
                                        .collect(),
                                },
                            );
                        }
                    }
                }

                Ok((
                    layer.layer_index,
                    Layer::TileLayer(TileLayer {
                        name: Some(layer.name.clone()),
                        visible: layer.visible,
                        opacity: layer.opacity,
                        chunks,
                        properties: deserialize_properties(&layer.properties)?,
                    }),
                ))
            })
            .collect::<Result<Vec<_>>>()?;

        layers.sort_by_key(|&(i, _)| i);
        Ok(TiledMap {
            source: path.to_owned(),

            width: tiled.width,
            height: tiled.height,

            tile_width: tiled.tile_width,
            tile_height: tiled.tile_height,

            tile_sheets,
            layers: layers.into_iter().map(|(_, v)| v).collect(),
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn tile_dimensions(&self) -> (u32, u32) {
        (self.tile_width, self.tile_height)
    }

    pub fn layers(&self) -> &[Layer<LayerProps>] {
        &self.layers
    }

    pub fn tile_sheets(&self) -> &[TileSheet<TileProps>] {
        &self.tile_sheets
    }

    pub fn get_tile_sheet_for_gid(&self, gid: u32) -> Option<&TileSheet<TileProps>> {
        self.tile_sheets
            .iter()
            .find(|ts| ts.first_global_id <= gid && gid <= ts.last_global_id())
    }

    pub fn get_tile_data_for_gid(&self, gid: u32) -> Option<&TileData<TileProps>> {
        self.get_tile_sheet_for_gid(gid)
            .and_then(|ts| ts.tile_data.get(&(gid - ts.first_global_id)))
    }
}

impl<C, TileProps> Load<C, Key> for TileSheet<TileProps>
where
    Self: for<'a> Inspect<'a, C, &'a SharedResources<'static>>,
    TileProps: DeserializeOwned + 'static,
{
    type Error = Error;

    fn load(key: Key, _storage: &mut Storage<C, Key>, ctx: &mut C) -> Result<Loaded<Self, Key>> {
        match key {
            Key::Path(path) => {
                let fh = Self::inspect(ctx).fetch_mut::<Filesystem>().open(&path)?;
                let tiled = xml_parser::parse_tileset(fh, 1)?;
                Ok(TileSheet::from_tiled(&tiled)?.into())
            } //_ => bail!("can only load from logical"),
        }
    }
}

impl<C, LayerProps, TileProps> Load<C, Key> for TiledMap<LayerProps, TileProps>
where
    Self: for<'a> Inspect<'a, C, &'a SharedResources<'static>>,
    LayerProps: DeserializeOwned + 'static,
    TileProps: DeserializeOwned + 'static,
{
    type Error = Error;

    fn load(key: Key, _storage: &mut Storage<C, Key>, ctx: &mut C) -> Result<Loaded<Self, Key>> {
        match key {
            Key::Path(path) => {
                let tiled = xml_parser::parse_file(
                    &mut *Self::inspect(ctx).fetch_mut::<Filesystem>(),
                    &path,
                )?;

                let mut deps = vec![];
                for ts in tiled.tilesets.iter() {
                    if let Some(src) = ts.source.as_ref() {
                        deps.push(Key::from_path(Path::new(src)));
                    }
                }

                Ok(Loaded::with_deps(Self::from_tiled(&path, &tiled)?, deps))
            } //_ => bail!("can only load from path"),
        }
    }
}

#[derive(Debug)]
pub struct TileAnimationFrame<TileProps: TileProperties> {
    info: TileInfo<TileProps>,
    /// Duration in milliseconds
    duration: u32,
}

#[derive(Debug)]
pub struct TileAnimation<TileProps: TileProperties> {
    frames: Vec<TileAnimationFrame<TileProps>>,
}

impl<'a, TileProps: TileProperties + Component> SmartComponent<ScContext<'a>>
    for TileAnimation<TileProps>
{
}

impl<TileProps: TileProperties> ops::Index<u32> for TileAnimation<TileProps> {
    type Output = TileAnimationFrame<TileProps>;

    fn index(&self, i: u32) -> &Self::Output {
        &self.frames[i as usize]
    }
}

impl<TileProps: TileProperties> TileAnimation<TileProps> {
    pub fn next_frame(&self, current_frame: u32) -> u32 {
        let incremented = current_frame + 1;
        if incremented >= self.frames.len() as u32 {
            0
        } else {
            incremented
        }
    }
}

#[derive(Debug)]
pub struct TileAnimationState {
    remaining: f32,
    tile_gid: u32,
    current_frame: u32,
    animation: Entity,
}

impl<'a> SmartComponent<ScContext<'a>> for TileAnimationState {}

#[derive(Bundle)]
pub struct UnbatchedAnimatedTile {
    pub sprite_id: DrawableNodeId<Sprite>,
    pub animation_state: TileAnimationState,
}

#[derive(Bundle)]
pub struct BatchedAnimatedTile {
    pub batch_id: DrawableNodeId<SpriteBatch>,
    pub sprite_id: SpriteId,
    pub animation_state: TileAnimationState,
}

#[derive(Debug, Clone)]
pub struct TileInfo<TileProps: TileProperties> {
    sheet: Atom,
    region: TileSheetRegion,
    special: Option<TileData<TileProps>>,
}

#[derive(Debug)]
pub struct TiledLayerManager<TileProps: TileProperties> {
    tile_width: u32,
    tile_height: u32,

    sheets: HashMap<Atom, Texture>,
    tiles: HashMap<u32, TileInfo<TileProps>>,

    meshes: Vec<DrawableNodeId<Mesh>>,
    sprites: Vec<DrawableNodeId<Sprite>>,
    batches: Vec<DrawableNodeId<SpriteBatch>>,

    animations: HashMap<u32, Entity>,
    owned_entities: Vec<Entity>,
}

impl<TileProps: TileProperties> TiledLayerManager<TileProps> {
    pub fn new(tile_width: u32, tile_height: u32) -> Self {
        Self {
            tile_width,
            tile_height,

            sheets: HashMap::new(),
            tiles: HashMap::new(),

            meshes: Vec::new(),
            sprites: Vec::new(),
            batches: Vec::new(),

            animations: HashMap::new(),
            owned_entities: Vec::new(),
        }
    }

    pub fn from_map<LayerProps: LayerProperties>(
        map: &TiledMap<LayerProps, TileProps>,
        gfx: &mut Graphics,
        scene: &mut DrawableGraph,
        sorted_layer_parent: Option<DrawableNodeId<()>>,
        fs: &mut Filesystem,
        world: &World,
        cmds: &mut CommandBuffer,
    ) -> Self {
        let mut this = Self::new(map.tile_width, map.tile_height);
        this.add_tilesheets(gfx, fs, world, cmds, map.tile_sheets());

        for layer in map.layers() {
            let tile_layer = match layer {
                Layer::TileLayer(tile_layer) => tile_layer,
                //_ => continue,
            };

            this.add_layer(gfx, scene, sorted_layer_parent, world, cmds, tile_layer);
        }

        this
    }

    #[inline]
    pub fn is_animated(&self, gid: u32) -> bool {
        self.tiles[&gid]
            .special
            .as_ref()
            .map_or(false, |data| data.frames.is_some())
    }

    pub fn add_tilesheets<'a>(
        &mut self,
        gfx: &mut Graphics,
        fs: &mut Filesystem,
        world: &World,
        cmds: &mut CommandBuffer,
        sheets: impl IntoIterator<Item = &'a TileSheet<TileProps>>,
    ) {
        for tile_sheet in sheets {
            let sheet_name = Atom::from(tile_sheet.name.as_str());
            log::info!("adding tilesheet {}", sheet_name);
            let mut texture_file = fs.open(tile_sheet.source()).unwrap();
            let texture = Texture::from_reader(gfx, &mut texture_file).unwrap();
            assert!(self.sheets.insert(sheet_name.clone(), texture).is_none());
            self.tiles.extend(tile_sheet.iter_regions().map(|region| {
                let data = tile_sheet.tile_data.get(&region.local_id).cloned();
                let info = TileInfo {
                    sheet: sheet_name.clone(),
                    region,
                    special: data,
                };
                (region.global_id, info)
            }));

            for (gid, tile_data) in tile_sheet
                .iter_tile_data()
                .filter(|(gid, _)| tile_sheet.is_tile_animated_by_global_id(*gid))
            {
                log::info!("animated tile found with gid {}", gid);

                let entity_id = world.reserve_entity();
                let frames = tile_data.frames.as_ref().unwrap().iter().map(|frame| {
                    let global_id = tile_sheet.first_global_id() + frame.local_id;
                    let tile_info = self.tiles[&global_id].clone();
                    TileAnimationFrame {
                        info: tile_info,
                        duration: frame.duration,
                    }
                });
                let animation = TileAnimation {
                    frames: frames.collect(),
                };
                cmds.insert_one(entity_id, animation);
                self.animations.insert(gid, entity_id);
            }
        }
    }

    pub fn add_layer<L: LayerProperties>(
        &mut self,
        gfx: &mut Graphics,
        scene: &mut DrawableGraph,
        sorted_layer_parent: Option<DrawableNodeId<()>>,
        world: &World,
        cmds: &mut CommandBuffer,
        layer: &TileLayer<L>,
    ) {
        let index = layer.properties.index();
        let mut batches = HashMap::new();
        for (_, chunk) in layer.chunks() {
            for ((tile_x, tile_y), gid) in chunk.tiles().filter(|&(_, gid)| gid != 0) {
                let dest = Vector2::new(
                    tile_x as f32 * self.tile_width as f32,
                    tile_y as f32 * self.tile_height as f32,
                );

                let tile_info = &self.tiles[&gid];
                let src_rect = tile_info.region.uv;

                if index != 0 {
                    let batch_id = *batches.entry(tile_info.sheet.clone()).or_insert_with(|| {
                        let texture = self.sheets[&tile_info.sheet].clone();
                        let batch = SpriteBatch::with_capacity(gfx, texture, 1024);
                        scene.insert(batch).layer(index).get()
                    });

                    let sprite_id =
                        scene[batch_id].insert(InstanceParam::new().translate2(dest).src(src_rect));

                    if self.is_animated(gid) {
                        log::info!(
                            "spawning batched animated tile entity with gid {} at coordinates {:?}",
                            gid,
                            (tile_x, tile_y)
                        );

                        let entity_id = world.reserve_entity();
                        cmds.insert(
                            entity_id,
                            BatchedAnimatedTile {
                                batch_id,
                                sprite_id,
                                animation_state: TileAnimationState {
                                    tile_gid: gid,
                                    remaining: 0.,
                                    current_frame: 0,
                                    animation: self.animations[&gid],
                                },
                            },
                        );
                    }
                } else {
                    let sprite = Sprite::new(
                        self.sheets[&tile_info.sheet].clone(),
                        InstanceParam::new().translate2(dest).src(src_rect),
                    );
                    let sprite_id = scene
                        .insert(sprite)
                        .layer(0)
                        .parent(sorted_layer_parent)
                        .get();
                    self.sprites.push(sprite_id);

                    if self.is_animated(gid) {
                        log::info!(
                            "spawning unbatched animated tile entity with gid {} at coordinates {:?}",
                            gid,
                            (tile_x, tile_y)
                        );

                        let entity_id = world.reserve_entity();
                        cmds.insert(
                            entity_id,
                            UnbatchedAnimatedTile {
                                sprite_id,
                                animation_state: TileAnimationState {
                                    tile_gid: gid,
                                    remaining: 0.,
                                    current_frame: 0,
                                    animation: self.animations[&gid],
                                },
                            },
                        );
                    }
                }
            }
        }
    }

    pub fn clear(&self, scene: &mut DrawableGraph, cmds: &mut CommandBuffer) {
        for &entity in self.animations.values() {
            cmds.despawn(entity);
        }

        for &entity in &self.owned_entities {
            cmds.despawn(entity);
        }

        for &id in &self.sprites {
            scene.remove(id);
        }

        for &id in &self.meshes {
            scene.remove(id);
        }

        for &id in &self.batches {
            scene.remove(id);
        }
    }
}

#[derive(Debug)]
pub struct TiledMapManager<LayerProps: LayerProperties, TileProps: TileProperties> {
    sorted_layer_parent: Option<DrawableNodeId<()>>,
    render_objects: HashMap<Entity, TiledLayerManager<TileProps>>,
    component_events: ReaderId<ComponentEvent>,
    _marker: PhantomData<LayerProps>,
}

impl<LayerProps: LayerProperties, TileProps: TileProperties>
    TiledMapManager<LayerProps, TileProps>
{
    pub fn new(world: &mut World, sorted_layer_parent: Option<DrawableNodeId<()>>) -> Self {
        let component_events = world.track::<TiledMap<LayerProps, TileProps>>();
        Self {
            sorted_layer_parent,
            render_objects: HashMap::new(),
            component_events,
            _marker: PhantomData,
        }
    }

    pub fn update(
        &mut self,
        world: &World,
        fs: &mut Filesystem,
        gfx: &mut Graphics,
        scene: &mut DrawableGraph,
        dt: f32,
    ) {
        let mut cmds = world.get_buffer();
        for &event in world.poll::<TiledMap<LayerProps, TileProps>>(&mut self.component_events) {
            match event {
                ComponentEvent::Inserted(entity) => {
                    let map = world
                        .get::<TiledMap<LayerProps, TileProps>>(entity)
                        .unwrap();
                    self.render_objects.insert(
                        entity,
                        TiledLayerManager::from_map(
                            &map,
                            gfx,
                            scene,
                            self.sorted_layer_parent,
                            fs,
                            world,
                            &mut cmds,
                        ),
                    );
                }
                ComponentEvent::Modified(entity) => {
                    if let Some(objects) = self.render_objects.remove(&entity) {
                        objects.clear(scene, &mut cmds);
                    }

                    let map = world
                        .get::<TiledMap<LayerProps, TileProps>>(entity)
                        .unwrap();
                    self.render_objects.insert(
                        entity,
                        TiledLayerManager::from_map(
                            &map,
                            gfx,
                            scene,
                            self.sorted_layer_parent,
                            fs,
                            world,
                            &mut cmds,
                        ),
                    );
                }
                ComponentEvent::Removed(entity) => {
                    if let Some(objects) = self.render_objects.remove(&entity) {
                        objects.clear(scene, &mut cmds);
                    }
                }
            }
        }

        for (_entity, (sprite_id, mut animation_state)) in world
            .query::<(&DrawableNodeId<Sprite>, &mut TileAnimationState)>()
            .iter()
        {
            let animation = world
                .get::<TileAnimation<TileProps>>(animation_state.animation)
                .unwrap();

            animation_state.remaining -= dt * 1_000.;
            if animation_state.remaining < 0. {
                let next_frame = animation.next_frame(animation_state.current_frame);
                animation_state.current_frame = next_frame;
                let frame = &animation[animation_state.current_frame];
                animation_state.remaining += frame.duration as f32;
                scene[*sprite_id].params.src = frame.info.region.uv;
            }
        }

        for (_entity, (batch_id, sprite_id, mut animation_state)) in world
            .query::<(
                &DrawableNodeId<SpriteBatch>,
                &SpriteId,
                &mut TileAnimationState,
            )>()
            .iter()
        {
            let animation = world
                .get::<TileAnimation<TileProps>>(animation_state.animation)
                .unwrap();

            animation_state.remaining -= dt * 1_000.;
            if animation_state.remaining < 0. {
                animation_state.current_frame = animation.next_frame(animation_state.current_frame);
                let frame = &animation[animation_state.current_frame];
                animation_state.remaining += frame.duration as f32;
                scene[*batch_id][*sprite_id].src = frame.info.region.uv;
            }
        }

        world.queue_buffer(cmds);
    }
}

pub struct TiledMapManagerSystem<L, T> {
    sorted_layer_parent: Option<DrawableNodeId<()>>,
    _marker: PhantomData<(L, T)>,
}

impl<L, T> TiledMapManagerSystem<L, T> {
    pub fn new(sorted_layer_parent: Option<DrawableNodeId<()>>) -> Self {
        Self {
            sorted_layer_parent,
            _marker: PhantomData,
        }
    }
}

impl<L, T> crate::System for TiledMapManagerSystem<L, T>
where
    L: LayerProperties,
    T: TileProperties,
{
    fn init(
        &self,
        _lua: LuaContext,
        resources: &mut Resources,
        _: Option<&SharedResources>,
    ) -> Result<()> {
        if !resources.has_value::<TiledMapManager<L, T>>() {
            let map_manager = TiledMapManager::<L, T>::new(
                resources.get_mut::<World>().expect("no World resource!"),
                self.sorted_layer_parent,
            );
            resources.insert(map_manager);
        }
        Ok(())
    }

    fn update(
        &self,
        _lua: LuaContext,
        local: &SharedResources,
        maybe_global: Option<&SharedResources>,
    ) -> Result<()> {
        let global = maybe_global
            .expect("TiledMapManager needs `Graphics` and `Filesystem` global resources!");

        let world = local.fetch::<World>();
        let mut fs = global.fetch_mut::<Filesystem>();
        let mut gfx = global.fetch_mut::<Graphics>();
        let mut scene = local.fetch_mut::<DrawableGraph>();

        // FIXME(sleffy): HAAAAAAAAAAAAAACK!
        let dt: f32 = 1. / 60.;

        let mut map_manager = local.fetch_mut::<TiledMapManager<L, T>>();
        map_manager.update(&*world, &mut *fs, &mut *gfx, &mut *scene, dt);

        Ok(())
    }
}
