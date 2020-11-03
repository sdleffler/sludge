use {
    anyhow::*,
    atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut},
    im::HashMap,
    rlua::prelude::*,
    std::{
        any::{self, Any, TypeId},
        marker::PhantomData,
        ops,
        pin::Pin,
        ptr::NonNull,
        sync::Arc,
    },
};

#[derive(Debug)]
pub struct Shared<'a, T: 'static> {
    inner: Arc<AtomicRefCell<StoredResource<'a>>>,
    _marker: PhantomData<T>,
}

impl<'a, T: 'static> Clone for Shared<'a, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: 'static> Shared<'a, T> {
    fn new(inner: Arc<AtomicRefCell<StoredResource<'a>>>) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }

    pub fn borrow(&self) -> Fetch<'a, '_, T> {
        let _borrow = self.inner.borrow();
        let ptr = unsafe {
            let any_ref = match &*_borrow {
                StoredResource::Owned { pointer } => &**pointer,
                StoredResource::Mutable { pointer, .. }
                | StoredResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>().unwrap() as *const _ as *mut _)
        };

        Fetch { _borrow, ptr }
    }

    pub fn borrow_mut(&self) -> FetchMut<'a, '_, T> {
        let mut _borrow = self.inner.borrow_mut();
        let ptr = unsafe {
            let any_ref = match &mut *_borrow {
                StoredResource::Owned { pointer } => &mut **pointer,
                StoredResource::Mutable { pointer, .. } => pointer.as_mut(),
                StoredResource::Immutable { .. } => {
                    panic!("cannot fetch immutably borrowed resource as mutable")
                }
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>().unwrap() as *mut _)
        };

        FetchMut { _borrow, ptr }
    }

    pub fn try_borrow(&self) -> Option<Fetch<'a, '_, T>> {
        let _borrow = self.inner.try_borrow().ok()?;
        let ptr = unsafe {
            let any_ref = match &*_borrow {
                StoredResource::Owned { pointer } => &**pointer,
                StoredResource::Mutable { pointer, .. }
                | StoredResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>().unwrap() as *const _ as *mut _)
        };

        Some(Fetch { _borrow, ptr })
    }

    pub fn try_borrow_mut(&self) -> Option<FetchMut<'a, '_, T>> {
        let mut _borrow = self.inner.try_borrow_mut().ok()?;
        let ptr = unsafe {
            let any_ref = match &mut *_borrow {
                StoredResource::Owned { pointer } => &mut **pointer,
                StoredResource::Mutable { pointer, .. } => pointer.as_mut(),
                StoredResource::Immutable { .. } => return None,
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>().unwrap() as *mut _)
        };

        Some(FetchMut { _borrow, ptr })
    }
}

#[derive(Debug)]
enum StoredResource<'a> {
    Owned {
        pointer: Box<dyn Any + Send + Sync>,
    },
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
pub struct Fetch<'a: 'b, 'b, T: ?Sized> {
    _borrow: AtomicRef<'b, StoredResource<'a>>,
    ptr: NonNull<T>,
}

impl<'a: 'b, 'b, T: ?Sized> Clone for Fetch<'a, 'b, T> {
    fn clone(&self) -> Self {
        Self {
            _borrow: AtomicRef::clone(&self._borrow),
            ptr: self.ptr,
        }
    }
}

impl<'a: 'b, 'b, T: ?Sized> ops::Deref for Fetch<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

unsafe impl<'a: 'b, 'b, T: ?Sized> Send for Fetch<'a, 'b, T> where T: Sync {}
unsafe impl<'a: 'b, 'b, T: ?Sized> Sync for Fetch<'a, 'b, T> where T: Sync {}

#[derive(Debug)]
pub struct FetchMut<'a: 'b, 'b, T: ?Sized> {
    _borrow: AtomicRefMut<'b, StoredResource<'a>>,
    ptr: NonNull<T>,
}

impl<'a: 'b, 'b, T: ?Sized> ops::Deref for FetchMut<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a: 'b, 'b, T: ?Sized> ops::DerefMut for FetchMut<'a, 'b, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

unsafe impl<'a: 'b, 'b, T: ?Sized> Send for FetchMut<'a, 'b, T> where T: Send {}
unsafe impl<'a: 'b, 'b, T: ?Sized> Sync for FetchMut<'a, 'b, T> where T: Sync {}

// Implementation ripped from the `Box::downcast` method for `Box<dyn Any + 'static + Send>`
fn downcast_send_sync<T: Any>(
    this: Box<dyn Any + Send + Sync>,
) -> Result<Box<T>, Box<dyn Any + Send + Sync>> {
    <Box<dyn Any>>::downcast(this).map_err(|s| unsafe {
        // reapply the Send + Sync markers
        Box::from_raw(Box::into_raw(s) as *mut (dyn Any + Send + Sync))
    })
}

#[derive(Debug)]
pub struct OwnedResources<'a> {
    map: HashMap<TypeId, Arc<AtomicRefCell<StoredResource<'a>>>>,
}

impl<'a> OwnedResources<'a> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn has_value<T: Any + Send + Sync>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    pub fn remove<T: Any + Send + Sync>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|t| Arc::try_unwrap(t).ok())
            .and_then(|t| match t.into_inner() {
                StoredResource::Owned { pointer } => Some(*downcast_send_sync(pointer).unwrap()),
                _ => None,
            })
    }

    pub fn insert<T: Any + Send + Sync + 'static>(&mut self, res: T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.map.contains_key(&type_id));
        let entry = StoredResource::Owned {
            pointer: Box::new(res),
        };
        self.map
            .insert(type_id, Arc::new(AtomicRefCell::new(entry)));
    }

    pub fn insert_ref<T: Any + Send + Sync>(&mut self, res: &'a T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.map.contains_key(&type_id));
        let entry = StoredResource::Immutable {
            pointer: unsafe {
                NonNull::new_unchecked(res as &'a (dyn Any + Send + Sync) as *const _ as *mut _)
            },
            _marker: PhantomData,
        };
        self.map
            .insert(type_id, Arc::new(AtomicRefCell::new(entry)));
    }

    pub fn insert_mut<T: Any + Send + Sync>(&mut self, res: &'a mut T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.map.contains_key(&type_id));
        let entry = StoredResource::Mutable {
            pointer: unsafe {
                NonNull::new_unchecked(res as &'a mut (dyn Any + Send + Sync) as *mut _)
            },
            _marker: PhantomData,
        };
        self.map
            .insert(type_id, Arc::new(AtomicRefCell::new(entry)));
    }

    pub fn fetch<'b, T: Any + Send + Sync>(&'b self) -> Fetch<'a, 'b, T> {
        let borrow = self
            .map
            .get(&TypeId::of::<T>())
            .expect("entry not found")
            .borrow();
        let ptr = unsafe {
            let any_ref = match &*borrow {
                StoredResource::Owned { pointer } => &**pointer,
                StoredResource::Mutable { pointer, .. }
                | StoredResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>().unwrap() as *const _ as *mut _)
        };

        Fetch {
            _borrow: borrow,
            ptr,
        }
    }

    pub fn fetch_mut<'b, T: Any + Send>(&'b self) -> FetchMut<'a, 'b, T> {
        let mut borrow = self
            .map
            .get(&TypeId::of::<T>())
            .expect("entry not found")
            .borrow_mut();
        let ptr = unsafe {
            let any_ref = match &mut *borrow {
                StoredResource::Owned { pointer } => &mut **pointer,
                StoredResource::Mutable { pointer, .. } => pointer.as_mut(),
                StoredResource::Immutable { .. } => {
                    panic!("cannot fetch immutably borrowed resource as mutable")
                }
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>().unwrap() as *mut _)
        };

        FetchMut {
            _borrow: borrow,
            ptr,
        }
    }

    pub fn try_fetch<'b, T: Any + Send + Sync>(&'b self) -> Option<Fetch<'a, 'b, T>> {
        let borrow = self.map.get(&TypeId::of::<T>())?.try_borrow().ok()?;
        let ptr = unsafe {
            let any_ref = match &*borrow {
                StoredResource::Owned { pointer } => &**pointer,
                StoredResource::Mutable { pointer, .. }
                | StoredResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>().unwrap() as *const _ as *mut _)
        };

        Some(Fetch {
            _borrow: borrow,
            ptr,
        })
    }

    pub fn try_fetch_mut<'b, T: Any + Send>(&'b self) -> Option<FetchMut<'a, 'b, T>> {
        let mut borrow = self.map.get(&TypeId::of::<T>())?.try_borrow_mut().ok()?;
        let ptr = unsafe {
            let any_ref = match &mut *borrow {
                StoredResource::Owned { pointer } => &mut **pointer,
                StoredResource::Mutable { pointer, .. } => pointer.as_mut(),
                StoredResource::Immutable { .. } => return None,
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>().unwrap() as *mut _)
        };

        Some(FetchMut {
            _borrow: borrow,
            ptr,
        })
    }

    pub fn fetch_shared<T: Any>(&self) -> Option<Shared<'a, T>> {
        self.map.get(&TypeId::of::<T>()).cloned().map(Shared::new)
    }

    pub fn get_mut<T: Any + Send + Sync>(&mut self) -> Option<&mut T> {
        match self
            .map
            .get_mut(&TypeId::of::<T>())
            .and_then(Arc::get_mut)?
            .get_mut()
        {
            StoredResource::Owned { pointer } => Some(pointer.downcast_mut().unwrap()),
            StoredResource::Mutable { pointer, .. } => {
                Some(unsafe { pointer.as_mut() }.downcast_mut().unwrap())
            }
            _ => None,
        }
    }
}

unsafe impl<'a> Send for OwnedResources<'a> {}
unsafe impl<'a> Sync for OwnedResources<'a> {}

pub struct SharedFetch<'a: 'b, 'b, T> {
    _outer: AtomicRef<'b, OwnedResources<'a>>,
    inner: Fetch<'a, 'b, T>,
}

impl<'a: 'b, 'b, T> ops::Deref for SharedFetch<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct SharedFetchMut<'a: 'b, 'b, T> {
    _outer: AtomicRef<'b, OwnedResources<'a>>,
    inner: FetchMut<'a, 'b, T>,
}

impl<'a: 'b, 'b, T> ops::Deref for SharedFetchMut<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a: 'b, 'b, T> ops::DerefMut for SharedFetchMut<'a, 'b, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[derive(Debug, Clone)]
pub struct SharedResources<'a> {
    shared: Pin<Arc<AtomicRefCell<OwnedResources<'a>>>>,
}

impl<'a> LuaUserData for SharedResources<'a> {}

impl<'a> From<OwnedResources<'a>> for SharedResources<'a> {
    fn from(resources: OwnedResources<'a>) -> Self {
        Self {
            shared: Arc::pin(AtomicRefCell::new(resources)),
        }
    }
}

impl<'a> Default for SharedResources<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SharedResources<'a> {
    pub fn new() -> Self {
        Self::from(OwnedResources::new())
    }
}

impl<'a> Resources<'a> for SharedResources<'a> {
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>> {
        self.shared.borrow()
    }

    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>> {
        self.shared.borrow_mut()
    }

    fn fetch<T: Any + Send + Sync>(&self) -> SharedFetch<'a, '_, T> {
        let outer = self.shared.borrow();
        let inner = unsafe {
            let inner_ptr = &*outer as *const OwnedResources;
            (*inner_ptr).fetch::<T>()
        };

        SharedFetch {
            inner,
            _outer: outer,
        }
    }

    fn fetch_mut<T: Any + Send>(&self) -> SharedFetchMut<'a, '_, T> {
        let outer = self.shared.borrow();
        let inner = unsafe {
            let inner_ptr = &*outer as *const OwnedResources;
            (*inner_ptr).fetch_mut::<T>()
        };

        SharedFetchMut {
            inner,
            _outer: outer,
        }
    }

    fn try_fetch<T: Any + Send + Sync>(&self) -> Option<SharedFetch<'a, '_, T>> {
        let outer = self.shared.try_borrow().ok()?;
        let inner = unsafe {
            let inner_ptr = &*outer as *const OwnedResources;
            (*inner_ptr).try_fetch::<T>()?
        };

        Some(SharedFetch {
            inner,
            _outer: outer,
        })
    }

    fn try_fetch_mut<T: Any + Send>(&self) -> Option<SharedFetchMut<'a, '_, T>> {
        let outer = self.shared.try_borrow().ok()?;
        let inner = unsafe {
            let inner_ptr = &*outer as *const OwnedResources;
            (*inner_ptr).try_fetch_mut::<T>()?
        };

        Some(SharedFetchMut {
            inner,
            _outer: outer,
        })
    }

    fn fetch_shared<T: Any>(&self) -> Option<Shared<'a, T>> {
        self.shared.borrow().fetch_shared::<T>()
    }
}

#[derive(Debug, Clone)]
pub struct UnifiedResources<'a> {
    pub local: SharedResources<'a>,
    pub global: SharedResources<'a>,
}

impl<'a> UnifiedResources<'a> {
    pub fn new() -> Self {
        Self {
            local: SharedResources::new(),
            global: SharedResources::new(),
        }
    }
}

impl<'a> Resources<'a> for UnifiedResources<'a> {
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>> {
        self.local.borrow()
    }

    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>> {
        self.local.borrow_mut()
    }

    fn fetch<T: Any + Send + Sync>(&self) -> SharedFetch<'a, '_, T> {
        match self.try_fetch::<T>() {
            Some(fetched) => fetched,
            None => panic!(
                "entry `{}` not found in local or global resources",
                any::type_name::<T>()
            ),
        }
    }

    fn fetch_mut<T: Any + Send>(&self) -> SharedFetchMut<'a, '_, T> {
        match self.try_fetch_mut::<T>() {
            Some(fetched) => fetched,
            None => panic!(
                "entry `{}` not found in local or global resources",
                any::type_name::<T>()
            ),
        }
    }

    fn try_fetch<T: Any + Send + Sync>(&self) -> Option<SharedFetch<'a, '_, T>> {
        self.local
            .try_fetch::<T>()
            .or_else(|| self.global.try_fetch::<T>())
    }

    fn try_fetch_mut<T: Any + Send>(&self) -> Option<SharedFetchMut<'a, '_, T>> {
        self.local
            .try_fetch_mut::<T>()
            .or_else(|| self.global.try_fetch_mut::<T>())
    }

    fn fetch_shared<T: Any>(&self) -> Option<Shared<'a, T>> {
        self.local
            .fetch_shared::<T>()
            .or_else(|| self.global.fetch_shared::<T>())
    }
}

impl<'a> LuaUserData for UnifiedResources<'a> {}

pub trait Resources<'a> {
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>>;
    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>>;
    fn fetch<T: Any + Send + Sync>(&self) -> SharedFetch<'a, '_, T>;
    fn fetch_mut<T: Any + Send>(&self) -> SharedFetchMut<'a, '_, T>;
    fn try_fetch<T: Any + Send + Sync>(&self) -> Option<SharedFetch<'a, '_, T>>;
    fn try_fetch_mut<T: Any + Send>(&self) -> Option<SharedFetchMut<'a, '_, T>>;
    fn fetch_shared<T: Any>(&self) -> Option<Shared<'a, T>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_resources() {
        let mut a = 5i32;
        let mut b: &'static str = "hello";
        let c = true;

        {
            let mut borrowed_resources = OwnedResources::new();
            borrowed_resources.insert_mut(&mut a);
            borrowed_resources.insert_mut(&mut b);
            borrowed_resources.insert_ref(&c);

            {
                assert_eq!(*borrowed_resources.fetch::<i32>(), 5i32);
                *borrowed_resources.fetch_mut::<&'static str>() = "world";
                assert_eq!(*borrowed_resources.fetch::<bool>(), true);
            }

            let _ = borrowed_resources;
        }

        assert_eq!(b, "world");
    }
}
