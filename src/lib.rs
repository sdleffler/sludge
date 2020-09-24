#![feature(min_const_generics)]

use {
    anyhow::{anyhow, Result},
    crossbeam_channel::{Receiver, Sender},
    derivative::Derivative,
    rlua::prelude::*,
    std::{collections::BinaryHeap, ops},
};

mod utils;

pub mod ecs;
pub mod module;
pub mod resources;

#[derive(Derivative)]
#[derivative(Debug)]
pub struct SludgeState {
    #[derivative(Debug = "ignore")]
    lua: Lua,
}

impl SludgeState {
    pub fn new() -> Self {
        Self { lua: Lua::new() }
    }

    pub fn context<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(Sludge) -> Result<R>,
    {
        self.lua.context(|ctx| f(Sludge::new(ctx)?))
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

pub trait Module {
    fn load<'lua>(&self, lua: LuaContext<'lua>) -> Result<(&str, LuaTable<'lua>)>;
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Sludge<'lua> {
    #[derivative(Debug = "ignore")]
    lua: LuaContext<'lua>,
    modules: LuaTable<'lua>,
}

impl<'lua> ops::Deref for Sludge<'lua> {
    type Target = LuaContext<'lua>;

    fn deref(&self) -> &Self::Target {
        &self.lua
    }
}

impl<'lua> ops::DerefMut for Sludge<'lua> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.lua
    }
}

impl<'lua> Sludge<'lua> {
    fn new(lua: LuaContext<'lua>) -> Result<Self> {
        let modules = lua.create_table()?;
        lua.globals().set("sludge", modules.clone())?;

        Ok(Self { lua, modules })
    }

    pub fn load_module<M: Module>(&mut self, module: &M) -> Result<()> {
        let (k, v) = module.load(self.lua)?;
        self.modules.set(k, v)?;

        Ok(())
    }
}
