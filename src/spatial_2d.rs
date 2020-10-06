use crate::{ecs::*, math::*};
use {
    ncollide2d::shape::ShapeHandle,
    serde::{Deserialize, Serialize},
    std::ops,
};

pub mod spatial_hash;
pub mod tile_grid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "PositionProxy", into = "PositionProxy")]
pub struct Position(pub Isometry2<f32>);

impl Default for Position {
    fn default() -> Self {
        Self(Isometry2::from_parts(na::one(), na::one()))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename = "Position")]
struct PositionProxy {
    x: f32,
    y: f32,

    #[serde(default)]
    angle: f32,
}

impl Default for PositionProxy {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            angle: 0.,
        }
    }
}

impl From<PositionProxy> for Position {
    fn from(de: PositionProxy) -> Self {
        Self(Isometry2::from_parts(
            Translation2::new(de.x, de.y),
            UnitComplex::new(de.angle),
        ))
    }
}

impl From<Position> for PositionProxy {
    fn from(Position(ser): Position) -> Self {
        Self {
            x: ser.translation.vector.x,
            y: ser.translation.vector.y,
            angle: ser.rotation.angle(),
        }
    }
}

impl<'a> SmartComponent<&'a Flags> for Position {}

impl ops::Deref for Position {
    type Target = Isometry2<f32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Position {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Position {
    pub fn position(&self) -> Point2<f32> {
        Point2::from(self.0.translation.vector)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "VelocityProxy", into = "VelocityProxy")]
pub struct Velocity(pub Isometry2<f32>);

#[derive(Serialize, Deserialize)]
#[serde(rename = "Velocity")]
#[serde(default)]
struct VelocityProxy {
    velocity: Vector2<f32>,
    angle: f32,
}

impl Default for VelocityProxy {
    fn default() -> Self {
        Self {
            velocity: Vector2::zeros(),
            angle: 0.,
        }
    }
}

impl From<VelocityProxy> for Velocity {
    fn from(de: VelocityProxy) -> Self {
        Self(Isometry2::from_parts(
            Translation2::from(de.velocity),
            UnitComplex::new(de.angle),
        ))
    }
}

impl From<Velocity> for VelocityProxy {
    fn from(Velocity(ser): Velocity) -> Self {
        Self {
            velocity: ser.translation.vector,
            angle: ser.rotation.angle(),
        }
    }
}

impl<'a> SmartComponent<&'a Flags> for Velocity {}

impl ops::Deref for Velocity {
    type Target = Isometry2<f32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Velocity {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Velocity {
    pub fn linear(&self) -> Vector2<f32> {
        self.translation.vector
    }

    pub fn angular(&self) -> f32 {
        self.rotation.angle()
    }
}

#[derive(Clone)]
pub struct Shape {
    pub local: Isometry2<f32>,
    pub handle: ShapeHandle<f32>,
}

impl<'a> SmartComponent<&'a Flags> for Shape {}

impl Shape {
    pub fn new(local: Isometry2<f32>, handle: ShapeHandle<f32>) -> Self {
        Self { local, handle }
    }
}
