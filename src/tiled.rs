use {
    anyhow::*,
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    serde::{de::DeserializeOwned, Deserialize, Serialize},
    std::{
        marker::PhantomData,
        path::{Path, PathBuf},
    },
};

use crate::{
    api::LuaComponentInterface,
    assets::{Asset, Cache, Key, Loaded},
    ecs::*,
    filesystem::Filesystem,
    math::*,
    tiled::xml_parser::LayerData,
    Resources, SludgeLuaContextExt, SludgeResultExt,
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.sheet_width, self.sheet_height)
    }

    pub fn get_tile_data_from_local_id(&self, local_id: u32) -> Option<&TileData<TileProps>> {
        self.tile_data.get(&local_id)
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

    pub fn source(&self) -> &Path {
        &self.source
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

impl<TileProps> Asset for TileSheet<TileProps>
where
    TileProps: DeserializeOwned + Send + Sync + 'static,
{
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<'static, Self>> {
        match key {
            Key::Path(path) => {
                let fh = resources.fetch_mut::<Filesystem>().open(&path)?;
                let tiled = xml_parser::parse_tileset(fh, 1)?;
                Ok(TileSheet::from_tiled(&tiled)?.into())
            } //_ => bail!("can only load from logical"),
        }
    }
}

impl<LayerProps, TileProps> Asset for TiledMap<LayerProps, TileProps>
where
    LayerProps: DeserializeOwned + Send + Sync + 'static,
    TileProps: DeserializeOwned + Send + Sync + 'static,
{
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<'static, Self>> {
        match key {
            Key::Path(path) => {
                let tiled =
                    xml_parser::parse_file(&mut *resources.fetch_mut::<Filesystem>(), &path)?;

                let mut deps = vec![];
                for ts in tiled.tilesets.iter() {
                    if let Some(src) = ts.source.as_ref() {
                        deps.push(Key::from_path(Path::new(src)).clone_static());
                    }
                }

                Ok(Loaded::with_deps(Self::from_tiled(&path, &tiled)?, deps))
            } //_ => bail!("can only load from path"),
        }
    }
}

pub struct TiledMapAccessor<L: LayerProperties, T: TileProperties>(Entity, PhantomData<(L, T)>);

impl<L: LayerProperties, T: TileProperties> LuaUserData for TiledMapAccessor<L, T> {}

impl<L: LayerProperties + DeserializeOwned, T: TileProperties + DeserializeOwned>
    LuaComponentInterface for TiledMap<L, T>
{
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        TiledMapAccessor::<L, T>(entity, PhantomData).to_lua(lua)
    }

    fn bundler<'lua>(
        lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let map_path = String::from_lua(args, lua)?;
        let resources = lua.resources();

        // FIXME(sleffy): type parameter for determining the cache type to load from.
        let mut tiled_map = resources
            .fetch_mut::<crate::assets::DefaultCache>()
            .get::<TiledMap<L, T>>(&Key::from_path(&map_path))
            .log_error_err(module_path!())
            .to_lua_err()?;

        builder.add::<TiledMap<L, T>>(tiled_map.load_cached().clone());
        Ok(())
    }
}
