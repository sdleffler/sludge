use {
    hecs::SmartComponent,
    serde::{Deserialize, Serialize},
};

use crate::ecs::Flags;

pub use crate::{
    hierarchy::Parent,
    math::*,
    sprite::{SpriteFrame, SpriteName, SpriteTag},
    transform::Transform,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    name: String,
}

impl<'a> SmartComponent<&'a Flags> for Template {}

impl Template {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "Position2Proxy", into = "Position2Proxy")]
pub struct Position2(pub Isometry2<f32>);

#[derive(Serialize, Deserialize)]
#[serde(rename = "Position2")]
#[serde(default)]
struct Position2Proxy {
    position: Vector2<f32>,
    angle: f32,
}

impl Default for Position2Proxy {
    fn default() -> Self {
        Self {
            position: Vector2::zeros(),
            angle: 0.,
        }
    }
}

impl From<Position2Proxy> for Position2 {
    fn from(de: Position2Proxy) -> Self {
        Self(Isometry2::from_parts(
            Translation2::from(de.position),
            UnitComplex::new(de.angle),
        ))
    }
}

impl From<Position2> for Position2Proxy {
    fn from(Position2(ser): Position2) -> Self {
        Self {
            position: ser.translation.vector,
            angle: ser.rotation.angle(),
        }
    }
}

impl<'a> SmartComponent<&'a Flags> for Position2 {}

impl Position2 {
    pub fn position(&self) -> Point2<f32> {
        Point2::from(self.0.translation.vector)
    }
}
