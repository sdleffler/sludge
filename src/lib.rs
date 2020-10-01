#![feature(min_const_generics)]

use {
    anyhow::*,
    atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut},
    crossbeam_channel::{Receiver, Sender},
    derivative::Derivative,
    generational_arena::{Arena, Index},
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    std::{
        any::{Any, TypeId},
        collections::BinaryHeap,
        ops,
        pin::Pin,
        sync::Arc,
    },
    string_cache::DefaultAtom,
};

pub type Atom = DefaultAtom;

mod utils;

pub mod api;
pub mod components;
pub mod dependency_graph;
pub mod ecs;
pub mod filesystem;
pub mod hierarchy;
pub mod input;
pub mod math;
pub mod resources;
pub mod scene;
pub mod sprite;
pub mod systems;
pub mod tiled;
pub mod transform;
pub mod vfs;

pub use anyhow;
pub use nalgebra;
pub use rlua;
pub use serde;
pub use warmy;

pub mod prelude {
    pub use anyhow::*;
    pub use inventory;
    pub use rlua::prelude::*;
    pub use rlua_serde;
    pub use serde_json;

    pub use crate::{
        api::{Accessor, StaticAccessor, StaticTemplate, Template},
        ecs::*,
        math::*,
        Scheduler, SludgeLuaContextExt, Space,
    };
}

use crate::{api::Registry, dependency_graph::DependencyGraph};

const RESOURCES_REGISTRY_KEY: &'static str = "sludge.resources";

pub trait SludgeLuaContextExt {
    fn resources(self) -> SharedResources;
}

impl<'lua> SludgeLuaContextExt for LuaContext<'lua> {
    fn resources(self) -> SharedResources {
        self.named_registry_value::<_, SharedResources>(RESOURCES_REGISTRY_KEY)
            .unwrap()
    }
}

pub trait System: 'static {
    fn init(&self, lua: LuaContext, resources: &mut Resources) -> Result<()>;
    fn update(&self, lua: LuaContext, resources: &SharedResources) -> Result<()>;
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Space {
    #[derivative(Debug = "ignore")]
    lua: Lua,

    #[derivative(Debug = "ignore")]
    resources: SharedResources,

    #[derivative(Debug = "ignore")]
    dependency_graph: DependencyGraph<Box<dyn System>>,
}

impl Space {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let mut resources = Resources::new();

        let (scheduler, queue_handle) = Scheduler::new();
        resources.insert(scheduler);
        resources.insert(queue_handle);
        resources.insert(Registry::new()?);

        let shared_resources = SharedResources::from(resources);

        lua.context(|lua_ctx| -> Result<_> {
            lua_ctx.set_named_registry_value(RESOURCES_REGISTRY_KEY, shared_resources.clone())?;
            crate::api::load(lua_ctx)?;

            Ok(())
        })?;

        let mut this = Self {
            lua,
            resources: shared_resources,
            dependency_graph: DependencyGraph::new(),
        };

        this.register(crate::systems::WorldEventSystem, "WorldEvent", &[])?;
        this.register(
            crate::systems::DefaultHierarchySystem::new(),
            "Hierarchy",
            &["WorldEvent"],
        )?;
        this.register(
            crate::systems::DefaultTransformSystem::new(),
            "Transform",
            &["WorldEvent", "Hierarchy"],
        )?;

        this.refresh()?;

        Ok(this)
    }

    pub fn register<S: System>(&mut self, system: S, name: &str, deps: &[&str]) -> Result<()> {
        ensure!(
            self.dependency_graph
                .insert(Box::new(system), name, deps.iter().copied())?
                .is_none(),
            "system already exists!"
        );

        Ok(())
    }

    pub fn refresh(&mut self) -> Result<()> {
        if self.dependency_graph.update()? {
            for (name, sys) in self.dependency_graph.sorted() {
                self.lua
                    .context(|lua| sys.init(lua, &mut *self.resources.borrow_mut()))?;
                log::info!("initialized system `{}`", name);
            }
        }

        Ok(())
    }

    pub fn update(&mut self) -> Result<()> {
        self.refresh()?;

        for (_, sys) in self.dependency_graph.sorted() {
            self.lua.context(|lua| sys.update(lua, &self.resources))?;
        }

        Ok(())
    }

    pub fn fetch<T: Any + Send + Sync>(&self) -> SharedFetch<T> {
        self.resources.fetch()
    }

    pub fn fetch_mut<T: Any + Send + Sync>(&self) -> SharedFetchMut<T> {
        self.resources.fetch_mut()
    }

    pub fn try_fetch<T: Any + Send + Sync>(&self) -> Option<SharedFetch<T>> {
        self.resources.try_fetch()
    }

    pub fn try_fetch_mut<T: Any + Send + Sync>(&self) -> Option<SharedFetchMut<T>> {
        self.resources.try_fetch_mut()
    }

    pub fn resources(&self) -> &SharedResources {
        &self.resources
    }

    /// You shouldn't need this.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

#[derive(Debug, Derivative)]
#[derivative(PartialEq, Eq, PartialOrd, Ord)]
pub struct ScheduledThread {
    /// The running Lua coroutine. We ignore it in comparison here because the
    /// comparisons on `ScheduledThread` are used to place it in a priority queue
    /// ordered by wakeup times.
    #[derivative(PartialEq = "ignore", PartialOrd = "ignore", Ord = "ignore")]
    thread: LuaRegistryKey,

    /// The tick (time in 60ths of a second) on which this thread wants to wake up.
    /// We want a reversed order because we want a min-heap based on `wakeup`.
    #[derivative(
        PartialOrd(compare_with = "utils::partial_cmp_reversed"),
        Ord(compare_with = "utils::cmp_reversed")
    )]
    wakeup: u64,
}

#[derive(Debug, Clone, Copy, Derivative)]
#[derivative(PartialEq, Eq, PartialOrd, Ord)]
pub struct TimedWakeup {
    #[derivative(PartialEq = "ignore", PartialOrd = "ignore", Ord = "ignore")]
    thread: Index,

    #[derivative(
        PartialOrd(compare_with = "utils::partial_cmp_reversed"),
        Ord(compare_with = "utils::cmp_reversed")
    )]
    wakeup: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Event(Atom);

#[derive(Debug, Clone)]
pub struct SchedulerQueueChannel {
    spawn: Sender<LuaRegistryKey>,
    event: Sender<Event>,
}

#[derive(Debug)]
pub struct Scheduler {
    /// Priority queue of scheduled threads, ordered by wakeup.
    queue: BinaryHeap<TimedWakeup>,

    /// Hashmap of threads which aren't currently scheduled. These
    /// will be woken when the scheduler is notified of an event,
    /// and added to the queue with `wakeup == 0`.
    waiting: HashMap<Event, Vec<Index>>,

    /// The generational arena allows us to ensure that threads that
    /// are waiting for multiple events and also possibly a timer don't
    /// get woken up multiple times.
    threads: Arena<LuaRegistryKey>,

    /// Shared channel for sending events to wake up sleeping threads.
    event_channel: Receiver<Event>,

    /// Shared channel for sending new threads to be scheduled.
    spawn_channel: Receiver<LuaRegistryKey>,

    /// "Discrete" time in "ticks" (60ths of a second, 60FPS)
    discrete: u64,

    /// "Continuous" time used to convert from seconds to ticks
    /// (stored in 60ths of a second, "consumed" and converted
    /// to discrete time on update, used to measure how many ticks
    /// to run per a given update)
    continuous: f32,
}

impl Scheduler {
    pub const CHANNEL_BOUND: usize = 4096;

    pub(crate) fn new() -> (Self, SchedulerQueueChannel) {
        let (spawn_sender, spawn_channel) = crossbeam_channel::bounded(Self::CHANNEL_BOUND);
        let (event_sender, event_channel) = crossbeam_channel::bounded(Self::CHANNEL_BOUND);

        (
            Self {
                queue: BinaryHeap::new(),
                waiting: HashMap::new(),

                threads: Arena::new(),

                event_channel,
                spawn_channel,

                discrete: 0,
                continuous: 0.,
            },
            SchedulerQueueChannel {
                spawn: spawn_sender,
                event: event_sender,
            },
        )
    }

    pub fn with_context<'s, 'lua>(
        &'s mut self,
        lua: LuaContext<'lua>,
    ) -> SchedulerWithContext<'s, 'lua> {
        SchedulerWithContext::new(self, lua)
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() || self.queue.peek().unwrap().wakeup > self.discrete
    }

    pub(crate) fn queue_all_spawned(&mut self) {
        for key in self.spawn_channel.try_iter() {
            let index = self.threads.insert(key);
            self.queue.push(TimedWakeup {
                thread: index,
                wakeup: 0,
            });
        }
    }

    pub(crate) fn poll_events_and_queue_all_notified(&mut self) {
        for event in self.event_channel.try_iter() {
            if let Some(threads) = self.waiting.get_mut(&event) {
                for index in threads.drain(..) {
                    // `None` will get returned here if the thread's already been rescheduled.
                    // `threads.increment_gen` invalidates all of the indices which previously
                    // pointed to this thread.
                    if let Some(new_index) = self.threads.increment_gen(index) {
                        self.queue.push(TimedWakeup {
                            thread: new_index,
                            wakeup: 0,
                        });
                    }
                }
            }
        }
    }

    pub(crate) fn run_all_queued(&mut self, lua: LuaContext) -> Result<()> {
        while let Some(top) = self.queue.peek() {
            // If this thread isn't ready to wake up on this tick, then
            // none of the other threads in this queue are.
            if top.wakeup > self.discrete {
                break;
            }

            let sleeping = self.queue.pop().unwrap();
            if let Some(key) = self.threads.get(sleeping.thread) {
                let value = lua.registry_value::<LuaThread>(key)?;
                match value.resume::<_, LuaMultiValue>(()) {
                    Ok(mv) if value.status() == LuaThreadStatus::Resumable => {
                        let new_index = self.threads.increment_gen(sleeping.thread).unwrap();

                        // Take the yielded values provided by the coroutine and turn
                        // them into events/wakeup times.
                        for value in mv.into_iter() {
                            match value {
                                // If we see an integer, then treat it as ticks-until-next-wake.
                                LuaValue::Integer(i) => {
                                    self.queue.push(TimedWakeup {
                                        thread: new_index,
                                        // Threads aren't allowed to yield and resume on the same tick
                                        // forever.
                                        wakeup: self.discrete + na::max(i, 1) as u64,
                                    });
                                }
                                // If we see a string, then treat it as an event which the thread
                                // wants to listen for.
                                LuaValue::String(lua_str) => {
                                    if let Ok(s) = lua_str.to_str() {
                                        let threads =
                                            self.waiting.entry(Event(Atom::from(s))).or_default();
                                        match threads.binary_search(&sleeping.thread) {
                                            Ok(i) => threads[i] = new_index,
                                            Err(i) => threads.insert(i, new_index),
                                        }
                                    }
                                }
                                other => {
                                    log::error!("unknown yield return value {:?}", other);
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(lua_error) => {
                        log::error!(
                            "fatal error in Lua thread {:?}: {}",
                            sleeping.thread.into_raw_parts(),
                            lua_error
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

pub struct SchedulerWithContext<'s, 'lua> {
    scheduler: &'s mut Scheduler,
    lua: LuaContext<'lua>,
}

impl<'s, 'lua> SchedulerWithContext<'s, 'lua> {
    pub fn new(scheduler: &'s mut Scheduler, lua: LuaContext<'lua>) -> Self {
        Self { scheduler, lua }
    }

    pub fn update(&mut self, dt: f32) -> Result<()> {
        let Self { scheduler, lua } = self;

        scheduler.continuous += dt;
        while scheduler.continuous > 0. {
            // Our core update step consists of two steps:
            // 1. Run all threads scheduled to run on or before the current tick.
            // 2. Check for threads spawned/woken by newly run threads. If there are new
            //    threads to be run immediately, go to step 1.
            //
            // `LOOP_CAP` is our limit on how many times we go to step 1 in a given
            // tick. This stops us from hitting an infinitely spawning loop.
            const LOOP_CAP: usize = 8;

            for i in 0..LOOP_CAP {
                scheduler.run_all_queued(*lua)?;
                scheduler.queue_all_spawned();
                scheduler.poll_events_and_queue_all_notified();

                if scheduler.is_idle() {
                    log::trace!("trampoline loop broken at index {}", i);
                    break;
                } else if i == LOOP_CAP - 1 {
                    log::warn!("trampoline loop cap exceeded");
                }
            }

            scheduler.continuous -= 1.;
            scheduler.discrete += 1;
        }

        Ok(())
    }
}

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

// /// Basic logging setup to log to the console with `fern`.
// fn setup_logging() -> Result<()> {
//     use fern::colors::{Color, ColoredLevelConfig};
//     let colors = ColoredLevelConfig::default()
//         .info(Color::Green)
//         .debug(Color::BrightMagenta)
//         .trace(Color::BrightBlue);
//     // This sets up a `fern` logger and initializes `log`.
//     fern::Dispatch::new()
//         // Formats logs
//         .format(move |out, message, record| {
//             out.finish(format_args!(
//                 "[{}][{:<5}][{}] {}",
//                 chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
//                 colors.color(record.level()),
//                 record.target(),
//                 message
//             ))
//         })
//         .level(log::LevelFilter::Warn)
//         // Filter out unnecessary stuff
//         .level_for("sludge", log::LevelFilter::Warn)
//         // Hooks up console output.
//         // env var for outputting to a file?
//         // Haven't needed it yet!
//         .chain(std::io::stderr())
//         .apply()?;

//     Ok(())
// }
