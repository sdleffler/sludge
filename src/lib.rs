#![feature(drain_filter, exact_size_is_empty, option_expect_none, duration_zero)]

use {
    anyhow::*,
    crossbeam_channel::{Receiver, Sender},
    derivative::*,
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    serde::{Deserialize, Serialize},
    smallvec::SmallVec,
    std::{
        any::Any,
        cmp::Ordering,
        collections::BinaryHeap,
        fmt,
        io::{Read, Write},
        iter,
    },
    string_cache::DefaultAtom,
    thunderdome::{Arena, Index},
};

pub type Atom = DefaultAtom;

pub mod api;
pub mod assets;
pub mod chunked_grid;
pub mod components;
pub mod conf;
pub mod dependency_graph;
pub mod dispatcher;
pub mod ecs;
pub mod event;
pub mod filesystem;
pub mod graphics;
pub mod hierarchy;
pub mod input;
pub mod math;
pub mod path_clean;
pub mod persist;
pub mod resources;
pub mod scene;
pub mod spatial_2d;
pub mod sprite;
pub mod systems;
pub mod tiled;
pub mod timer;
pub mod transform;
pub mod vfs;

pub mod prelude {
    pub use anyhow::*;
    pub use inventory;
    pub use rlua::prelude::*;

    pub use crate::{
        api::LuaEntity,
        ecs::*,
        math::*,
        resources::{OwnedResources, Resources, SharedResources, UnifiedResources},
        Scheduler, SludgeLuaContextExt, SludgeResultExt, Space, System,
    };

    pub use sludge_macros::*;
}

#[doc(hidden)]
pub use ::{anyhow, inventory, nalgebra, ncollide2d, rlua, rlua_serde, serde, sludge_macros::*};

#[doc(hidden)]
pub mod sludge {
    #[doc(hidden)]
    pub use {
        crate::ecs::{Entity, FlaggedComponent, ScContext, SmartComponent},
        inventory,
        std::any::TypeId,
    };
}

#[doc(hidden)]
pub use crate::sludge::*;

use crate::{api::EntityUserDataRegistry, dispatcher::Dispatcher, ecs::World, resources::*};

pub trait SludgeResultExt: Sized {
    type Ok;
    type Err;

    fn log_err(self, target: &str, level: log::Level) -> Self
    where
        Self::Err: fmt::Display;

    fn log_warn_err(self, target: &str) -> Self
    where
        Self::Err: fmt::Display,
    {
        self.log_err(target, log::Level::Warn)
    }

    fn log_error_err(self, target: &str) -> Self
    where
        Self::Err: fmt::Display,
    {
        self.log_err(target, log::Level::Error)
    }
}

impl<T, E: fmt::Debug> SludgeResultExt for Result<T, E> {
    type Ok = T;
    type Err = E;

    #[track_caller]
    fn log_err(self, target: &str, level: log::Level) -> Self
    where
        E: fmt::Display,
    {
        if let Err(ref e) = &self {
            log::log!(target: target, level, "{:#?}", e);
        }

        self
    }
}

const RESOURCES_REGISTRY_KEY: &'static str = "sludge.resources";

pub trait SludgeLuaContextExt<'lua> {
    fn resources(self) -> UnifiedResources<'static>;
    fn spawn<T: ToLua<'lua>>(self, task: T) -> LuaResult<()>;
    fn broadcast<S: AsRef<str>, T: ToLuaMulti<'lua>>(self, event_name: S, args: T)
        -> LuaResult<()>;
}

impl<'lua> SludgeLuaContextExt<'lua> for LuaContext<'lua> {
    fn resources(self) -> UnifiedResources<'static> {
        self.named_registry_value::<_, UnifiedResources>(RESOURCES_REGISTRY_KEY)
            .with_context(|| anyhow!("error while extracing resources from Lua registry"))
            .unwrap()
    }

    fn spawn<T: ToLua<'lua>>(self, task: T) -> LuaResult<()> {
        let thread = match task.to_lua(self)? {
            LuaValue::Function(f) => self.create_thread(f)?,
            LuaValue::Thread(th) => th,
            _ => {
                return Err(LuaError::FromLuaConversionError {
                    to: "thread or function",
                    from: "lua value",
                    message: None,
                })
            }
        };

        let key = self.create_registry_value(thread)?;
        self.resources()
            .fetch::<SchedulerQueueChannel>()
            .spawn
            .try_send(key)
            .unwrap();
        Ok(())
    }

    fn broadcast<S: AsRef<str>, T: ToLuaMulti<'lua>>(
        self,
        event_name: S,
        args: T,
    ) -> LuaResult<()> {
        let args = args.to_lua_multi(self)?;
        let event = Event {
            name: EventName(Atom::from(event_name.as_ref())),
            args: if args.is_empty() {
                None
            } else {
                Some(
                    args.into_iter()
                        .map(|v| self.create_registry_value(v))
                        .collect::<LuaResult<_>>()?,
                )
            },
        };

        self.resources()
            .fetch::<SchedulerQueueChannel>()
            .event
            .try_send(event)
            .unwrap();
        Ok(())
    }
}

pub trait System {
    fn init(
        &self,
        _lua: LuaContext,
        _local: &mut OwnedResources,
        _global: Option<&SharedResources>,
    ) -> Result<()> {
        Ok(())
    }

    fn update(&self, lua: LuaContext, resources: &UnifiedResources) -> Result<()>;
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Space {
    #[derivative(Debug = "ignore")]
    lua: Lua,

    #[derivative(Debug = "ignore")]
    resources: UnifiedResources<'static>,

    #[derivative(Debug = "ignore")]
    maintainers: Dispatcher<'static>,
}

impl Space {
    pub fn new() -> Result<Self> {
        Self::with_global_resources(SharedResources::new())
    }

    pub fn with_global_resources(global: SharedResources<'static>) -> Result<Self> {
        use rlua::StdLib;
        let lua = Lua::new_with(
            StdLib::BASE
                | StdLib::COROUTINE
                | StdLib::TABLE
                | StdLib::STRING
                | StdLib::UTF8
                | StdLib::MATH
                | StdLib::ERIS,
        );
        let mut local = OwnedResources::new();

        local.insert(World::new());
        let (scheduler, queue_handle) = Scheduler::new();
        local.insert(scheduler);
        local.insert(queue_handle);
        local.insert(EntityUserDataRegistry::new());

        let local = SharedResources::from(local);
        let resources = UnifiedResources { local, global };

        lua.context(|lua_ctx| -> Result<_> {
            lua_ctx.set_named_registry_value(RESOURCES_REGISTRY_KEY, resources.clone())?;
            crate::api::load(lua_ctx)?;

            Ok(())
        })?;

        let mut this = Self {
            lua,
            resources,
            maintainers: Dispatcher::new(),
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

        this.maintain()?;

        Ok(this)
    }

    pub fn register<S>(&mut self, system: S, name: &str, deps: &[&str]) -> Result<()>
    where
        S: System + 'static,
    {
        self.maintainers.register(system, name, deps)
    }

    pub fn maintain(&mut self) -> Result<()> {
        let Self {
            lua,
            maintainers,
            resources,
        } = self;

        lua.context(|lua| maintainers.update(lua, resources))
    }

    pub fn fetch<T: Any + Send + Sync>(&self) -> SharedFetch<'static, '_, T> {
        self.resources.fetch()
    }

    pub fn fetch_mut<T: Any + Send + Sync>(&self) -> SharedFetchMut<'static, '_, T> {
        self.resources.fetch_mut()
    }

    pub fn try_fetch<T: Any + Send + Sync>(&self) -> Option<SharedFetch<'static, '_, T>> {
        self.resources.try_fetch()
    }

    pub fn try_fetch_mut<T: Any + Send + Sync>(&self) -> Option<SharedFetchMut<'static, '_, T>> {
        self.resources.try_fetch_mut()
    }

    pub fn fetch_shared<T: Any>(&self) -> Option<Shared<'static, T>> {
        self.resources.fetch_shared()
    }

    pub fn resources(&self) -> &UnifiedResources<'static> {
        &self.resources
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn refresh(&self, dispatcher: &mut Dispatcher) -> Result<()> {
        let local_resources = &mut *self.resources.local.borrow_mut();
        let global_resources = &self.resources.global;
        self.lua
            .context(|lua| dispatcher.refresh(lua, local_resources, Some(global_resources)))
    }

    pub fn dispatch(&self, dispatcher: &mut Dispatcher) -> Result<()> {
        self.lua
            .context(|lua| dispatcher.update(lua, &self.resources))
    }

    #[inline]
    pub fn world(&self) -> SharedFetch<'static, '_, World> {
        self.fetch()
    }

    #[inline]
    pub fn world_mut(&self) -> SharedFetchMut<'static, '_, World> {
        self.fetch_mut()
    }

    #[inline]
    pub fn scheduler(&self) -> SharedFetch<'static, '_, Scheduler> {
        self.fetch()
    }

    #[inline]
    pub fn scheduler_mut(&self) -> SharedFetchMut<'static, '_, Scheduler> {
        self.fetch_mut()
    }

    pub fn save<W: Write>(&self, writer: W) -> Result<()> {
        self.lua.context(|lua| persist::persist(lua, self, writer))
    }

    pub fn load<R: Read>(&self, reader: R) -> Result<()> {
        self.lua
            .context(|lua| persist::unpersist(lua, self, reader))
    }
}

/// A thread waiting to be woken up, living in the scheduler's queue. This
/// can represent a thread which is scheduled for a given tick, or a thread
/// which was waiting for an event which was previously broadcast this tick
/// and is ready to be run.
///
/// An event wakeup will always appear as if it's scheduled for tick 0, and
/// as such will always be at the front of the priority queue.
///
/// Wakeups may not point to a valid thread. When a thread is resumed, all
/// previous indices referring to it become invalidated. Popping a wakeup
/// which no longer has a valid thread is not an error, but simply to be
/// ignored.
#[derive(Debug)]
pub enum Wakeup {
    Immediate(Index),
    Event {
        thread: Index,
        name: EventName,
        args: Option<Index>,
    },
    Timed {
        thread: Index,
        scheduled_for: u64,
    },
}

impl Wakeup {
    pub fn scheduled_for(&self) -> u64 {
        match self {
            Self::Immediate(..) | Self::Event { .. } => 0,
            Self::Timed { scheduled_for, .. } => *scheduled_for,
        }
    }

    pub fn thread(&self) -> Index {
        match self {
            Self::Immediate(thread) | Self::Event { thread, .. } | Self::Timed { thread, .. } => {
                *thread
            }
        }
    }
}

impl PartialEq for Wakeup {
    fn eq(&self, rhs: &Self) -> bool {
        self.scheduled_for() == rhs.scheduled_for() && self.thread() == rhs.thread()
    }
}

impl Eq for Wakeup {}

impl PartialOrd for Wakeup {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

/// We want wakeups with *lesser* wakeup times to be "greater" than wakups with later
/// times, so that the stdlib `BinaryHeap` (which is a max-heap) gives us the proper
/// result.
impl Ord for Wakeup {
    fn cmp(&self, rhs: &Self) -> Ordering {
        if matches!(self, Self::Immediate(..)) || matches!(rhs, Self::Immediate(..)) {
            if matches!(self, Self::Immediate(..)) && matches!(rhs, Self::Immediate(..)) {
                return Ordering::Equal;
            } else if matches!(self, Self::Immediate(..)) {
                return Ordering::Greater;
            } else if matches!(rhs, Self::Immediate(..)) {
                return Ordering::Less;
            }
        }

        self.scheduled_for()
            .cmp(&rhs.scheduled_for())
            .reverse()
            .then_with(|| self.thread().cmp(&rhs.thread()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventName(Atom);

pub type EventArgs = SmallVec<[LuaRegistryKey; 3]>;

#[derive(Debug)]
pub struct Event {
    name: EventName,
    args: Option<EventArgs>,
}

#[derive(Debug, Clone)]
pub struct SchedulerQueueChannel {
    spawn: Sender<LuaRegistryKey>,
    event: Sender<Event>,
}

/// The scheduler controls the execution of Lua "threads", under a cooperative
/// concurrency model. It is a priority queue of coroutines to be resumed,
/// ordered by how soon they should be woken. It also supports waking threads
/// via string-keyed events, with Lua-valued arguments for event broadcasts.
///
/// # Persistence and the `Scheduler`
///
/// In order to robustly save/load the state of a `Space`, it is necessary to
/// persist/load the scheduler itself. There are a few things to note about this.
///
/// Persistence of Lua values is implemented through Eris, which is capable of
/// robustly serializing *any* pure Lua value, up to and including coroutines
/// and closures. Userdata cannot be persisted, and is serialized through a sort
/// of bridging which persists userdata objects as closures which reconstruct
/// equivalent objects.
///
/// It is not possible for Eris to persist the currently running thread. As a
/// corollary, it seems like a good idea for serialization to be forced only
/// outside of Lua, and provide in Lua only an API which *requests* serialization
/// asynchronously.
///
/// Persisting a `Space`'s state involves serializing data from the ECS, among
/// other sources. The ECS is particularly troublesome because it references through
/// indices which are not stable across instances of a program. As a result,
/// we must leverage Eris's "permanents" table, which allows for custom handling
/// of non-trivial data on a per-value basis. The permanents table will have
/// to be generated separately, and will contain all userdata and bound functions
/// from Sludge's API as well as mappings from userdata to tables containing the
/// necessary data to reconstruct them.
///
/// The scheduler itself can be represented purely in Lua. In order to serialize
/// it, it may be beneficial to convert the scheduler to a Lua representation to
/// be bundled alongside all other Lua data and then serialized in the context of
/// the permanents table. Whether it should be legal to serialize a scheduler
/// with pending non-timed wakeups is an unanswered question. If the answer is "yes"
/// then it actually does become possible to serialize "synchronously" from Lua
/// by setting a flag, yielding from the requesting thread, breaking from the
/// scheduler, and then immediately serializing the resulting state, with the
/// requesting thread given a special wakeup priority.
#[derive(Debug)]
pub struct Scheduler {
    /// Priority queue of scheduled threads, ordered by wakeup.
    queue: BinaryHeap<Wakeup>,

    /// Hashmap of threads which aren't currently scheduled. These
    /// will be woken when the scheduler is notified of an event,
    /// and added to the queue with `wakeup == 0`.
    waiting: HashMap<EventName, Vec<Index>>,

    /// The generational arena allows us to ensure that threads that
    /// are waiting for multiple events and also possibly a timer don't
    /// get woken up multiple times.
    threads: Arena<LuaRegistryKey>,

    /// `EventArgs` are bundles of Lua multivalues, and having them in
    /// an arena means they can be 1.) shared between different `Wakeup`s
    /// and 2.) we clear the entire arena all in one go later!
    event_args: Arena<EventArgs>,

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
                event_args: Arena::new(),

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
        self.queue.is_empty() || self.queue.peek().unwrap().scheduled_for() > self.discrete
    }

    pub(crate) fn queue_all_spawned(&mut self) {
        for key in self.spawn_channel.try_iter() {
            let index = self.threads.insert(key);
            self.queue.push(Wakeup::Timed {
                thread: index,
                scheduled_for: 0,
            });
        }
    }

    pub(crate) fn poll_events_and_queue_all_notified(&mut self) {
        let Self {
            queue,
            threads,
            waiting,
            event_args,
            event_channel,
            ..
        } = self;

        for event in event_channel.try_iter() {
            let event_index = event.args.map(|args| event_args.insert(args));

            if let Some(running_threads) = waiting.get_mut(&event.name) {
                for index in running_threads.drain(..) {
                    // `None` will get returned here if the thread's already been rescheduled.
                    // `threads.increment_gen` invalidates all of the indices which previously
                    // pointed to this thread.
                    if let Some(new_index) = threads.invalidate(index) {
                        queue.push(Wakeup::Event {
                            thread: new_index,
                            name: event.name.clone(),
                            args: event_index,
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
            if top.scheduled_for() > self.discrete {
                break;
            }

            let sleeping = self.queue.pop().unwrap();
            if let Some(key) = self.threads.get(sleeping.thread()) {
                let thread = lua.registry_value::<LuaThread>(key)?;

                let resumed = match &sleeping {
                    Wakeup::Immediate(..) => thread.resume::<_, LuaMultiValue>(()),
                    Wakeup::Timed { scheduled_for, .. } => {
                        thread.resume::<_, LuaMultiValue>(*scheduled_for)
                    }
                    Wakeup::Event {
                        name,
                        args: Some(args),
                        ..
                    } => {
                        let args_unpacked =
                            iter::once(lua.create_string(name.0.as_ref()).map(LuaValue::String))
                                .chain(
                                    self.event_args[*args]
                                        .iter()
                                        .map(|key| lua.registry_value(key)),
                                )
                                .collect::<Result<LuaMultiValue, _>>();
                        args_unpacked.and_then(|xs| thread.resume::<_, LuaMultiValue>(xs))
                    }
                    Wakeup::Event {
                        name, args: None, ..
                    } => thread.resume::<_, LuaMultiValue>(name.0.as_ref()),
                };

                match resumed {
                    Ok(mv) if thread.status() == LuaThreadStatus::Resumable => {
                        let new_index = self.threads.invalidate(sleeping.thread()).unwrap();

                        // Take the yielded values provided by the coroutine and turn
                        // them into events/wakeup times.
                        for value in mv.into_iter() {
                            match value {
                                // If we see an integer, then treat it as ticks-until-next-wake.
                                LuaValue::Integer(i) => {
                                    self.queue.push(Wakeup::Timed {
                                        thread: new_index,
                                        // Threads aren't allowed to yield and resume on the same tick
                                        // forever.
                                        scheduled_for: self.discrete + na::max(i, 1) as u64,
                                    });
                                }
                                // If we see a string, then treat it as an event which the thread
                                // wants to listen for.
                                LuaValue::String(lua_str) => {
                                    if let Ok(s) = lua_str.to_str() {
                                        let threads = self
                                            .waiting
                                            .entry(EventName(Atom::from(s)))
                                            .or_default();
                                        match threads.binary_search(&sleeping.thread()) {
                                            Ok(i) => threads[i] = new_index,
                                            Err(i) if threads.get(i) != Some(&new_index) => {
                                                threads.insert(i, new_index)
                                            }
                                            _ => {}
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
                            sleeping.thread().to_bits(),
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
                scheduler.event_args.clear();
                scheduler.queue_all_spawned();
                scheduler.poll_events_and_queue_all_notified();

                if scheduler.is_idle() {
                    break;
                } else if i == LOOP_CAP - 1 {
                    log::warn!("trampoline loop cap exceeded");
                }
            }

            scheduler.continuous -= 1.;
            scheduler.discrete += 1;
        }

        lua.expire_registry_values();

        Ok(())
    }
}
