use crate::math::*;
use {
    hashbrown::HashMap,
    smallvec::SmallVec,
    std::ops,
    thunderdome::{Arena, Index},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BucketIndex(Index);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectIndex(Index);

#[derive(Debug)]
pub struct SpatialBucket {
    bounds: AABB<f32>,
    members: Vec<ObjectIndex>,
}

impl SpatialBucket {
    fn new(bounds: AABB<f32>) -> Self {
        Self {
            bounds,
            members: Vec::new(),
        }
    }

    pub fn bounds(&self) -> &AABB<f32> {
        &self.bounds
    }

    pub fn members(&self) -> &[ObjectIndex] {
        &self.members
    }
}

#[derive(Debug)]
pub struct ObjectEntry<T> {
    position: Point2<f32>,
    bounds: Cuboid<f32>,
    buckets: SmallVec<[BucketIndex; 4]>,
    userdata: T,
}

impl<T> ObjectEntry<T> {
    pub fn userdata(&self) -> &T {
        &self.userdata
    }

    pub fn userdata_mut(&mut self) -> &mut T {
        &mut self.userdata
    }
}

#[derive(Debug)]
pub struct SpatialHasher<T> {
    bucket_size: f32,
    spatial_map: HashMap<(i32, i32), BucketIndex>,
    buckets: Arena<SpatialBucket>,
    objects: Arena<ObjectEntry<T>>,
}

impl<T> ops::Index<BucketIndex> for SpatialHasher<T> {
    type Output = SpatialBucket;

    fn index(&self, index: BucketIndex) -> &Self::Output {
        &self.buckets[index.0]
    }
}

impl<T> ops::Index<ObjectIndex> for SpatialHasher<T> {
    type Output = ObjectEntry<T>;

    fn index(&self, index: ObjectIndex) -> &Self::Output {
        &self.objects[index.0]
    }
}

impl<T> ops::IndexMut<ObjectIndex> for SpatialHasher<T> {
    fn index_mut(&mut self, index: ObjectIndex) -> &mut Self::Output {
        &mut self.objects[index.0]
    }
}

impl<T> SpatialHasher<T> {
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
    buckets: &mut Arena<SpatialBucket>,
    (i, j): (i32, i32),
) -> BucketIndex {
    *spatial_map.entry((i, j)).or_insert_with(|| {
        let mins = Point2::new(i as f32 * bucket_size, j as f32 * bucket_size);
        let maxs = mins + Vector2::repeat(bucket_size);
        BucketIndex(buckets.insert(SpatialBucket::new(AABB::new(mins, maxs))))
    })
}

fn to_spatial_indices(bucket_size: f32, aabb: AABB<f32>) -> impl Iterator<Item = (i32, i32)> {
    let x_start = (aabb.mins.x / bucket_size).floor() as i32;
    let x_end = (aabb.maxs.x / bucket_size).ceil() as i32;
    let y_start = (aabb.mins.y / bucket_size).floor() as i32;
    let y_end = (aabb.maxs.y / bucket_size).ceil() as i32;

    (x_start..x_end).flat_map(move |i| (y_start..y_end).map(move |j| (i, j)))
}

impl<T> SpatialHasher<T> {
    pub fn insert(&mut self, position: &Point2<f32>, bb: &Cuboid<f32>, userdata: T) -> ObjectIndex {
        let aabb = bounding_volume::aabb(bb, &na::convert(Translation2::from(position.coords)));
        let object_id = ObjectIndex(self.objects.insert(ObjectEntry {
            position: *position,
            bounds: *bb,
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

    pub fn remove(&mut self, object: ObjectIndex) {
        for &bucket_id in self.objects[object.0].buckets.iter() {
            let members = &mut self.buckets[bucket_id.0].members;
            let index = members.binary_search(&object).unwrap_or_else(|x| x);
            members.remove(index);
        }

        self.objects.remove(object.0);
    }

    pub fn update(
        &mut self,
        object_id: ObjectIndex,
        position: &Point2<f32>,
        maybe_bounds: Option<&Cuboid<f32>>,
    ) {
        let object = &mut self.objects[object_id.0];

        // TODO: fudge value to avoid recomputing for small movements?
        if position == &object.position && maybe_bounds.is_none() {
            return;
        }

        if let Some(bounds) = maybe_bounds {
            object.bounds = *bounds;
        }

        object.position = *position;

        let aabb = bounding_volume::aabb(
            &object.bounds,
            &na::convert(Translation2::from(object.position.coords)),
        );

        for &bucket_id in object.buckets.iter() {
            let bucket = &mut self.buckets[bucket_id.0];
            if !bucket.bounds.intersects(&aabb) {
                if let Ok(idx) = bucket.members.binary_search(&object_id) {
                    bucket.members.remove(idx);
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
            }
        }
    }

    pub fn query<'a>(&'a self, aabb: &AABB<f32>) -> impl Iterator<Item = ObjectIndex> + 'a {
        to_spatial_indices(self.bucket_size, *aabb)
            .flat_map(move |coords| self.spatial_map.get(&coords).copied().into_iter())
            .flat_map(move |bucket_id| self.buckets[bucket_id.0].members.iter().copied())
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
                AABB::from_half_extents(Point2::new(23., 24.), Vector2::new(8., 8.))
            )),
            set_of(vec![(0, 0)])
        );

        assert_eq!(
            set_of(to_spatial_indices(
                64.,
                AABB::from_half_extents(Point2::new(-2., -3.), Vector2::new(4., 4.))
            )),
            set_of(vec![(0, 0), (-1, -1), (-1, 0), (0, -1)])
        );

        assert_eq!(
            set_of(to_spatial_indices(
                64.,
                AABB::from_half_extents(Point2::new(35., 35.), Vector2::new(36., 36.))
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
        let mut spatial_hasher = SpatialHasher::new(64.);
        let mut bucket_count = 0;

        let a = spatial_hasher.insert(
            &Point2::new(23., 42.),
            &Cuboid::new(Vector2::new(8., 8.)),
            "a",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );
        bucket_count = spatial_hasher.buckets.len();

        let b = spatial_hasher.insert(
            &Point2::new(-2., -3.),
            &Cuboid::new(Vector2::new(4., 4.)),
            "b",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );
        bucket_count = spatial_hasher.buckets.len();

        let c = spatial_hasher.insert(
            &Point2::new(35., 35.),
            &Cuboid::new(Vector2::new(36., 36.)),
            "c",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );
        bucket_count = spatial_hasher.buckets.len();

        let d = spatial_hasher.insert(
            &Point2::new(84., 20.),
            &Cuboid::new(Vector2::new(8., 8.)),
            "d",
        );

        println!(
            "{} new buckets...",
            spatial_hasher.buckets.len() - bucket_count
        );

        assert_eq!(
            set_of(spatial_hasher.query(&AABB::from_half_extents(
                Point2::new(20., 40.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![a, b, c])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&AABB::from_half_extents(
                Point2::new(-13., 14.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![b, c])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&AABB::from_half_extents(
                Point2::new(96., 96.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&AABB::from_half_extents(
                Point2::new(96., 32.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c, d])
        );

        spatial_hasher.update(d, &Point2::new(45., 20.), None);

        assert_eq!(
            set_of(spatial_hasher.query(&AABB::from_half_extents(
                Point2::new(96., 32.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c])
        );
    }
}
