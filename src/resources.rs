use {
    atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut},
    derivative::Derivative,
    hashbrown::HashMap,
    std::{
        any::{Any, TypeId},
        ops,
        pin::Pin,
        sync::Arc,
    },
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

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Resources {
    #[derivative(Debug = "ignore")]
    map: HashMap<TypeId, AtomicRefCell<Box<dyn Any + Send>>>,
}

impl Resources {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn insert<T: Any + Send>(&mut self, resource: T) -> Option<T> {
        let typeid = TypeId::of::<T>();
        let wrapped = AtomicRefCell::new(Box::new(resource) as Box<dyn Any + Send>);
        let maybe_old = self.map.insert(typeid, wrapped);

        maybe_old.map(|t| *t.into_inner().downcast().unwrap())
    }

    pub fn remove<T: Any + Send>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .map(|t| *t.into_inner().downcast().unwrap())
    }

    pub fn fetch<T: Any + Send>(&self) -> Fetch<T> {
        let borrow = self.map[&TypeId::of::<T>()].borrow();
        Fetch(AtomicRef::map(borrow, |boxed| {
            boxed.downcast_ref().unwrap()
        }))
    }

    pub fn fetch_mut<T: Any + Send>(&self) -> FetchMut<T> {
        let borrow = self.map[&TypeId::of::<T>()].borrow_mut();
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
