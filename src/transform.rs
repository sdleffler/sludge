use {
    anyhow::*,
    hashbrown::HashSet,
    serde::{Deserialize, Serialize},
    shrev::ReaderId,
    sludge_macros::*,
    std::marker::PhantomData,
};

use crate::{
    components::Parent,
    ecs::{ComponentEvent, ComponentSubscriber, Entity, World},
    hierarchy::{HierarchyEvent, HierarchyManager, ParentComponent},
    math::Transform3,
    Resources,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TrackedComponent)]
pub struct Transform {
    pub(crate) local: Transform3<f32>,
    pub(crate) global: Transform3<f32>,
}

impl Transform {
    pub fn new(transform: Transform3<f32>) -> Self {
        Self {
            local: transform,
            global: transform,
        }
    }

    pub fn local(&self) -> &Transform3<f32> {
        &self.local
    }

    pub fn local_mut(&mut self) -> &mut Transform3<f32> {
        &mut self.local
    }

    pub fn global(&self) -> &Transform3<f32> {
        &self.global
    }
}

pub struct TransformManager<P: ParentComponent = Parent> {
    hierarchy_events: ReaderId<HierarchyEvent>,
    transform_events: ComponentSubscriber<Transform>,

    modified: HashSet<Entity>,
    removed: HashSet<Entity>,

    _marker: PhantomData<P>,
}

impl<P: ParentComponent> TransformManager<P> {
    pub fn new(world: &mut World, hierarchy: &mut HierarchyManager<P>) -> Self {
        let transform_events = world.track::<Transform>();
        let hierarchy_events = hierarchy.track();

        Self {
            hierarchy_events,
            transform_events,

            modified: HashSet::new(),
            removed: HashSet::new(),

            _marker: PhantomData,
        }
    }

    pub fn update<'a, R: Resources<'a>>(&mut self, resources: &R) -> Result<()> {
        self.modified.clear();
        self.removed.clear();

        let (shared_world, shared_hierarchy) = resources.fetch::<(World, HierarchyManager<P>)>()?;
        let hierarchy = shared_hierarchy.borrow_mut();
        let world = shared_world.borrow_mut();

        for event in hierarchy.changed().read(&mut self.hierarchy_events) {
            match event {
                HierarchyEvent::ModifiedOrCreated(entity) => {
                    self.modified.insert(*entity);
                }
                HierarchyEvent::Removed(entity) => {
                    self.removed.insert(*entity);
                }
            }
        }

        for &event in world.poll::<Transform>(&mut self.transform_events) {
            match event {
                ComponentEvent::Inserted(entity) => {
                    self.modified.insert(entity);
                }
                ComponentEvent::Modified(entity) => {
                    self.modified.insert(entity);
                }
                ComponentEvent::Removed(entity) => {
                    self.modified
                        .extend(hierarchy.children(entity).iter().copied());
                }
            }
        }

        for entity in self.removed.iter().copied() {
            if let Ok(mut transform) = world.get_mut_raw::<Transform>(entity) {
                transform.global = transform.local;
            }
        }

        for entity in hierarchy.all().iter().copied() {
            if self.modified.remove(&entity) {
                self.modified.extend(hierarchy.children(entity));

                let parent_global = world
                    .get_raw::<Transform>(hierarchy.parent(entity).expect("exists in hierarchy"))
                    .expect("exists in hierarchy")
                    .global;

                let mut transform = world
                    .get_mut_raw::<Transform>(entity)
                    .expect("exists in hierarchy");

                transform.global = parent_global * transform.local;
            }
        }

        for entity in self.modified.iter().copied() {
            if let Ok(mut transform) = world.get_mut_raw::<Transform>(entity) {
                transform.global = transform.local;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{components::Parent, math::*, SharedResources};
    use approx::assert_relative_eq;

    #[test]
    fn parent_update() -> Result<()> {
        let resources = SharedResources::new();

        let mut world = World::new();
        let mut hierarchy = HierarchyManager::<Parent>::new(&mut world);
        let transforms = TransformManager::new(&mut world, &mut hierarchy);

        resources.borrow_mut().insert(world);
        resources.borrow_mut().insert(hierarchy);
        resources.borrow_mut().insert(transforms);

        let e1 = {
            let mut tx = Transform3::identity();
            tx *= &Translation3::new(-5., -7., 0.);
            tx *= &Rotation3::from_axis_angle(&Vector3::z_axis(), ::std::f32::consts::PI);
            resources
                .fetch_one::<World>()?
                .borrow_mut()
                .spawn((Transform::new(tx),))
        };

        resources
            .fetch_one::<HierarchyManager<Parent>>()?
            .borrow_mut()
            .update(&resources)?;
        resources
            .fetch_one::<TransformManager>()?
            .borrow_mut()
            .update(&resources)?;

        let e2 = {
            let mut tx = Transform3::identity();
            tx *= &Translation3::new(5., 3., 0.);
            resources
                .fetch_one::<World>()?
                .borrow_mut()
                .spawn((Transform::new(tx), Parent::new(e1)))
        };

        resources
            .fetch_one::<HierarchyManager<Parent>>()?
            .borrow_mut()
            .update(&resources)?;
        resources
            .fetch_one::<TransformManager>()?
            .borrow_mut()
            .update(&resources)?;

        let tx2 = *resources
            .fetch_one::<World>()?
            .borrow()
            .get::<Transform>(e2)
            .unwrap();

        assert_relative_eq!(
            tx2.global.transform_point(&Point3::origin()),
            Point3::new(-10., -10., 0.)
        );

        resources
            .fetch_one::<World>()?
            .borrow_mut()
            .despawn(e1)
            .unwrap();

        resources
            .fetch_one::<HierarchyManager<Parent>>()?
            .borrow_mut()
            .update(&resources)?;
        resources
            .fetch_one::<TransformManager>()?
            .borrow_mut()
            .update(&resources)?;

        let tx2 = *resources
            .fetch_one::<World>()?
            .borrow()
            .get::<Transform>(e2)
            .unwrap();

        assert_relative_eq!(
            tx2.global.transform_point(&Point3::origin()),
            Point3::new(5., 3., 0.)
        );

        Ok(())
    }
}
