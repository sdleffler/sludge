use crate::{dependency_graph::DependencyGraph, Resources, SharedResources, System};
use {anyhow::*, rlua::prelude::*};

pub type DefaultDispatcher = Dispatcher<'static, ()>;

pub struct Dispatcher<'a, P: 'a> {
    dependency_graph: DependencyGraph<Box<dyn System<P> + 'a>>,
}

impl<'a, P: 'a> Dispatcher<'a, P> {
    pub fn new() -> Self {
        Self {
            dependency_graph: DependencyGraph::new(),
        }
    }

    pub fn register<S>(&mut self, system: S, name: &str, deps: &[&str]) -> Result<()>
    where
        S: System<P> + 'a,
    {
        ensure!(
            self.dependency_graph
                .insert(Box::new(system), name, deps.iter().copied())?
                .is_none(),
            "system already exists!"
        );

        Ok(())
    }

    pub fn refresh_with<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        resources: &mut Resources,
        params: &mut P,
    ) -> Result<()> {
        if self.dependency_graph.update()? {
            for (name, sys) in self.dependency_graph.sorted() {
                sys.init(lua, resources, params)?;
                log::info!("initialized system `{}`", name);
            }
        }

        Ok(())
    }

    pub fn update_with<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        resources: &SharedResources,
        params: &mut P,
    ) -> Result<()> {
        self.refresh_with(lua, &mut *resources.borrow_mut(), params)?;

        for (_, sys) in self.dependency_graph.sorted() {
            sys.update(lua, resources, params)?;
        }

        Ok(())
    }
}

impl DefaultDispatcher {
    pub fn refresh<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        resources: &mut Resources,
    ) -> Result<()> {
        self.refresh_with(lua, resources, &mut ())
    }

    pub fn update<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        resources: &SharedResources,
    ) -> Result<()> {
        self.update_with(lua, resources, &mut ())
    }
}
