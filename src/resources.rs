use {
    anyhow::*,
    atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut},
    crossbeam_channel::{Receiver, Sender},
    derivative::*,
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    smallvec::SmallVec,
    std::{
        any::{Any, TypeId},
        cmp::Ordering,
        collections::BinaryHeap,
        fmt, iter,
        marker::PhantomData,
        ops,
        pin::Pin,
        ptr::NonNull,
        sync::Arc,
    },
    string_cache::DefaultAtom,
    thunderdome::{Arena, Index},
};

pub struct Fetch<'a, T>(AtomicRef<'a, T>);

impl<'a, T> Clone for Fetch<'a, T> {
    fn clone(&self) -> Self {
        Fetch(AtomicRef::clone(&self.0))
    }
}

impl<'a, T> ops::Deref for Fetch<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct FetchMut<'a, T>(AtomicRefMut<'a, T>);

impl<'a, T> ops::Deref for FetchMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T> ops::DerefMut for FetchMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// Implementation ripped from the `Box::downcast` method for `Box<dyn Any + 'static + Send>`
fn downcast_send_sync<T: Any>(
    this: Box<dyn Any + Send + Sync>,
) -> Result<Box<T>, Box<dyn Any + Send + Sync>> {
    <Box<dyn Any>>::downcast(this).map_err(|s| unsafe {
        // reapply the Send + Sync markers
        Box::from_raw(Box::into_raw(s) as *mut (dyn Any + Send + Sync))
    })
}

#[derive(Default, Derivative)]
#[derivative(Debug)]
pub struct Resources {
    #[derivative(Debug = "ignore")]
    map: HashMap<TypeId, AtomicRefCell<Box<dyn Any + Send + Sync>>>,
}

impl Resources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T: Any + Send + Sync>(&mut self, resource: T) -> Option<T> {
        let typeid = TypeId::of::<T>();
        let wrapped = AtomicRefCell::new(Box::new(resource) as Box<dyn Any + Send + Sync>);
        let maybe_old = self.map.insert(typeid, wrapped);

        maybe_old.map(|t| *downcast_send_sync(t.into_inner()).unwrap())
    }

    pub fn has_value<T: Any + Send + Sync>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    pub fn remove<T: Any + Send + Sync>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .map(|t| *downcast_send_sync(t.into_inner()).unwrap())
    }

    pub fn fetch<T: Any + Send>(&self) -> Fetch<T> {
        let borrow = self
            .map
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("no entry found for `{}`", std::any::type_name::<T>()))
            .borrow();
        Fetch(AtomicRef::map(borrow, |boxed| {
            boxed.downcast_ref().unwrap()
        }))
    }

    pub fn fetch_mut<T: Any + Send>(&self) -> FetchMut<T> {
        let borrow = self
            .map
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("no entry found for `{}`", std::any::type_name::<T>()))
            .borrow_mut();
        FetchMut(AtomicRefMut::map(borrow, |boxed| {
            boxed.downcast_mut().unwrap()
        }))
    }

    pub fn try_fetch<T: Any + Send>(&self) -> Option<Fetch<T>> {
        let borrow = self.map.get(&TypeId::of::<T>())?.borrow();
        Some(Fetch(AtomicRef::map(borrow, |boxed| {
            boxed.downcast_ref().unwrap()
        })))
    }

    pub fn try_fetch_mut<T: Any + Send>(&self) -> Option<FetchMut<T>> {
        let borrow = self.map.get(&TypeId::of::<T>())?.borrow_mut();
        Some(FetchMut(AtomicRefMut::map(borrow, |boxed| {
            boxed.downcast_mut().unwrap()
        })))
    }

    pub fn get_mut<T: Any + Send>(&mut self) -> Option<&mut T> {
        Some(
            self.map
                .get_mut(&TypeId::of::<T>())?
                .get_mut()
                .downcast_mut()
                .unwrap(),
        )
    }
}

pub struct SharedFetch<'a, T> {
    _outer: AtomicRef<'a, Resources>,
    inner: AtomicRef<'a, T>,
}

impl<'a, T> ops::Deref for SharedFetch<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct SharedFetchMut<'a, T> {
    _outer: AtomicRef<'a, Resources>,
    inner: AtomicRefMut<'a, T>,
}

impl<'a, T> ops::Deref for SharedFetchMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> ops::DerefMut for SharedFetchMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[derive(Clone)]
pub struct SharedResources {
    shared: Pin<Arc<AtomicRefCell<Resources>>>,
}

impl LuaUserData for SharedResources {}

impl From<Resources> for SharedResources {
    fn from(resources: Resources) -> Self {
        Self {
            shared: Arc::pin(AtomicRefCell::new(resources)),
        }
    }
}

impl SharedResources {
    pub fn new() -> Self {
        Self::from(Resources::new())
    }

    pub fn borrow(&self) -> AtomicRef<Resources> {
        self.shared.borrow()
    }

    pub fn borrow_mut(&self) -> AtomicRefMut<Resources> {
        self.shared.borrow_mut()
    }

    pub fn fetch<T: Any + Send + Sync>(&self) -> SharedFetch<T> {
        let outer = self.shared.borrow();
        let inner = unsafe {
            let inner_ptr = &*outer as *const Resources;
            (*inner_ptr).fetch::<T>().0
        };

        SharedFetch {
            inner,
            _outer: outer,
        }
    }

    pub fn fetch_mut<T: Any + Send>(&self) -> SharedFetchMut<T> {
        let outer = self.shared.borrow();
        let inner = unsafe {
            let inner_ptr = &*outer as *const Resources;
            (*inner_ptr).fetch_mut::<T>().0
        };

        SharedFetchMut {
            inner,
            _outer: outer,
        }
    }

    pub fn try_fetch<T: Any + Send + Sync>(&self) -> Option<SharedFetch<T>> {
        let outer = self.shared.borrow();
        let maybe_inner = unsafe {
            let inner_ptr = &*outer as *const Resources;
            (*inner_ptr).try_fetch::<T>().map(|fetch| fetch.0)
        };

        maybe_inner.map(|inner| SharedFetch {
            inner,
            _outer: outer,
        })
    }

    pub fn try_fetch_mut<T: Any + Send>(&self) -> Option<SharedFetchMut<T>> {
        let outer = self.shared.borrow();
        let maybe_inner = unsafe {
            let inner_ptr = &*outer as *const Resources;
            (*inner_ptr).try_fetch_mut::<T>().map(|fetch| fetch.0)
        };

        maybe_inner.map(|inner| SharedFetchMut {
            inner,
            _outer: outer,
        })
    }
}

#[derive(Debug, Copy, Clone)]
enum BorrowedResource<'a> {
    Mutable {
        pointer: NonNull<dyn Any + Send + Sync>,
        _marker: PhantomData<&'a mut ()>,
    },
    Immutable {
        pointer: NonNull<dyn Any + Send + Sync>,
        _marker: PhantomData<&'a ()>,
    },
}

#[derive(Debug)]
pub struct BorrowedRef<'a: 'b, 'b, T: ?Sized> {
    _borrow: AtomicRef<'b, BorrowedResource<'a>>,
    ptr: NonNull<T>,
}

impl<'a: 'b, 'b, T: ?Sized> Clone for BorrowedRef<'a, 'b, T> {
    fn clone(&self) -> Self {
        Self {
            _borrow: AtomicRef::clone(&self._borrow),
            ptr: self.ptr,
        }
    }
}

impl<'a: 'b, 'b, T: ?Sized> ops::Deref for BorrowedRef<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

unsafe impl<'a: 'b, 'b, T: ?Sized> Send for BorrowedRef<'a, 'b, T> where T: Sync {}
unsafe impl<'a: 'b, 'b, T: ?Sized> Sync for BorrowedRef<'a, 'b, T> where T: Sync {}

#[derive(Debug)]
pub struct BorrowedRefMut<'a: 'b, 'b, T: ?Sized> {
    _borrow: AtomicRefMut<'b, BorrowedResource<'a>>,
    ptr: NonNull<T>,
}

impl<'a: 'b, 'b, T: ?Sized> ops::Deref for BorrowedRefMut<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a: 'b, 'b, T: ?Sized> ops::DerefMut for BorrowedRefMut<'a, 'b, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

unsafe impl<'a: 'b, 'b, T: ?Sized> Send for BorrowedRefMut<'a, 'b, T> where T: Send {}
unsafe impl<'a: 'b, 'b, T: ?Sized> Sync for BorrowedRefMut<'a, 'b, T> where T: Sync {}

#[derive(Debug)]
pub struct BorrowedResources<'a> {
    values: HashMap<TypeId, AtomicRefCell<BorrowedResource<'a>>>,
}

impl<'a> BorrowedResources<'a> {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub fn insert_ref<T: Any + Send + Sync>(&mut self, res: &'a T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.values.contains_key(&type_id));
        let entry = BorrowedResource::Immutable {
            pointer: unsafe {
                NonNull::new_unchecked(res as &'a (dyn Any + Send + Sync) as *const _ as *mut _)
            },
            _marker: PhantomData,
        };
        self.values.insert(type_id, AtomicRefCell::new(entry));
    }

    pub fn insert_mut<T: Any + Send + Sync>(&mut self, res: &'a mut T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.values.contains_key(&type_id));
        let entry = BorrowedResource::Mutable {
            pointer: unsafe {
                NonNull::new_unchecked(res as &'a mut (dyn Any + Send + Sync) as *mut _)
            },
            _marker: PhantomData,
        };
        self.values.insert(type_id, AtomicRefCell::new(entry));
    }

    pub fn get<'b, T: Any + Send + Sync>(&'b self) -> Option<BorrowedRef<'a, 'b, T>> {
        let borrow = self.values.get(&TypeId::of::<T>())?.borrow();
        let ptr = unsafe {
            let any_ref = match &*borrow {
                BorrowedResource::Mutable { pointer, .. }
                | BorrowedResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>()? as *const _ as *mut _)
        };

        Some(BorrowedRef {
            _borrow: borrow,
            ptr,
        })
    }

    pub fn get_mut<'b, T: Any + Send + Sync>(&'b self) -> Option<BorrowedRefMut<'a, 'b, T>> {
        let mut borrow = self.values.get(&TypeId::of::<T>())?.borrow_mut();
        let ptr = unsafe {
            let any_ref = match &mut *borrow {
                BorrowedResource::Mutable { pointer, .. } => pointer.as_mut(),
                BorrowedResource::Immutable { .. } => return None,
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>()? as *mut _)
        };

        Some(BorrowedRefMut {
            _borrow: borrow,
            ptr,
        })
    }
}

unsafe impl<'a> Send for BorrowedResources<'a> {}
unsafe impl<'a> Sync for BorrowedResources<'a> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_resources() {
        let mut a = 5i32;
        let mut b: &'static str = "hello";
        let c = true;

        {
            let mut borrowed_resources = BorrowedResources::new();
            borrowed_resources.insert_mut(&mut a);
            borrowed_resources.insert_mut(&mut b);
            borrowed_resources.insert_ref(&c);

            {
                assert_eq!(
                    borrowed_resources.get::<i32>().as_deref().copied(),
                    Some(5i32)
                );

                *borrowed_resources
                    .get_mut::<&'static str>()
                    .as_deref_mut()
                    .unwrap() = "world";

                assert_eq!(
                    borrowed_resources.get::<bool>().as_deref().copied(),
                    Some(true)
                );
            }

            let _ = borrowed_resources;
        }

        assert_eq!(b, "world");
    }
}
