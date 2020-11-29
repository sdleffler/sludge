use crate::{Atom, Event, EventName, SchedulerQueue, SludgeLuaContextExt};
use {anyhow::*, rlua::prelude::*};

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    // Steal coroutine then get rid of it from the global table so that
    // all coroutine manipulation goes through Space.
    let coroutine = lua.globals().get::<_, LuaTable>("coroutine")?;
    lua.globals().set("coroutine", LuaValue::Nil)?;

    let spawn = lua.create_function(|ctx, task: LuaValue| {
        let thread = match task {
            LuaValue::Function(f) => ctx.create_thread(f)?,
            LuaValue::Thread(th) => th,
            _ => {
                return Err(LuaError::FromLuaConversionError {
                    to: "thread or function",
                    from: "lua value",
                    message: None,
                })
            }
        };

        let key = ctx.create_registry_value(thread.clone())?;
        ctx.fetch_one::<SchedulerQueue>()?
            .borrow()
            .spawn
            .try_send(key)
            .unwrap();
        Ok(thread)
    })?;

    let broadcast = lua.create_function(|ctx, (string, args): (LuaString, LuaMultiValue)| {
        let event = Event::Broadcast {
            name: EventName(Atom::from(string.to_str()?)),
            args: if args.is_empty() {
                None
            } else {
                Some(
                    args.into_iter()
                        .map(|v| ctx.create_registry_value(v))
                        .collect::<LuaResult<_>>()?,
                )
            },
        };

        ctx.fetch_one::<SchedulerQueue>()?
            .borrow()
            .event
            .try_send(event)
            .unwrap();
        Ok(())
    })?;

    let notify = lua.create_function(|ctx, (target, args): (LuaThread, LuaMultiValue)| {
        let thread = ctx.create_registry_value(target)?;
        let event = Event::Notify {
            thread,
            args: if args.is_empty() {
                None
            } else {
                Some(
                    args.into_iter()
                        .map(|v| ctx.create_registry_value(v))
                        .collect::<LuaResult<_>>()?,
                )
            },
        };

        ctx.fetch_one::<SchedulerQueue>()?
            .borrow()
            .event
            .try_send(event)
            .unwrap();
        Ok(())
    })?;

    let yield_ = coroutine.get::<_, LuaFunction>("yield")?;
    let create = coroutine.get::<_, LuaFunction>("create")?;
    let wrap = coroutine.get::<_, LuaFunction>("wrap")?;
    let running = coroutine.get::<_, LuaFunction>("running")?;
    let status = coroutine.get::<_, LuaFunction>("status")?;
    let resume = coroutine.get::<_, LuaFunction>("resume")?;

    Ok(LuaValue::Table(lua.create_table_from(vec![
        ("spawn", spawn),
        ("broadcast", broadcast),
        ("notify", notify),
        ("yield", yield_),
        ("create", create),
        ("wrap", wrap),
        ("running", running),
        ("status", status),
        ("resume", resume),
    ])?))
}

inventory::submit! {
    crate::api::Module::parse("sludge.thread", load)
}
