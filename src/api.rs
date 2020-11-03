use crate::{
    ecs::{Component, Entity, EntityBuilder, World},
    filesystem::Filesystem,
    Resources, SimpleComponent, SludgeLuaContextExt, SludgeResultExt,
};
use {
    anyhow::*,
    derivative::*,
    hashbrown::HashMap,
    rlua::prelude::*,
    std::{
        any::TypeId,
        io::Read,
        sync::{Arc, Mutex},
    },
};

mod log;
mod math;
mod thread;

pub const SERIALIZER_THUNK_REGISTRY_KEY: &'static str = "sludge.serialize";
pub const LOOKUP_THUNK_REGISTRY_KEY: &'static str = "sludge.lookup";
pub const WORLD_TABLE_REGISTRY_KEY: &'static str = "sludge.world_table";
pub const PERMANENTS_SER_TABLE_REGISTRY_KEY: &'static str = "sludge.permanents_ser";
pub const PERMANENTS_DE_TABLE_REGISTRY_KEY: &'static str = "sludge.permanents_de";
pub const PLAYBACK_THUNK_REGISTRY_KEY: &'static str = "sludge.playback_thunk";
pub const PACKAGE_REGISTRY_KEY: &'static str = "sludge.package";
pub const DEFAULT_PACKAGE_PATH: &'static str = "/?.lua";

pub struct EntityUserDataRegistry {
    archetypes: Mutex<HashMap<Vec<TypeId>, Vec<(&'static str, LuaComponent)>>>,
    registered: HashMap<TypeId, LuaComponent>,
    named: HashMap<String, LuaComponent>,
}

impl EntityUserDataRegistry {
    pub fn new() -> Self {
        let mut registered = HashMap::new();
        let mut named = HashMap::new();

        for component in inventory::iter::<LuaComponent> {
            registered.insert(component.type_id, component.clone());
            assert!(
                named
                    .insert(component.type_name.to_owned(), component.clone())
                    .is_none(),
                "component already registered with type name `{}`",
                component.type_name
            );
        }

        Self {
            archetypes: Mutex::new(HashMap::new()),
            registered,
            named,
        }
    }

    pub fn get_archetype<'lua>(
        &self,
        lua: LuaContext<'lua>,
        entity: Entity,
    ) -> LuaResult<LuaTable<'lua>> {
        let resources = lua.resources();
        let world = resources.fetch::<World>();
        let entity_ref = world.entity(entity).unwrap();
        let archetype = entity_ref.component_types();

        let mut scratch = Vec::new();
        scratch.extend(archetype);

        let mut archetypes = self.archetypes.lock().unwrap();
        if !archetypes.contains_key(&scratch) {
            let components = scratch
                .iter()
                .filter_map(|type_id| self.registered.get(&type_id))
                .map(|c| (c.type_name, c.clone()))
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

pub trait LuaComponentInterface: Component {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>>;
    fn bundler<'lua>(
        lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()>;
}

pub type AccessorConstructor =
    Arc<dyn for<'lua> Fn(LuaContext<'lua>, Entity) -> LuaResult<LuaValue<'lua>> + Send + Sync>;

pub type BundlerConstructor = Arc<
    dyn for<'lua> Fn(LuaContext<'lua>, LuaValue<'lua>, &mut EntityBuilder) -> LuaResult<()>
        + Send
        + Sync,
>;

#[derive(Derivative, Clone)]
#[derivative(Debug)]
pub struct LuaComponent {
    type_name: &'static str,
    type_id: TypeId,

    #[derivative(Debug = "ignore")]
    accessor: AccessorConstructor,

    #[derivative(Debug = "ignore")]
    bundler: BundlerConstructor,
}

impl LuaComponent {
    pub fn new<T: LuaComponentInterface>(type_name: &'static str) -> Self {
        Self {
            type_name,
            type_id: TypeId::of::<T>(),
            accessor: Arc::new(|lua, entity| T::accessor(lua, entity)?.to_lua(lua)),
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

        methods.add_meta_method(
            LuaMetaMethod::Persist,
            |lua, this, ()| -> LuaResult<Option<LuaFunction>> {
                let lookup_thunk =
                    lua.named_registry_value::<_, LuaFunction>(LOOKUP_THUNK_REGISTRY_KEY)?;
                let world_table =
                    lua.named_registry_value::<_, LuaTable>(WORLD_TABLE_REGISTRY_KEY)?;
                lookup_thunk.call((
                    LuaLightUserData(Entity::from_bits(this.0).id() as *mut _),
                    world_table,
                ))
            },
        );
    }
}

impl From<LuaEntityUserData> for Entity {
    fn from(leud: LuaEntityUserData) -> Entity {
        Entity::from_bits(leud.0)
    }
}

/// An [`Entity`] wrapped for use with Lua and provided with a metatable that
/// allows for Lua operations on it, for components which support such.
///
/// # Persistence
///
/// Once passed to `Lua`, a `LuaEntity` becomes a userdata object which is
/// persisted as light userdata containing the 32-bit version of the entity
/// ID.
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
        let registry = resources.fetch::<EntityUserDataRegistry>();

        let ud = lua.create_userdata(LuaEntityUserData(self.0))?;
        let entity = Entity::from_bits(self.0);
        let fields = registry.get_archetype(lua, entity)?;

        ud.set_user_value(fields)?;
        ud.to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for LuaEntity {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        LuaEntityUserData::from_lua(lua_value, lua).map(|ud| LuaEntity(ud.0))
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

/// A component providing special behavior to an entity through hooks in the Lua API,
/// such as serialization/deserialization behavior.
///
/// # Persistence
///
/// ```lua
/// -- This function is called when the world is being serialized. It receives the
/// -- entity as well as a table created from the results of calling the `to_table`
/// -- method on all of its components' accessors, and is expected to return a Lua
/// -- table used to reconstruct the entity.
/// function EntityTable.serialize(entity, table)
///     return table -- by default we just pass the table through.
/// end
/// -- This function is called when the world is being deserialized. It receives
/// -- the table which returned by `EntityTable.serialize` and is expected to reconstruct
/// -- the serialized entity.
/// function EntityTable.deserialize(table)
///     sludge.spawn(table) -- by default we just assume the table can be spawned.
/// end
/// ```
#[derive(Debug, SimpleComponent)]
pub struct EntityTable {
    pub(crate) key: LuaRegistryKey,
}

#[derive(Debug, Clone, Copy)]
pub struct EntityTableAccessor(Entity);

impl LuaUserData for EntityTableAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("get", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let et = world.get::<EntityTable>(this.0).to_lua_err()?;
            lua.registry_value::<LuaValue>(&et.key)
        });

        methods.add_method("to_table", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let et = world.get::<EntityTable>(this.0).to_lua_err()?;
            lua.registry_value::<LuaTable>(&et.key)
        });
    }
}

impl LuaComponentInterface for EntityTable {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        EntityTableAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let table = LuaTable::from_lua(args, lua)?;
        let key = lua.create_registry_value(table)?;
        builder.add(EntityTable { key });
        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<EntityTable>("Table")
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

pub fn insert<'lua>(
    lua: LuaContext<'lua>,
    (entity, table): (LuaEntity, LuaTable<'lua>),
) -> LuaResult<()> {
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

    world.insert(entity.into(), builder.build()).to_lua_err()?;

    Ok(())
}

pub fn despawn<'lua>(lua: LuaContext<'lua>, entity: LuaEntity) -> LuaResult<Result<bool, String>> {
    Ok(lua
        .resources()
        .fetch_mut::<World>()
        .despawn(entity.into())
        .map(|_| true)
        .map_err(|err| err.to_string()))
}

pub fn clear<'lua>(lua: LuaContext<'lua>, _: ()) -> LuaResult<()> {
    lua.resources().fetch_mut::<World>().clear();
    Ok(())
}

inventory::submit! {
    Module::parse("sludge", |lua| {
        let table = lua.create_table_from(vec![
            ("spawn", lua.create_function(spawn)?),
            ("insert", lua.create_function(insert)?),
            ("despawn", lua.create_function(despawn)?),
            ("clear", lua.create_function(clear)?),
        ])?;

        Ok(LuaValue::Table(table))
    })
}

/// Lua-exposed function for loading a module from sludge's `Filesystem`.
///
/// Similar to Lua's built-in `require`, this will search along paths found in
/// `sludge.package.path`, which is expected to be a colon-separated list of
/// paths to search, where any `?` characters found are replaced by the module
/// path being searched for. The default value of `sludge.package.path` is "/?.lua",
/// which will simply search for any Lua files found in the VFS.
///
/// The limitations of opening files through this `require` are the same as opening
/// any file through the `Filesystem`.
pub fn require<'lua>(lua: LuaContext<'lua>, module: String) -> LuaResult<LuaValue> {
    let package = lua.named_registry_value::<_, LuaTable>(PACKAGE_REGISTRY_KEY)?;
    let loaded_modules = package.get::<_, LuaTable>("modules")?;
    if let Some(module) = loaded_modules.get::<_, Option<LuaValue>>(module.as_str())? {
        Ok(module)
    } else {
        let resources = lua.resources();
        let mut fs = resources.fetch_mut::<Filesystem>();
        let package_path = package.get::<_, LuaString>("path")?;
        let segments = package_path.to_str()?.split(":");

        for segment in segments {
            let path = segment.replace('?', &module);
            let mut file = match fs.open(&path) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let mut buf = String::new();
            file.read_to_string(&mut buf)
                .log_error_err(module_path!())
                .to_lua_err()?;
            let loaded = lua
                .load(&buf)
                .set_name(&module)?
                .into_function()?
                .call::<_, LuaValue>(())?;
            loaded_modules.set(path.as_str(), loaded.clone())?;
            return Ok(loaded);
        }

        // FIXME: better error reporting here; collect errors from individual module attempts
        // and log them?
        Err(anyhow!("module not found!")).to_lua_err()
    }
}

inventory::submit! {
    Module::parse("sludge.package", |lua| {
        let table = lua.create_table()?;
        table.set("path", DEFAULT_PACKAGE_PATH)?;

        let req_fn = lua.create_function(require)?;
        table.set("require", req_fn.clone())?;
        lua.globals().set("require", req_fn)?;

        let modules = lua.create_table()?;
        table.set("modules", modules.clone())?;

        lua.set_named_registry_value(PACKAGE_REGISTRY_KEY, table.clone())?;

        Ok(LuaValue::Table(table))
    })
}

pub trait SludgeApiLuaContextExt<'lua> {
    fn register_permanents(&self, key: &str, value: impl ToLua<'lua>) -> LuaResult<()>;
}

impl<'lua> SludgeApiLuaContextExt<'lua> for LuaContext<'lua> {
    fn register_permanents(&self, key: &str, value: impl ToLua<'lua>) -> LuaResult<()> {
        let ser_table =
            self.named_registry_value::<_, LuaTable>(PERMANENTS_SER_TABLE_REGISTRY_KEY)?;
        let de_table =
            self.named_registry_value::<_, LuaTable>(PERMANENTS_DE_TABLE_REGISTRY_KEY)?;
        let value = value.to_lua(*self)?;

        if ser_table.contains_key(value.clone())? {
            return Ok(());
        }

        ser_table.set(value.clone(), key)?;
        de_table.set(key, value.clone())?;

        let table = match value {
            LuaValue::Table(t) => t,
            _ => return Ok(()),
        };

        let mut buf = String::new();
        for pair in table.pairs() {
            let (k, v): (LuaValue, LuaValue) = pair?;
            if let Some(s) = self.coerce_string(k)? {
                buf.clear();
                buf.push_str(key);
                buf.push('.');
                buf.push_str(s.to_str()?);
                self.register_permanents(&buf, v)?;
            }
        }

        Ok(())
    }
}

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<()> {
    [
        "dofile",
        "load",
        "loadfile",
        "loadstring",
        "print",
        "require",
    ]
    .iter()
    .try_for_each(|&s| lua.globals().set(s, LuaValue::Nil))?;

    lua.set_named_registry_value(PERMANENTS_SER_TABLE_REGISTRY_KEY, lua.create_table()?)?;
    lua.set_named_registry_value(PERMANENTS_DE_TABLE_REGISTRY_KEY, lua.create_table()?)?;

    for pair in lua.globals().pairs::<LuaValue, LuaValue>() {
        let (k, v) = pair?;

        if let Ok(lua_str) = LuaString::from_lua(k, lua) {
            let s = lua_str.to_str()?;
            lua.register_permanents(s, v)?;
        }
    }

    // Sort the modules by their paths in lexicographical order, so that parent modules
    // are always loaded before their children and we don't end up with a parent thinking
    // it's a duplicate because loading the child caused the parent's table to be created.
    // Also avoids overwriting loaded children.
    let mut modules = inventory::iter::<Module>.into_iter().collect::<Vec<_>>();
    modules.sort_unstable_by_key(|m| &m.path);

    for module in modules.iter() {
        let mut t = lua.globals();
        let (&last, rest) = module
            .path
            .split_last()
            .ok_or_else(|| anyhow!("empty module path!"))?;

        let mut path = String::new();
        for &ident in rest.iter() {
            t = match t.get::<_, Option<LuaTable<'lua>>>(ident)? {
                Some(subtable) => subtable,
                None => {
                    let subtable = lua.create_table()?;
                    t.set(ident, subtable.clone())?;
                    subtable
                }
            };

            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(ident);
            lua.register_permanents(&path, t.clone())?;
        }

        ensure!(
            !t.contains_key(last)?,
            "name collision while loading modules: two modules have the same path `{}`",
            module.path.join(".")
        );
        let table = (module.load)(lua)?;
        lua.register_permanents(&module.path.join("."), table.clone())?;
        t.set(last, table)?;
    }

    lua.set_named_registry_value(
        SERIALIZER_THUNK_REGISTRY_KEY,
        lua.load(include_str!("api/lua/serializer_thunk.lua"))
            .set_name("serializer")?
            .eval::<LuaFunction>()?,
    )?;

    lua.set_named_registry_value(
        LOOKUP_THUNK_REGISTRY_KEY,
        lua.load(include_str!("api/lua/lookup_thunk.lua"))
            .set_name("lookup")?
            .eval::<LuaFunction>()?,
    )?;

    lua.set_named_registry_value(
        PLAYBACK_THUNK_REGISTRY_KEY,
        lua.load(include_str!("api/lua/playback_thunk.lua"))
            .set_name("playback")?
            .eval::<LuaFunction>()?,
    )?;

    Ok(())
}
