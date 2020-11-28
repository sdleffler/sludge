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
    thiserror::Error,
};

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("resource of type `{0}` not found")]
    NotFound(String),
}

impl FetchError {
    pub fn not_found<T: Any + Send + Sync>() -> Self {
        Self::NotFound(any::type_name::<T>().to_owned())
    }
}

pub type FetchResult<T> = Result<T, FetchError>;

impl From<FetchError> for LuaError {
    fn from(ferr: FetchError) -> Self {
        LuaError::external(ferr)
    }
}

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

    pub fn fetch_one<T: Any + Send + Sync>(&self) -> FetchResult<Shared<'a, T>> {
        let maybe_shared = self.map.get(&TypeId::of::<T>()).cloned().map(Shared::new);
        maybe_shared.ok_or_else(|| FetchError::not_found::<T>())
    }

    pub fn fetch<T: FetchAll<'a>>(&self) -> FetchResult<T::Fetched> {
        T::fetch_components_owned(self)
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

    fn fetch_one<T: Any + Send + Sync>(&self) -> FetchResult<Shared<'a, T>> {
        self.shared.borrow().fetch_one()
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

    fn fetch_one<T: Any + Send + Sync>(&self) -> FetchResult<Shared<'a, T>> {
        self.local
            .fetch_one::<T>()
            .or_else(|_| self.global.fetch_one::<T>())
    }
}

impl<'a> LuaUserData for UnifiedResources<'a> {}

pub trait Fetchable: Any + Send + Sync {}
impl<T> Fetchable for T where T: Any + Send + Sync {}

pub trait Resources<'a> {
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>>;
    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>>;
    fn fetch_one<T: Any + Send + Sync>(&self) -> FetchResult<Shared<'a, T>>;

    fn fetch<T: FetchAll<'a>>(&self) -> FetchResult<T::Fetched> {
        T::fetch_components(self)
    }
}

pub trait FetchAll<'a> {
    type Fetched;
    fn fetch_components<R>(resources: &R) -> FetchResult<Self::Fetched>
    where
        R: Resources<'a> + ?Sized;

    fn fetch_components_owned(resources: &OwnedResources<'a>) -> FetchResult<Self::Fetched>;
}

macro_rules! impl_tuple {
    ($($id:ident),*) => {
        #[allow(non_snake_case)]
        impl<'a, $($id: Fetchable),*> FetchAll<'a> for ($($id,)*) {
            type Fetched = ($(Shared<'a, $id>,)*);
            fn fetch_components<Res>(_resources: &Res) -> FetchResult<Self::Fetched>
                where Res: Resources<'a> + ?Sized
            {
                $(let $id = _resources.fetch_one()?;)*
                Ok(($($id,)*))
            }

            fn fetch_components_owned(_resources: &OwnedResources<'a>) -> FetchResult<Self::Fetched> {
                $(let $id = _resources.fetch_one()?;)*
                Ok(($($id,)*))
            }
        }
    };
}

macro_rules! impl_all_tuples {
    ($t:ident $(, $ts:ident)*) => {
        impl_tuple!($t $(, $ts)*);
        impl_all_tuples!($($ts),*);
    };
    () => {
        impl_tuple!();
    };
}

impl_all_tuples!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_resources() -> Result<()> {
        let mut a = 5i32;
        let mut b: &'static str = "hello";
        let c = true;

        {
            let mut borrowed_resources = OwnedResources::new();
            borrowed_resources.insert_mut(&mut a);
            borrowed_resources.insert_mut(&mut b);
            borrowed_resources.insert_ref(&c);

            let shared_a = borrowed_resources.fetch_one::<i32>()?;
            let shared_b = borrowed_resources.fetch_one::<&'static str>()?;
            let shared_c = borrowed_resources.fetch_one::<bool>()?;

            {
                assert_eq!(*shared_a.borrow(), 5i32);
                *shared_b.borrow_mut() = "world";
                assert_eq!(*shared_c.borrow(), true);
            }

            let _ = borrowed_resources;
        }

        assert_eq!(b, "world");

        Ok(())
    }
}
