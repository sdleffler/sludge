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

impl<P> crate::System<P> for WorldEventSystem {
    fn init(&self, _lua: LuaContext, resources: &mut Resources, _params: &mut P) -> Result<()> {
        if !resources.has_value::<World>() {
            resources.insert(World::new());
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources, _params: &P) -> Result<()> {
        let _ = resources
            .fetch_mut::<World>()
            .flush_queue()
            .log_error_err("sludge::ecs");

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HierarchySystem<C: ParentComponent>(PhantomData<C>);

pub type DefaultHierarchySystem = HierarchySystem<Parent>;

impl<C: ParentComponent> HierarchySystem<C> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<P, C: ParentComponent> crate::System<P> for HierarchySystem<C> {
    fn init(&self, _lua: LuaContext, resources: &mut Resources, _params: &mut P) -> Result<()> {
        if !resources.has_value::<HierarchyManager<C>>() {
            let hierarchy = {
                let world = resources
                    .get_mut::<World>()
                    .ok_or_else(|| anyhow!("no World resource yet"))?;
                HierarchyManager::<C>::new(world)
            };
            resources.insert(hierarchy);
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources, _params: &P) -> Result<()> {
        let hierarchy = &mut *resources.fetch_mut::<HierarchyManager<C>>();
        hierarchy.update(resources);

        Ok(())
    }
}

pub struct TransformSystem<C: ParentComponent>(PhantomData<C>);

pub type DefaultTransformSystem = TransformSystem<Parent>;

impl<C: ParentComponent> TransformSystem<C> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<P, C: ParentComponent> crate::System<P> for TransformSystem<C> {
    fn init(&self, _lua: LuaContext, resources: &mut Resources, _params: &mut P) -> Result<()> {
        if !resources.has_value::<TransformManager<C>>() {
            let transform_graph = {
                let world = &mut *resources
                    .try_fetch_mut::<World>()
                    .ok_or_else(|| anyhow!("no World resource yet"))?;
                let hierarchy = &mut *resources
                    .try_fetch_mut::<HierarchyManager<C>>()
                    .ok_or_else(|| anyhow!("no HierarchyManager resource yet"))?;
                TransformManager::<C>::new(world, hierarchy)
            };
            resources.insert(transform_graph);
        }
        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &SharedResources, _params: &P) -> Result<()> {
        let transforms = &mut *resources.fetch_mut::<TransformManager<C>>();
        transforms.update(resources);

        Ok(())
    }
}
