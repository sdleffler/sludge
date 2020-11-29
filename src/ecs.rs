//! An entity component system supporting mutation tracking and Lua access.
//!
//! See [the `api` module](crate::api) for more information on integrating
//! entities and components with Lua scripting.
//!
//! This module contains an ECS [`World`](World) type which is built on top
//! of a custom fork of the `hecs` ECS. It uses the `Entity` type from `hecs`
//! and supports tracking of component insertions and deletions for any component
//! with a registered reader, and tracking of component insertions, mutations,
//! and deletions for components which support mutation tracking. Whether or
//! not a component supports mutation tracking depends on its implementation
//! of [`SmartComponent`](SmartComponent).
//!
//! For implementing the `SmartComponent` trait, sludge provides two `#[derive]`
//! macros, `SimpleComponent` and `TrackedComponent`. `#[derive(SimpleComponent)]`
//! can be used to derive a `SmartComponent` implementation which is un-flagged,
//! and `#[derive(TrackedComponent)]` will generate an implementation which flags
//! changes on mutable borrow and registers the component type with the ECS.

use {
    anyhow::*,
    derivative::*,
    hashbrown::HashMap,
    hibitset::*,
    shrev::{EventChannel, EventIterator},
    std::{
        any::{Any, TypeId},
        fmt,
        marker::PhantomData,
        pin::Pin,
        sync::{Mutex, RwLock, RwLockReadGuard},
    },
};

pub use hecs::{
    Archetype, ArchetypesGeneration, Bundle, Component, ComponentError, DynamicBundle, Entity,
    EntityBuilder, EntityRef, Iter, NoSuchEntity, Query, QueryBorrow, QueryOne, Ref, RefMut,
    SmartComponent, SpawnBatchIter,
};

pub use shrev::ReaderId;

#[doc(hidden)]
pub type ScContext<'a> = &'a HashMap<TypeId, EventEmitter>;

#[doc(hidden)]
pub struct FlaggedComponent(TypeId);

impl FlaggedComponent {
    pub fn of<T: Any>() -> Self {
        Self(TypeId::of::<T>())
    }
}

inventory::collect!(FlaggedComponent);

enum Command {
    Spawn(EntityBuilder),
    Insert(Entity, EntityBuilder),
    Remove(
        Entity,
        fn(
            channels: &mut HashMap<TypeId, EventEmitter>,
            ecs: &mut hecs::World,
            entity: Entity,
        ) -> Result<(), ComponentError>,
    ),
    Despawn(Entity),
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Spawn(_) => f.debug_tuple("Spawn").field(&format_args!("_")).finish(),
            Self::Insert(entity, _) => f
                .debug_tuple("Insert")
                .field(entity)
                .field(&format_args!("_"))
                .finish(),
            Self::Remove(entity, _) => f
                .debug_tuple("Remove")
                .field(entity)
                .field(&format_args!("_"))
                .finish(),
            Self::Despawn(entity) => f.debug_tuple("Despawn").field(entity).finish(),
        }
    }
}

#[derive(Default)]
#[must_use = "CommandBuffers do nothing unless queued!"]
pub struct CommandBuffer {
    pool: Vec<EntityBuilder>,
    cmds: Vec<Command>,
}

impl fmt::Debug for CommandBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("CommandBuffer")
            .field("pool", &self.pool.len())
            .field("cmds", &self.cmds)
            .finish()
    }
}

impl CommandBuffer {
    pub fn new() -> Self {
        Self {
            pool: Vec::new(),
            cmds: Vec::new(),
        }
    }

    #[inline]
    fn get_or_make_builder(&mut self) -> EntityBuilder {
        self.pool.pop().unwrap_or_default()
    }

    #[inline]
    pub fn spawn(&mut self, bundle: impl DynamicBundle) -> &mut Self {
        let mut eb = self.get_or_make_builder();
        eb.add_bundle(bundle);
        self.cmds.push(Command::Spawn(eb));
        self
    }

    #[inline]
    pub fn insert(&mut self, entity: Entity, bundle: impl DynamicBundle) -> &mut Self {
        let mut eb = self.get_or_make_builder();
        eb.add_bundle(bundle);
        self.cmds.push(Command::Insert(entity, eb));
        self
    }

    #[inline]
    pub fn insert_one<T: Component>(&mut self, entity: Entity, component: T) -> &mut Self {
        self.insert(entity, (component,))
    }

    #[inline]
    pub fn remove<T: Bundle>(&mut self, entity: Entity) -> &mut Self {
        fn do_remove<T: Bundle>(
            channels: &mut HashMap<TypeId, EventEmitter>,
            ecs: &mut hecs::World,
            entity: Entity,
        ) -> Result<(), ComponentError> {
            World::do_remove(channels, ecs, entity)?;
            Ok(())
        }

        self.cmds.push(Command::Remove(entity, do_remove::<T>));
        self
    }

    #[inline]
    pub fn despawn(&mut self, entity: Entity) -> &mut Self {
        self.cmds.push(Command::Despawn(entity));
        self
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    #[inline]
    pub fn drain_into(&mut self, world: &mut World) -> Result<()> {
        let mut errs = Vec::new();
        for cmd in self.cmds.drain(..) {
            match cmd {
                Command::Spawn(mut bundle) => {
                    World::do_spawn(&mut world.channels, &mut world.ecs, bundle.build());
                    self.pool.push(bundle);
                }
                Command::Insert(entity, mut bundle) => {
                    let res = World::do_insert(
                        &mut world.channels,
                        &mut world.ecs,
                        entity,
                        bundle.build(),
                    );
                    self.pool.push(bundle);

                    if let Err(err) = res {
                        errs.push(err.to_string());
                    }
                }
                Command::Remove(entity, remover) => {
                    if let Err(err) = remover(&mut world.channels, &mut world.ecs, entity) {
                        errs.push(err.to_string());
                    }
                }
                Command::Despawn(entity) => {
                    if let Err(err) = World::do_despawn(&mut world.channels, &mut world.ecs, entity)
                    {
                        errs.push(err.to_string());
                    }
                }
            }
        }

        ensure!(
            errs.is_empty(),
            "one or more errors occurred while draining command buffer: {}",
            errs.join(", ")
        );

        Ok(())
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

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
pub struct ComponentSubscriber<T> {
    #[derivative(Debug = "ignore")]
    _marker: PhantomData<T>,
    reader_id: ReaderId<ComponentEvent>,
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

    pub fn emit_batch_inserted<I>(&mut self, batch: I)
    where
        I: IntoIterator<Item = Entity>,
        I::IntoIter: ExactSizeIterator,
    {
        let Self {
            inserted, channel, ..
        } = self;
        channel.get_mut().unwrap().iter_write(
            batch
                .into_iter()
                .inspect(|e| assert!(!inserted.add(e.id())))
                .map(ComponentEvent::Inserted),
        );
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
    buffers: Mutex<Vec<CommandBuffer>>,
    queued: Mutex<Vec<CommandBuffer>>,
    channels: HashMap<TypeId, EventEmitter>,
}

impl World {
    pub fn new() -> Self {
        Self {
            ecs: hecs::World::new(),
            buffers: Mutex::new(Vec::new()),
            queued: Mutex::new(Vec::new()),
            channels: inventory::iter::<FlaggedComponent>
                .into_iter()
                .map(|fc| (fc.0, EventEmitter::default()))
                .collect(),
        }
    }

    pub fn archetypes(&self) -> impl ExactSizeIterator<Item = &Archetype> + '_ {
        self.ecs.archetypes()
    }

    pub fn archetypes_generation(&self) -> ArchetypesGeneration {
        self.ecs.archetypes_generation()
    }

    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        Self::do_spawn(&mut self.channels, &mut self.ecs, components)
    }

    fn do_spawn(
        channels: &mut HashMap<TypeId, EventEmitter>,
        ecs: &mut hecs::World,
        components: impl DynamicBundle,
    ) -> Entity {
        let entity = ecs.spawn(components);

        for typeid in ecs.entity(entity).expect("just created").component_types() {
            if let Some(channel) = channels.get_mut(&typeid) {
                channel.emit_inserted(entity);
            }
        }

        entity
    }

    pub fn spawn_batch<I>(&mut self, iter: I) -> Vec<Entity>
    where
        I: IntoIterator,
        I::Item: Bundle,
    {
        let batched = self.ecs.spawn_batch(iter).collect::<Vec<_>>();
        I::Item::with_static_ids(|ids| {
            for typeid in ids {
                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.emit_batch_inserted(batched.iter().copied());
                }
            }
        });

        batched
    }

    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        Self::do_despawn(&mut self.channels, &mut self.ecs, entity)
    }

    fn do_despawn(
        channels: &mut HashMap<TypeId, EventEmitter>,
        ecs: &mut hecs::World,
        entity: Entity,
    ) -> Result<(), NoSuchEntity> {
        for typeid in ecs.entity(entity)?.component_types() {
            if let Some(channel) = channels.get_mut(&typeid) {
                channel.emit_removed(entity);
            }
        }

        ecs.despawn(entity)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.ecs.contains(entity)
    }

    pub fn reserve_entity(&self) -> Entity {
        self.ecs.reserve_entity()
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

    pub fn iter(&self) -> Iter<'_> {
        self.ecs.iter()
    }

    pub fn insert(
        &mut self,
        entity: Entity,
        bundle: impl DynamicBundle,
    ) -> Result<(), NoSuchEntity> {
        Self::do_insert(&mut self.channels, &mut self.ecs, entity, bundle)
    }

    fn do_insert(
        channels: &mut HashMap<TypeId, EventEmitter>,
        ecs: &mut hecs::World,
        entity: Entity,
        bundle: impl DynamicBundle,
    ) -> Result<(), NoSuchEntity> {
        // FIXME: find a way to do this w/o the undocumented/unstable DynamicBundle::with_ids
        bundle.with_ids(|typeids| {
            for typeid in typeids.iter().copied() {
                if let Some(channel) = channels.get_mut(&typeid) {
                    channel.emit_inserted(entity);
                }
            }
        });

        ecs.insert(entity, bundle)
    }

    pub fn insert_one<C: Component>(
        &mut self,
        entity: Entity,
        component: C,
    ) -> Result<(), NoSuchEntity> {
        let typeid = TypeId::of::<C>();

        if let Some(channel) = self.channels.get_mut(&typeid) {
            channel.emit_inserted(entity);
        }

        self.ecs.insert_one(entity, component)
    }

    pub fn remove<T: Bundle>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        Self::do_remove::<T>(&mut self.channels, &mut self.ecs, entity)
    }

    fn do_remove<T: Bundle>(
        channels: &mut HashMap<TypeId, EventEmitter>,
        ecs: &mut hecs::World,
        entity: Entity,
    ) -> Result<T, ComponentError> {
        T::with_static_ids(|typeids| {
            for typeid in typeids.iter().copied() {
                if let Some(channel) = channels.get_mut(&typeid) {
                    channel.emit_removed(entity);
                }
            }
        });

        ecs.remove(entity)
    }

    pub fn remove_one<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        if let Some(channel) = self.channels.get_mut(&TypeId::of::<T>()) {
            channel.emit_removed(entity);
        }

        self.ecs.remove_one(entity)
    }

    pub fn clear(&mut self) {
        for (id, e) in self.ecs.iter() {
            for typeid in e.component_types() {
                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.emit_removed(id);
                }
            }
        }

        self.ecs.clear();
    }

    pub unsafe fn find_entity_from_id(&self, id: u32) -> Entity {
        self.ecs.find_entity_from_id(id)
    }

    pub fn poll<'a, T: Component>(
        &'a self,
        subscriber: &'a mut ComponentSubscriber<T>,
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
            &mut subscriber.reader_id,
        )
    }

    pub fn track<T: Component>(&mut self) -> ComponentSubscriber<T> {
        let reader_id = self
            .channels
            .entry(TypeId::of::<T>())
            .or_default()
            .channel
            .get_mut()
            .unwrap()
            .register_reader();

        ComponentSubscriber {
            _marker: PhantomData,
            reader_id,
        }
    }

    pub fn get_buffer(&self) -> CommandBuffer {
        self.buffers.lock().unwrap().pop().unwrap_or_default()
    }

    pub fn queue_buffer(&self, buffer: CommandBuffer) {
        self.queued.lock().unwrap().push(buffer);
    }

    pub fn flush_queue(&mut self) -> Result<()> {
        let pool = self.buffers.get_mut().unwrap();
        let mut errors = Vec::new();
        let queued = self.queued.get_mut().unwrap();

        let nonempty_count = queued.iter().filter(|buf| !buf.is_empty()).count();
        if nonempty_count > 0 {
            log::info!(
                "flushing {} nonempty queued command buffers",
                nonempty_count,
            );
        }

        for mut buffer in queued.drain(..) {
            for cmd in buffer.cmds.drain(..) {
                let res = match cmd {
                    Command::Spawn(mut bundle) => {
                        Self::do_spawn(&mut self.channels, &mut self.ecs, bundle.build());
                        buffer.pool.push(bundle);
                        Ok(())
                    }
                    Command::Insert(entity, mut bundle) => {
                        let res = Self::do_insert(
                            &mut self.channels,
                            &mut self.ecs,
                            entity,
                            bundle.build(),
                        );
                        buffer.pool.push(bundle);
                        res.map_err(|err| Error::from(err).to_string())
                    }
                    Command::Remove(entity, remover) => {
                        remover(&mut self.channels, &mut self.ecs, entity)
                            .map_err(|err| Error::from(err).to_string())
                    }
                    Command::Despawn(entity) => {
                        Self::do_despawn(&mut self.channels, &mut self.ecs, entity)
                            .map_err(|err| Error::from(err).to_string())
                    }
                };

                if let Err(err) = res {
                    errors.push(err);
                }
            }

            pool.push(buffer);
        }

        for (_, channel) in self.channels.iter_mut() {
            channel.clear();
        }

        match errors.is_empty() {
            true => Ok(()),
            false => Err(anyhow!(
                "errors while flushing command queue: `{}`",
                errors.join(",")
            )),
        }
    }
}
