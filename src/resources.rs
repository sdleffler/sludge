use {
    atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut},
    derivative::Derivative,
    hashbrown::HashMap,
    std::{
        any::{Any, TypeId},
        ops,
    },
};

pub struct Fetch<'a, T> {
    inner: AtomicRef<'a, T>,
}

impl<'a, T> Clone for Fetch<'a, T> {
    fn clone(&self) -> Self {
        Self {
            inner: AtomicRef::clone(&self.inner),
        }
    }
}

impl<'a, T> ops::Deref for Fetch<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct FetchMut<'a, T> {
    inner: AtomicRefMut<'a, T>,
}

impl<'a, T> ops::Deref for FetchMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> ops::DerefMut for FetchMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
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
        let mapped = AtomicRef::map(borrow, |boxed| boxed.downcast_ref().unwrap());
        Fetch { inner: mapped }
    }

    pub fn fetch_mut<T: Any + Send>(&self) -> FetchMut<T> {
        let borrow = self.map[&TypeId::of::<T>()].borrow_mut();
        let mapped = AtomicRefMut::map(borrow, |boxed| boxed.downcast_mut().unwrap());
        FetchMut { inner: mapped }
    }

    pub fn try_fetch<T: Any + Send>(&self) -> Option<Fetch<T>> {
        let borrow = self.map.get(&TypeId::of::<T>())?.borrow();
        let mapped = AtomicRef::map(borrow, |boxed| boxed.downcast_ref().unwrap());
        Some(Fetch { inner: mapped })
    }

    pub fn try_fetch_mut<T: Any + Send>(&self) -> Option<FetchMut<T>> {
        let borrow = self.map.get(&TypeId::of::<T>())?.borrow_mut();
        let mapped = AtomicRefMut::map(borrow, |boxed| boxed.downcast_mut().unwrap());
        Some(FetchMut { inner: mapped })
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
