use crate::SludgeLuaContextExt;
use {anyhow::*, rlua::prelude::*};

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    Ok(LuaValue::Table(
        lua.create_table_from(vec![("foo", LuaValue::Nil)])?,
    ))
}

inventory::submit! {
    crate::api::Module::new("components", load)
}
