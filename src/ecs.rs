use {
    hashbrown::HashMap,
    hibitset::*,
    rlua::prelude::*,
    shrev::{EventChannel, EventIterator, ReaderId},
    std::{
        any::{Any, TypeId},
        pin::Pin,
        sync::{RwLock, RwLockReadGuard},
    },
};

pub use hecs::{
    Bundle, Component, ComponentError, DynamicBundle, Entity, EntityBuilder, EntityRef,
    NoSuchEntity, Query, QueryBorrow, QueryOne, Ref, RefMut, SmartComponent,
};

#[doc(hidden)]
pub type ScContext<'a> = &'a HashMap<TypeId, EventEmitter>;

pub struct FlaggedComponent(TypeId);

impl FlaggedComponent {
    pub fn of<T: Any>() -> Self {
        Self(TypeId::of::<T>())
    }
}

inventory::collect!(FlaggedComponent);

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

pub struct ComponentEventIterator<'a> {
    _outer: Pin<RwLockReadGuard<'a, EventChannel<ComponentEvent>>>,
    iter: EventIterator<'a, ComponentEvent>,
}

impl<'a> ComponentEventIterator<'a> {
    fn new(
        guard: Pin<RwLockReadGuard<'a, EventChannel<ComponentEvent>>>,
        reader_id: &'a mut ReaderId<ComponentEvent>,
    ) -> Self {
        let iter = unsafe {
            let inner_ptr = &*guard as *const EventChannel<ComponentEvent>;
            (*inner_ptr).read(reader_id)
        };

        Self {
            _outer: guard,
            iter,
        }
    }
}

impl<'a> Iterator for ComponentEventIterator<'a> {
    type Item = &'a ComponentEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ComponentEvent {
    Inserted(Entity),
    Modified(Entity),
    Removed(Entity),
}

#[derive(Default)]
pub struct EventEmitter {
    inserted: BitSet,
    modified: AtomicBitSet,
    removed: BitSet,
    channel: RwLock<EventChannel<ComponentEvent>>,
}

impl EventEmitter {
    pub fn emit_inserted(&mut self, entity: Entity) {
        if !self.inserted.add(entity.id()) {
            self.channel
                .get_mut()
                .unwrap()
                .single_write(ComponentEvent::Inserted(entity));
        }
    }

    pub fn emit_modified(&mut self, entity: Entity) {
        if !self.modified.add(entity.id()) {
            self.channel
                .get_mut()
                .unwrap()
                .single_write(ComponentEvent::Modified(entity));
        }
    }

    pub fn emit_modified_atomic(&self, entity: Entity) {
        if !self.modified.add_atomic(entity.id()) {
            self.channel
                .write()
                .unwrap()
                .single_write(ComponentEvent::Modified(entity));
        }
    }

    pub fn emit_removed(&mut self, entity: Entity) {
        if !self.removed.add(entity.id()) {
            self.channel
                .get_mut()
                .unwrap()
                .single_write(ComponentEvent::Removed(entity));
        }
    }

    pub fn clear(&mut self) {
        self.inserted.clear();
        self.modified.clear();
        self.removed.clear();
    }
}

pub struct World {
    ecs: hecs::World,
    channels: HashMap<TypeId, EventEmitter>,
}

impl World {
    pub fn new() -> Self {
        Self {
            ecs: hecs::World::new(),
            channels: inventory::iter::<FlaggedComponent>
                .into_iter()
                .map(|fc| (fc.0, EventEmitter::default()))
                .collect(),
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
            self.channels.entry(typeid).or_default();

            if let Some(channel) = self.channels.get_mut(&typeid) {
                channel.emit_inserted(entity);
            }
        }

        entity
    }

    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        for typeid in self.ecs.entity(entity)?.component_types() {
            if let Some(channel) = self.channels.get_mut(&typeid) {
                channel.emit_removed(entity);
            }
        }

        self.ecs.despawn(entity)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.ecs.contains(entity)
    }

    pub fn query<'w, Q>(&'w self) -> QueryBorrow<'w, Q, ScContext<'w>>
    where
        Q: Query<'w, ScContext<'w>>,
    {
        self.ecs.query_with_context(&self.channels)
    }

    pub fn query_one<'w, Q>(
        &'w self,
        entity: Entity,
    ) -> Result<QueryOne<'w, Q, ScContext<'w>>, NoSuchEntity>
    where
        Q: Query<'w, ScContext<'w>>,
    {
        self.ecs.query_one_with_context(entity, &self.channels)
    }

    pub fn get<'w, C: SmartComponent<ScContext<'w>>>(
        &'w self,
        entity: Entity,
    ) -> Result<Ref<'w, C, ScContext<'w>>, ComponentError> {
        self.ecs.get_with_context(entity, &self.channels)
    }

    pub fn get_mut<'w, C: SmartComponent<ScContext<'w>>>(
        &'w self,
        entity: Entity,
    ) -> Result<RefMut<'w, C, ScContext<'w>>, ComponentError> {
        self.ecs.get_mut_with_context(entity, &self.channels)
    }

    pub fn query_raw<'w, Q: Query<'w>>(&'w self) -> QueryBorrow<'w, Q, ()> {
        self.ecs.query_with_context(())
    }

    pub fn entity(&self, entity: Entity) -> Result<EntityRef<ScContext>, NoSuchEntity> {
        self.ecs.entity_with_context(entity, &self.channels)
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
                self.channels.entry(typeid).or_default();

                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.emit_inserted(entity);
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
        self.channels.entry(typeid).or_default();
        if let Some(channel) = self.channels.get_mut(&typeid) {
            channel.emit_inserted(entity);
        }

        self.ecs.insert_one(entity, component)
    }

    pub fn remove<T: Bundle>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        T::with_static_ids(|typeids| {
            for typeid in typeids.iter().copied() {
                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.emit_removed(entity);
                }
            }
        });

        self.ecs.remove(entity)
    }

    pub fn remove_one<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        if let Some(channel) = self.channels.get_mut(&TypeId::of::<T>()) {
            channel.emit_removed(entity);
        }

        self.ecs.remove_one(entity)
    }

    pub fn poll<'a, T: Component>(
        &'a self,
        reader_id: &'a mut ReaderId<ComponentEvent>,
    ) -> ComponentEventIterator<'a> {
        ComponentEventIterator::new(
            Pin::new(
                self.channels
                    .get(&TypeId::of::<T>())
                    .unwrap()
                    .channel
                    .read()
                    .unwrap(),
            ),
            reader_id,
        )
    }

    pub fn track<T: Component>(&mut self) -> ReaderId<ComponentEvent> {
        self.channels
            .entry(TypeId::of::<T>())
            .or_default()
            .channel
            .get_mut()
            .unwrap()
            .register_reader()
    }

    pub fn flush_events(&mut self) {
        for (_, channel) in self.channels.iter_mut() {
            channel.clear();
        }
    }
}
