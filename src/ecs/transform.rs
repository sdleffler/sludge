use {crossbeam_channel::Receiver, hashbrown::HashSet, shrev::ReaderId, std::marker::PhantomData};

use crate::ecs::{
    components::{Parent, Transform},
    hierarchy::{Hierarchy, HierarchyEvent, ParentComponent},
    ComponentEvent, Entity, World,
};

pub struct TransformGraph<P: ParentComponent = Parent> {
    hierarchy_events: ReaderId<HierarchyEvent>,
    transform_events: Receiver<ComponentEvent>,

    modified: HashSet<Entity>,
    removed: HashSet<Entity>,

    _marker: PhantomData<P>,
}

impl<P: ParentComponent> TransformGraph<P> {
    pub fn new(world: &mut World, hierarchy: &mut Hierarchy<P>) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        world.subscribe::<Transform>(Box::new(sender));
        let reader = hierarchy.track();

        Self {
            hierarchy_events: reader,
            transform_events: receiver,

            modified: HashSet::new(),
            removed: HashSet::new(),

            _marker: PhantomData,
        }
    }

    pub fn update(&mut self, world: &mut World, hierarchy: &Hierarchy<P>) {
        self.modified.clear();
        self.removed.clear();

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

        for event in self.transform_events.try_iter() {
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
            if self.modified.contains(&entity) {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::components::Parent;
    use {approx::assert_relative_eq, nalgebra as na};

    #[test]
    fn parent_update() {
        let mut world = World::new();
        let mut hierarchy = Hierarchy::<Parent>::new(&mut world);
        let mut transforms = TransformGraph::new(&mut world, &mut hierarchy);

        let e1 = {
            let mut tx = na::Transform2::identity();
            tx *= &na::Translation2::new(5., 7.);
            tx *= &na::Rotation2::new(::std::f32::consts::PI);
            world.spawn((Transform::new(tx),))
        };

        hierarchy.update(&mut world);
        transforms.update(&mut world, &hierarchy);

        let e2 = {
            let mut tx = na::Transform2::identity();
            tx *= &na::Translation2::new(5., 3.);
            world.spawn((Transform::new(tx), Parent::new(e1)))
        };

        hierarchy.update(&mut world);
        transforms.update(&mut world, &hierarchy);

        let tx2 = *world.get::<Transform>(e2).unwrap();

        assert_relative_eq!(
            tx2.global.transform_point(&na::Point2::origin()),
            na::Point2::new(-10., -10.)
        );

        world.despawn(e1).unwrap();

        hierarchy.update(&mut world);
        transforms.update(&mut world, &hierarchy);

        let tx2 = *world.get::<Transform>(e2).unwrap();

        assert_relative_eq!(
            tx2.global.transform_point(&na::Point2::origin()),
            na::Point2::new(5., 3.)
        );
    }
}
