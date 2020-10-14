use crate::{
    ecs::*,
    math::*,
    prelude::*,
    spatial_2d::{Position, Shape},
};
use {
    hashbrown::{HashMap, HashSet},
    shrev::ReaderId,
    smallvec::SmallVec,
    std::ops,
    thunderdome::{Arena, Index},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpatialIndex(Index);

impl<'a> SmartComponent<ScContext<'a>> for SpatialIndex {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BucketIndex(Index);

#[derive(Debug)]
pub struct Bucket {
    bounds: Box2<f32>,
    members: Vec<SpatialIndex>,
}

impl Bucket {
    fn new(bounds: Box2<f32>) -> Self {
        Self {
            bounds,
            members: Vec::new(),
        }
    }

    pub fn bounds(&self) -> &Box2<f32> {
        &self.bounds
    }

    pub fn members(&self) -> &[SpatialIndex] {
        &self.members
    }
}

#[derive(Debug)]
pub struct ObjectEntry<T> {
    bounds: Box2<f32>,
    buckets: SmallVec<[BucketIndex; 4]>,
    userdata: T,
}

impl<T> ObjectEntry<T> {
    pub fn bounds(&self) -> &Box2<f32> {
        &self.bounds
    }

    pub fn userdata(&self) -> &T {
        &self.userdata
    }

    pub fn userdata_mut(&mut self) -> &mut T {
        &mut self.userdata
    }
}

#[derive(Debug)]
pub struct HashGrid<T> {
    bucket_size: f32,
    spatial_map: HashMap<(i32, i32), BucketIndex>,
    buckets: Arena<Bucket>,
    objects: Arena<ObjectEntry<T>>,
}

impl<T> ops::Index<BucketIndex> for HashGrid<T> {
    type Output = Bucket;

    fn index(&self, index: BucketIndex) -> &Self::Output {
        &self.buckets[index.0]
    }
}

impl<T> ops::Index<SpatialIndex> for HashGrid<T> {
    type Output = ObjectEntry<T>;

    fn index(&self, index: SpatialIndex) -> &Self::Output {
        &self.objects[index.0]
    }
}

impl<T> ops::IndexMut<SpatialIndex> for HashGrid<T> {
    fn index_mut(&mut self, index: SpatialIndex) -> &mut Self::Output {
        &mut self.objects[index.0]
    }
}

impl<T> HashGrid<T> {
    pub fn new(bucket_size: f32) -> Self {
        Self {
            bucket_size,
            spatial_map: HashMap::new(),
            buckets: Arena::new(),
            objects: Arena::new(),
        }
    }
}

fn get_or_insert_bucket(
    bucket_size: f32,
    spatial_map: &mut HashMap<(i32, i32), BucketIndex>,
    buckets: &mut Arena<Bucket>,
    (i, j): (i32, i32),
) -> BucketIndex {
    *spatial_map.entry((i, j)).or_insert_with(|| {
        let mins = Point2::new(i as f32 * bucket_size, j as f32 * bucket_size);
        let maxs = mins + Vector2::repeat(bucket_size);
        BucketIndex(buckets.insert(Bucket::new(Box2::from_corners(mins, maxs))))
    })
}

fn to_spatial_indices(bucket_size: f32, aabb: Box2<f32>) -> impl Iterator<Item = (i32, i32)> {
    let x_start = (aabb.mins.x / bucket_size).floor() as i32;
    let x_end = (aabb.maxs.x / bucket_size).ceil() as i32;
    let y_start = (aabb.mins.y / bucket_size).floor() as i32;
    let y_end = (aabb.maxs.y / bucket_size).ceil() as i32;

    (x_start..x_end).flat_map(move |i| (y_start..y_end).map(move |j| (i, j)))
}

impl<T> HashGrid<T> {
    pub fn insert(&mut self, aabb: impl Into<Box2<f32>>, userdata: T) -> SpatialIndex {
        let aabb = aabb.into();
        let object_id = SpatialIndex(self.objects.insert(ObjectEntry {
            bounds: aabb,
            buckets: SmallVec::new(),
            userdata,
        }));
        let mut buckets = SmallVec::new();

        for coords in to_spatial_indices(self.bucket_size, aabb) {
            let bucket_id = get_or_insert_bucket(
                self.bucket_size,
                &mut self.spatial_map,
                &mut self.buckets,
                coords,
            );
            let members = &mut self.buckets[bucket_id.0].members;
            let index = members.binary_search(&object_id).unwrap_or_else(|x| x);
            members.insert(index, object_id);
            buckets.push(bucket_id);
        }

        self.objects[object_id.0].buckets = buckets;
        object_id
    }

    pub fn remove(&mut self, object: SpatialIndex) {
        for &bucket_id in self.objects[object.0].buckets.iter() {
            let members = &mut self.buckets[bucket_id.0].members;
            let index = members.binary_search(&object).unwrap_or_else(|x| x);
            members.remove(index);
        }

        self.objects.remove(object.0);
    }

    /// Update the object's state in the hash grid, removing it from buckets it no
    /// longer inhabits and add it to buckets it newly inhabits. Returns true if
    /// the object has been removed from/added to a new bucket.
    pub fn update(&mut self, object_id: SpatialIndex, aabb: impl Into<Box2<f32>>) -> bool {
        let object = &mut self.objects[object_id.0];
        let aabb = aabb.into();

        // TODO: fudge value to avoid recomputing for small movements?
        if aabb == object.bounds {
            return false;
        }

        object.bounds = aabb;

        let mut dirty = false;

        for &bucket_id in object.buckets.iter() {
            let bucket = &mut self.buckets[bucket_id.0];
            if !bucket.bounds.intersects(&object.bounds) {
                if let Ok(idx) = bucket.members.binary_search(&object_id) {
                    bucket.members.remove(idx);
                    dirty = true;
                }
            }
        }

        for coords in to_spatial_indices(self.bucket_size, aabb) {
            let bucket_id = get_or_insert_bucket(
                self.bucket_size,
                &mut self.spatial_map,
                &mut self.buckets,
                coords,
            );
            let members = &mut self.buckets[bucket_id.0].members;
            if let Err(index) = members.binary_search(&object_id) {
                members.insert(index, object_id);
                object.buckets.push(bucket_id);
                dirty = true;
            }
        }

        dirty
    }

    pub fn buckets(&self) -> impl Iterator<Item = (BucketIndex, &Bucket)> + '_ {
        self.buckets.iter().map(|(i, b)| (BucketIndex(i), b))
    }

    pub fn query<'a>(&'a self, aabb: &Box2<f32>) -> impl Iterator<Item = SpatialIndex> + 'a {
        to_spatial_indices(self.bucket_size, *aabb)
            .flat_map(move |coords| self.spatial_map.get(&coords).copied().into_iter())
            .flat_map(move |bucket_id| self.buckets[bucket_id.0].members.iter().copied())
    }
}

#[derive(Debug)]
pub struct SpatialHasher {
    position_events: ReaderId<ComponentEvent>,
    shape_events: ReaderId<ComponentEvent>,

    grid: HashGrid<Entity>,

    added: HashSet<Entity>,
    modified: HashSet<Entity>,
    removed: HashSet<Entity>,
}

impl SpatialHasher {
    pub fn new(bucket_size: f32, world: &mut World) -> Self {
        let position_events = world.track::<Position>();
        let shape_events = world.track::<Shape>();

        Self {
            position_events,
            shape_events,

            grid: HashGrid::new(bucket_size),

            added: HashSet::new(),
            modified: HashSet::new(),
            removed: HashSet::new(),
        }
    }

    pub fn grid(&self) -> &HashGrid<Entity> {
        &self.grid
    }

    pub fn update(&mut self, resources: &UnifiedResources) {
        self.added.clear();
        self.modified.clear();
        self.removed.clear();

        // TODO: command buffer queuing for asynchronous add/remove
        let world = &*resources.fetch::<World>();

        for &event in world.poll::<Position>(&mut self.position_events) {
            match event {
                ComponentEvent::Inserted(entity) => {
                    self.added.insert(entity);
                    self.removed.remove(&entity);
                }
                ComponentEvent::Modified(entity) => {
                    self.modified.insert(entity);
                }
                ComponentEvent::Removed(entity) => {
                    self.added.remove(&entity);
                    self.removed.insert(entity);
                }
            }
        }

        for &event in world.poll::<Shape>(&mut self.shape_events) {
            match event {
                ComponentEvent::Inserted(entity) => {
                    self.added.insert(entity);
                    self.removed.remove(&entity);
                }
                ComponentEvent::Modified(entity) => {
                    self.modified.insert(entity);
                }
                ComponentEvent::Removed(entity) => {
                    self.added.remove(&entity);
                    self.removed.insert(entity);
                }
            }
        }

        let mut cmds = world.get_buffer();

        for added in self.added.drain() {
            let mut query = world.query_one::<(&Position, &Shape)>(added).unwrap();
            if let Some((pos, shape)) = query.get() {
                let index = self.grid.insert(
                    nc2d::bounding_volume::aabb(&*shape.handle, &(**pos * shape.local)),
                    added,
                );
                cmds.insert(added, (index,));
            }
        }

        for (_, (pos, shape, index)) in world.query::<(&Position, &Shape, &SpatialIndex)>().iter() {
            self.grid.update(
                *index,
                nc2d::bounding_volume::aabb(&*shape.handle, &(**pos * shape.local)),
            );
        }

        for removed in self.removed.drain() {
            if let Ok(_index) = world.get::<SpatialIndex>(removed) {
                cmds.remove::<(SpatialIndex,)>(removed);
            }
        }

        world.queue_buffer(cmds);
    }
}

pub struct SpatialHashingSystem;

impl System for SpatialHashingSystem {
    fn init(
        &self,
        _lua: LuaContext,
        resources: &mut Resources,
        _: Option<&SharedResources>,
    ) -> Result<()> {
        if !resources.has_value::<SpatialHasher>() {
            let spatial_hasher = SpatialHasher::new(64., &mut *resources.fetch_mut::<World>());
            resources.insert(spatial_hasher);
        }

        let mut spatial_hasher = resources.fetch_mut::<SpatialHasher>();
        let mut world = resources.fetch_mut::<World>();
        let mut added_buf = Vec::new();
        for (e, (pos, shape)) in world.query::<(&Position, &Shape)>().iter() {
            let index = spatial_hasher.grid.insert(
                nc2d::bounding_volume::aabb(&*shape.handle, &(**pos * shape.local)),
                e,
            );
            added_buf.push((e, index));
        }

        for (entity, index) in added_buf {
            let _ = world.insert(entity, (index,));
        }

        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &UnifiedResources) -> Result<()> {
        let mut spatial_hasher = resources.fetch_mut::<SpatialHasher>();
        spatial_hasher.update(resources);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use {hashbrown::HashSet, std::hash::Hash};

    fn set_of<T: Hash + Eq>(iter: impl IntoIterator<Item = T>) -> HashSet<T> {
        iter.into_iter().collect()
    }

    #[test]
    fn spatial_indices() {
        assert_eq!(
            set_of(to_spatial_indices(
                64.,
                Box2::from_half_extents(Point2::new(23., 24.), Vector2::new(8., 8.))
            )),
            set_of(vec![(0, 0)])
        );

        assert_eq!(
            set_of(to_spatial_indices(
                64.,
                Box2::from_half_extents(Point2::new(-2., -3.), Vector2::new(4., 4.))
            )),
            set_of(vec![(0, 0), (-1, -1), (-1, 0), (0, -1)])
        );

        assert_eq!(
            set_of(to_spatial_indices(
                64.,
                Box2::from_half_extents(Point2::new(35., 35.), Vector2::new(36., 36.))
            )),
            set_of(vec![
                (0, 0),
                (-1, -1),
                (-1, 0),
                (0, -1),
                (1, 1),
                (1, 0),
                (0, 1),
                (-1, 1),
                (1, -1),
            ])
        );
    }

    #[test]
    fn spatial_hash_simple() {
        let mut spatial_hasher = HashGrid::new(64.);
        let mut bucket_count = 0;

        let a = spatial_hasher.insert(
            Box2::from_half_extents(Point2::new(23., 42.), Vector2::new(8., 8.)),
            "a",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );
        bucket_count = spatial_hasher.buckets.len();

        let b = spatial_hasher.insert(
            Box2::from_half_extents(Point2::new(-2., -3.), Vector2::new(4., 4.)),
            "b",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );
        bucket_count = spatial_hasher.buckets.len();

        let c = spatial_hasher.insert(
            Box2::from_half_extents(Point2::new(35., 35.), Vector2::new(36., 36.)),
            "c",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );
        bucket_count = spatial_hasher.buckets.len();

        let d = spatial_hasher.insert(
            Box2::from_half_extents(Point2::new(84., 20.), Vector2::new(8., 8.)),
            "d",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );

        assert_eq!(
            set_of(spatial_hasher.query(&Box2::from_half_extents(
                Point2::new(20., 40.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![a, b, c])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&Box2::from_half_extents(
                Point2::new(-13., 14.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![b, c])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&Box2::from_half_extents(
                Point2::new(96., 96.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&Box2::from_half_extents(
                Point2::new(96., 32.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c, d])
        );

        spatial_hasher.update(
            d,
            Box2::from_half_extents(Point2::new(45., 20.), Vector2::new(8., 8.)),
        );

        assert_eq!(
            set_of(spatial_hasher.query(&Box2::from_half_extents(
                Point2::new(96., 32.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c])
        );
    }
}
