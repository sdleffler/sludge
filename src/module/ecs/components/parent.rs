use crate::{
    ecs::{hierarchy::Parent as ParentComponent, World},
    module::ecs::{ComponentWrapper, EntityWrapper, RegisterableComponent},
    SludgeLuaContextExt,
};
use {anyhow::*, rlua::prelude::*};

impl RegisterableComponent for ParentComponent {
    fn constructor(lua: LuaContext) -> Result<Option<(&'static str, LuaFunction)>> {
        let f = lua.create_function(|_ctx, parent_entity: EntityWrapper| {
            Ok(ComponentWrapper::new(ParentComponent::new(parent_entity.0)))
        })?;

        Ok(Some(("Parent", f)))
    }

    fn method_table(lua: LuaContext) -> Result<LuaTable> {
        let table = lua.create_table()?;

        table.set(
            "get_parent",
            lua.create_function(|ctx, this: EntityWrapper| {
                Ok(EntityWrapper(
                    ctx.resources()
                        .fetch::<World>()
                        .get::<ParentComponent>(this.0)
                        .unwrap()
                        .parent_entity,
                ))
            })?,
        )?;

        table.set(
            "set_parent",
            lua.create_function(|ctx, (this, new_parent): (EntityWrapper, EntityWrapper)| {
                ctx.resources()
                    .fetch::<World>()
                    .get_mut::<ParentComponent>(this.0)
                    .unwrap()
                    .parent_entity = new_parent.0;
                Ok(())
            })?,
        )?;

        Ok(table)
    }
}
