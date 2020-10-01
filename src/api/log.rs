use {
    anyhow::Result,
    log::{log, Level},
    rlua::prelude::*,
};

pub fn log_message(
    _lua: LuaContext,
    (level, target, message): (&str, Option<&str>, &str),
) -> LuaResult<()> {
    let level = match level {
        l if l.eq_ignore_ascii_case("error") => Level::Error,
        l if l.eq_ignore_ascii_case("warn") => Level::Warn,
        l if l.eq_ignore_ascii_case("info") => Level::Info,
        l if l.eq_ignore_ascii_case("debug") => Level::Debug,
        l if l.eq_ignore_ascii_case("trace") => Level::Trace,
        _ => {
            return Err(LuaError::FromLuaConversionError {
                from: "string",
                to: "log level",
                message: Some(format!(
                    "expected one of 'error', 'warn', 'info', 'debug', or 'trace'; found '{}'",
                    level
                )),
            })
        }
    };

    log!(target: target.unwrap_or("unknown lua script"), level, "{}", message);

    Ok(())
}

pub fn log<'lua>(
    lua: LuaContext<'lua>,
    (level, message): (LuaValue<'lua>, LuaString<'lua>),
) -> LuaResult<()> {
    let (level, target): (String, Option<String>) = match level {
        LuaValue::Table(table) => (table.get("level")?, table.get("target")?),
        other => (
            lua.coerce_string(other)?
                .ok_or_else(|| LuaError::FromLuaConversionError {
                    from: "lua value",
                    to: "log level or logging parameter table",
                    message: Some(format!(
                        "expected a logging level string or a table describing \
                        logging level and/or logging target"
                    )),
                })?
                .to_str()?
                .to_owned(),
            None,
        ),
    };

    log_message(lua, (&level, target.as_deref(), message.to_str()?))
}

pub fn trace<'lua>(
    lua: LuaContext<'lua>,
    (first, last): (String, Option<String>),
) -> LuaResult<()> {
    if let Some(message) = last {
        log_message(lua, ("trace", Some(&first), &message))
    } else {
        log_message(lua, ("trace", None, &first))
    }
}

pub fn debug<'lua>(
    lua: LuaContext<'lua>,
    (first, last): (String, Option<String>),
) -> LuaResult<()> {
    if let Some(message) = last {
        log_message(lua, ("debug", Some(&first), &message))
    } else {
        log_message(lua, ("debug", None, &first))
    }
}

pub fn info<'lua>(lua: LuaContext<'lua>, (first, last): (String, Option<String>)) -> LuaResult<()> {
    if let Some(message) = last {
        log_message(lua, ("info", Some(&first), &message))
    } else {
        log_message(lua, ("info", None, &first))
    }
}

pub fn warn<'lua>(lua: LuaContext<'lua>, (first, last): (String, Option<String>)) -> LuaResult<()> {
    if let Some(message) = last {
        log_message(lua, ("warn", Some(&first), &message))
    } else {
        log_message(lua, ("warn", None, &first))
    }
}

pub fn error<'lua>(
    lua: LuaContext<'lua>,
    (first, last): (String, Option<String>),
) -> LuaResult<()> {
    if let Some(message) = last {
        log_message(lua, ("error", Some(&first), &message))
    } else {
        log_message(lua, ("error", None, &first))
    }
}

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    let table = lua.create_table_from(vec![
        ("log", lua.create_function(log)?),
        ("error", lua.create_function(error)?),
        ("warn", lua.create_function(warn)?),
        ("info", lua.create_function(info)?),
        ("debug", lua.create_function(debug)?),
        ("trace", lua.create_function(trace)?),
    ])?;

    Ok(LuaValue::Table(table))
}

inventory::submit! {
    crate::api::Module::new("log", load)
}
