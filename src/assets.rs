use crate::{
    ecs::{Entity, ScContext, SmartComponent},
    Resources, UnifiedResources,
};
use {
    anyhow::*,
    arc_swap::ArcSwap,
    hashbrown::{HashMap, HashSet},
    serde::{de::DeserializeOwned, *},
    serde_hashkey::OrderedFloatPolicy,
    std::{
        any::{self, Any, TypeId},
        borrow::Cow,
        fmt,
        marker::PhantomData,
        ops,
        path::{Path, PathBuf},
        sync::{Arc, Condvar, Mutex},
        thread::{self, ThreadId},
    },
};

pub type DefaultCache = Cache<'static, UnifiedResources<'static>>;

pub struct Loaded<T> {
    pub deps: Vec<Key<'static>>,
    pub value: T,
}

impl<T> Loaded<T> {
    pub fn new(value: T) -> Self {
        Self {
            deps: Vec::new(),
            value,
        }
    }

    pub fn with_deps(value: T, deps: Vec<Key<'static>>) -> Self {
        Self { value, deps }
    }
}

impl<T> From<T> for Loaded<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

pub trait Asset: Send + Sync + 'static + Sized {
    /// Load this asset with the given key and resources, along with the provided reference
    /// to the cache it's being loaded into.
    ///
    /// # Concurrency
    ///
    /// [`Cache::get`] has several caveats with respect to concurrency. Please see the docs on
    /// [`Cache::get`] for more information on when `Cache::get` can be called from within a
    /// prior call to `Asset::load` without danger of deadlocks or errors.
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StructuredKey {
    inner: serde_hashkey::Key<OrderedFloatPolicy>,
}

impl fmt::Display for StructuredKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = serde_json::to_string_pretty(&self.inner).map_err(|_| fmt::Error)?;
        write!(f, "{}", s)
    }
}

impl StructuredKey {
    pub fn to_rust<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_hashkey::from_key(&self.inner)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key<'a> {
    Path(Cow<'a, Path>),
    Structured(StructuredKey),
}

impl<'a> fmt::Display for Key<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Path(path) => fmt::Display::fmt(&path.display(), f),
            Self::Structured(key) => fmt::Display::fmt(key, f),
        }
    }
}

impl<'a> From<&'a Path> for Key<'a> {
    fn from(path: &'a Path) -> Self {
        Self::from_path(path)
    }
}

impl<'a> From<PathBuf> for Key<'a> {
    fn from(pathbuf: PathBuf) -> Self {
        Self::Path(Cow::Owned(pathbuf))
    }
}

impl<'a> Key<'a> {
    pub fn from_path<P: AsRef<Path> + ?Sized>(path: &'a P) -> Self {
        Self::Path(Cow::Borrowed(path.as_ref()))
    }

    pub fn from_structured<T: Serialize>(structured: &T) -> Result<Self> {
        Ok(Self::Structured(StructuredKey {
            inner: serde_hashkey::to_key_with_ordered_float(structured)?,
        }))
    }

    pub fn clone_static(&self) -> Key<'static> {
        match self {
            Key::Path(cow_path) => Key::Path(Cow::Owned(cow_path.clone().into_owned())),
            Key::Structured(structured) => Key::Structured(structured.clone()),
        }
    }

    pub fn to_path(&self) -> Result<&Path> {
        match self {
            Key::Path(path) => Ok(path),
            Key::Structured(key) => bail!("expected path but found structured key: {}", key),
        }
    }

    pub fn to_rust<T: DeserializeOwned>(&self) -> Result<T> {
        match self {
            Key::Path(path) => bail!(
                "expected structured key deserializable to type {} but found path: {}",
                any::type_name::<T>(),
                path.display()
            ),
            Key::Structured(structured) => structured.to_rust().with_context(|| {
                anyhow!(
                    "error parsing structured key {} into type {}",
                    structured,
                    any::type_name::<T>()
                )
            }),
        }
    }
}

#[derive(Debug)]
enum ResourceState {
    Done(Arc<dyn Any + Send + Sync>),
    Loading(ThreadId, Arc<Condvar>),
}

#[derive(Default)]
struct KeyEntry {
    types: HashMap<TypeId, ResourceState>,
}

pub struct Cache<'a, R: Resources<'a>> {
    resources: R,
    entries: Mutex<HashMap<Key<'static>, KeyEntry>>,
    dependencies: Mutex<HashMap<Key<'static>, HashSet<Key<'static>>>>,
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

    /// Load a resource, inserting it into the cache if unloaded and returning a reference
    /// to the cached value if already loaded.
    ///
    /// # Concurrency
    ///
    /// This method has several caveats with respect to concurrency. The first is that with
    /// the current implementation, a deadlock is possible if `Asset::load` ends up somehow
    /// blocking on another thread which is attempting to load the exact same asset (type
    /// and key.) The second is that if another thread is already trying to load the resource,
    /// any other threads which attempt to load the same resource will block until the first
    /// thread finishes (succeeds or fails.) The third is that if we detect that a thread which
    /// is already loading a given resource attempts to load that resource *again*, we treat
    /// the occurrence as if the resource is recursively dependent on itself, and the method
    /// will return an error.
    ///
    /// The bottom line is, if your `Asset` implementation keeps all its recursive calls to
    /// `Cache::get` on the same thread it's called on, then this method will act reasonably
    /// and will not deadlock or fail (unless the loading itself fails.)
    pub fn get<T>(&self, key: &Key) -> Result<Cached<T>>
    where
        T: Asset,
    {
        // Scope here to ensure that `entries` isn't locked while we `load` the asset,
        // if necessary.
        //
        // Inside this we have several cases to handle, four to be precise:
        // 1.) The resource is already loaded; in this case we return the cached resource.
        // 2.) The resource is currently being loaded by another thread. In this case, we block
        //     and wait on the associated condvar to be signalled. Once the condvar triggers,
        //     the resource has either been loaded successfully or failed to load; in the former
        //     case, the entry will be seen as in case 1. In any other case we assume that
        //     the load failed, and bail.
        // 3.) The resource is currently being loaded by the current thread. This means that
        //     the resource depends on itself. To prevent infinite recursion, we bail here.
        // 4.) The resource is not loaded. In this case, create a placeholder `ResourceState`
        //     containing the current thread (so we can detect bad recursion/re-calls on the
        //     same thread) and return the created condvar so we can signal to any other
        //     threads waiting on our loading resource when we are done (or fail.)
        let signal_loaded = {
            let mut entries = self.entries.lock().unwrap();
            match entries
                .get(key)
                .and_then(|e| e.types.get(&TypeId::of::<T>()))
            {
                Some(ResourceState::Done(value)) => {
                    let downcast = value.clone().downcast::<ArcSwap<T>>().unwrap();
                    return Ok(Cached(arc_swap::Cache::new(downcast)));
                }
                Some(ResourceState::Loading(thread_id, loaded))
                    if *thread_id != thread::current().id() =>
                {
                    let loaded = loaded.clone();
                    let entries = loaded.wait(entries).unwrap();
                    let entry = entries
                        .get(key)
                        .and_then(|e| e.types.get(&TypeId::of::<T>()));
                    if let Some(ResourceState::Done(value)) = entry {
                        let downcast = value.clone().downcast::<ArcSwap<T>>().unwrap();
                        return Ok(Cached(arc_swap::Cache::new(downcast)));
                    } else {
                        bail!("an error occurred while waiting for another thread to load a resource with the key {}", key);
                    }
                }
                Some(ResourceState::Loading(_, _)) => {
                    bail!("resource with key {} recursively depends on itself!", key)
                }
                None => {
                    let loaded = Arc::new(Condvar::new());
                    entries.entry(key.clone_static()).or_default().types.insert(
                        TypeId::of::<T>(),
                        ResourceState::Loading(thread::current().id(), loaded.clone()),
                    );
                    loaded
                }
            }
        };

        let loaded = match T::load(key, self, &self.resources) {
            Ok(t) => t,
            Err(err) => {
                // On an error, we unblock the other threads waiting for this resource
                // to be loaded, as we rethrow the error.
                signal_loaded.notify_all();
                bail!(err.context(anyhow!(
                    "error loading asset of type {} for key {}",
                    any::type_name::<T>(),
                    key
                )));
            }
        };

        if !loaded.deps.is_empty() {
            let mut dependencies = self.dependencies.lock().unwrap();
            dependencies
                .entry(key.clone_static())
                .or_default()
                .extend(loaded.deps);
        }
        let wrapped = Arc::new(ArcSwap::from_pointee(loaded.value));

        {
            let mut entries = self.entries.lock().unwrap();
            entries.entry(key.clone_static()).or_default().types.insert(
                TypeId::of::<T>(),
                ResourceState::Done(wrapped.clone() as Arc<dyn Any + Send + Sync>),
            );
        }
        signal_loaded.notify_all();

        Ok(Cached(arc_swap::Cache::new(wrapped)))
    }
}
