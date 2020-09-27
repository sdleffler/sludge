use {hashbrown::HashSet, shrev::ReaderId, std::marker::PhantomData};

use crate::{
    ecs::{
        components::{Parent, Transform},
        hierarchy::{Hierarchy, HierarchyEvent, ParentComponent},
        ComponentEvent, Entity, World,
    },
    resources::SharedResources,
};

pub struct TransformGraph<P: ParentComponent = Parent> {
    hierarchy_events: ReaderId<HierarchyEvent>,
    transform_events: ReaderId<ComponentEvent>,

    modified: HashSet<Entity>,
    removed: HashSet<Entity>,

    _marker: PhantomData<P>,
}

impl<P: ParentComponent> TransformGraph<P> {
    pub fn new(world: &mut World, hierarchy: &mut Hierarchy<P>) -> Self {
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

    pub fn update(&mut self, resources: &SharedResources) {
        self.modified.clear();
        self.removed.clear();

        let world = &*resources.fetch::<World>();
        let hierarchy = &*resources.fetch::<Hierarchy<P>>();

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::components::Parent;
    use {approx::assert_relative_eq, nalgebra as na};

    #[test]
    fn parent_update() {
        let resources = crate::resources::SharedResources::new();

        let mut world = World::new();
        let mut hierarchy = Hierarchy::<Parent>::new(&mut world);
        let transforms = TransformGraph::new(&mut world, &mut hierarchy);

        resources.borrow_mut().insert(world);
        resources.borrow_mut().insert(hierarchy);
        resources.borrow_mut().insert(transforms);

        let e1 = {
            let mut tx = na::Transform2::identity();
            tx *= &na::Translation2::new(5., 7.);
            tx *= &na::Rotation2::new(::std::f32::consts::PI);
            resources.fetch_mut::<World>().spawn((Transform::new(tx),))
        };

        resources
            .fetch_mut::<Hierarchy<Parent>>()
            .update(&resources);
        resources.fetch_mut::<TransformGraph>().update(&resources);

        let e2 = {
            let mut tx = na::Transform2::identity();
            tx *= &na::Translation2::new(5., 3.);
            resources
                .fetch_mut::<World>()
                .spawn((Transform::new(tx), Parent::new(e1)))
        };

        resources
            .fetch_mut::<Hierarchy<Parent>>()
            .update(&resources);
        resources.fetch_mut::<TransformGraph>().update(&resources);

        let tx2 = *resources.fetch::<World>().get::<Transform>(e2).unwrap();

        assert_relative_eq!(
            tx2.global.transform_point(&na::Point2::origin()),
            na::Point2::new(-10., -10.)
        );

        resources.fetch_mut::<World>().despawn(e1).unwrap();

        resources
            .fetch_mut::<Hierarchy<Parent>>()
            .update(&resources);
        resources.fetch_mut::<TransformGraph>().update(&resources);

        let tx2 = *resources.fetch::<World>().get::<Transform>(e2).unwrap();

        assert_relative_eq!(
            tx2.global.transform_point(&na::Point2::origin()),
            na::Point2::new(5., 3.)
        );
    }
}
