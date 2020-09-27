use crate::Atom;
use {anyhow::*, hashbrown::HashMap, petgraph::prelude::*, std::borrow::Borrow};

pub struct DependencyGraph<T> {
    graph: StableGraph<(Atom, T), ()>,
    indices: HashMap<Atom, NodeIndex>,
    sorted: Vec<NodeIndex>,
    changed: bool,
}

impl<T> DependencyGraph<T> {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            indices: HashMap::new(),
            sorted: Vec::new(),
            changed: false,
        }
    }

    pub fn insert<I, N, S>(&mut self, value: T, name: N, deps: I) -> Result<Option<T>>
    where
        I: IntoIterator<Item = S>,
        S: Borrow<str>,
        N: Borrow<str>,
    {
        let name = Atom::from(name.borrow());
        let node = self.graph.add_node((name.clone(), value));
        let maybe_old = self.indices.insert(name, node);
        for dep in deps.into_iter() {
            let dep_node = self
                .indices
                .get(&Atom::from(dep.borrow()))
                .ok_or_else(|| anyhow!("no such dependency {}", dep.borrow()))?;
            self.graph.add_edge(*dep_node, node, ());
        }
        self.changed = true;
        Ok(maybe_old.map(|old| self.graph.remove_node(old).unwrap().1))
    }

    pub fn update(&mut self) -> Result<bool> {
        if !self.changed {
            return Ok(false);
        }

        self.sorted = petgraph::algo::toposort(&self.graph, None).map_err(|cycle| {
            let node = &self.graph[cycle.node_id()].0;
            anyhow!(
                "A cycle was found which includes the node `{}`, \
                but the dependency graph must be acyclic to allow \
                a proper ordering of dependencies!",
                node
            )
        })?;
        self.changed = false;

        Ok(true)
    }

    pub fn sorted(&self) -> impl Iterator<Item = (&str, &T)> {
        assert!(!self.changed);
        self.sorted.iter().copied().map(move |index| {
            let (ref name, ref value) = self.graph[index];
            (name.as_ref(), value)
        })
    }
}
