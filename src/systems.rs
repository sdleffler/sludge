use {anyhow::*, rlua::prelude::*, std::marker::PhantomData};

use crate::{
    components::Parent,
    ecs::World,
    hierarchy::{Hierarchy, ParentComponent},
    transform::TransformGraph,
    Resources, SharedResources,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct WorldEventSystem;

impl crate::System for WorldEventSystem {
    fn init(&self, _lua: LuaContext, resources: &mut Resources) -> Result<()> {
        if !resources.has_value::<World>() {
            resources.insert(World::new());
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources) -> Result<()> {
        resources.fetch_mut::<World>().flush_queue()?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HierarchySystem<P: ParentComponent>(PhantomData<P>);

pub type DefaultHierarchySystem = HierarchySystem<Parent>;

impl<P: ParentComponent> HierarchySystem<P> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<P: ParentComponent> crate::System for HierarchySystem<P> {
    fn init(&self, _lua: LuaContext, resources: &mut Resources) -> Result<()> {
        if !resources.has_value::<Hierarchy<P>>() {
            let hierarchy = {
                let world = resources
                    .get_mut::<World>()
                    .ok_or_else(|| anyhow!("no World resource yet"))?;
                Hierarchy::<P>::new(world)
            };
            resources.insert(hierarchy);
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources) -> Result<()> {
        let hierarchy = &mut *resources.fetch_mut::<Hierarchy<P>>();
        hierarchy.update(resources);

        Ok(())
    }
}

pub struct TransformSystem<P: ParentComponent>(PhantomData<P>);

pub type DefaultTransformSystem = TransformSystem<Parent>;

impl<P: ParentComponent> TransformSystem<P> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<P: ParentComponent> crate::System for TransformSystem<P> {
    fn init(&self, _lua: LuaContext, resources: &mut Resources) -> Result<()> {
        if !resources.has_value::<TransformGraph<P>>() {
            let transform_graph = {
                let world = &mut *resources
                    .try_fetch_mut::<World>()
                    .ok_or_else(|| anyhow!("no World resource yet"))?;
                let hierarchy = &mut *resources
                    .try_fetch_mut::<Hierarchy<P>>()
                    .ok_or_else(|| anyhow!("no Hierarchy resource yet"))?;
                TransformGraph::<P>::new(world, hierarchy)
            };
            resources.insert(transform_graph);
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources) -> Result<()> {
        let transforms = &mut *resources.fetch_mut::<TransformGraph<P>>();
        transforms.update(resources);

        Ok(())
    }
}
