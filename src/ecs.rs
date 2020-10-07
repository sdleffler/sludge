use {
    hashbrown::HashMap,
    hibitset::{AtomicBitSet, BitSet, BitSetLike},
    rlua::prelude::*,
    shrev::{EventChannel, EventIterator, ReaderId},
    std::any::TypeId,
};

pub use hecs::{
    Bundle, Component, ComponentError, DynamicBundle, Entity, EntityBuilder, EntityRef,
    NoSuchEntity, Query, QueryBorrow, QueryOne, Ref, RefMut, SmartComponent,
};

#[doc(hidden)]
pub type SmartComponentContext<'a> = &'a Flags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LightEntity(u64);

impl From<Entity> for LightEntity {
    fn from(entity: Entity) -> LightEntity {
        Self(entity.to_bits())
    }
}

impl From<LightEntity> for Entity {
    fn from(wrapped: LightEntity) -> Entity {
        Entity::from_bits(wrapped.0)
    }
}

impl<'lua> ToLua<'lua> for LightEntity {
    fn to_lua(self, _lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        Ok(LuaValue::LightUserData(LuaLightUserData(self.0 as *mut _)))
    }
}

impl<'lua> FromLua<'lua> for LightEntity {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let lud = LuaLightUserData::from_lua(lua_value, lua)?;
        Ok(Self(lud.0 as u64))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ComponentEvent {
    Inserted(Entity),
    Modified(Entity),
    Removed(Entity),
}

#[derive(Default)]
pub(crate) struct Debouncer {
    inserted: BitSet,
    removed: BitSet,
    channel: EventChannel<ComponentEvent>,
}

impl Debouncer {
    pub(crate) fn track_inserted(&mut self, entity: Entity) {
        self.inserted.add(entity.id());
    }

    pub(crate) fn track_removed(&mut self, entity: Entity) {
        self.removed.add(entity.id());
    }
}

pub struct World {
    ecs: hecs::World,
    flags: HashMap<TypeId, AtomicBitSet>,
    channels: HashMap<TypeId, Debouncer>,
}

pub type Flags = HashMap<TypeId, AtomicBitSet>;

impl World {
    pub fn new() -> Self {
        Self {
            ecs: hecs::World::new(),
            flags: HashMap::new(),
            channels: HashMap::new(),
        }
    }

    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        let entity = self.ecs.spawn(components);

        for typeid in self
            .ecs
            .entity(entity)
            .expect("just created")
            .component_types()
        {
            self.flags.entry(typeid).or_default();

            if let Some(channel) = self.channels.get_mut(&typeid) {
                channel.track_inserted(entity);
            }
        }

        entity
    }

    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        for typeid in self.ecs.entity(entity)?.component_types() {
            if let Some(channel) = self.channels.get_mut(&typeid) {
                channel.track_removed(entity);
            }
        }

        self.ecs.despawn(entity)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.ecs.contains(entity)
    }

    pub fn query<'w, Q>(&'w self) -> QueryBorrow<'w, Q, &'w Flags>
    where
        Q: Query<'w, &'w Flags>,
    {
        self.ecs.query_with_context(&self.flags)
    }

    pub fn query_one<'w, Q>(
        &'w self,
        entity: Entity,
    ) -> Result<QueryOne<'w, Q, &'w Flags>, NoSuchEntity>
    where
        Q: Query<'w, &'w Flags>,
    {
        self.ecs.query_one_with_context(entity, &self.flags)
    }

    pub fn get<'w, C: SmartComponent<&'w Flags>>(
        &'w self,
        entity: Entity,
    ) -> Result<Ref<'w, C, &'w Flags>, ComponentError> {
        self.ecs.get_with_context(entity, &self.flags)
    }

    pub fn get_mut<'w, C: SmartComponent<&'w Flags>>(
        &'w self,
        entity: Entity,
    ) -> Result<RefMut<'w, C, &'w Flags>, ComponentError> {
        self.ecs.get_mut_with_context(entity, &self.flags)
    }

    pub fn query_raw<'w, Q: Query<'w>>(&'w self) -> QueryBorrow<'w, Q, ()> {
        self.ecs.query_with_context(())
    }

    pub fn entity(&self, entity: Entity) -> Result<EntityRef<&Flags>, NoSuchEntity> {
        self.ecs.entity_with_context(entity, &self.flags)
    }

    pub fn query_one_raw<'w, Q: Query<'w>>(
        &'w self,
        entity: Entity,
    ) -> Result<QueryOne<'w, Q, ()>, NoSuchEntity> {
        self.ecs.query_one_with_context(entity, ())
    }

    pub fn get_raw<C: Component>(&self, entity: Entity) -> Result<Ref<C>, ComponentError> {
        self.ecs.get_with_context(entity, ())
    }

    pub fn get_mut_raw<C: Component>(&self, entity: Entity) -> Result<RefMut<C>, ComponentError> {
        self.ecs.get_mut_with_context(entity, ())
    }

    pub fn entity_raw(&self, entity: Entity) -> Result<EntityRef<'_>, NoSuchEntity> {
        self.ecs.entity_with_context(entity, ())
    }

    pub fn insert(
        &mut self,
        entity: Entity,
        bundle: impl DynamicBundle,
    ) -> Result<(), NoSuchEntity> {
        // FIXME: find a way to do this w/o the undocumented/unstable DynamicBundle::with_ids
        bundle.with_ids(|typeids| {
            for typeid in typeids.iter().copied() {
                self.flags.entry(typeid).or_default();

                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.track_inserted(entity);
                }
            }
        });

        self.ecs.insert(entity, bundle)
    }

    pub fn insert_one<C: Component>(
        &mut self,
        entity: Entity,
        component: C,
    ) -> Result<(), NoSuchEntity> {
        let typeid = TypeId::of::<C>();
        self.flags.entry(typeid).or_default();
        if let Some(channel) = self.channels.get_mut(&typeid) {
            channel.track_inserted(entity);
        }

        self.ecs.insert_one(entity, component)
    }

    pub fn remove<T: Bundle>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        T::with_static_ids(|typeids| {
            for typeid in typeids.iter().copied() {
                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.track_removed(entity);
                }
            }
        });

        self.ecs.remove(entity)
    }

    pub fn remove_one<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        if let Some(channel) = self.channels.get_mut(&TypeId::of::<T>()) {
            channel.track_removed(entity);
        }

        self.ecs.remove_one(entity)
    }

    pub fn poll<T: Component>(
        &self,
        reader_id: &mut ReaderId<ComponentEvent>,
    ) -> EventIterator<ComponentEvent> {
        self.channels
            .get(&TypeId::of::<T>())
            .unwrap()
            .channel
            .read(reader_id)
    }

    pub fn track<T: Component>(&mut self) -> ReaderId<ComponentEvent> {
        self.channels
            .entry(TypeId::of::<T>())
            .or_default()
            .channel
            .register_reader()
    }

    pub fn flush_events(&mut self) {
        for (typeid, set) in self.flags.iter_mut() {
            if !set.is_empty() {
                if let Some(channel) = self.channels.get_mut(&typeid) {
                    let ecs = &self.ecs;
                    for entity in set
                        .iter()
                        .filter_map(|id| unsafe { ecs.resolve_unknown_gen(id) })
                    {
                        unimplemented!("debounce");
                        //channel.single_write(ComponentEvent::Modified(entity));
                    }
                }

                set.clear();
            }
        }
    }
}
