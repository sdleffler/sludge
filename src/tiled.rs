use {
    anyhow::*,
    hashbrown::HashMap,
    nalgebra as na,
    serde::{Deserialize, Serialize},
    std::{
        fs::File,
        path::{Path, PathBuf},
    },
    tiled::LayerData,
    warmy::{Load, Loaded, SimpleKey, Storage},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Frame {
    local_id: u32,
    duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileData {
    local_id: u32,
    frames: Vec<Frame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileSheet {
    name: String,

    first_global_id: u32,

    /// Path to the image file.
    source: PathBuf,

    sheet_width: u32,
    sheet_height: u32,

    tile_width: u32,
    tile_height: u32,

    margin: u32,
    spacing: u32,

    tile_data: Vec<TileData>,

    tile_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TileSheetRegion {
    pub global_id: u32,
    pub local_id: u32,

    pub origin: na::Point2<u32>,
    pub extents: na::Vector2<u32>,
}

impl TileSheet {
    fn from_tiled(tiled: &tiled::Tileset) -> Result<Self> {
        ensure!(
            tiled.images.len() == 1,
            "tileset must have exactly one image"
        );
        let image = &tiled.images[0];

        let tile_data = tiled
            .tiles
            .iter()
            .map(|tile| TileData {
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
            })
            .collect();

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileLayer {
    pub name: Option<String>,
    pub opacity: f32,
    pub visible: bool,
    pub chunks: HashMap<(i32, i32), Chunk>,
}

impl Default for TileLayer {
    fn default() -> Self {
        Self {
            name: None,
            opacity: 1.0,
            visible: true,
            chunks: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Layer {
    TileLayer(TileLayer),
}

#[derive(Debug, Clone)]
pub struct Map {
    source: PathBuf,

    width: u32,
    height: u32,

    tile_width: u32,
    tile_height: u32,

    tile_sheets: Vec<TileSheet>,
    layers: Vec<Layer>,
}

impl Map {
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn tile_dimensions(&self) -> (u32, u32) {
        (self.tile_width, self.tile_height)
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn tile_sheets(&self) -> &[TileSheet] {
        &self.tile_sheets
    }
}

impl<C> Load<C, SimpleKey> for TileSheet {
    type Error = Error;

    fn load(
        key: SimpleKey,
        _storage: &mut Storage<C, SimpleKey>,
        _: &mut C,
    ) -> Result<Loaded<Self, SimpleKey>> {
        match key {
            SimpleKey::Path(path) => {
                let fh = File::open(&path)?;
                let tiled = tiled::parse_tileset(fh, 1)?;
                Ok(TileSheet::from_tiled(&tiled)?.into())
            }
            SimpleKey::Logical(_) => bail!("cannot load from logical"),
        }
    }
}

impl<C> Load<C, SimpleKey> for Map {
    type Error = Error;

    fn load(
        key: SimpleKey,
        _storage: &mut Storage<C, SimpleKey>,
        _ctx: &mut C,
    ) -> Result<Loaded<Self, SimpleKey>> {
        match key {
            SimpleKey::Path(path) => {
                let tiled = tiled::parse_file(&path)?;

                let mut deps = vec![];
                let tile_sheets = tiled
                    .tilesets
                    .iter()
                    .map(|ts| {
                        if let Some(src) = ts.source.as_ref() {
                            deps.push(SimpleKey::from_path(Path::new(src)));
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
            }
            SimpleKey::Logical(_) => bail!("cannot load from logical"),
        }
    }
}
