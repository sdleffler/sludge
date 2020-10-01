use crate::{
    ecs::{Entity, LightEntity},
    SludgeLuaContextExt,
};
use {
    anyhow::*,
    hashbrown::HashMap,
    rlua::{prelude::*, Variadic as LuaVariadic},
    std::{any::TypeId, sync::Arc},
};

mod log;
mod math;
mod thread;

pub trait Accessor: Send + Sync + 'static {
    fn to_userdata<'lua>(
        &self,
        lua: LuaContext<'lua>,
        entity: Entity,
    ) -> Result<LuaAnyUserData<'lua>>;
}

pub struct StaticAccessor {
    name: &'static str,
    accessor: Arc<dyn Accessor>,
}

impl StaticAccessor {
    pub fn new<T: Accessor + 'static>(name: &'static str, accessor: T) -> Self {
        Self {
            name,
            accessor: Arc::new(accessor),
        }
    }
}

inventory::collect!(StaticAccessor);

pub trait Template: Send + Sync + 'static {
    fn archetype(&self) -> Option<&[TypeId]> {
        None
    }

    fn constructor<'lua>(
        &self,
        lua: LuaContext<'lua>,
        args: LuaMultiValue<'lua>,
    ) -> Result<Entity> {
        self.from_table(lua, LuaTable::from_lua_multi(args, lua)?)
    }

    fn to_table<'lua>(
        &self,
        _lua: LuaContext<'lua>,
        _instance: Entity,
    ) -> Result<Option<LuaTable<'lua>>> {
        Ok(None)
    }

    fn from_table<'lua>(&self, lua: LuaContext<'lua>, table: LuaTable<'lua>) -> Result<Entity>;
}

#[derive(Debug)]
pub struct LuaTemplate {
    key: LuaRegistryKey,
}

impl Template for LuaTemplate {
    fn constructor<'lua>(
        &self,
        lua: LuaContext<'lua>,
        args: LuaMultiValue<'lua>,
    ) -> Result<Entity> {
        Ok(lua
            .registry_value::<LuaTable<'lua>>(&self.key)?
            .get::<_, LuaFunction<'lua>>("new")?
            .call::<_, LightEntity>(args)?
            .into())
    }

    fn to_table<'lua>(
        &self,
        lua: LuaContext<'lua>,
        instance: Entity,
    ) -> Result<Option<LuaTable<'lua>>> {
        Ok(lua
            .registry_value::<LuaTable<'lua>>(&self.key)?
            .get::<_, LuaFunction<'lua>>("to_table")?
            .call(LightEntity::from(instance))?)
    }

    fn from_table<'lua>(&self, lua: LuaContext<'lua>, table: LuaTable<'lua>) -> Result<Entity> {
        Ok(lua
            .registry_value::<LuaTable<'lua>>(&self.key)?
            .get::<_, LuaFunction<'lua>>("from_table")?
            .call::<_, LightEntity>(table)?
            .into())
    }
}

pub struct StaticTemplate {
    name: &'static str,
    template: Arc<dyn Template>,
}

impl StaticTemplate {
    pub fn new<T: Template + 'static>(name: &'static str, template: T) -> Self {
        Self {
            name,
            template: Arc::new(template),
        }
    }
}

inventory::collect!(StaticTemplate);

pub struct Registry {
    accessors: HashMap<String, Arc<dyn Accessor>>,
    templates: HashMap<String, Arc<dyn Template>>,
}

impl Registry {
    pub fn new() -> Result<Self> {
        let mut this = Self {
            accessors: HashMap::new(),
            templates: HashMap::new(),
        };

        inventory::iter::<StaticAccessor>()
            .into_iter()
            .try_for_each(|st| this.insert_accessor_inner(st.name, st.accessor.clone()))?;

        inventory::iter::<StaticTemplate>
            .into_iter()
            .try_for_each(|st| this.insert_template_inner(st.name, st.template.clone()))?;

        Ok(this)
    }

    fn insert_template_inner(&mut self, name: &str, template: Arc<dyn Template>) -> Result<()> {
        ensure!(
            !self.templates.contains_key(name),
            "template already exists"
        );

        self.templates.insert(name.to_owned(), template);

        Ok(())
    }

    pub fn insert_template<S, T>(&mut self, name: S, template: T) -> Result<()>
    where
        S: AsRef<str>,
        T: Template,
    {
        self.insert_template_inner(name.as_ref(), Arc::new(template))
    }

    fn insert_accessor_inner(&mut self, name: &str, accessor: Arc<dyn Accessor>) -> Result<()> {
        ensure!(
            !self.accessors.contains_key(name),
            "accessor already exists"
        );

        self.accessors.insert(name.to_owned(), accessor);

        Ok(())
    }

    pub fn insert_accessor<S, T>(&mut self, name: S, accessor: T) -> Result<()>
    where
        S: AsRef<str>,
        T: Accessor,
    {
        self.insert_accessor_inner(name.as_ref(), Arc::new(accessor))
    }
}

pub struct WrappedTemplate(Arc<dyn Template>);

impl LuaUserData for WrappedTemplate {
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Call, |lua, this, args| {
            let entity = this.0.constructor(lua, args).to_lua_err()?;
            Ok(LightEntity::from(entity))
        });

        methods.add_method("to_table", |lua, this, entity: LightEntity| {
            this.0.to_table(lua, entity.into()).to_lua_err()
        });

        methods.add_method("from_table", |lua, this, table: LuaTable<'lua>| {
            this.0
                .from_table(lua, table)
                .map(LightEntity::from)
                .to_lua_err()
        });
    }
}

pub type ModuleLoader = Box<dyn for<'lua> Fn(LuaContext<'lua>) -> Result<LuaValue<'lua>> + 'static>;

pub struct Module {
    name: &'static str,
    load: ModuleLoader,
}

impl Module {
    pub fn new<F>(name: &'static str, load: F) -> Self
    where
        F: for<'lua> Fn(LuaContext<'lua>) -> Result<LuaValue<'lua>> + 'static,
    {
        Self {
            name,
            load: Box::new(load),
        }
    }
}

inventory::collect!(Module);

pub fn sludge_template<'lua>(lua: LuaContext<'lua>, name: String) -> LuaResult<LuaTable<'lua>> {
    let table = lua.create_table()?;
    let key = lua.create_registry_value(table.clone())?;
    lua.resources()
        .fetch_mut::<Registry>()
        .insert_template(&name, LuaTemplate { key })
        .to_lua_err()?;

    Ok(table)
}

pub struct Templates;

impl LuaUserData for Templates {
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Index, |lua, _, name: LuaString<'lua>| {
            Ok(WrappedTemplate(
                lua.resources().fetch::<Registry>().templates[name.to_str()?].clone(),
            ))
        });
    }
}

pub fn sludge_to_accessor<'lua>(
    lua: LuaContext<'lua>,
    (entity, accessors): (LightEntity, LuaVariadic<String>),
) -> LuaResult<LuaMultiValue<'lua>> {
    let mut out = Vec::new();
    let resources = lua.resources();
    let registry = resources.fetch::<Registry>();
    for accessor_name in accessors {
        if let Some(accessor) = registry.accessors.get(&accessor_name) {
            let userdata = accessor.to_userdata(lua, entity.into()).to_lua_err()?;
            out.push(LuaValue::UserData(userdata));
        } else {
            out.push(LuaValue::Nil);
        }
    }
    Ok(LuaMultiValue::from_vec(out))
}

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    let table = lua.create_table_from(vec![
        (
            "Template",
            LuaValue::Function(lua.create_function(sludge_template)?),
        ),
        (
            "templates",
            LuaValue::UserData(lua.create_userdata(Templates)?),
        ),
        (
            "to_accessor",
            LuaValue::Function(lua.create_function(sludge_to_accessor)?),
        ),
    ])?;

    ["print", "dofile", "load", "loadstring", "loadfile"]
        .iter()
        .try_for_each(|&s| lua.globals().set(s, LuaValue::Nil))?;

    for module in inventory::iter::<Module> {
        table.set(module.name, (module.load)(lua)?)?;
    }

    Ok(LuaValue::Table(table))
}
