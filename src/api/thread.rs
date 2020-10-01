use crate::{Atom, Event, SchedulerQueueChannel, SludgeLuaContextExt};
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
        ctx.resources()
            .fetch::<SchedulerQueueChannel>()
            .spawn
            .try_send(key)
            .unwrap();
        Ok(thread)
    })?;

    let broadcast = lua.create_function(|ctx, string: LuaString| {
        ctx.resources()
            .fetch::<SchedulerQueueChannel>()
            .event
            .try_send(Event(Atom::from(string.to_str()?)))
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
        ("yield", yield_),
        ("create", create),
        ("wrap", wrap),
        ("running", running),
        ("status", status),
        ("resume", resume),
    ])?))
}

inventory::submit! {
    crate::api::Module::new("thread", load)
}
