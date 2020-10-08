use {
    std::{mem, ops},
    thunderdome::{Arena, Index},
};

pub struct ObjectBuilder<'a, T> {
    index: Index,
    graph: &'a mut SceneGraph<T>,
}

impl<'a, T> ObjectBuilder<'a, T> {
    pub fn layer(&mut self, layer: i32) -> &mut Self {
        self.graph.objects[self.index].layer = layer;
        self
    }

    pub fn parent(&mut self, index: ObjectId) -> &mut Self {
        let parent_node = &mut self.graph.objects[index.0];
        if let Err(i) = parent_node.children.binary_search(&self.index) {
            parent_node.children.insert(i, self.index);
        }

        let old_parent = mem::replace(&mut self.graph.objects[self.index].parent, Some(index.0));

        if let Some(old_parent_id) = old_parent {
            let old_parent_node = &mut self.graph.objects[old_parent_id];
            let i = old_parent_node
                .children
                .binary_search(&self.index)
                .expect("invalid scene graph");
            old_parent_node.children.remove(i);
        }

        self
    }

    pub fn get(&self) -> ObjectId {
        ObjectId(self.index)
    }
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct ObjectId(Index);

#[derive(Debug)]
struct Node<T> {
    value: T,
    layer: i32,
    parent: Option<Index>,
    children: Vec<Index>,
}

#[derive(Debug)]
pub struct SceneGraph<T> {
    objects: Arena<Node<T>>,
    roots: Vec<Index>,
    sorted: Vec<Index>,

    scratch_buf: Vec<Index>,
    scratch_stack: Vec<Index>,

    dirty: bool,
}

impl<T> ops::Index<ObjectId> for SceneGraph<T> {
    type Output = T;

    fn index(&self, i: ObjectId) -> &Self::Output {
        &self.objects[i.0].value
    }
}

impl<T> ops::IndexMut<ObjectId> for SceneGraph<T> {
    fn index_mut(&mut self, i: ObjectId) -> &mut Self::Output {
        &mut self.objects[i.0].value
    }
}

impl<T> SceneGraph<T> {
    pub fn new() -> Self {
        Self {
            objects: Arena::new(),
            roots: Vec::new(),
            sorted: Vec::new(),

            scratch_buf: Vec::new(),
            scratch_stack: Vec::new(),

            dirty: true,
        }
    }

    pub fn insert(&mut self, value: T) -> ObjectBuilder<T> {
        let index = self.objects.insert(Node {
            value,
            layer: 0,
            parent: None,
            children: vec![],
        });

        self.dirty = true;

        ObjectBuilder { index, graph: self }
    }

    pub fn remove(&mut self, object: ObjectId) -> Option<T> {
        let node = self.objects.remove(object.0)?;

        for child in node.children {
            self.objects[child].parent = None;
        }

        if let Some(parent) = node.parent {
            let parent_node = &mut self.objects[parent];
            let i = parent_node
                .children
                .binary_search(&object.0)
                .expect("invalid scene graph");
            parent_node.children.remove(i);
        }

        self.dirty = true;

        Some(node.value)
    }

    pub fn sort(&mut self) {
        let Self {
            objects,
            sorted,
            roots,
            scratch_buf: buf,
            scratch_stack: stack,
            dirty,
        } = self;

        roots.clear();
        for (index, node) in objects.iter() {
            if node.parent.is_none() {
                roots.push(index);
            }
        }
        roots.sort_by_key(|&root| objects[root].layer);

        sorted.clear();
        stack.clear();

        stack.extend(roots.iter().rev());
        while let Some(index) = stack.pop() {
            sorted.push(index);
            buf.clear();
            buf.extend_from_slice(&objects[index].children);
            buf.sort_by_key(|&k| objects[k].layer);
            stack.extend(buf.drain(..).rev());
        }

        *dirty = false;
    }

    pub fn sorted(&mut self) -> impl Iterator<Item = &T> + '_ {
        if self.dirty {
            self.sort();
        }

        let Self {
            ref objects,
            ref sorted,
            ..
        } = self;

        sorted.iter().map(move |&index| &objects[index].value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort() {
        let mut scene = SceneGraph::new();
        let root = scene.insert("root").get();
        let ui = scene.insert("ui").layer(1).parent(root).get();
        let level = scene.insert("level").layer(-1).parent(root).get();

        scene.insert("healthbar").parent(ui).layer(4).get();
        scene.insert("magicbar").parent(ui).layer(-3).get();

        scene.insert("foreground").layer(2).parent(level).get();
        scene.insert("background").layer(-1).parent(level).get();
        scene.insert("player").layer(1).parent(level).get();
        scene.insert("terrain").layer(0).parent(level).get();

        assert_eq!(
            &scene.sorted().collect::<Vec<_>>(),
            &[
                &"root",
                &"level",
                &"background",
                &"terrain",
                &"player",
                &"foreground",
                &"ui",
                &"magicbar",
                &"healthbar"
            ]
        );
    }
}
