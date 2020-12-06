use crate::SludgeLuaContextExt;
use {anyhow::*, rlua::prelude::*, thiserror::*};

#[derive(Debug, Error)]
#[error("a Lua thread made a graceful premature exit after being killed")]
pub struct GracefulExit;

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    // Steal coroutine then get rid of it from the global table so that
    // all coroutine manipulation goes through Space.
    let coroutine = lua.globals().get::<_, LuaTable>("coroutine")?;
    lua.globals().set("coroutine", LuaValue::Nil)?;

    let spawn =
        lua.create_function(|ctx, (task, args): (LuaValue, LuaMultiValue)| ctx.spawn(task, args))?;

    let broadcast = lua.create_function(|ctx, (string, args): (LuaString, LuaMultiValue)| {
        ctx.broadcast(string.to_str()?, args)
    })?;

    let notify = lua.create_function(|ctx, (target, args): (LuaThread, LuaMultiValue)| {
        ctx.notify(target, args)
    })?;

    let kill = lua.create_function(|ctx, (target, args): (LuaThread, LuaMultiValue)| {
        ctx.kill(target, args)
    })?;

    let graceful_exit =
        lua.create_function(|_, _: ()| -> LuaResult<()> { Err(LuaError::external(GracefulExit)) })?;

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
        ("kill", kill),
        ("rawyield", yield_),
        ("graceful_exit", graceful_exit),
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
