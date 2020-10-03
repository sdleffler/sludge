use crate::{dependency_graph::DependencyGraph, Resources, SharedResources, System};
use {anyhow::*, rlua::prelude::*};

pub struct Dispatcher {
    dependency_graph: DependencyGraph<Box<dyn System>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            dependency_graph: DependencyGraph::new(),
        }
    }

    pub fn register<S: System>(&mut self, system: S, name: &str, deps: &[&str]) -> Result<()> {
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
        resources: &mut Resources,
    ) -> Result<()> {
        if self.dependency_graph.update()? {
            for (name, sys) in self.dependency_graph.sorted() {
                sys.init(lua, resources)?;
                log::info!("initialized system `{}`", name);
            }
        }

        Ok(())
    }

    pub fn update<'lua>(
        &mut self,
        lua: LuaContext<'lua>,
        resources: &SharedResources,
    ) -> Result<()> {
        self.refresh(lua, &mut *resources.borrow_mut())?;

        for (_, sys) in self.dependency_graph.sorted() {
            sys.update(lua, resources)?;
        }

        Ok(())
    }
}
