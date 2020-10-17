use crate::{
    ecs::{Component, Entity, EntityBuilder, World},
    Resources, SludgeLuaContextExt,
};
use {
    anyhow::*,
    derivative::*,
    hashbrown::{HashMap, HashSet},
    rlua::{prelude::*, Variadic as LuaVariadic},
    std::{
        any::TypeId,
        sync::{Arc, Mutex},
    },
};

mod log;
mod math;
mod thread;

pub struct EntityUserDataRegistry {
    archetypes: Mutex<HashMap<Vec<TypeId>, Vec<(&'static str, LuaComponent)>>>,
    registered: HashMap<TypeId, LuaComponent>,
    named: HashMap<String, LuaComponent>,
}

impl EntityUserDataRegistry {
    pub fn new() -> Self {
        let mut registered = HashMap::new();
        let mut named = HashMap::new();
        let mut fields = HashSet::new();

        for component in inventory::iter::<LuaComponent> {
            registered.insert(component.type_id, component.clone());
            assert!(
                named
                    .insert(component.type_name.to_owned(), component.clone())
                    .is_none(),
                "component already registered with type name `{}`",
                component.type_name
            );

            assert!(
                fields.insert(component.field_name.to_owned()),
                "component already registered with field name `{}`",
                component.field_name,
            );
        }

        Self {
            archetypes: Mutex::new(HashMap::new()),
            registered,
            named,
        }
    }

    fn get_archetype<'lua>(
        &self,
        lua: LuaContext<'lua>,
        entity: Entity,
        archetype: impl IntoIterator<Item = TypeId>,
    ) -> LuaResult<LuaTable<'lua>> {
        let mut scratch = Vec::new();
        scratch.extend(archetype);

        let mut archetypes = self.archetypes.lock().unwrap();
        if !archetypes.contains_key(&scratch) {
            let components = scratch
                .iter()
                .filter_map(|type_id| self.registered.get(&type_id))
                .map(|c| (c.field_name, c.clone()))
                .collect();
            archetypes.insert(scratch.clone(), components);
        }

        let table = lua.create_table()?;
        for &(field_name, ref component) in &archetypes[&scratch] {
            table.set(field_name, (component.accessor)(lua, entity)?)?;
        }

        Ok(table)
    }
}

pub trait LuaComponentUserData: Component {
    type Accessor: LuaUserData + Send;
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<Self::Accessor>;
    fn bundler<'lua>(
        lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()>;
}

pub type AccessorConstructor = Arc<
    dyn for<'lua> Fn(LuaContext<'lua>, Entity) -> LuaResult<LuaAnyUserData<'lua>> + Send + Sync,
>;

pub type BundlerConstructor = Arc<
    dyn for<'lua> Fn(LuaContext<'lua>, LuaValue<'lua>, &mut EntityBuilder) -> LuaResult<()>
        + Send
        + Sync,
>;

#[derive(Derivative, Clone)]
#[derivative(Debug)]
pub struct LuaComponent {
    type_name: &'static str,
    field_name: &'static str,
    type_id: TypeId,

    #[derivative(Debug = "ignore")]
    accessor: AccessorConstructor,

    #[derivative(Debug = "ignore")]
    bundler: BundlerConstructor,
}

impl LuaComponent {
    pub fn new<T: LuaComponentUserData>(type_name: &'static str, field_name: &'static str) -> Self {
        Self {
            type_name,
            field_name,
            type_id: TypeId::of::<T>(),
            accessor: Arc::new(|lua, entity| lua.create_userdata(T::accessor(lua, entity)?)),
            bundler: Arc::new(T::bundler),
        }
    }
}

inventory::collect!(LuaComponent);

#[derive(Debug, Clone, Copy)]
struct LuaEntityUserData(u64);

impl LuaUserData for LuaEntityUserData {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_meta_function(
            LuaMetaMethod::Index,
            |_lua, (ud, key): (LuaAnyUserData, LuaString)| {
                let table = ud.get_user_value::<LuaTable>()?;
                table.get::<_, LuaValue>(key)
            },
        );

        methods.add_meta_method(
            LuaMetaMethod::NewIndex,
            |lua, this, (k, v): (LuaString, _)| {
                let resources = lua.resources();
                let registry = resources.fetch::<EntityUserDataRegistry>();
                let mut world = resources.fetch_mut::<World>();
                let mut builder = EntityBuilder::new();

                let s = k.to_str()?;
                let bundler = match registry.named.get(s) {
                    Some(comp) => &comp.bundler,
                    None => return Err(format_err!("unknown component {}", s)).to_lua_err(),
                };
                bundler(lua, v, &mut builder)?;
                world
                    .insert(Entity::from_bits(this.0), builder.build())
                    .to_lua_err()?;

                Ok(())
            },
        );

        methods.add_method("despawn", |lua, this, ()| {
            lua.resources()
                .fetch_mut::<World>()
                .despawn(Entity::from(*this))
                .to_lua_err()?;
            Ok(())
        });

        methods.add_meta_method(LuaMetaMethod::ToString, |_lua, this, ()| {
            Ok(format!("{:?}", Entity::from_bits(this.0)))
        });

        methods.add_meta_function(
            LuaMetaMethod::Eq,
            |lua, (this, other): (LuaValue, LuaValue)| {
                let (this, other) = match (
                    LuaAnyUserData::from_lua(this, lua),
                    LuaAnyUserData::from_lua(other, lua),
                ) {
                    (Ok(this), Ok(other)) => (this, other),
                    _ => return Ok(false),
                };

                // Temporary here to pacify borrow checker.
                let t = (this.borrow::<Self>(), other.borrow::<Self>());
                match t {
                    (Ok(this), Ok(other)) => {
                        Ok(Entity::from_bits(this.0) == Entity::from_bits(other.0))
                    }
                    _ => Ok(false),
                }
            },
        );

        methods.add_meta_function(
            LuaMetaMethod::Lt,
            |lua, (this, other): (LuaValue, LuaValue)| {
                let (this, other) = match (
                    LuaAnyUserData::from_lua(this, lua),
                    LuaAnyUserData::from_lua(other, lua),
                ) {
                    (Ok(this), Ok(other)) => (this, other),
                    _ => return Ok(false),
                };

                // Temporary here to pacify borrow checker.
                let t = (this.borrow::<Self>(), other.borrow::<Self>());
                match t {
                    (Ok(this), Ok(other)) => {
                        Ok(Entity::from_bits(this.0) < Entity::from_bits(other.0))
                    }
                    _ => Ok(false),
                }
            },
        );

        methods.add_meta_function(
            LuaMetaMethod::Le,
            |lua, (this, other): (LuaValue, LuaValue)| {
                let (this, other) = match (
                    LuaAnyUserData::from_lua(this, lua),
                    LuaAnyUserData::from_lua(other, lua),
                ) {
                    (Ok(this), Ok(other)) => (this, other),
                    _ => return Ok(false),
                };

                // Temporary here to pacify borrow checker.
                let t = (this.borrow::<Self>(), other.borrow::<Self>());
                match t {
                    (Ok(this), Ok(other)) => {
                        Ok(Entity::from_bits(this.0) <= Entity::from_bits(other.0))
                    }
                    _ => Ok(false),
                }
            },
        );

        methods.add_meta_method(LuaMetaMethod::Persist, |_lua, _this, ()| -> LuaResult<()> {
            Err(format_err!("persistence not supported yet")).to_lua_err()
        });
    }
}

impl From<LuaEntityUserData> for Entity {
    fn from(leud: LuaEntityUserData) -> Entity {
        Entity::from_bits(leud.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LuaEntity(u64);

impl From<Entity> for LuaEntity {
    fn from(entity: Entity) -> LuaEntity {
        Self(entity.to_bits())
    }
}

impl From<LuaEntity> for Entity {
    fn from(wrapped: LuaEntity) -> Entity {
        Entity::from_bits(wrapped.0)
    }
}

impl<'lua> ToLua<'lua> for LuaEntity {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        let resources = lua.resources();
        let world = resources.fetch::<World>();
        let registry = resources.fetch::<EntityUserDataRegistry>();

        let ud = lua.create_userdata(LuaEntityUserData(self.0))?;
        let entity = Entity::from_bits(self.0);
        let entity_ref = world.entity(entity).unwrap();
        let archetype = entity_ref.component_types();
        let fields = registry.get_archetype(lua, entity, archetype)?;

        ud.set_user_value(fields)?;
        ud.to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for LuaEntity {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        LuaEntityUserData::from_lua(lua_value, lua).map(|ud| LuaEntity(ud.0))
    }
}

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
            .get::<_, LuaFunction<'lua>>("spawn")?
            .call::<_, LuaEntity>(args)?
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
            .call(LuaEntity::from(instance))?)
    }

    fn from_table<'lua>(&self, lua: LuaContext<'lua>, table: LuaTable<'lua>) -> Result<Entity> {
        Ok(lua
            .registry_value::<LuaTable<'lua>>(&self.key)?
            .get::<_, LuaFunction<'lua>>("from_table")?
            .call::<_, LuaEntity>(table)?
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
        methods.add_method("spawn", |lua, this, args| {
            let entity = this.0.constructor(lua, args).to_lua_err()?;
            Ok(LuaEntity::from(entity))
        });

        methods.add_method("to_table", |lua, this, entity: LuaEntity| {
            this.0.to_table(lua, entity.into()).to_lua_err()
        });

        methods.add_method("from_table", |lua, this, table: LuaTable<'lua>| {
            this.0
                .from_table(lua, table)
                .map(LuaEntity::from)
                .to_lua_err()
        });
    }
}

pub type ModuleLoader = Box<dyn for<'lua> Fn(LuaContext<'lua>) -> Result<LuaValue<'lua>> + 'static>;

pub struct Module {
    path: Vec<&'static str>,
    load: ModuleLoader,
}

impl Module {
    pub fn new<F>(path: &'static [&'static str], load: F) -> Self
    where
        F: for<'lua> Fn(LuaContext<'lua>) -> Result<LuaValue<'lua>> + 'static,
    {
        Self {
            path: path.to_owned(),
            load: Box::new(load),
        }
    }

    pub fn parse<F>(path: &'static str, load: F) -> Self
    where
        F: for<'lua> Fn(LuaContext<'lua>) -> Result<LuaValue<'lua>> + 'static,
    {
        Self {
            path: path.split(".").collect(),
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

inventory::submit! {
    Module::parse("sludge.Template", |lua| {
        Ok(LuaValue::Function(lua.create_function(sludge_template)?))
    })
}

pub struct Templates;

impl LuaUserData for Templates {
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Index, |lua, _, lua_name: LuaString<'lua>| {
            let name = lua_name.to_str()?;
            Ok(WrappedTemplate(
                lua.resources()
                    .fetch::<Registry>()
                    .templates
                    .get(name)
                    .ok_or_else(|| anyhow!("no such template `{}`", name))
                    .to_lua_err()?
                    .clone(),
            ))
        });
    }
}

inventory::submit! {
    Module::parse("sludge.templates", |lua| {
        Ok(LuaValue::UserData(lua.create_userdata(Templates)?))
    })
}

pub fn sludge_to_accessor<'lua>(
    lua: LuaContext<'lua>,
    (entity, accessors): (LuaEntity, LuaVariadic<String>),
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

inventory::submit! {
    Module::parse("sludge.to_accessor", |lua| {
        Ok(LuaValue::Function(lua.create_function(sludge_to_accessor)?))
    })
}

pub fn spawn<'lua>(lua: LuaContext<'lua>, table: LuaTable<'lua>) -> LuaResult<LuaEntity> {
    let resources = lua.resources();
    let registry = resources.fetch::<EntityUserDataRegistry>();
    let mut world = resources.fetch_mut::<World>();
    let mut builder = EntityBuilder::new();

    for pair in table.pairs::<LuaString, LuaValue<'lua>>() {
        let (k, v) = pair?;
        let s = k.to_str()?;
        let bundler = match registry.named.get(s) {
            Some(comp) => &comp.bundler,
            None => return Err(format_err!("unknown component {}", s)).to_lua_err(),
        };
        bundler(lua, v, &mut builder)?;
    }

    Ok(LuaEntity::from(world.spawn(builder.build())))
}

inventory::submit! {
    Module::parse("sludge.spawn", |lua| {
        Ok(LuaValue::Function(lua.create_function(spawn)?))
    })
}

pub fn despawn<'lua>(lua: LuaContext<'lua>, entity: LuaEntity) -> LuaResult<Result<bool, String>> {
    Ok(lua
        .resources()
        .fetch_mut::<World>()
        .despawn(entity.into())
        .map(|_| true)
        .map_err(|err| err.to_string()))
}

inventory::submit! {
    Module::parse("sludge.despawn", |lua| {
        Ok(LuaValue::Function(lua.create_function(despawn)?))
    })
}

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<()> {
    ["print", "dofile", "load", "loadstring", "loadfile"]
        .iter()
        .try_for_each(|&s| lua.globals().set(s, LuaValue::Nil))?;

    for module in inventory::iter::<Module> {
        let mut t = lua.globals();
        let (&head, rest) = module
            .path
            .split_last()
            .ok_or_else(|| anyhow!("empty module path!"))?;

        for &ident in rest.iter() {
            t = match t.get::<_, Option<LuaTable<'lua>>>(ident)? {
                Some(subtable) => subtable,
                None => {
                    let subtable = lua.create_table()?;
                    t.set(ident, subtable.clone())?;
                    subtable
                }
            };
        }

        ensure!(
            !t.contains_key(head)?,
            "name collision while loading modules: two modules have the same path `{}`",
            module.path.join(".")
        );
        t.set(head, (module.load)(lua)?)?;
    }

    Ok(())
}
