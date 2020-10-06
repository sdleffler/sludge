use crate::{math::*, tiled::Map};
use {
    hashbrown::HashMap,
    std::{num::NonZeroU32, ops},
};

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct TileId(NonZeroU32);

impl TileId {
    fn new(gid: u32) -> Self {
        Self(NonZeroU32::new(gid).expect("expected nonzero GID!"))
    }

    fn get(&self) -> usize {
        (self.0.get() - 1) as usize
    }
}

#[derive(Debug)]
pub struct Chunk {
    tiles: Vec<Option<TileId>>,
}

#[derive(Debug)]
pub struct TileGrid<T = ()> {
    tile_size: f32,
    chunk_size: u16,
    tiles: Vec<T>,
    chunks: HashMap<(i32, i32), Chunk>,
}

impl<T> ops::Index<(i32, i32)> for TileGrid<T> {
    type Output = Option<TileId>;

    fn index(&self, (x, y): (i32, i32)) -> &Self::Output {
        let chunk_size = self.chunk_size as i32;

        let chunk_index = (x.div_euclid(chunk_size), y.div_euclid(chunk_size));

        let chunk = &self.chunks[&chunk_index];

        let (lx, ly) = (x.rem_euclid(chunk_size), y.rem_euclid(chunk_size));
        let tile_index = (lx + ly * chunk_size) as usize;

        &chunk.tiles[tile_index]
    }
}

impl<T> ops::Index<TileId> for TileGrid<T> {
    type Output = T;

    fn index(&self, index: TileId) -> &Self::Output {
        &self.tiles[index.get()]
    }
}

impl<T> TileGrid<T> {
    pub fn new(tile_size: f32, chunk_size: u16) -> Self {
        Self {
            tile_size,
            chunk_size,
            tiles: Vec::new(),
            chunks: HashMap::new(),
        }
    }

    pub fn from_map<L>(map: &Map<L, T>) -> Self
    where
        T: Clone + Default,
    {
        let tile_size = {
            let (w, h) = map.tile_dimensions();
            assert_eq!(w, h, "non-square tiles?? uh oh");
            w as f32
        };
        let chunk_size = 16;

        let mut tiles = Vec::<T>::new();
        let mut chunks = HashMap::new();

        for sheet in map.tile_sheets() {
            if sheet.first_global_id() as usize > tiles.len() {
                tiles.resize_with(sheet.last_global_id() as usize, Default::default);
            }

            for (gid, tile_data) in sheet.iter_tile_data() {
                let id = TileId::new(gid);
                tiles[id.get()].clone_from(&tile_data.properties);
            }
        }

        Self {
            tile_size,
            chunk_size,
            tiles,
            chunks,
        }
    }

    pub fn get_potential_collisions<'a>(
        &'a self,
        aabb: &AABB<f32>,
    ) -> impl Iterator<Item = ((i32, i32), TileId)> + 'a {
        grid_2d::to_grid_indices(self.tile_size, aabb)
            .filter_map(move |(x, y)| self[(x, y)].map(|id| ((x, y), id)))
    }
}
