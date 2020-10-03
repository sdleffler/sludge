use crate::{
    ecs::{Flags, SmartComponent},
    math::*,
};
use {
    hashbrown::HashMap,
    serde::{Deserialize, Serialize},
    smallvec::SmallVec,
    std::ops,
    thunderdome::{Arena, Index},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "BoundingBox2Proxy", into = "BoundingBox2Proxy")]
pub struct BoundingBox2(pub AABB<f32>);

#[derive(Serialize, Deserialize)]
#[serde(rename = "BoundingBox2")]
pub struct BoundingBox2Proxy {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl From<BoundingBox2> for BoundingBox2Proxy {
    fn from(bb: BoundingBox2) -> Self {
        Self {
            x: bb.0.mins.x,
            y: bb.0.mins.y,
            w: bb.0.extents().x,
            h: bb.0.extents().y,
        }
    }
}

impl From<BoundingBox2Proxy> for BoundingBox2 {
    fn from(proxy: BoundingBox2Proxy) -> Self {
        Self(AABB::new(
            Point2::new(proxy.x, proxy.y),
            Point2::new(proxy.x + proxy.w, proxy.y + proxy.h),
        ))
    }
}

impl<'a> SmartComponent<&'a Flags> for BoundingBox2 {}

impl ops::Deref for BoundingBox2 {
    type Target = AABB<f32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for BoundingBox2 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

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
}

#[derive(Debug)]
pub struct ObjectEntry<T> {
    pub position: Point2<f32>,
    pub bounds: Cuboid<f32>,
    pub buckets: SmallVec<[BucketIndex; 4]>,
    pub userdata: T,
}

#[derive(Debug)]
pub struct SpatialHasher<T> {
    bucket_size: f32,
    spatial_map: HashMap<(i32, i32), BucketIndex>,
    buckets: Arena<SpatialBucket>,
    objects: Arena<ObjectEntry<T>>,
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

    fn get_or_insert_bucket(&mut self, i: i32, j: i32) -> BucketIndex {
        let Self {
            bucket_size,
            spatial_map,
            buckets,
            ..
        } = self;

        *spatial_map.entry((i, j)).or_insert_with(|| {
            let mins = Point2::new(i as f32 * (*bucket_size), j as f32 * (*bucket_size));
            let maxs = mins + Vector2::repeat(*bucket_size);
            BucketIndex(buckets.insert(SpatialBucket::new(AABB::new(mins, maxs))))
        })
    }
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

        for (i, j) in to_spatial_indices(self.bucket_size, aabb) {
            let bucket_id = self.get_or_insert_bucket(i, j);
            self.buckets[bucket_id.0].members.push(object_id);
            buckets.push(bucket_id);
        }

        self.objects[object_id.0].buckets = buckets;
        object_id
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
                Point2::new(96., 32.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c, d])
        );

        assert_eq!(
            set_of(spatial_hasher.query(&AABB::from_half_extents(
                Point2::new(96., 96.),
                Vector2::new(8., 8.),
            ))),
            set_of(vec![c])
        );
    }
}
