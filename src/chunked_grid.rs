use crate::math::*;
use {hashbrown::HashMap, hibitset::BitSet, std::iter};

pub const DEFAULT_CHUNK_SIZE: u16 = 64;

fn to_chunk_and_subindices(chunk_size: u16, (x, y): (i32, i32)) -> ((i32, i32), usize) {
    let chunk_size = chunk_size as i32;
    let chunk_index = (x.div_euclid(chunk_size), y.div_euclid(chunk_size));
    let (lx, ly) = (x.rem_euclid(chunk_size), y.rem_euclid(chunk_size));
    let tile_index = (lx + ly * chunk_size) as usize;
    (chunk_index, tile_index)
}

#[derive(Debug, Clone)]
pub struct Chunk<T: Default> {
    elements: Vec<T>,
}

impl<T: Default> Chunk<T> {
    fn empty(chunk_size: u16) -> Self {
        Self {
            elements: iter::repeat_with(T::default)
                .take(chunk_size as usize)
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkedGrid<T: Default> {
    chunk_size: u16,
    chunks: HashMap<(i32, i32), Chunk<T>>,
}

impl<T: Default> ChunkedGrid<T> {
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            chunks: HashMap::new(),
        }
    }

    pub fn with_chunk_size(chunk_size: u16) -> Self {
        Self {
            chunk_size,
            ..Self::new()
        }
    }

    pub fn get(&self, (x, y): (i32, i32)) -> Option<&T> {
        let (chunk_indices, offset) = to_chunk_and_subindices(self.chunk_size, (x, y));
        self.chunks
            .get(&chunk_indices)
            .map(|chunk| &chunk.elements[offset])
    }

    pub fn get_mut(&mut self, (x, y): (i32, i32)) -> Option<&mut T> {
        let (chunk_indices, offset) = to_chunk_and_subindices(self.chunk_size, (x, y));
        self.chunks
            .get_mut(&chunk_indices)
            .map(|chunk| &mut chunk.elements[offset])
    }

    pub fn set(&mut self, (x, y): (i32, i32), value: T) {
        // Copy here to appease borrowck on closure.
        let chunk_size = self.chunk_size;
        let (chunk_indices, offset) = to_chunk_and_subindices(chunk_size, (x, y));
        let chunk = self
            .chunks
            .entry(chunk_indices)
            .or_insert_with(|| Chunk::empty(chunk_size));
        chunk.elements[offset] = value;
    }
}

#[derive(Debug, Clone)]
pub struct ChunkedBitGrid {
    scale: f32,
    chunk_size: u16,
    chunks: HashMap<(i32, i32), BitSet>,
}

impl ChunkedBitGrid {
    pub fn new(scale: f32) -> Self {
        Self {
            scale,
            chunk_size: DEFAULT_CHUNK_SIZE,
            chunks: HashMap::new(),
        }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn with_chunk_size(scale: f32, chunk_size: u16) -> Self {
        Self {
            chunk_size,
            ..Self::new(scale)
        }
    }

    pub fn get(&self, (x, y): (i32, i32)) -> bool {
        let (chunk_indices, offset) = to_chunk_and_subindices(self.chunk_size, (x, y));
        self.chunks
            .get(&chunk_indices)
            .map(|chunk| chunk.contains(offset as u32))
            .unwrap_or(false)
    }

    pub fn set(&mut self, (x, y): (i32, i32), value: bool) {
        // Copy here to appease borrowck on closure.
        let chunk_size = self.chunk_size;
        let (chunk_indices, offset) = to_chunk_and_subindices(chunk_size, (x, y));
        let chunk = self
            .chunks
            .entry(chunk_indices)
            .or_insert_with(|| BitSet::with_capacity((chunk_size as u32).pow(2)));

        if value {
            chunk.add(offset as u32);
        } else {
            chunk.remove(offset as u32);
        }
    }

    pub fn query<'a>(&'a self, aabb: &Box2<f32>) -> impl Iterator<Item = (i32, i32)> + 'a {
        coords_2d::to_grid_indices(self.scale, aabb).filter(move |&c| self.get(c))
    }

    pub fn bounds_at(&self, (x, y): (i32, i32)) -> Box2<f32> {
        let mins = Point2::new(x as f32 * self.scale, y as f32 * self.scale);
        let maxs = mins + Vector2::repeat(self.scale);
        Box2::from_corners(mins, maxs)
    }
}
