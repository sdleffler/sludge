use {rlua::prelude::*, serde::*, sludge_macros::SimpleComponent};

pub use crate::{
    api::*,
    ecs::*,
    hierarchy::Parent,
    math::*,
    resources::Resources,
    sprite::{SpriteFrame, SpriteName, SpriteTag},
    transform::Transform,
    SludgeLuaContextExt,
};

#[derive(Debug, Clone, Serialize, Deserialize, SimpleComponent)]
pub struct Name(pub String);

impl Name {
    pub fn new(name: String) -> Self {
        Self(name)
    }
}

pub struct NameAccessor(Entity);

impl LuaUserData for NameAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("get", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let name = world.get::<Name>(this.0).to_lua_err()?;
            name.0.as_str().to_lua(lua)
        });

        methods.add_method("to_table", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let name = world.get::<Name>(this.0).to_lua_err()?;
            rlua_serde::to_value(lua, &*name)
        });
    }
}

impl LuaComponentInterface for Name {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        NameAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let name = String::from_lua(args, lua)?;
        builder.add(Name(name));
        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<Name>("Name")
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
