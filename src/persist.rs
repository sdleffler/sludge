use {
    anyhow::*,
    hashbrown::HashMap,
    rlua::prelude::*,
    std::io::{Read, Write},
};

use crate::{
    api::*, components::Persistent, ecs::*, EventArgs, EventName, Scheduler, SludgeLuaContextExt,
    Space, Wakeup,
};

/// Create a new table under the `WORLD_TABLE_REGISTRY_KEY` and fill it with a mapping from
/// 32-bit transient hecs entity IDs to serializer thunks.
pub fn record_world_table<'lua>(lua: LuaContext<'lua>, world: &World) -> LuaResult<LuaTable<'lua>> {
    let tmp = lua.fetch_one::<EntityUserDataRegistry>()?;
    let entity_ud_registry = tmp.borrow();

    let to_table = lua
        .load(include_str!("api/lua/component_value_thunk.lua"))
        .set_name("component_value")?
        .eval::<LuaFunction>()?;

    let world_table = lua.create_table()?;
    let world_metatable = lua.create_table()?;
    world_metatable.set(
        "__persist",
        lua.named_registry_value::<_, LuaFunction>(PLAYBACK_THUNK_REGISTRY_KEY)?,
    )?;
    world_table.set_metatable(Some(world_metatable));
    lua.set_named_registry_value(WORLD_TABLE_REGISTRY_KEY, world_table.clone())?;

    for (e, (maybe_et,)) in world
        .query::<(Option<&EntityTable>,)>()
        .with::<Persistent>()
        .iter()
    {
        let id = LuaLightUserData(e.id() as *mut _);
        let archetype = entity_ud_registry.get_archetype(lua, e)?;
        let components = lua.create_table()?;
        for pair in archetype.pairs::<LuaValue, LuaValue>() {
            let (k, v) = pair?;
            let t = to_table.call::<_, LuaValue>(v)?;
            components.set(k, t)?;
        }

        let persisted_entity = lua.create_table()?;
        persisted_entity.set("id", id)?;

        if let Some(entity_table_key) = maybe_et {
            let entity_table = lua.registry_value::<LuaTable>(&entity_table_key.key)?;
            if let Some(serialize) = entity_table.get::<_, Option<LuaFunction>>("serialize")? {
                let value = serialize.call::<_, LuaValue>((LuaEntity::from(e), components))?;
                persisted_entity.set("components", value)?;
            } else {
                persisted_entity.set("components", components)?;
            }

            if let Some(deserialize) = entity_table.get::<_, Option<LuaFunction>>("deserialize")? {
                persisted_entity.set("deserialize", deserialize)?;
            }
        } else {
            persisted_entity.set("components", components)?;
        }

        world_table.set(world_table.len()? + 1, persisted_entity)?;
    }

    Ok(world_table)
}

/// Create a new table under the fill it with entries for queued and waiting threads.
pub fn record_scheduler_table<'lua>(
    lua: LuaContext<'lua>,
    scheduler: &Scheduler,
) -> LuaResult<LuaTable<'lua>> {
    let waiting_table = lua.create_table()?;
    let queue_table = lua.create_table()?;

    let mut threads = HashMap::new();
    for (i, thread) in scheduler.threads.iter() {
        let thread = lua.registry_value::<LuaThread>(thread)?;
        threads.insert(i, thread.clone());
        waiting_table.set(thread, lua.create_table()?)?;
    }

    for (event_name, waiting_thread) in scheduler
        .waiting
        .iter()
        .flat_map(|(ev, ts)| ts.iter().map(move |t| (ev, t)))
    {
        let thread_entry = waiting_table.get::<_, LuaTable>(threads[waiting_thread].clone())?;
        thread_entry.set(thread_entry.len()? + 1, &*event_name.0)?;
    }

    for wakeup in scheduler.queue.iter() {
        let wakeup_table = lua.create_table()?;
        match wakeup {
            Wakeup::Call { thread: i, args } => {
                wakeup_table.set("type", "call")?;
                wakeup_table.set("thread", threads[i].clone())?;

                if let Some(args_i) = *args {
                    let tmp = scheduler.event_args[args_i]
                        .iter()
                        .map(|k| lua.registry_value::<LuaValue>(k))
                        .collect::<LuaResult<Vec<_>>>()?;
                    wakeup_table.set("args", tmp)?;
                }
            }
            Wakeup::Notify { thread: i, args } => {
                wakeup_table.set("type", "notify")?;
                wakeup_table.set("thread", threads[i].clone())?;

                if let Some(args_i) = *args {
                    let tmp = scheduler.event_args[args_i]
                        .iter()
                        .map(|k| lua.registry_value::<LuaValue>(k))
                        .collect::<LuaResult<Vec<_>>>()?;
                    wakeup_table.set("args", tmp)?;
                }
            }
            Wakeup::Kill { thread: i, args } => {
                wakeup_table.set("type", "kill")?;
                wakeup_table.set("thread", threads[i].clone())?;

                if let Some(args_i) = *args {
                    let tmp = scheduler.event_args[args_i]
                        .iter()
                        .map(|k| lua.registry_value::<LuaValue>(k))
                        .collect::<LuaResult<Vec<_>>>()?;
                    wakeup_table.set("args", tmp)?;
                }
            }
            Wakeup::Broadcast {
                thread: i,
                name,
                args,
            } => {
                wakeup_table.set("type", "event")?;
                wakeup_table.set("thread", threads[i].clone())?;
                wakeup_table.set("event", &*name.0)?;

                if let Some(args_i) = *args {
                    let tmp = scheduler.event_args[args_i]
                        .iter()
                        .map(|k| lua.registry_value::<LuaValue>(k))
                        .collect::<LuaResult<Vec<_>>>()?;
                    wakeup_table.set("args", tmp)?;
                }
            }
            Wakeup::Timed {
                thread: i,
                scheduled_for,
            } => {
                wakeup_table.set("type", "timed")?;
                wakeup_table.set("thread", threads[i].clone())?;
                wakeup_table.set(
                    "scheduled_for",
                    scheduled_for.saturating_sub(scheduler.discrete),
                )?;
            }
        }

        queue_table.set(queue_table.len()? + 1, wakeup_table)?;
    }

    let scheduler_table = lua.create_table()?;

    scheduler_table.set("queue", queue_table)?;
    scheduler_table.set("waiting", waiting_table)?;

    Ok(scheduler_table)
}

pub fn playback_scheduler_table<'lua>(
    lua: LuaContext<'lua>,
    scheduler_table: LuaTable<'lua>,
    scheduler: &mut Scheduler,
) -> Result<()> {
    let queue_table = scheduler_table.get::<_, LuaTable>("queue")?;
    // FIXME(sleffy): persisting the scheduler table does not yet persist threads which
    // exist outside the queue
    let _slots_table = lua.registry_value::<LuaTable>(&scheduler.slots)?;
    for item in queue_table.sequence_values::<LuaTable>() {
        let table = item?;
        let thread = table.get::<_, LuaThread>("thread")?;
        let key = lua.create_registry_value(thread.clone())?;
        let i = scheduler.threads.insert(key);
        match table.get::<_, LuaString>("type")?.to_str()? {
            "call" => {
                let event_args =
                    if let Some(args) = table.get::<_, Option<Vec<LuaValue>>>("args")? {
                        let args_registered = args
                            .into_iter()
                            .map(|v| lua.create_registry_value(v))
                            .collect::<LuaResult<EventArgs>>()?;
                        let i = scheduler.event_args.insert(args_registered);
                        Some(i)
                    } else {
                        None
                    };
                scheduler.queue.push(Wakeup::Call {
                    thread: i,
                    args: event_args,
                });
            }
            "notify" => {
                let event_args =
                    if let Some(args) = table.get::<_, Option<Vec<LuaValue>>>("args")? {
                        let args_registered = args
                            .into_iter()
                            .map(|v| lua.create_registry_value(v))
                            .collect::<LuaResult<EventArgs>>()?;
                        let i = scheduler.event_args.insert(args_registered);
                        Some(i)
                    } else {
                        None
                    };
                scheduler.queue.push(Wakeup::Notify {
                    thread: i,
                    args: event_args,
                });
            }
            "kill" => {
                let event_args =
                    if let Some(args) = table.get::<_, Option<Vec<LuaValue>>>("args")? {
                        let args_registered = args
                            .into_iter()
                            .map(|v| lua.create_registry_value(v))
                            .collect::<LuaResult<EventArgs>>()?;
                        let i = scheduler.event_args.insert(args_registered);
                        Some(i)
                    } else {
                        None
                    };
                scheduler.queue.push(Wakeup::Kill {
                    thread: i,
                    args: event_args,
                });
            }
            "event" => {
                let event_name = EventName(table.get::<_, LuaString>("event")?.to_str()?.into());
                let event_args =
                    if let Some(args) = table.get::<_, Option<Vec<LuaValue>>>("args")? {
                        let args_registered = args
                            .into_iter()
                            .map(|v| lua.create_registry_value(v))
                            .collect::<LuaResult<EventArgs>>()?;
                        let i = scheduler.event_args.insert(args_registered);
                        Some(i)
                    } else {
                        None
                    };
                scheduler.queue.push(Wakeup::Broadcast {
                    thread: i,
                    name: event_name,
                    args: event_args,
                });
            }
            "timed" => {
                scheduler.queue.push(Wakeup::Timed {
                    thread: i,
                    scheduled_for: scheduler.discrete + table.get::<_, u64>("scheduled_for")?,
                });
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

pub fn persist<'lua, W: Write>(lua: LuaContext<'lua>, space: &Space, writer: W) -> Result<()> {
    let world_table = record_world_table(lua, &*space.world()?.borrow())?;
    let scheduler_table = record_scheduler_table(lua, &*space.scheduler()?.borrow())?;
    let permanents = lua.named_registry_value::<_, LuaTable>(PERMANENTS_SER_TABLE_REGISTRY_KEY)?;

    let persisted_table =
        lua.create_table_from(vec![("world", world_table), ("scheduler", scheduler_table)])?;

    lua.set_dump_setting("path", true)?;
    lua.dump_value(writer, permanents, persisted_table)?;

    Ok(())
}

pub fn unpersist<'lua, R: Read>(lua: LuaContext<'lua>, space: &Space, reader: R) -> Result<()> {
    let permanents = lua.named_registry_value::<_, LuaTable>(PERMANENTS_DE_TABLE_REGISTRY_KEY)?;
    lua.set_dump_setting("path", true)?;
    let persisted_table = lua.undump_value::<_, _, LuaTable>(reader, permanents)?;

    playback_scheduler_table(
        lua,
        persisted_table.get("scheduler")?,
        &mut *space.scheduler()?.borrow_mut(),
    )?;

    Ok(())
}
