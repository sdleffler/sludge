use crate::graphics::*;
use {
    ordered_float::OrderedFloat,
    std::{
        ops,
        sync::{RwLock, RwLockReadGuard},
    },
    thunderdome::{Arena, Index},
};

pub type SortedLayerId<T> = DrawableId<T, SortedLayer>;

pub struct SortedLayerIter<'a> {
    _outer: RwLockReadGuard<'a, Vec<Index>>,
    inner: ::std::slice::Iter<'a, Index>,
    objects: &'a Arena<Box<dyn DrawableAny>>,
}

impl<'a> Iterator for SortedLayerIter<'a> {
    type Item = &'a dyn DrawableAny;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|&index| self.objects[index].as_ref())
    }
}

pub struct SortedLayer {
    objects: Arena<Box<dyn DrawableAny>>,
    sorted: RwLock<Vec<Index>>,
}

impl<T: DrawableAny> ops::Index<SortedLayerId<T>> for SortedLayer {
    type Output = T;

    fn index(&self, id: SortedLayerId<T>) -> &Self::Output {
        self.objects[id.0].as_any().downcast_ref().unwrap()
    }
}

impl<T: DrawableAny> ops::IndexMut<SortedLayerId<T>> for SortedLayer {
    fn index_mut(&mut self, id: SortedLayerId<T>) -> &mut Self::Output {
        self.objects[id.0].as_any_mut().downcast_mut().unwrap()
    }
}

impl SortedLayer {
    pub fn new() -> Self {
        Self {
            objects: Arena::new(),
            sorted: RwLock::new(Vec::new()),
        }
    }

    pub fn insert<T: DrawableAny>(&mut self, drawable: T) -> SortedLayerId<T> {
        let index = self.objects.insert(Box::new(drawable));
        self.sorted.get_mut().unwrap().push(index);
        DrawableId::new(index)
    }

    pub fn remove<T: DrawableAny>(&mut self, id: SortedLayerId<T>) -> Option<T> {
        self.objects
            .remove(id.0)
            .and_then(<dyn DrawableAny>::downcast)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn DrawableAny> + '_ {
        self.objects.iter().map(|(_, boxed)| boxed.as_ref())
    }

    pub fn sort(&self) {
        let sorted = &mut *match self.sorted.try_write() {
            Ok(guard) => guard,
            Err(_) => return,
        };

        sorted.retain(|&id| self.objects.get(id).is_some());
        sorted.sort_by_cached_key(|&index| {
            OrderedFloat(self.objects[index].as_drawable().aabb().maxs.y)
        });
    }

    pub fn sorted(&self) -> SortedLayerIter {
        self.sort();
        let Self {
            objects, sorted, ..
        } = self;
        let sorted = sorted.read().unwrap();
        // Extend the lifetime of the iterator to the lifetime
        // of the read guard. Safe because we are guaranteed
        // nothing will move; there are immutable references
        // to the inner scene graph which are guaranteed to outlive
        // the iterator and read guard.
        let iter = unsafe {
            let inner_ptr = &*sorted as *const Vec<Index>;
            (*inner_ptr).iter()
        };

        SortedLayerIter {
            _outer: sorted,
            inner: iter,
            objects,
        }
    }
}

impl Drawable for SortedLayer {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        for drawable in self.sorted() {
            ctx.draw(drawable.as_drawable(), instance);
        }
    }

    fn aabb(&self) -> AABB<f32> {
        let mut aabb = AABB::new_invalid();
        for drawable in self.objects.iter().map(|(_, obj)| obj.as_drawable()) {
            aabb.merge(&drawable.aabb());
        }
        aabb
    }
}
