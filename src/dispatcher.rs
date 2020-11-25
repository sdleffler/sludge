use crate::{
    dependency_graph::DependencyGraph, OwnedResources, SharedResources, System, UnifiedResources,
};
use {anyhow::*, rlua::prelude::*};

pub struct Dispatcher<'a> {
    dependency_graph: DependencyGraph<Box<dyn System + 'a>>,
}

impl<'a> Dispatcher<'a> {
    pub fn new() -> Self {
        Self {
            dependency_graph: DependencyGraph::new(),
        }
    }

    pub fn register<S>(&mut self, system: S, name: &str, deps: &[&str]) -> Result<()>
    where
        S: System + 'a,
    {
        ensure!(
            self.dependency_graph
                .insert(Box::new(system), name, deps.iter().copied())?
                .is_none(),
            "system already exists!"
        );

        Ok(())
    }

    pub fn refresh<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        local_resources: &mut OwnedResources,
        global_resources: Option<&SharedResources>,
    ) -> Result<()> {
        if self.dependency_graph.update()? {
            for (name, sys) in self.dependency_graph.sorted() {
                sys.init(lua, local_resources, global_resources)?;
                log::info!("initialized system `{}`", name);
            }
        }

        Ok(())
    }

    pub fn update<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        resources: &UnifiedResources,
    ) -> Result<()> {
        ensure!(
            !self.dependency_graph.is_dirty(),
            "dispatcher has been modified but not refreshed!"
        );

        for (_, sys) in self.dependency_graph.sorted() {
            sys.update(lua, resources)?;
        }

        Ok(())
    }
}
