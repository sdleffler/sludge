use crate::{
    ecs::{transform_graph::Transform as TransformComponent, World},
    module::{
        ecs::{ComponentWrapper, EntityWrapper, RegisterableComponent},
        math::Transform,
    },
    SludgeLuaContextExt,
};
use {anyhow::*, rlua::prelude::*};

impl RegisterableComponent for TransformComponent {
    fn constructor(lua: LuaContext) -> Result<Option<(&'static str, LuaFunction)>> {
        let f = lua.create_function(|_ctx, transform: Transform| {
            Ok(ComponentWrapper::new(TransformComponent::new(transform.0)))
        })?;

        Ok(Some(("Transform", f)))
    }

    fn method_table(lua: LuaContext) -> Result<LuaTable> {
        let table = lua.create_table()?;

        table.set(
            "get_local_transform",
            lua.create_function(|ctx, (this, dst): (EntityWrapper, LuaAnyUserData)| {
                dst.borrow_mut::<Transform>()?.0 = *ctx
                    .resources()
                    .fetch::<World>()
                    .get::<TransformComponent>(this.0)
                    .unwrap()
                    .local();
                Ok(dst)
            })?,
        )?;

        table.set(
            "set_local_transform",
            lua.create_function(|ctx, (this, src): (EntityWrapper, Transform)| {
                *ctx.resources()
                    .fetch::<World>()
                    .get_mut::<TransformComponent>(this.0)
                    .unwrap()
                    .local_mut() = src.0;
                Ok(())
            })?,
        )?;

        table.set(
            "get_global_transform",
            lua.create_function(|ctx, (this, dst): (EntityWrapper, LuaAnyUserData)| {
                dst.borrow_mut::<Transform>()?.0 = *ctx
                    .resources()
                    .fetch::<World>()
                    .get::<TransformComponent>(this.0)
                    .unwrap()
                    .global();
                Ok(dst)
            })?,
        )?;

        Ok(table)
    }
}
