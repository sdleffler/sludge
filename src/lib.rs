#![feature(min_const_generics)]

use {
    anyhow::*,
    crossbeam_channel::{Receiver, Sender},
    derivative::Derivative,
    generational_arena::{Arena, Index},
    hashbrown::HashMap,
    nalgebra as na,
    rlua::prelude::*,
    std::{any::Any, collections::BinaryHeap},
    string_cache::DefaultAtom,
};

pub type Atom = DefaultAtom;

mod utils;

pub mod api;
pub mod components;
pub mod dependency_graph;
pub mod ecs;
pub mod hierarchy;
pub mod input;
pub mod math;
pub mod resources;
pub mod scene;
pub mod sprite;
pub mod systems;
pub mod tiled;
pub mod transform;

pub use anyhow;
pub use aseprite;
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

use crate::{
    api::Registry,
    dependency_graph::DependencyGraph,
    resources::{Resources, SharedFetch, SharedFetchMut, SharedResources},
};

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
            lua_ctx
                .globals()
                .set("sludge", crate::api::load(lua_ctx)?)?;

            Ok(())
        })?;

        let mut this = Self {
            lua,
            resources: shared_resources,
            dependency_graph: DependencyGraph::new(),
        };

        this.register(crate::systems::WorldEventSystem, "world", &[])?;
        this.register(
            crate::systems::DefaultHierarchySystem::new(),
            "hierarchy",
            &["world"],
        )?;
        this.register(
            crate::systems::DefaultTransformSystem::new(),
            "transform",
            &["world", "hierarchy"],
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
