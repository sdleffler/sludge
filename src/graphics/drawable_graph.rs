use crate::{
    graphics::{AnyDrawable, Drawable, DrawableId, ErasedDrawableId, Graphics, InstanceParam},
    math::*,
};
use {
    hashbrown::HashMap,
    ordered_float::OrderedFloat,
    std::{
        marker::PhantomData,
        mem, ops,
        sync::{
            atomic::{self, AtomicBool},
            RwLock, RwLockReadGuard,
        },
    },
    thunderdome::{Arena, Index},
};

#[derive(Debug)]
pub struct Entry<T: AnyDrawable + ?Sized> {
    pub tx: Transform3<f32>,
    pub value: T,
}

impl<T: AnyDrawable + ?Sized> ops::Deref for Entry<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: AnyDrawable + ?Sized> ops::DerefMut for Entry<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl Entry<dyn AnyDrawable> {
    pub fn downcast<T: AnyDrawable>(self: Box<Self>) -> Option<Entry<T>> {
        if self.value.as_any().is::<T>() {
            let raw = Box::into_raw(self);
            let boxed =
                unsafe { Box::from_raw(raw as *mut Entry<dyn AnyDrawable> as *mut Entry<T>) };
            Some(*boxed)
        } else {
            None
        }
    }

    pub fn downcast_ref<T: AnyDrawable>(&self) -> Option<&Entry<T>> {
        if self.value.as_any().is::<T>() {
            unsafe { Some(&*(self as *const Entry<dyn AnyDrawable> as *const Entry<T>)) }
        } else {
            None
        }
    }

    pub fn downcast_mut<T: AnyDrawable>(&mut self) -> Option<&mut Entry<T>> {
        if self.value.as_any().is::<T>() {
            unsafe { Some(&mut *(self as *mut Entry<dyn AnyDrawable> as *mut Entry<T>)) }
        } else {
            None
        }
    }
}

pub type DrawableNodeId<T> = DrawableId<T, DrawableGraph>;
pub type ErasedDrawableNodeId = ErasedDrawableId<DrawableGraph>;

pub struct DrawableGraphIter<'a> {
    _outer: RwLockReadGuard<'a, DrawableGraphInner>,
    inner: ::std::slice::Iter<'a, (Index, Transform3<f32>)>,
    objects: &'a Arena<Node>,
}

impl<'a> Iterator for DrawableGraphIter<'a> {
    type Item = (&'a dyn AnyDrawable, &'a Transform3<f32>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(index, tx)| (&self.objects[*index].entry.value, tx))
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

    pub fn parent(&mut self, index: impl Into<Option<ErasedDrawableNodeId>>) -> &mut Self {
        let index = index.into();
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
    entry: Box<Entry<dyn AnyDrawable>>,
    layer: i32,
    y_sorted: bool,
    hidden: bool,
    parent: Option<Index>,
    children: Vec<Index>,
}

#[derive(Debug)]
struct DrawableGraphInner {
    y_cache: HashMap<Index, OrderedFloat<f32>>,
    roots: Vec<(Index, Transform3<f32>)>,
    sorted: Vec<(Index, Transform3<f32>)>,
    buf: Vec<(Index, Transform3<f32>)>,
    stack: Vec<(Index, Transform3<f32>)>,
}

pub struct DrawableGraph {
    objects: Arena<Node>,
    inner: RwLock<DrawableGraphInner>,
    dirty: AtomicBool,
}

impl<T: AnyDrawable> ops::Index<DrawableNodeId<T>> for DrawableGraph {
    type Output = Entry<T>;

    #[inline]
    fn index(&self, i: DrawableNodeId<T>) -> &Self::Output {
        self[ErasedDrawableNodeId::from(i)]
            .downcast_ref()
            .expect(std::any::type_name::<T>())
    }
}

impl<T: AnyDrawable> ops::IndexMut<DrawableNodeId<T>> for DrawableGraph {
    #[inline]
    fn index_mut(&mut self, i: DrawableNodeId<T>) -> &mut Self::Output {
        self[ErasedDrawableNodeId::from(i)]
            .downcast_mut()
            .expect(std::any::type_name::<T>())
    }
}

impl ops::Index<ErasedDrawableNodeId> for DrawableGraph {
    type Output = Entry<dyn AnyDrawable>;

    #[inline]
    fn index(&self, i: ErasedDrawableNodeId) -> &Self::Output {
        &*self.objects[i.0].entry
    }
}

impl ops::IndexMut<ErasedDrawableNodeId> for DrawableGraph {
    #[inline]
    fn index_mut(&mut self, i: ErasedDrawableNodeId) -> &mut Self::Output {
        if matches!(self.objects[i.0].parent, Some(j) if self.objects[j].y_sorted) {
            *self.dirty.get_mut() = true;
        }
        &mut *self.objects[i.0].entry
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
            entry: Box::new(Entry {
                tx: Transform3::identity(),
                value,
            }),
            layer: 0,
            y_sorted: false,
            hidden: false,
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

    pub fn insert_entry(
        &mut self,
        entry: Box<Entry<dyn AnyDrawable>>,
        layer: i32,
        y_sorted: bool,
        parent: Option<impl Into<ErasedDrawableNodeId>>,
    ) -> ErasedDrawableNodeId {
        let index = self.objects.insert(Node {
            entry,
            layer,
            y_sorted,
            hidden: false,
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
        let object = &mut self.objects[object.into().0];
        *self.dirty.get_mut() |= object.layer != layer;
        object.layer = layer;
    }

    pub fn set_hidden(&mut self, object: impl Into<ErasedDrawableNodeId>, hidden: bool) {
        let object = &mut self.objects[object.into().0];
        *self.dirty.get_mut() |= object.hidden != hidden;
        object.hidden = hidden;
    }

    pub fn remove<T: AnyDrawable>(&mut self, object: DrawableNodeId<T>) -> Option<T> {
        self.remove_any(object.into())
            .map(|boxed| boxed.downcast().unwrap().value)
    }

    pub fn remove_any(
        &mut self,
        object: ErasedDrawableNodeId,
    ) -> Option<Box<Entry<dyn AnyDrawable>>> {
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

        Some(node.entry)
    }

    pub fn children(
        &self,
        object: impl Into<ErasedDrawableNodeId>,
    ) -> impl Iterator<Item = (DrawableNodeId<dyn AnyDrawable>, &dyn AnyDrawable)> + '_ {
        self.objects[object.into().0]
            .children
            .iter()
            .map(move |&index| (DrawableId::new(index), &self.objects[index].entry.value))
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
            if node.parent.is_none() && !node.hidden {
                roots.push((index, node.entry.tx));
            }
        }
        roots.sort_by_key(|&(root, _)| objects[root].layer);

        y_cache.clear();
        sorted.clear();
        stack.clear();

        stack.extend(roots.iter().rev());

        while let Some((index, tx)) = stack.pop() {
            let object = &objects[index];

            if object.hidden {
                continue;
            }

            sorted.push((index, tx));
            buf.clear();

            buf.extend(
                object
                    .children
                    .iter()
                    .map(|&child| (child, tx * objects[child].entry.tx)),
            );

            if object.y_sorted {
                buf.sort_unstable_by(|&(a, tx_a), &(b, tx_b)| {
                    let (obj_a, obj_b) = (&objects[a], &objects[b]);
                    obj_a.layer.cmp(&obj_b.layer).then_with(|| {
                        let a_y = *y_cache.entry(a).or_insert_with(|| {
                            let aabb = obj_a.entry.value.as_drawable().aabb2();
                            OrderedFloat(aabb.transformed_by(tx_a.matrix()).maxs.y)
                        });

                        let b_y = *y_cache.entry(b).or_insert_with(|| {
                            let aabb = obj_b.entry.value.as_drawable().aabb2();
                            OrderedFloat(aabb.transformed_by(tx_b.matrix()).maxs.y)
                        });

                        a_y.cmp(&b_y)
                    })
                });
            } else {
                buf.sort_by_key(|&(k, _)| objects[k].layer);
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
        for (drawable, tx) in self.sorted() {
            ctx.draw(drawable.as_drawable(), instance.prepend_transform(tx));
        }
    }

    fn aabb2(&self) -> Box2<f32> {
        let mut aabb = Box2::invalid();
        for (drawable, tx) in self.sorted() {
            aabb.merge(&drawable.aabb2().transformed_by(&tx.matrix()));
        }
        aabb
    }
}
