use crate::math::*;
use {
    hashbrown::{hash_map::Entry, HashMap},
    hibitset::{BitSet, BitSetLike},
    std::{iter, ops},
};

pub const DEFAULT_CHUNK_SIZE: u16 = 64;

fn to_grid_indices(grid_size: f32, aabb: &Box2<f32>) -> impl Iterator<Item = (i32, i32)> + Clone {
    let x_start = (aabb.mins.x / grid_size).floor() as i32;
    let x_end = (aabb.maxs.x / grid_size).ceil() as i32;
    let y_start = (aabb.mins.y / grid_size).floor() as i32;
    let y_end = (aabb.maxs.y / grid_size).ceil() as i32;

    (x_start..x_end).flat_map(move |i| (y_start..y_end).map(move |j| (i, j)))
}

fn to_grid_chunks(
    grid_size: f32,
    chunk_size: u16,
    aabb: &Box2<f32>,
) -> impl Iterator<Item = (i32, i32)> + Clone {
    let chunk_size = chunk_size as i32;
    let x_start = ((aabb.mins.x / grid_size).floor() as i32).div_euclid(chunk_size);
    let x_end = ((aabb.maxs.x / grid_size).ceil() as i32).div_euclid(chunk_size);
    let y_start = ((aabb.mins.y / grid_size).floor() as i32).div_euclid(chunk_size);
    let y_end = ((aabb.maxs.y / grid_size).ceil() as i32).div_euclid(chunk_size);

    (x_start..x_end).flat_map(move |i| (y_start..y_end).map(move |j| (i, j)))
}

fn to_chunk_and_subindices(chunk_size: u16, (x, y): (i32, i32)) -> ((i32, i32), usize) {
    let chunk_size = chunk_size as i32;
    let chunk_index = (x.div_euclid(chunk_size), y.div_euclid(chunk_size));
    let (lx, ly) = (x.rem_euclid(chunk_size), y.rem_euclid(chunk_size));
    let tile_index = (lx + ly * chunk_size) as usize;
    (chunk_index, tile_index)
}

fn from_chunk_and_subindices(
    chunk_size: u16,
    (chunk_x, chunk_y): (i32, i32),
    subindex: u32,
) -> (i32, i32) {
    let (sub_x, sub_y) = (
        (subindex as i32).rem_euclid(chunk_size as i32),
        (subindex as i32).div_euclid(chunk_size as i32),
    );

    (
        chunk_x * (chunk_size as i32) + sub_x,
        chunk_y * (chunk_size as i32) + sub_y,
    )
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

    /// Clear the bitgrid without losing any allocated chunk
    /// memory.
    pub fn clear(&mut self) {
        for chunk in self.chunks.values_mut() {
            chunk.clear();
        }
    }

    /// Remove any empty allocated chunks.
    pub fn sweep_chunks(&mut self) {
        self.chunks.retain(|_, chunk| !chunk.is_empty());
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
        to_grid_indices(self.scale, aabb).filter(move |&c| self.get(c))
    }

    pub fn bounds_at(&self, (x, y): (i32, i32)) -> Box2<f32> {
        let mins = Point2::new(x as f32 * self.scale, y as f32 * self.scale);
        let maxs = mins + Vector2::repeat(self.scale);
        Box2::from_corners(mins, maxs)
    }

    pub fn iter(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let chunk_size = self.chunk_size;
        self.chunks.iter().flat_map(move |(&chunk_coords, chunk)| {
            chunk
                .iter()
                .map(move |subindex| from_chunk_and_subindices(chunk_size, chunk_coords, subindex))
        })
    }

    pub fn union_region<'a>(
        &mut self,
        others: impl IntoIterator<Item = &'a ChunkedBitGrid>,
        aabb: &Box2<f32>,
    ) {
        let affected_chunks = to_grid_chunks(self.scale, self.chunk_size, aabb);
        for other_grid in others {
            for coord in affected_chunks.clone() {
                if let Some(other_chunk) = other_grid.chunks.get(&coord) {
                    match self.chunks.entry(coord) {
                        Entry::Occupied(mut chunk) => {
                            *chunk.get_mut() |= other_chunk;
                        }
                        Entry::Vacant(empty) => {
                            empty.insert(other_chunk.clone());
                        }
                    }
                }
            }
        }
    }
}

impl<'a> ops::BitOrAssign<&'a ChunkedBitGrid> for ChunkedBitGrid {
    fn bitor_assign(&mut self, other: &'a ChunkedBitGrid) {
        assert_eq!(self.chunk_size, other.chunk_size);
        for (&coord, other_chunk) in other.chunks.iter() {
            match self.chunks.entry(coord) {
                Entry::Occupied(mut chunk) => {
                    *chunk.get_mut() |= other_chunk;
                }
                Entry::Vacant(empty) => {
                    empty.insert(other_chunk.clone());
                }
            }
        }
    }
}
