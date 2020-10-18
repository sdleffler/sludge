use crate::{
    graphics::{AnyDrawable, Drawable, DrawableId, ErasedDrawableId, Graphics, InstanceParam},
    math::*,
};
use {
    hashbrown::HashMap,
    ordered_float::OrderedFloat,
    std::{
        any::Any,
        marker::PhantomData,
        mem, ops,
        sync::{
            atomic::{self, AtomicBool},
            RwLock, RwLockReadGuard,
        },
    },
    thunderdome::{Arena, Index},
};

pub type DrawableNodeId<T> = DrawableId<T, DrawableGraph>;
pub type ErasedDrawableNodeId = ErasedDrawableId<DrawableGraph>;

pub struct DrawableGraphIter<'a> {
    _outer: RwLockReadGuard<'a, DrawableGraphInner>,
    inner: ::std::slice::Iter<'a, Index>,
    objects: &'a Arena<Node>,
}

impl<'a> Iterator for DrawableGraphIter<'a> {
    type Item = &'a dyn AnyDrawable;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|&index| self.objects[index].value.as_ref())
    }
}

pub struct DrawableNodeBuilder<'a, T: AnyDrawable> {
    index: Index,
    graph: &'a mut DrawableGraph,
    marker: PhantomData<&'a mut T>,
}

impl<'a, T: AnyDrawable> DrawableNodeBuilder<'a, T> {
    pub fn layer(&mut self, layer: i32) -> &mut Self {
        self.graph.objects[self.index].layer = layer;
        self
    }

    pub fn parent(&mut self, index: Option<impl Into<ErasedDrawableNodeId>>) -> &mut Self {
        let index = index.map(Into::into);
        if let Some(parent_idx) = index {
            let parent_node = &mut self.graph.objects[parent_idx.0];
            if let Err(i) = parent_node.children.binary_search(&self.index) {
                parent_node.children.insert(i, self.index);
            }
        }

        let old_parent = mem::replace(
            &mut self.graph.objects[self.index].parent,
            index.map(|i| i.0),
        );

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

    pub fn y_sort(&mut self, enabled: bool) -> &mut Self {
        self.graph.objects[self.index].y_sorted = enabled;
        self
    }

    pub fn get(&self) -> DrawableNodeId<T> {
        DrawableId::new(self.index)
    }
}

struct Node {
    value: Box<dyn AnyDrawable>,
    layer: i32,
    y_sorted: bool,
    parent: Option<Index>,
    children: Vec<Index>,
}

#[derive(Debug)]
struct DrawableGraphInner {
    y_cache: HashMap<Index, OrderedFloat<f32>>,
    roots: Vec<Index>,
    sorted: Vec<Index>,
    buf: Vec<Index>,
    stack: Vec<Index>,
}

pub struct DrawableGraph {
    objects: Arena<Node>,
    inner: RwLock<DrawableGraphInner>,
    dirty: AtomicBool,
}

impl<T: AnyDrawable> ops::Index<DrawableNodeId<T>> for DrawableGraph {
    type Output = T;

    #[inline]
    fn index(&self, i: DrawableNodeId<T>) -> &Self::Output {
        self[ErasedDrawableNodeId::from(i)]
            .as_any()
            .downcast_ref()
            .unwrap()
    }
}

impl<T: AnyDrawable> ops::IndexMut<DrawableNodeId<T>> for DrawableGraph {
    #[inline]
    fn index_mut(&mut self, i: DrawableNodeId<T>) -> &mut Self::Output {
        self[ErasedDrawableNodeId::from(i)]
            .as_any_mut()
            .downcast_mut()
            .unwrap()
    }
}

impl ops::Index<ErasedDrawableNodeId> for DrawableGraph {
    type Output = dyn AnyDrawable;

    #[inline]
    fn index(&self, i: ErasedDrawableNodeId) -> &Self::Output {
        &*self.objects[i.0].value
    }
}

impl ops::IndexMut<ErasedDrawableNodeId> for DrawableGraph {
    #[inline]
    fn index_mut(&mut self, i: ErasedDrawableNodeId) -> &mut Self::Output {
        if matches!(self.objects[i.0].parent, Some(j) if self.objects[j].y_sorted) {
            *self.dirty.get_mut() = true;
        }
        &mut *self.objects[i.0].value
    }
}

impl DrawableGraph {
    pub fn new() -> Self {
        Self {
            objects: Arena::new(),
            inner: DrawableGraphInner {
                roots: Vec::new(),
                sorted: Vec::new(),

                y_cache: HashMap::new(),
                buf: Vec::new(),
                stack: Vec::new(),
            }
            .into(),
            dirty: AtomicBool::new(true),
        }
    }

    pub fn insert<T: AnyDrawable>(&mut self, value: T) -> DrawableNodeBuilder<T> {
        let index = self.objects.insert(Node {
            value: Box::new(value),
            layer: 0,
            y_sorted: false,
            parent: None,
            children: vec![],
        });

        *self.dirty.get_mut() = true;

        DrawableNodeBuilder {
            index,
            graph: self,
            marker: PhantomData,
        }
    }

    pub fn insert_any(
        &mut self,
        value: Box<dyn AnyDrawable>,
        layer: i32,
        y_sorted: bool,
        parent: Option<impl Into<ErasedDrawableNodeId>>,
    ) -> ErasedDrawableNodeId {
        let index = self.objects.insert(Node {
            value,
            layer,
            y_sorted,
            parent: parent.map(|t| t.into().0),
            children: vec![],
        });

        *self.dirty.get_mut() = true;

        ErasedDrawableNodeId::new(index)
    }

    pub fn set_parent(
        &mut self,
        object: impl Into<ErasedDrawableNodeId>,
        new_parent: Option<impl Into<ErasedDrawableNodeId>>,
    ) {
        let object = object.into();
        let new_parent = new_parent.map(Into::into);

        let old_parent = mem::replace(&mut self.objects[object.0].parent, new_parent.map(|t| t.0));

        if let Some(p) = old_parent {
            let old_parent_node = &mut self.objects[p];
            let i = old_parent_node
                .children
                .binary_search(&object.0)
                .expect("invalid scene graph");
            old_parent_node.children.remove(i);
        }

        if let Some(p) = new_parent {
            let new_parent_node = &mut self.objects[p.0];
            let i = new_parent_node
                .children
                .binary_search(&object.0)
                .expect_err("invalid scene graph");
            new_parent_node.children.insert(i, object.0);
        }

        *self.dirty.get_mut() = true;
    }

    pub fn set_layer(&mut self, object: impl Into<ErasedDrawableNodeId>, layer: i32) {
        let object = object.into();
        self.objects[object.0].layer = layer;
        *self.dirty.get_mut() = true;
    }

    pub fn remove<T: AnyDrawable>(&mut self, object: DrawableNodeId<T>) -> Option<T> {
        self.remove_any(object.into())
            .map(|boxed| *Box::<dyn Any>::downcast(boxed.to_box_any()).unwrap())
    }

    pub fn remove_any(&mut self, object: ErasedDrawableNodeId) -> Option<Box<dyn AnyDrawable>> {
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

        *self.dirty.get_mut() = true;

        Some(node.value)
    }

    pub fn children(
        &self,
        object: impl Into<ErasedDrawableNodeId>,
    ) -> impl Iterator<Item = (DrawableNodeId<dyn AnyDrawable>, &dyn AnyDrawable)> + '_ {
        self.objects[object.into().0]
            .children
            .iter()
            .map(move |&index| (DrawableId::new(index), &*self.objects[index].value))
    }

    pub fn sort(&self) {
        let Self {
            objects,
            inner,
            dirty,
        } = self;

        let DrawableGraphInner {
            y_cache,
            roots,
            sorted,
            stack,
            buf,
        } = &mut *inner.write().unwrap();

        if !dirty.load(atomic::Ordering::Acquire) {
            return;
        }

        roots.clear();
        for (index, node) in objects.iter() {
            if node.parent.is_none() {
                roots.push(index);
            }
        }
        roots.sort_by_key(|&root| objects[root].layer);

        y_cache.clear();
        sorted.clear();
        stack.clear();

        stack.extend(roots.iter().rev());
        while let Some(index) = stack.pop() {
            sorted.push(index);
            buf.clear();

            let object = &objects[index];
            buf.extend_from_slice(&object.children);

            if object.y_sorted {
                buf.sort_unstable_by(|&a, &b| {
                    let (obj_a, obj_b) = (&objects[a], &objects[b]);
                    obj_a.layer.cmp(&obj_b.layer).then_with(|| {
                        let a_y = *y_cache.entry(a).or_insert_with(|| {
                            OrderedFloat(obj_a.value.as_drawable().aabb().maxs.y)
                        });

                        let b_y = *y_cache.entry(b).or_insert_with(|| {
                            OrderedFloat(obj_b.value.as_drawable().aabb().maxs.y)
                        });

                        a_y.cmp(&b_y)
                    })
                });
            } else {
                buf.sort_by_key(|&k| objects[k].layer);
            }
            stack.extend(buf.drain(..).rev());
        }

        dirty.store(false, atomic::Ordering::Release);
    }

    pub fn sorted(&self) -> DrawableGraphIter {
        if self.dirty.load(atomic::Ordering::Relaxed) {
            self.sort();
        }

        let Self { objects, inner, .. } = self;
        let sorted = inner.read().unwrap();
        // Extend the lifetime of the iterator to the lifetime
        // of the read guard. Safe because we are guaranteed
        // nothing will move; there are immutable references
        // to the inner scene graph which are guaranteed to outlive
        // the iterator and read guard.
        let iter = unsafe {
            let inner_ptr = &*sorted as *const DrawableGraphInner;
            (*inner_ptr).sorted.iter()
        };

        DrawableGraphIter {
            _outer: sorted,
            inner: iter,
            objects,
        }
    }
}

impl Drawable for DrawableGraph {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        for drawable in self.sorted() {
            ctx.draw(drawable.as_drawable(), instance);
        }
    }

    fn aabb(&self) -> Box2<f32> {
        let mut aabb = Box2::invalid();
        for drawable in self.sorted() {
            aabb.merge(&drawable.aabb());
        }
        aabb
    }
}
