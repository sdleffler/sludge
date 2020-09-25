#![feature(min_const_generics)]

use {
    anyhow::*,
    crossbeam_channel::{Receiver, Sender},
    derivative::Derivative,
    rlua::prelude::*,
    std::collections::BinaryHeap,
};

mod utils;

pub mod ecs;
pub mod module;
pub mod resources;

use crate::resources::{Resources, SharedResources};

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

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Sludge {
    #[derivative(Debug = "ignore")]
    lua: Lua,

    #[derivative(Debug = "ignore")]
    shared_resources: SharedResources,
}

impl Sludge {
    pub fn new(resources: Resources) -> Result<Self> {
        let lua = Lua::new();
        let shared_resources = SharedResources::from(resources);

        lua.context(|lua_ctx| -> Result<()> {
            lua_ctx.set_named_registry_value(RESOURCES_REGISTRY_KEY, shared_resources.clone())?;
            Ok(())
        })?;

        Ok(Self {
            lua,
            shared_resources,
        })
    }
}

#[derive(Debug, Derivative)]
#[derivative(PartialEq, Eq, PartialOrd, Ord)]
pub struct ScheduledThread<'lua> {
    /// The running Lua coroutine. We ignore it in comparison here because the
    /// comparisons on `ScheduledThread` are used to place it in a priority queue
    /// ordered by wakeup times.
    #[derivative(PartialEq = "ignore", PartialOrd = "ignore", Ord = "ignore")]
    thread: LuaThread<'lua>,

    /// The tick (time in 60ths of a second) on which this thread wants to wake up.
    /// We want a reversed order because we want a min-heap based on `wakeup`.
    #[derivative(
        PartialOrd(compare_with = "utils::partial_cmp_reversed"),
        Ord(compare_with = "utils::cmp_reversed")
    )]
    wakeup: u64,
}

#[derive(Debug)]
pub struct Scheduler<'lua> {
    /// Priority queue of scheduled threads, ordered by wakeup.
    queue: BinaryHeap<ScheduledThread<'lua>>,

    /// Shared channel for sending new threads to be scheduled.
    spawn_channel: Receiver<LuaThread<'lua>>,

    /// Sender for the spawn channel, to be handed out.
    spawn_sender: Sender<LuaThread<'lua>>,

    /// "Discrete" time in "ticks" (60ths of a second, 60FPS)
    discrete: u64,

    /// "Continuous" time used to convert from seconds to ticks
    /// (stored in 60ths of a second, "consumed" and converted
    /// to discrete time on update, used to measure how many ticks
    /// to run per a given update)
    continuous: f32,
}

impl<'lua> Scheduler<'lua> {
    pub const CHANNEL_BOUND: usize = 4096;

    pub fn new() -> Self {
        let (spawn_sender, spawn_channel) = crossbeam_channel::bounded(Self::CHANNEL_BOUND);

        Self {
            queue: BinaryHeap::new(),

            spawn_channel,
            spawn_sender,

            discrete: 0,
            continuous: 0.,
        }
    }

    pub fn sender(&self) -> Sender<LuaThread<'lua>> {
        self.spawn_sender.clone()
    }

    pub fn go(&self, thread: LuaThread<'lua>) -> Result<()> {
        self.spawn_sender
            .try_send(thread)
            .map_err(|_| anyhow!("spawn buffer full"))
    }
}
