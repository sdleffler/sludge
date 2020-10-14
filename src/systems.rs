use {anyhow::*, rlua::prelude::*, std::marker::PhantomData};

use crate::{
    components::Parent,
    ecs::World,
    hierarchy::{HierarchyManager, ParentComponent},
    transform::TransformManager,
    Resources, SharedResources, SludgeResultExt,
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
        let _ = resources
            .fetch_mut::<World>()
            .flush_queue()
            .log_error_err("sludge::ecs");

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
        if !resources.has_value::<HierarchyManager<P>>() {
            let hierarchy = {
                let world = resources
                    .get_mut::<World>()
                    .ok_or_else(|| anyhow!("no World resource yet"))?;
                HierarchyManager::<P>::new(world)
            };
            resources.insert(hierarchy);
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources) -> Result<()> {
        let hierarchy = &mut *resources.fetch_mut::<HierarchyManager<P>>();
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
        if !resources.has_value::<TransformManager<P>>() {
            let transform_graph = {
                let world = &mut *resources
                    .try_fetch_mut::<World>()
                    .ok_or_else(|| anyhow!("no World resource yet"))?;
                let hierarchy = &mut *resources
                    .try_fetch_mut::<HierarchyManager<P>>()
                    .ok_or_else(|| anyhow!("no HierarchyManager resource yet"))?;
                TransformManager::<P>::new(world, hierarchy)
            };
            resources.insert(transform_graph);
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources) -> Result<()> {
        let transforms = &mut *resources.fetch_mut::<TransformManager<P>>();
        transforms.update(resources);

        Ok(())
    }
}
