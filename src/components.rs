use {rlua::prelude::*, serde::*, sludge_macros::SimpleComponent};

pub use crate::{
    api::*,
    ecs::*,
    hierarchy::Parent,
    math::*,
    sprite::{SpriteFrame, SpriteName, SpriteTag},
    transform::Transform,
};

#[derive(Debug, Clone, Serialize, Deserialize, SimpleComponent)]
pub struct Template {
    name: String,
}

impl Template {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SimpleComponent)]
pub struct Persistent;

pub struct PersistentAccessor(Entity);

impl LuaUserData for PersistentAccessor {}

impl LuaComponentInterface for Persistent {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        PersistentAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        _lua: LuaContext<'lua>,
        _args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        builder.add(Persistent);
        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<Persistent>("Persistent")
}
