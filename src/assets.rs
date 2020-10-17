use crate::{
    ecs::{Entity, ScContext, SmartComponent},
    Resources, UnifiedResources,
};
use {
    anyhow::*,
    arc_swap::ArcSwap,
    hashbrown::{HashMap, HashSet},
    std::{
        any::{Any, TypeId},
        marker::PhantomData,
        ops,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    },
};

pub type DefaultCache = Cache<'static, UnifiedResources<'static>>;

pub struct Loaded<T> {
    pub deps: Vec<Key>,
    pub value: T,
}

impl<T> Loaded<T> {
    pub fn new(value: T) -> Self {
        Self {
            deps: Vec::new(),
            value,
        }
    }

    pub fn with_deps(value: T, deps: Vec<Key>) -> Self {
        Self { value, deps }
    }
}

impl<T> From<T> for Loaded<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

pub trait Asset: Send + Sync + 'static + Sized {
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>>;
}

#[derive(Debug)]
pub struct Guard<'a, T>(arc_swap::Guard<'a, Arc<T>>);

impl<'a, T> ops::Deref for Guard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

#[derive(Debug)]
pub struct Cached<T: Send + Sync>(arc_swap::Cache<Arc<ArcSwap<T>>, Arc<T>>);

impl<T: Send + Sync> Clone for Cached<T> {
    fn clone(&self) -> Self {
        Cached(self.0.clone())
    }
}

impl<T: Send + Sync> From<T> for Cached<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Send + Sync> Cached<T> {
    pub fn new(value: T) -> Self {
        Cached(arc_swap::Cache::new(Arc::new(ArcSwap::from_pointee(value))))
    }

    pub fn load(&self) -> Guard<'static, T> {
        Guard(self.0.arc_swap().load())
    }

    pub fn load_cached(&mut self) -> &T {
        &**self.0.load()
    }
}

impl<'a, T: SmartComponent<ScContext<'a>>> SmartComponent<ScContext<'a>> for Cached<T> {
    fn on_borrow(&self, id: Entity, x: ScContext<'a>) {
        T::on_borrow(&*self.load(), id, x)
    }

    fn on_borrow_mut(&mut self, id: Entity, x: ScContext<'a>) {
        T::on_borrow(self.load_cached(), id, x)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Path(PathBuf),
}

impl Key {
    pub fn from_path<P: AsRef<Path> + ?Sized>(path: &P) -> Self {
        Self::Path(path.as_ref().to_owned())
    }
}

#[derive(Default)]
struct KeyEntry {
    types: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

pub struct Cache<'a, R: Resources<'a>> {
    resources: R,
    entries: Mutex<HashMap<Key, KeyEntry>>,
    dependencies: Mutex<HashMap<Key, HashSet<Key>>>,
    _marker: PhantomData<&'a ()>,
}

impl<'a, R: Resources<'a>> Cache<'a, R> {
    pub fn new(resources: R) -> Self {
        Self {
            resources,
            entries: Mutex::new(HashMap::new()),
            dependencies: Mutex::new(HashMap::new()),
            _marker: PhantomData,
        }
    }

    pub fn get<T>(&self, key: &Key) -> Result<Cached<T>>
    where
        T: Asset,
    {
        // Scope here to ensure that `entries` isn't locked while we `load` the asset,
        // if necessary.
        {
            let entries = self.entries.lock().unwrap();
            if let Some(cached) = entries
                .get(key)
                .and_then(|e| e.types.get(&TypeId::of::<T>()))
            {
                let downcast = cached.clone().downcast::<ArcSwap<T>>().unwrap();
                return Ok(Cached(arc_swap::Cache::new(downcast)));
            }
        }

        let loaded = T::load(key, self, &self.resources)?;
        if !loaded.deps.is_empty() {
            let mut dependencies = self.dependencies.lock().unwrap();
            dependencies
                .entry(key.clone())
                .or_default()
                .extend(loaded.deps);
        }
        let wrapped = Arc::new(ArcSwap::from_pointee(loaded.value));

        let mut entries = self.entries.lock().unwrap();
        entries.entry(key.clone()).or_default().types.insert(
            TypeId::of::<T>(),
            wrapped.clone() as Arc<dyn Any + Send + Sync>,
        );

        Ok(Cached(arc_swap::Cache::new(wrapped)))
    }
}
