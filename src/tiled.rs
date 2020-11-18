use {
    anyhow::*,
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    serde::{de::DeserializeOwned, Deserialize, Serialize},
    serde_json::{json, Number, Value},
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

fn unwrap_object(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("not a map!"),
    }
}

fn deserialize_properties<T: Properties>(
    properties: &xml_parser::Properties,
    initial: Option<serde_json::Map<String, Value>>,
) -> Result<T> {
    use xml_parser::PropertyValue::*;

    let mut json_map = initial.unwrap_or_default();

    for (k, v) in properties {
        let key = k.to_owned();
        let value = match v {
            BoolValue(b) => Value::Bool(*b),
            FloatValue(f) => Value::Number(Number::from_f64(*f as f64).unwrap()),
            IntValue(i) => Value::Number((*i).into()),
            ColorValue(_) => bail!("Color property values not yet supported!"),
            StringValue(s) => Value::String(s.to_owned()),
        };

        json_map.insert(key, value);
    }

    serde_json::from_value(Value::Object(json_map)).map_err(Error::from)
}

pub trait Properties: DeserializeOwned + Serialize + Send + Sync + Clone + 'static {}
impl<T> Properties for T where T: DeserializeOwned + Serialize + Send + Sync + Clone + 'static {}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Frame {
    pub local_id: u32,
    pub duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Properties"))]
pub struct TileData<T> {
    pub tile_type: Option<String>,
    pub local_id: u32,
    pub frames: Option<Vec<Frame>>,

    #[serde(bound(deserialize = "T: Properties"))]
    pub properties: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Properties"))]
pub struct TileSheet<T> {
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
    #[serde(bound(deserialize = "T: Properties"))]
    tile_data: HashMap<u32, TileData<T>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TileSheetRegion {
    pub global_id: u32,
    pub local_id: u32,

    pub bounds: Box2<u32>,
    pub uv: Box2<f32>,
}

impl<T> TileSheet<T> {
    fn from_tiled(tiled: &xml_parser::Tileset) -> Result<Self>
    where
        T: Properties,
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
                    properties: deserialize_properties(
                        &tile.properties,
                        Some(unwrap_object(json!({
                            "type": tile.tile_type,
                        }))),
                    )?,
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

    pub fn get_tile_data_from_local_id(&self, local_id: u32) -> Option<&TileData<T>> {
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

    pub fn iter_tile_data(&self) -> impl Iterator<Item = (u32, &TileData<T>)> + '_ {
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
#[serde(bound(deserialize = "L: Properties"))]
pub struct TileLayer<L> {
    pub name: Option<String>,
    pub opacity: f32,
    pub visible: bool,
    pub chunks: HashMap<(i32, i32), Chunk>,
    pub properties: L,
}

pub type DefaultTileLayer = TileLayer<Value>;

impl Default for DefaultTileLayer {
    fn default() -> Self {
        Self {
            name: None,
            opacity: 1.0,
            visible: true,
            chunks: HashMap::new(),
            properties: Value::Object(Default::default()),
        }
    }
}

impl<L> TileLayer<L> {
    pub fn chunks(&self) -> impl Iterator<Item = ((i32, i32), &Chunk)> + '_ {
        self.chunks.iter().map(|(&k, v)| (k, v))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "L: Properties"))]
pub struct ImageLayer<L> {
    pub name: Option<String>,
    pub opacity: f32,
    pub visible: bool,
    pub offset_x: f32,
    pub offset_y: f32,
    pub source: PathBuf,
    pub image_width: u32,
    pub image_height: u32,
    pub properties: L,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObjectShape {
    Rect { width: f32, height: f32 },
    Polygon { points: Vec<(f32, f32)> },
    Point(f32, f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "O: Properties"))]
pub struct Object<O> {
    pub id: u32,
    pub gid: u32,
    pub name: String,
    pub object_type: String,
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
    pub rot: f32,
    pub visible: bool,
    pub shape: ObjectShape,
    pub properties: O,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "L: Properties, O: Properties"))]
pub struct ObjectLayer<L, O> {
    pub name: String,
    pub opacity: f32,
    pub visible: bool,
    pub objects: Vec<Object<O>>,
    pub properties: L,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "L: Properties, O: Properties"))]
pub enum Layer<L, O> {
    TileLayer(TileLayer<L>),
    ImageLayer(ImageLayer<L>),
    ObjectLayer(ObjectLayer<L, O>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "L: Properties, T: Properties, O: Properties"))]
pub struct TiledMap<L, T, O> {
    source: PathBuf,

    width: u32,
    height: u32,

    tile_width: u32,
    tile_height: u32,

    tile_sheets: Vec<TileSheet<T>>,
    layers: Vec<Layer<L, O>>,
}

impl<'a, L: Component, T: Component, O: Component> SmartComponent<ScContext<'a>>
    for TiledMap<L, T, O>
{
}

impl<L, T, O> TiledMap<L, T, O> {
    pub fn from_tiled(path: &Path, tiled: &xml_parser::Map) -> Result<Self>
    where
        L: Properties,
        T: Properties,
        O: Properties,
    {
        let tile_sheets = tiled
            .tilesets
            .iter()
            .map(|ts| TileSheet::from_tiled(ts))
            .collect::<Result<_>>()?;

        let mut layers = Vec::new();

        for layer in tiled.layers.iter() {
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

            let tile_layer = Layer::TileLayer(TileLayer {
                name: Some(layer.name.clone()),
                visible: layer.visible,
                opacity: layer.opacity,
                chunks,
                properties: deserialize_properties(
                    &layer.properties,
                    Some(unwrap_object(json!({
                        "name": layer.name,
                        "type": "Tile",
                    }))),
                )
                .with_context(|| {
                    anyhow!(
                        "error deserializing properties for tile layer `{}`",
                        layer.name
                    )
                })?,
            });

            layers.push((layer.layer_index, tile_layer));
        }

        for layer in tiled.image_layers.iter() {
            let image = layer.image.as_ref().unwrap();
            let image_layer = Layer::ImageLayer(ImageLayer {
                name: Some(layer.name.clone()),
                visible: layer.visible,
                opacity: layer.opacity,
                offset_x: layer.offset_x,
                offset_y: layer.offset_y,
                source: image.source.clone(),
                image_width: image.width as u32,
                image_height: image.height as u32,
                properties: deserialize_properties(
                    &layer.properties,
                    Some(unwrap_object(json!({
                        "type": "Image",
                        "name": layer.name,
                    }))),
                )
                .with_context(|| {
                    anyhow!(
                        "error deserializing layer properties for image layer `{}`",
                        layer.name
                    )
                })?,
            });

            layers.push((layer.layer_index, image_layer));
        }

        for layer in tiled.object_groups.iter() {
            let mut objects = Vec::new();
            for object in layer.objects.iter() {
                use xml_parser::ObjectShape::*;
                let shape = match &object.shape {
                    Rect { width, height } => ObjectShape::Rect {
                        width: *width,
                        height: *height,
                    },
                    Polygon { points } => ObjectShape::Polygon {
                        points: points.clone(),
                    },
                    Point(x, y) => ObjectShape::Point(*x, *y),
                    _ => bail!(
                        "unsupported shape type for object named `{}` (id {})",
                        object.name,
                        object.id
                    ),
                };

                let object = Object {
                    id: object.id,
                    gid: object.gid,
                    name: object.name.clone(),
                    object_type: object.obj_type.clone(),
                    width: object.width,
                    height: object.height,
                    x: object.x,
                    y: object.y,
                    rot: object.rotation,
                    visible: object.visible,
                    shape,
                    properties: deserialize_properties(&object.properties, Some(unwrap_object(json!({
                        "type": object.obj_type,
                        "name": object.name,
                    })))).with_context(|| {
                        anyhow!(
                            "error deserializing object properties for object `{}` (id #{}) from layer `{}`",
                            object.name,
                            object.id,
                            layer.name
                        )
                    })?,
                };

                objects.push(object);
            }

            let object_layer = Layer::ObjectLayer(ObjectLayer {
                name: layer.name.clone(),
                opacity: layer.opacity,
                visible: layer.visible,
                objects,
                properties: deserialize_properties(
                    &layer.properties,
                    Some(unwrap_object(json!({
                        "type": "Object",
                        "name": layer.name,
                    }))),
                )
                .with_context(|| {
                    anyhow!(
                        "error deserializing layer properties for object layer `{}`",
                        layer.name
                    )
                })?,
            });

            layers.push((layer.layer_index.unwrap_or(0), object_layer));
        }

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

    pub fn layers(&self) -> &[Layer<L, O>] {
        &self.layers
    }

    pub fn tile_sheets(&self) -> &[TileSheet<T>] {
        &self.tile_sheets
    }

    pub fn get_tile_sheet_for_gid(&self, gid: u32) -> Option<&TileSheet<T>> {
        self.tile_sheets
            .iter()
            .find(|ts| ts.first_global_id <= gid && gid <= ts.last_global_id())
    }

    pub fn get_tile_data_for_gid(&self, gid: u32) -> Option<&TileData<T>> {
        self.get_tile_sheet_for_gid(gid)
            .and_then(|ts| ts.tile_data.get(&(gid - ts.first_global_id)))
    }
}

impl<T> Asset for TileSheet<T>
where
    T: Properties,
{
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>> {
        let path = key.to_path()?;
        let fh = resources.fetch_mut::<Filesystem>().open(&path)?;
        let tiled = xml_parser::parse_tileset(fh, 1)?;
        Ok(TileSheet::from_tiled(&tiled)?.into())
    }
}

impl<L, T, O> Asset for TiledMap<L, T, O>
where
    L: Properties,
    T: Properties,
    O: Properties,
{
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>> {
        let path = key.to_path()?;
        let tiled = xml_parser::parse_file(&mut *resources.fetch_mut::<Filesystem>(), &path)?;

        let mut deps = vec![];
        for ts in tiled.tilesets.iter() {
            if let Some(src) = ts.source.as_ref() {
                deps.push(Key::from_path(Path::new(src)).clone_static());
            }
        }

        for ls in tiled.image_layers.iter() {
            if let Some(img) = &ls.image {
                deps.push(Key::from_path(Path::new(&img.source)).clone_static());
            }
        }

        let tiled_map = Self::from_tiled(&path, &tiled).with_context(|| {
            anyhow!(
                "error deserializing `{}` at path `{}`",
                ::std::any::type_name::<TiledMap<L, T, O>>(),
                path.display()
            )
        })?;

        Ok(Loaded::with_deps(tiled_map, deps))
    }
}

pub struct TiledMapAccessor<L, T, O>(Entity, PhantomData<(L, T, O)>)
where
    L: Properties,
    T: Properties,
    O: Properties;

impl<L, T, O> LuaUserData for TiledMapAccessor<L, T, O>
where
    L: Properties,
    T: Properties,
    O: Properties,
{
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("layers", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let map = world.get::<TiledMap<L, T, O>>(this.0.into()).to_lua_err()?;
            rlua_serde::to_value(lua, &map.layers)
        });
    }
}

impl<L, T, O> LuaComponentInterface for TiledMap<L, T, O>
where
    L: Properties,
    T: Properties,
    O: Properties,
{
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        TiledMapAccessor::<L, T, O>(entity, PhantomData).to_lua(lua)
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
            .get::<TiledMap<L, T, O>>(&Key::from_path(&map_path))
            .with_context(|| {
                anyhow!(
                    "error loading {} from path {} while inserting into bundle from Lua",
                    std::any::type_name::<TiledMap<L, T, O>>(),
                    map_path
                )
            })
            .log_error_err(module_path!())
            .to_lua_err()?;

        builder.add::<TiledMap<L, T, O>>(tiled_map.load_cached().clone());
        Ok(())
    }
}
