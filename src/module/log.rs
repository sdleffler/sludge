use {
    anyhow::Result,
    log::{log, Level},
    rlua::prelude::*,
};

use crate::Module;

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

    match target {
        Some(target) => log!(target: &target, level, "{}", message),
        None => log!(level, "{}", message),
    }

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

#[derive(Debug, Clone, Copy)]
pub struct LogModule;

impl Module for LogModule {
    fn load<'lua>(&self, lua: LuaContext<'lua>) -> Result<(&str, LuaTable<'lua>)> {
        let table = lua.create_table_from(vec![
            ("log", lua.create_function(log)?),
            ("error", lua.create_function(error)?),
            ("warn", lua.create_function(warn)?),
            ("info", lua.create_function(info)?),
            ("debug", lua.create_function(debug)?),
            ("trace", lua.create_function(trace)?),
        ])?;

        Ok(("log", table))
    }
}

/// Basic logging setup to log to the console with `fern`.
pub fn setup_logging() {
    use fern::colors::{Color, ColoredLevelConfig};
    let colors = ColoredLevelConfig::default()
        .info(Color::Green)
        .debug(Color::BrightMagenta)
        .trace(Color::BrightBlue);
    // This sets up a `fern` logger and initializes `log`.
    fern::Dispatch::new()
        // Formats logs
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}][{:<5}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                colors.color(record.level()),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Warn)
        // Filter out unnecessary stuff
        .level_for("gfx", log::LevelFilter::Off)
        // .level_for("walk", log::LevelFilter::Warn)
        // Set levels for stuff we care about
        .level_for("threething", log::LevelFilter::Trace)
        // Hooks up console output.
        // env var for outputting to a file?
        // Haven't needed it yet!
        .chain(std::io::stdout())
        .apply()
        .expect("Could not init logging!");
}
