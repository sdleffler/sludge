use crate::{
    ecs::{ScContext, SmartComponent},
    graphics::{Drawable, Graphics, InstanceParam},
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

/// Shorthand trait for types that are `Drawable` and `Any`, as well as
/// `Send + Sync`. This is blanket-impl'd and you should never have to implement
/// it manually.
pub trait DrawableAny: Drawable + Any + Send + Sync {
    #[doc(hidden)]
    fn as_any(&self) -> &dyn Any;

    #[doc(hidden)]
    fn as_any_mut(&mut self) -> &mut dyn Any;

    #[doc(hidden)]
    fn to_box_any(self: Box<Self>) -> Box<dyn Any>;

    #[doc(hidden)]
    fn as_drawable(&self) -> &dyn Drawable;
}

impl<T: Drawable + Any + Send + Sync> DrawableAny for T {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self as &mut dyn Any
    }

    fn to_box_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }

    fn as_drawable(&self) -> &dyn Drawable {
        self as &dyn Drawable
    }
}

#[derive(Debug)]
pub struct DrawableId<T: DrawableAny + ?Sized>(Index, PhantomData<T>);

impl<T: DrawableAny + ?Sized> Copy for DrawableId<T> {}
impl<T: DrawableAny + ?Sized> Clone for DrawableId<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T: DrawableAny> SmartComponent<ScContext<'a>> for DrawableId<T> {}

pub struct SceneGraphIter<'a> {
    _outer: RwLockReadGuard<'a, SceneGraphInner>,
    inner: ::std::slice::Iter<'a, Index>,
    objects: &'a Arena<Node>,
}

impl<'a> Iterator for SceneGraphIter<'a> {
    type Item = &'a dyn DrawableAny;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|&index| self.objects[index].value.as_ref())
    }
}

pub struct DrawableNodeBuilder<'a, T: DrawableAny> {
    index: Index,
    graph: &'a mut DrawableGraph,
    marker: PhantomData<&'a mut T>,
}

impl<'a, T: DrawableAny> DrawableNodeBuilder<'a, T> {
    pub fn layer(&mut self, layer: i32) -> &mut Self {
        self.graph.objects[self.index].layer = layer;
        self
    }

    pub fn parent<U: DrawableAny>(&mut self, index: DrawableId<U>) -> &mut Self {
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

    pub fn get(&self) -> DrawableId<T> {
        DrawableId(self.index, PhantomData)
    }
}

struct Node {
    value: Box<dyn DrawableAny>,
    layer: i32,
    parent: Option<Index>,
    children: Vec<Index>,
}

#[derive(Debug)]
struct SceneGraphInner {
    y_cache: HashMap<Index, OrderedFloat<f32>>,
    roots: Vec<Index>,
    sorted: Vec<Index>,
    buf: Vec<Index>,
    stack: Vec<Index>,
}

pub struct DrawableGraph {
    objects: Arena<Node>,
    inner: RwLock<SceneGraphInner>,
    dirty: AtomicBool,
    y_sort: bool,
}

impl<T: DrawableAny> ops::Index<DrawableId<T>> for DrawableGraph {
    type Output = T;

    fn index(&self, i: DrawableId<T>) -> &Self::Output {
        self.objects[i.0].value.as_any().downcast_ref().unwrap()
    }
}

impl<T: DrawableAny> ops::IndexMut<DrawableId<T>> for DrawableGraph {
    fn index_mut(&mut self, i: DrawableId<T>) -> &mut Self::Output {
        if self.y_sort {
            *self.dirty.get_mut() = true;
        }
        self.objects[i.0].value.as_any_mut().downcast_mut().unwrap()
    }
}

impl DrawableGraph {
    pub fn new() -> Self {
        Self {
            objects: Arena::new(),
            inner: SceneGraphInner {
                roots: Vec::new(),
                sorted: Vec::new(),

                y_cache: HashMap::new(),
                buf: Vec::new(),
                stack: Vec::new(),
            }
            .into(),
            dirty: AtomicBool::new(true),
            y_sort: false,
        }
    }

    pub fn set_y_sort_enabled(&mut self, enabled: bool) {
        self.y_sort = enabled;
    }

    pub fn insert<T: DrawableAny>(&mut self, value: T) -> DrawableNodeBuilder<T> {
        let index = self.objects.insert(Node {
            value: Box::new(value),
            layer: 0,
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

    pub fn remove<T: DrawableAny>(&mut self, object: DrawableId<T>) -> Option<T> {
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

        Some(*Box::<dyn Any>::downcast(node.value.to_box_any()).unwrap())
    }

    pub fn sort(&self) {
        let Self {
            objects,
            inner,
            dirty,
            y_sort,
        } = self;

        let SceneGraphInner {
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
            buf.extend_from_slice(&objects[index].children);

            if *y_sort {
                buf.sort_by(|&a, &b| {
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

    pub fn sorted(&self) -> SceneGraphIter {
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
            let inner_ptr = &*sorted as *const SceneGraphInner;
            (*inner_ptr).sorted.iter()
        };

        SceneGraphIter {
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

    fn aabb(&self) -> AABB<f32> {
        let mut aabb = AABB::new_invalid();
        for drawable in self.sorted() {
            aabb.merge(&drawable.aabb());
        }
        aabb
    }
}
