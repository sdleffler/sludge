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
