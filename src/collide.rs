use crate::{
    ecs::{Flags, SmartComponent},
    math::*,
};
use {
    serde::{Deserialize, Serialize},
    std::ops,
};

pub mod spatial_hash;

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
