use {hecs::SmartComponent, nalgebra as na, std::any::TypeId};

use crate::ecs::{hierarchy::ParentComponent, Entity, Flags};

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
