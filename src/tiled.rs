use {
    anyhow::*,
    hashbrown::HashMap,
    nalgebra as na,
    serde::{de::DeserializeOwned, Deserialize, Serialize},
    std::path::{Path, PathBuf},
};

use crate::{
    filesystem::Filesystem,
    resources::{Inspect, Key, Load, Loaded, Storage},
    tiled::xml_parser::LayerData,
    SharedResources,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Frame {
    pub local_id: u32,
    pub duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "TileProperties: DeserializeOwned"))]
pub struct TileData<TileProperties = ron::Value> {
    pub tile_type: Option<String>,
    pub local_id: u32,
    pub frames: Vec<Frame>,

    #[serde(bound(deserialize = "TileProperties: DeserializeOwned"))]
    pub properties: TileProperties,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "TileProperties: DeserializeOwned"))]
pub struct TileSheet<TileProperties = ron::Map> {
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
    #[serde(bound(deserialize = "TileProperties: DeserializeOwned"))]
    tile_data: HashMap<u32, TileData<TileProperties>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TileSheetRegion {
    pub global_id: u32,
    pub local_id: u32,

    pub origin: na::Point2<u32>,
    pub extents: na::Vector2<u32>,
}

impl<TileProperties> TileSheet<TileProperties> {
    fn from_tiled(tiled: &xml_parser::Tileset) -> Result<Self>
    where
        TileProperties: DeserializeOwned,
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
                    frames: tile
                        .animation
                        .as_ref()
                        .into_iter()
                        .flatten()
                        .map(|frame| Frame {
                            local_id: frame.tile_id,
                            duration: frame.duration,
                        })
                        .collect(),
                    properties: deserialize_properties(&tile.properties)?,
                };

                Ok((tile.id, tile_data))
            })
            .collect::<Result<_>>()?;

        Ok(TileSheet {
            name: tiled.name.clone(),

            first_global_id: tiled.first_gid,

            source: PathBuf::from(&image.source),

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

    pub fn iter_regions(&self) -> impl Iterator<Item = TileSheetRegion> + '_ {
        let origin = na::Point2::origin() + na::Vector2::repeat(self.margin);
        let extent = na::Vector2::new(self.tile_width, self.tile_height);
        let stride = na::Vector2::repeat(self.spacing) + extent;

        let columns = (self.sheet_width - self.margin) / (self.tile_width + self.spacing);

        (0..self.tile_count).map(move |local_id| {
            let coord = na::Vector2::new(local_id % columns, local_id / columns);
            let corner = origin + stride.component_mul(&coord);

            TileSheetRegion {
                global_id: self.first_global_id + local_id,
                local_id: local_id,
                origin: corner,
                extents: extent,
            }
        })
    }

    pub fn iter_tile_data(&self) -> impl Iterator<Item = (u32, &TileData<TileProperties>)> + '_ {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "LayerProperties: DeserializeOwned"))]
pub struct TileLayer<LayerProperties = ron::Value> {
    pub name: Option<String>,
    pub opacity: f32,
    pub visible: bool,
    pub chunks: HashMap<(i32, i32), Chunk>,

    #[serde(bound(deserialize = "LayerProperties: DeserializeOwned"))]
    pub properties: LayerProperties,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "LayerProperties: DeserializeOwned"))]
pub enum Layer<LayerProperties = ron::Value> {
    #[serde(bound(deserialize = "LayerProperties: DeserializeOwned"))]
    TileLayer(TileLayer<LayerProperties>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    deserialize = "LayerProperties: DeserializeOwned, TileProperties: DeserializeOwned"
))]
pub struct Map<LayerProperties = ron::Value, TileProperties = ron::Value> {
    source: PathBuf,

    width: u32,
    height: u32,

    tile_width: u32,
    tile_height: u32,

    tile_sheets: Vec<TileSheet<TileProperties>>,
    layers: Vec<Layer<LayerProperties>>,
}

impl<LayerProperties, TileProperties> Map<LayerProperties, TileProperties> {
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn tile_dimensions(&self) -> (u32, u32) {
        (self.tile_width, self.tile_height)
    }

    pub fn layers(&self) -> &[Layer<LayerProperties>] {
        &self.layers
    }

    pub fn tile_sheets(&self) -> &[TileSheet<TileProperties>] {
        &self.tile_sheets
    }

    pub fn get_tile_sheet_for_gid(&self, gid: u32) -> Option<&TileSheet<TileProperties>> {
        self.tile_sheets
            .iter()
            .find(|ts| ts.first_global_id <= gid && gid <= ts.last_global_id())
    }
}

impl<C, TileProperties> Load<C, Key> for TileSheet<TileProperties>
where
    Self: for<'a> Inspect<'a, C, &'a SharedResources>,
    TileProperties: DeserializeOwned + 'static,
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

impl<C, LayerProperties, TileProperties> Load<C, Key> for Map<LayerProperties, TileProperties>
where
    Self: for<'a> Inspect<'a, C, &'a SharedResources>,
    LayerProperties: DeserializeOwned + 'static,
    TileProperties: DeserializeOwned + 'static,
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
                let tile_sheets = tiled
                    .tilesets
                    .iter()
                    .map(|ts| {
                        if let Some(src) = ts.source.as_ref() {
                            deps.push(Key::from_path(Path::new(src)));
                        }
                        Ok(TileSheet::from_tiled(ts)?)
                    })
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
                let map = Map {
                    source: path,

                    width: tiled.width,
                    height: tiled.height,

                    tile_width: tiled.tile_width,
                    tile_height: tiled.tile_height,

                    tile_sheets,
                    layers: layers.into_iter().map(|(_, v)| v).collect(),
                };

                Ok(Loaded::with_deps(map, deps))
            } //_ => bail!("can only load from path"),
        }
    }
}
