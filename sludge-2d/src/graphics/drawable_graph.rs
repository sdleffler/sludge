use {
    hashbrown::HashMap,
    ordered_float::OrderedFloat,
    sludge::{
        graphics::{Drawable, Graphics, InstanceParam},
        math::*,
    },
    std::{
        any::{self},
        cmp::Ordering,
        fmt,
        hash::{Hash, Hasher},
        marker::PhantomData,
        mem, ops,
        sync::{
            atomic::{self, AtomicBool},
            RwLock, RwLockReadGuard,
        },
    },
    thunderdome::{Arena, Index},
};

use crate::graphics::{AnyDrawable2, Drawable2};

pub struct Drawable2Id<T: AnyDrawable2 + ?Sized>(
    pub(crate) Index,
    pub(crate) PhantomData<&'static T>,
);

impl<T: AnyDrawable2 + ?Sized> fmt::Debug for Drawable2Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple(&format!("Drawable2Id<{}>", any::type_name::<T>(),))
            .field(&self.0)
            .finish()
    }
}

impl<T: AnyDrawable2 + ?Sized> Copy for Drawable2Id<T> {}
impl<T: AnyDrawable2 + ?Sized> Clone for Drawable2Id<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: AnyDrawable2 + ?Sized> PartialEq for Drawable2Id<T> {
    fn eq(&self, rhs: &Self) -> bool {
        self.0 == rhs.0
    }
}

impl<T: AnyDrawable2 + ?Sized> Eq for Drawable2Id<T> {}

impl<T: AnyDrawable2 + ?Sized> PartialOrd for Drawable2Id<T> {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.0.cmp(&rhs.0))
    }
}

impl<T: AnyDrawable2 + ?Sized> Ord for Drawable2Id<T> {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.0.cmp(&rhs.0)
    }
}

impl<T: AnyDrawable2 + ?Sized> Hash for Drawable2Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T: AnyDrawable2 + ?Sized> Drawable2Id<T> {
    pub(crate) fn new(index: Index) -> Self {
        Self(index, PhantomData)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErasedDrawable2Id(pub(crate) Index);

impl ErasedDrawable2Id {
    pub(crate) fn new(index: Index) -> Self {
        Self(index)
    }
}

impl<T: AnyDrawable2 + ?Sized> From<Drawable2Id<T>> for ErasedDrawable2Id {
    fn from(id: Drawable2Id<T>) -> ErasedDrawable2Id {
        Self::new(id.0)
    }
}

impl<T: AnyDrawable2 + ?Sized> From<Drawable2Id<T>> for Option<ErasedDrawable2Id> {
    fn from(id: Drawable2Id<T>) -> Option<ErasedDrawable2Id> {
        Some(ErasedDrawable2Id::new(id.0))
    }
}

#[derive(Debug)]
pub struct Entry<T: AnyDrawable2 + ?Sized> {
    pub tx: Transform3<f32>,
    pub value: T,
}

impl<T: AnyDrawable2 + ?Sized> ops::Deref for Entry<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: AnyDrawable2 + ?Sized> ops::DerefMut for Entry<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl Entry<dyn AnyDrawable2> {
    pub fn downcast<T: AnyDrawable2>(self: Box<Self>) -> Option<Entry<T>> {
        if self.value.as_any().is::<T>() {
            let raw = Box::into_raw(self);
            let boxed =
                unsafe { Box::from_raw(raw as *mut Entry<dyn AnyDrawable2> as *mut Entry<T>) };
            Some(*boxed)
        } else {
            None
        }
    }

    pub fn downcast_ref<T: AnyDrawable2>(&self) -> Option<&Entry<T>> {
        if self.value.as_any().is::<T>() {
            unsafe { Some(&*(self as *const Entry<dyn AnyDrawable2> as *const Entry<T>)) }
        } else {
            None
        }
    }

    pub fn downcast_mut<T: AnyDrawable2>(&mut self) -> Option<&mut Entry<T>> {
        if self.value.as_any().is::<T>() {
            unsafe { Some(&mut *(self as *mut Entry<dyn AnyDrawable2> as *mut Entry<T>)) }
        } else {
            None
        }
    }
}

pub struct DrawableGraphIter<'a> {
    _outer: RwLockReadGuard<'a, DrawableGraphInner>,
    inner: ::std::slice::Iter<'a, (Index, Transform3<f32>)>,
    objects: &'a Arena<Node>,
}

impl<'a> Iterator for DrawableGraphIter<'a> {
    type Item = (&'a dyn AnyDrawable2, &'a Transform3<f32>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(index, tx)| (&self.objects[*index].entry.value, tx))
    }
}

pub struct DrawableNodeBuilder<'a, T: AnyDrawable2> {
    index: Index,
    graph: &'a mut DrawableGraph,
    marker: PhantomData<&'a mut T>,
}

impl<'a, T: AnyDrawable2> DrawableNodeBuilder<'a, T> {
    pub fn layer(&mut self, layer: i32) -> &mut Self {
        self.graph.objects[self.index].layer = layer;
        self
    }

    pub fn parent(&mut self, index: impl Into<Option<ErasedDrawable2Id>>) -> &mut Self {
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

    #[inline]
    pub fn translate2(&mut self, v: Vector2<f32>) -> &mut Self {
        self.graph.objects[self.index].entry.tx *= Translation3::from(v.push(0.));
        self
    }

    #[inline]
    pub fn scale2(&mut self, v: Vector2<f32>) -> &mut Self {
        self.graph.objects[self.index].entry.tx *=
            Transform3::from_matrix_unchecked(Matrix3::from_diagonal(&v.push(1.)).to_homogeneous());
        self
    }

    pub fn get(&self) -> Drawable2Id<T> {
        Drawable2Id::new(self.index)
    }
}

struct Node {
    entry: Box<Entry<dyn AnyDrawable2>>,
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

impl<T: AnyDrawable2> ops::Index<Drawable2Id<T>> for DrawableGraph {
    type Output = Entry<T>;

    #[inline]
    fn index(&self, i: Drawable2Id<T>) -> &Self::Output {
        self[ErasedDrawable2Id::from(i)]
            .downcast_ref()
            .expect(std::any::type_name::<T>())
    }
}

impl<T: AnyDrawable2> ops::IndexMut<Drawable2Id<T>> for DrawableGraph {
    #[inline]
    fn index_mut(&mut self, i: Drawable2Id<T>) -> &mut Self::Output {
        self[ErasedDrawable2Id::from(i)]
            .downcast_mut()
            .expect(std::any::type_name::<T>())
    }
}

impl ops::Index<ErasedDrawable2Id> for DrawableGraph {
    type Output = Entry<dyn AnyDrawable2>;

    #[inline]
    fn index(&self, i: ErasedDrawable2Id) -> &Self::Output {
        &*self.objects[i.0].entry
    }
}

impl ops::IndexMut<ErasedDrawable2Id> for DrawableGraph {
    #[inline]
    fn index_mut(&mut self, i: ErasedDrawable2Id) -> &mut Self::Output {
        // FIXME(sleffy): two-kinded dirtiness (transform change of child only vs. full?)
        // if matches!(self.objects[i.0].parent, Some(j) if self.objects[j].y_sorted) {
        *self.dirty.get_mut() = true;
        // }
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

    pub fn insert<T: AnyDrawable2>(&mut self, value: T) -> DrawableNodeBuilder<T> {
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
        entry: Box<Entry<dyn AnyDrawable2>>,
        layer: i32,
        y_sorted: bool,
        parent: Option<impl Into<ErasedDrawable2Id>>,
    ) -> ErasedDrawable2Id {
        let index = self.objects.insert(Node {
            entry,
            layer,
            y_sorted,
            hidden: false,
            parent: parent.map(|t| t.into().0),
            children: vec![],
        });

        *self.dirty.get_mut() = true;

        ErasedDrawable2Id::new(index)
    }

    pub fn set_parent(
        &mut self,
        object: impl Into<ErasedDrawable2Id>,
        new_parent: Option<impl Into<ErasedDrawable2Id>>,
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

    pub fn set_layer(&mut self, object: impl Into<ErasedDrawable2Id>, layer: i32) {
        let object = &mut self.objects[object.into().0];
        *self.dirty.get_mut() |= object.layer != layer;
        object.layer = layer;
    }

    pub fn set_hidden(&mut self, object: impl Into<ErasedDrawable2Id>, hidden: bool) {
        let object = &mut self.objects[object.into().0];
        *self.dirty.get_mut() |= object.hidden != hidden;
        object.hidden = hidden;
    }

    pub fn remove<T: AnyDrawable2>(&mut self, object: Drawable2Id<T>) -> Option<T> {
        self.remove_any(object.into())
            .map(|boxed| boxed.downcast().unwrap().value)
    }

    pub fn remove_any(
        &mut self,
        object: ErasedDrawable2Id,
    ) -> Option<Box<Entry<dyn AnyDrawable2>>> {
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
        object: impl Into<ErasedDrawable2Id>,
    ) -> impl Iterator<Item = (Drawable2Id<dyn AnyDrawable2>, &dyn AnyDrawable2)> + '_ {
        self.objects[object.into().0]
            .children
            .iter()
            .map(move |&index| (Drawable2Id::new(index), &self.objects[index].entry.value))
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.load(atomic::Ordering::Relaxed)
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
                            let aabb = obj_a.entry.value.as_drawable2().aabb();
                            OrderedFloat(aabb.transformed_by(tx_a.matrix()).maxs.y)
                        });

                        let b_y = *y_cache.entry(b).or_insert_with(|| {
                            let aabb = obj_b.entry.value.as_drawable2().aabb();
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
}

impl Drawable2 for DrawableGraph {
    fn aabb(&self) -> Box2<f32> {
        let mut aabb = Box2::invalid();
        for (drawable, tx) in self.sorted() {
            aabb.merge(&drawable.aabb().transformed_by(&tx.matrix()));
        }
        aabb
    }
}
