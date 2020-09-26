use crate::{
    ecs::{Component, Entity, World},
    SludgeLuaContextExt,
};
use {
    anyhow::Result,
    hashbrown::{HashMap, HashSet},
    hecs::{Archetype, EntityBuilder, NoSuchEntity},
    rlua::prelude::*,
    std::any::{Any, TypeId},
};

pub trait RegisterableComponent: Component {
    fn constructor(_lua: LuaContext) -> Result<Option<(&'static str, LuaFunction)>> {
        Ok(None)
    }

    fn method_table(lua: LuaContext) -> Result<LuaTable> {
        lua.create_table().map_err(Into::into)
    }
}

pub struct ArchetypeRegistryEntry {
    consolidated: LuaRegistryKey,
}

pub struct ArchetypeRegistry {
    constructors: LuaRegistryKey,
    method_tables: HashMap<TypeId, LuaRegistryKey>,
    fresh: HashSet<TypeId>,
    archetypes: HashMap<Box<[TypeId]>, ArchetypeRegistryEntry>,
}

impl ArchetypeRegistry {
    pub fn new(lua: LuaContext) -> Result<Self> {
        let constructors = lua.create_table()?;

        Ok(Self {
            constructors: lua.create_registry_value(constructors)?,
            method_tables: HashMap::new(),
            fresh: HashSet::new(),
            archetypes: HashMap::new(),
        })
    }

    pub fn register<T: RegisterableComponent>(&mut self, lua: LuaContext) -> Result<()> {
        if let Some((key, value)) = T::constructor(lua)? {
            let constructors = lua.registry_value::<LuaTable>(&self.constructors)?;
            constructors.set(key, value)?;
        }

        let table = lua.create_registry_value(T::method_table(lua)?)?;
        self.method_tables.insert(TypeId::of::<T>(), table);
        self.fresh.insert(TypeId::of::<T>());

        Ok(())
    }

    pub fn generate_method_tables(&mut self, lua: LuaContext, archetypes: &[Archetype]) {
        for archetype in archetypes {
            // Only generate new consolidated method tables for archetypes which have newly
            // registered method tables for at least one component type.
            if !archetype
                .component_types()
                .any(|typeid| self.fresh.contains(&typeid))
            {
                continue;
            }

            let types = archetype
                .component_types()
                .collect::<Vec<_>>()
                .into_boxed_slice();

            let methods = lua.create_table().unwrap();
            for key in types
                .iter()
                .filter_map(|typeid| self.method_tables.get(typeid))
            {
                let table = lua.registry_value::<LuaTable>(key).unwrap();
                for pair in table.pairs::<LuaString, LuaFunction>() {
                    let (k, v) = pair.unwrap();
                    methods.set(k, v).unwrap();
                }
            }

            let consolidated = lua.create_registry_value(methods).unwrap();
            let entry = ArchetypeRegistryEntry { consolidated };

            if let Some(old) = self.archetypes.insert(types, entry) {
                lua.remove_registry_value(old.consolidated).unwrap();
            }
        }

        self.fresh.clear();
    }

    pub fn constructors(&self) -> &LuaRegistryKey {
        &self.constructors
    }

    pub fn lookup_method_table(&self, components: &[TypeId]) -> Option<&LuaRegistryKey> {
        self.archetypes
            .get(components)
            .map(|entry| &entry.consolidated)
    }
}

pub trait AnyComponent: Component + Any {
    fn as_any(&self) -> &dyn Any;
    fn type_id(&self) -> TypeId {
        self.as_any().type_id()
    }
    fn clone_boxed(&self) -> Box<dyn AnyComponent>;

    fn insert_one(self: Box<Self>, world: &mut World, entity: Entity) -> Result<(), NoSuchEntity>;
    fn add(self: Box<Self>, entity_builder: &mut EntityBuilder);
}

impl<T: Component + Any + Clone> AnyComponent for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_boxed(&self) -> Box<dyn AnyComponent> {
        Box::new(self.clone())
    }

    fn insert_one(self: Box<Self>, world: &mut World, entity: Entity) -> Result<(), NoSuchEntity> {
        world.insert_one(entity, *self)
    }

    fn add(self: Box<Self>, entity_builder: &mut EntityBuilder) {
        entity_builder.add(*self);
    }
}

pub struct EntityBuilderWrapper(pub EntityBuilder);

impl LuaUserData for EntityBuilderWrapper {
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method_mut("add", |_ctx, this, wrapped: ComponentWrapper| {
            wrapped.boxed.clone_boxed().add(&mut this.0);
            Ok(())
        });
    }
}

pub struct ComponentWrapper {
    boxed: Box<dyn AnyComponent>,
}

impl Clone for ComponentWrapper {
    fn clone(&self) -> Self {
        Self {
            boxed: self.boxed.clone_boxed(),
        }
    }
}

impl ComponentWrapper {
    pub fn new<T: AnyComponent>(component: T) -> Self {
        Self {
            boxed: Box::new(component),
        }
    }
}

impl LuaUserData for ComponentWrapper {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityWrapper(pub Entity);

impl LuaUserData for EntityWrapper {
    fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_function(
            LuaMetaMethod::Index,
            |lua, (this, key): (LuaAnyUserData, LuaString)| {
                let table = match this.get_user_value::<Option<LuaTable>>()? {
                    Some(t) => t,
                    None => {
                        let entity = this.borrow::<EntityWrapper>().unwrap().0;
                        let resources = lua.resources();
                        let types = resources
                            .fetch::<World>()
                            .entity(entity)
                            .unwrap()
                            .component_types()
                            .collect::<Vec<_>>();

                        let registry = resources.fetch::<ArchetypeRegistry>();
                        match registry.lookup_method_table(&types) {
                            Some(key) => {
                                let t = lua.registry_value::<LuaTable>(key)?;
                                this.set_user_value(t.clone())?;
                                t
                            }
                            None => return Ok(None),
                        }
                    }
                };

                Ok(table.get::<_, Option<LuaFunction>>(key)?)
            },
        );
    }
}
