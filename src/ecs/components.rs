use {anyhow::*, hecs::SmartComponent, nalgebra as na, rlua::prelude::*, std::any::TypeId};

use crate::{
    ecs::{hierarchy::ParentComponent, Entity, Flags, World},
    modules::{
        ecs::{ComponentWrapper, EntityWrapper, RegisterableComponent},
        math,
    },
    SludgeLuaContextExt,
};

#[derive(Debug, Clone, Copy)]
pub struct Parent {
    pub parent_entity: Entity,
}

impl Parent {
    pub fn new(parent_entity: Entity) -> Self {
        Self { parent_entity }
    }
}

impl<'a> SmartComponent<&'a Flags> for Parent {
    fn on_borrow_mut(&mut self, entity: hecs::Entity, context: &'a Flags) {
        context[&TypeId::of::<Self>()].add_atomic(entity.id());
    }
}

impl ParentComponent for Parent {
    fn parent_entity(&self) -> Entity {
        self.parent_entity
    }
}

impl RegisterableComponent for Parent {
    fn constructor(lua: LuaContext) -> Result<Option<(&'static str, LuaFunction)>> {
        let f = lua.create_function(|_ctx, parent_entity: EntityWrapper| {
            Ok(ComponentWrapper::new(Parent::new(parent_entity.0)))
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
                        .get::<Parent>(this.0)
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
                    .get_mut::<Parent>(this.0)
                    .unwrap()
                    .parent_entity = new_parent.0;
                Ok(())
            })?,
        )?;

        Ok(table)
    }
}

pub type TransformObject = na::Transform2<f32>;

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub(crate) local: TransformObject,
    pub(crate) global: TransformObject,
}

impl Transform {
    pub fn new(transform: TransformObject) -> Self {
        Self {
            local: transform,
            global: transform,
        }
    }

    pub fn local(&self) -> &TransformObject {
        &self.local
    }

    pub fn local_mut(&mut self) -> &mut TransformObject {
        &mut self.local
    }

    pub fn global(&self) -> &TransformObject {
        &self.global
    }

    #[rustfmt::skip]
    pub fn local_to_mat4(&self) -> na::Matrix4<f32> {
        let mat3 = self.local.to_homogeneous();
        na::Matrix4::new(
            mat3[(0, 0)], mat3[(0, 1)],           0., mat3[(0, 2)],
            mat3[(1, 0)], mat3[(1, 1)],           0., mat3[(1, 2)],
                      0.,           0.,           1.,           0.,
            mat3[(2, 0)], mat3[(2, 1)],           0., mat3[(2, 2)],
        )
    }

    #[rustfmt::skip]
    pub fn global_to_mat4(&self) -> na::Matrix4<f32> {
        let mat3 = self.global.to_homogeneous();
        na::Matrix4::new(
            mat3[(0, 0)], mat3[(0, 1)],           0., mat3[(0, 2)],
            mat3[(1, 0)], mat3[(1, 1)],           0., mat3[(1, 2)],
                      0.,           0.,           1.,           0.,
            mat3[(2, 0)], mat3[(2, 1)],           0., mat3[(2, 2)],
        )
    }
}

impl<'a> SmartComponent<&'a Flags> for Transform {
    fn on_borrow_mut(&mut self, entity: Entity, flags: &'a Flags) {
        flags[&TypeId::of::<Self>()].add_atomic(entity.id());
    }
}

impl RegisterableComponent for Transform {
    fn constructor(lua: LuaContext) -> Result<Option<(&'static str, LuaFunction)>> {
        let f = lua.create_function(|_ctx, transform: math::Transform| {
            Ok(ComponentWrapper::new(Transform::new(transform.0)))
        })?;

        Ok(Some(("Transform", f)))
    }

    fn method_table(lua: LuaContext) -> Result<LuaTable> {
        let table = lua.create_table()?;

        table.set(
            "get_local_transform",
            lua.create_function(|ctx, (this, dst): (EntityWrapper, LuaAnyUserData)| {
                dst.borrow_mut::<math::Transform>()?.0 = *ctx
                    .resources()
                    .fetch::<World>()
                    .get::<Transform>(this.0)
                    .unwrap()
                    .local();
                Ok(dst)
            })?,
        )?;

        table.set(
            "set_local_transform",
            lua.create_function(|ctx, (this, src): (EntityWrapper, math::Transform)| {
                *ctx.resources()
                    .fetch::<World>()
                    .get_mut::<Transform>(this.0)
                    .unwrap()
                    .local_mut() = src.0;
                Ok(())
            })?,
        )?;

        table.set(
            "get_global_transform",
            lua.create_function(|ctx, (this, dst): (EntityWrapper, LuaAnyUserData)| {
                dst.borrow_mut::<math::Transform>()?.0 = *ctx
                    .resources()
                    .fetch::<World>()
                    .get::<Transform>(this.0)
                    .unwrap()
                    .global();
                Ok(dst)
            })?,
        )?;

        Ok(table)
    }
}
