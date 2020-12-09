#![deny(missing_docs)]

//! An entity component system supporting mutation tracking and access from
//! Lua scripting through the sludge Lua API.
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
    #[doc(hidden)]
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

/// A buffer for deferred operations on the world such as entity insertions, removals,
/// spawning, and despawning. Can be constructed with `new` or `Default`, or retrieved
/// from the internal pool of a [`World`](World) with [`World::get_buffer`](World::get_buffer).
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
    /// Construct a new empty command buffer.
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

    /// Queue spawning an entity (see [`World::spawn`](World::spawn).)
    ///
    /// Note that unlike its synchronous counterpart, `CommandBuffer::spawn` cannot return a fresh
    /// entity ID. If you need an entity ID but you need to spawn the entity asynchronously, you
    /// can use [`World::reserve_entity`](World::reserve_entity) to asynchronously reserve an entity
    /// ID, and then use [`CommandBuffer::insert`](CommandBuffer::insert) to insert a bundle of
    /// components onto it. When combined, this behavior is functionally identical to `CommandBuffer::spawn`.
    #[inline]
    pub fn spawn(&mut self, bundle: impl DynamicBundle) -> &mut Self {
        let mut eb = self.get_or_make_builder();
        eb.add_bundle(bundle);
        self.cmds.push(Command::Spawn(eb));
        self
    }

    /// Queue inserting a bundle of components onto an entity (see [`World::insert`](World::insert).)
    #[inline]
    pub fn insert(&mut self, entity: Entity, bundle: impl DynamicBundle) -> &mut Self {
        let mut eb = self.get_or_make_builder();
        eb.add_bundle(bundle);
        self.cmds.push(Command::Insert(entity, eb));
        self
    }

    /// Queue inserting a single component onto an entity (see [`World::insert_one`](World::insert_one).)
    #[inline]
    pub fn insert_one<T: Component>(&mut self, entity: Entity, component: T) -> &mut Self {
        self.insert(entity, (component,))
    }

    /// Queue removing some components from an entity (see [`World::remove`](World::remove).)
    ///
    /// Note that when you use the command buffer version, you won't be able to retrieve
    /// the removed component, unlike with the synchronous version.
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

    /// Queue despawning an entity (see [`World::despawn`](World::despawn).)
    #[inline]
    pub fn despawn(&mut self, entity: Entity) -> &mut Self {
        self.cmds.push(Command::Despawn(entity));
        self
    }

    /// Returns true if this command buffer has no commands queued in it.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Immediately run all queued commands on the given `World`.
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

/// An iterator over emitted `ComponentEvent`s for some `ComponentSubscriber`,
/// returned by [`World::poll`](World::poll).
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

/// `ComponentEvent`s are generated in three possible cases:
/// 1. When a component is inserted onto an entity during spawning or an `insert` call,
///    then if the component has a subscriber, a `ComponentEvent::Inserted` will be emitted.
/// 2. If a component is tracked (uses `#[derive(TrackedComponent)]` rather than
///    #[derive(SimpleComponent)]` for its `SmartComponent` implementation), then when it
///    is *mutably dereferenced* (when the smart pointer returned from a query or `get_mut`
///    call is mutably dereferenced), then a `ComponentEvent::Modified` event will be
///    emitted.
/// 3. When a component is removed from an entity during despawning or a `remove` call,
///    then if the component has a subscriber, a `ComponentEvent::Removed` will be emitted.
#[derive(Debug, Clone, Copy)]
pub enum ComponentEvent {
    /// An event indicating that the relevant component type was inserted onto the included
    /// `Entity`.
    Inserted(Entity),
    /// An event indicating that the relevant component type was mutably accessed on the
    /// included `Entity`.
    Modified(Entity),
    /// An event indicating that the relevant component type was recently removed from the
    /// included `Entity`.
    Removed(Entity),
}

/// A `ComponentSubscriber<T>` represents a subscriber of tracking information for the component
/// type `T`. See also [`World::track`], [`World::poll`], [`ComponentEvent`].
///
/// [`World::track`]: World::track
/// [`World::poll`]: World::poll
/// [`ComponentEvent`]: ComponentEvent
#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
pub struct ComponentSubscriber<T> {
    #[derivative(Debug = "ignore")]
    _marker: PhantomData<T>,
    reader_id: ReaderId<ComponentEvent>,
}

#[doc(hidden)]
#[derive(Default)]
pub struct EventEmitter {
    inserted: BitSet,
    modified: AtomicBitSet,
    removed: BitSet,
    channel: RwLock<EventChannel<ComponentEvent>>,
}

#[doc(hidden)]
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

/// An ECS world built on top of a modified fork of the hecs ECS crate.
///
/// It supports component insertion/mutation/removal tracking through
/// [`World::track`] and [`World::poll`], and also supports asynchronous
/// queueing of entity spawning/insertion/removal/despawning through the
/// [`CommandBuffer`] type and [`World::get_buffer`] and [`World::queue_buffer`]
/// methods. If you're using `World::queue_buffer`, make sure to allocate
/// the buffer that you're passing into it using `World::get_buffer`, in
/// order to take full advantage of the `World`'s internal `CommandBuffer`
/// pool and minimize allocation of new `CommandBuffer`s.
pub struct World {
    ecs: hecs::World,
    buffers: Mutex<Vec<CommandBuffer>>,
    queued: Mutex<Vec<CommandBuffer>>,
    channels: HashMap<TypeId, EventEmitter>,
}

impl World {
    /// Create an empty world.
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

    /// An [`Archetype`](Archetype) is a representation of a set of components for some
    /// entity in the ECS world. This method returns an iterator over all archetypes; this
    /// is useful for things like automatic parallel scheduling and generating entity tables
    /// per possible `Archetype`, so that all entities have a corresponding value for their
    /// `Archetype` (and there are no duplicates.)
    pub fn archetypes(&self) -> impl ExactSizeIterator<Item = &Archetype> + '_ {
        self.ecs.archetypes()
    }

    /// The `ArchetypesGeneration` is a value which changes when the archetypes of the ECS
    /// world are updated/have an archetype removed/added. You can use this to figure out
    /// when you need to update a value calculated from the [`World::archetypes`](World::archetypes)
    /// method.
    pub fn archetypes_generation(&self) -> ArchetypesGeneration {
        self.ecs.archetypes_generation()
    }

    /// Spawn an entity with a bundle of components.
    ///
    /// If you're spawning lots of entities at once with the same component types, you probably
    /// want to use [`World::spawn_batch`](World::spawn_batch) instead.
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

    /// Spawn a number of entities with identical component types. This is much faster than
    /// [`World::spawn`](World::spawn) when you have lots of entities, because it can
    /// try to allocate memory for the whole batch all at once.
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

    /// This method is very similar to `spawn_batch`, but under the hood, `spawn_batch`
    /// performs an allocation of a `Vec<Entity>` for various reasons needed internally.
    /// This allows you to move that allocation outside if you already have one available.
    pub fn spawn_batch_into_buf<I>(&mut self, iter: I, buf: &mut Vec<Entity>)
    where
        I: IntoIterator,
        I::Item: Bundle,
    {
        let start = buf.len();
        buf.extend(self.ecs.spawn_batch(iter));
        I::Item::with_static_ids(|ids| {
            for typeid in ids {
                if let Some(channel) = self.channels.get_mut(&typeid) {
                    channel.emit_batch_inserted(buf[start..].iter().copied());
                }
            }
        });
    }

    /// Despawn an entity, removing it from the world and dropping all its components.
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

    /// Returns true if `entity` refers to a valid `Entity` in the world.
    pub fn contains(&self, entity: Entity) -> bool {
        self.ecs.contains(entity)
    }

    /// Asynchronously reserve a fresh entity ID with no components on it. This
    /// is kind of like `world.spawn(())`, but it only needs `&self`, is thread-safe,
    /// and doesn't allocate. You can use this with the [`CommandBuffer`](CommandBuffer)
    /// type to asynchronously queue up entity insertions where you immediately
    /// need the ID of the newly created entity.
    pub fn reserve_entity(&self) -> Entity {
        self.ecs.reserve_entity()
    }

    /// Efficiently query the world for all entities with the given components.
    ///
    /// This method is much faster than simply iterating over all entities in
    /// the world; it also guarantees that entities will be accessed sequentially
    /// in contiguous blocks of memory depending on their archetypes. In other
    /// words, it's very cache-friendly, especially when you have lots of entities
    /// with the same set of components... like projectiles, perhaps.
    pub fn query<'w, Q>(&'w self) -> QueryBorrow<'w, Q, ScContext<'w>>
    where
        Q: Query<'w, ScContext<'w>>,
    {
        self.ecs.query_with_context(&self.channels)
    }

    /// Query the world for all the given components of a single entity. This
    /// is more efficient than [`World::get`] when you want multiple components
    /// of an entity.
    pub fn query_one<'w, Q>(
        &'w self,
        entity: Entity,
    ) -> Result<QueryOne<'w, Q, ScContext<'w>>, NoSuchEntity>
    where
        Q: Query<'w, ScContext<'w>>,
    {
        self.ecs.query_one_with_context(entity, &self.channels)
    }

    /// Immutably borrow a single component from a single entity.
    pub fn get<'w, C: SmartComponent<ScContext<'w>>>(
        &'w self,
        entity: Entity,
    ) -> Result<Ref<'w, C, ScContext<'w>>, ComponentError> {
        self.ecs.get_with_context(entity, &self.channels)
    }

    /// Mutably borrow a single component from a single entity.
    pub fn get_mut<'w, C: SmartComponent<ScContext<'w>>>(
        &'w self,
        entity: Entity,
    ) -> Result<RefMut<'w, C, ScContext<'w>>, ComponentError> {
        self.ecs.get_mut_with_context(entity, &self.channels)
    }

    /// Access a specific entity's components and archetype information.
    pub fn entity(&self, entity: Entity) -> Result<EntityRef<ScContext>, NoSuchEntity> {
        self.ecs.entity_with_context(entity, &self.channels)
    }

    /// Like [`World::query`](World::query), but will not emit mutation events for
    /// tracked components.
    pub fn query_raw<'w, Q: Query<'w>>(&'w self) -> QueryBorrow<'w, Q, ()> {
        self.ecs.query_with_context(())
    }

    /// Like [`World::query_one`](World::query_one), but will not emit mutation events
    /// for tracked components.
    pub fn query_one_raw<'w, Q: Query<'w>>(
        &'w self,
        entity: Entity,
    ) -> Result<QueryOne<'w, Q, ()>, NoSuchEntity> {
        self.ecs.query_one_with_context(entity, ())
    }

    /// Like [`World::get`](World::get), but won't trigger access events for tracked components.
    ///
    /// This corresponds to not calling the `SmartComponent::on_borrowed` method. Currently
    /// nothing sludge will do will cause `on_borrowed` to do anything useful or relevant at
    /// all, but it might in the future.
    pub fn get_raw<C: Component>(&self, entity: Entity) -> Result<Ref<C>, ComponentError> {
        self.ecs.get_with_context(entity, ())
    }

    /// Like [`World::get_mut`](World::get_mut), but won't emit mutation events for tracked components.
    pub fn get_mut_raw<C: Component>(&self, entity: Entity) -> Result<RefMut<C>, ComponentError> {
        self.ecs.get_mut_with_context(entity, ())
    }

    /// Like [`World::entity`](World::entity), but won't emit mutation events for tracked components.
    pub fn entity_raw(&self, entity: Entity) -> Result<EntityRef<'_>, NoSuchEntity> {
        self.ecs.entity_with_context(entity, ())
    }

    /// Returns an iterator over all live `Entity` IDs in the world.
    ///
    /// Prefer [`World::query`](World::query) over `iter` whenever possible! It is
    /// orders of magnitude more efficient if you're accessing components on the
    /// entities being iterated over!
    pub fn iter(&self) -> Iter<'_> {
        self.ecs.iter()
    }

    /// Insert a bundle of components onto an entity.
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

    /// Insert a single component onto an entity.
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

    /// Remove multiple components from an entity. If the components are found on the entity
    /// they will be returned; otherwise, a `ComponentError` will be returned.
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

    /// Remove a single component from the entity, returning it if it's found.
    pub fn remove_one<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        if let Some(channel) = self.channels.get_mut(&TypeId::of::<T>()) {
            channel.emit_removed(entity);
        }

        self.ecs.remove_one(entity)
    }

    /// Clear all entities from the world, dropping their components.
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

    /// Given an ID without a generation which corresponds to a live entity,
    /// resolve the corresponding generation and produce a valid `Entity` ID
    /// referring to that entity.
    pub unsafe fn find_entity_from_id(&self, id: u32) -> Entity {
        self.ecs.find_entity_from_id(id)
    }

    /// Iterate over recently emitted events for a given `ComponentSubscriber`.
    ///
    /// See also [`ComponentEvent`](ComponentEvent), [`World::track`](World::track)
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

    /// Subscribe to insertion/mutation/removal events for a specific component type.
    ///
    /// To read newly emitted events, you can use [`World::poll`](World::poll). Note
    /// that for `track`, you need mutable access to the `World`, but `poll` only
    /// needs immutable access.
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

    /// Retrieve a command buffer from the `World`'s internal pool. Buffers queued
    /// through [`World::queue_buffer`](World::queue_buffer) will be returned to
    /// this pool once flushed.
    pub fn get_buffer(&self) -> CommandBuffer {
        self.buffers.lock().unwrap().pop().unwrap_or_default()
    }

    /// Asynchronously queue a command buffer to be run on the world the next time
    /// [`World::flush_queue`](World::flush_queue) is called. When the queued buffer
    /// is flushed, it will be returned to the `World`'s internal command buffer pool
    /// and may be returned by subsequent calls to [`World::get_buffer`](World::get_buffer).
    pub fn queue_buffer(&self, buffer: CommandBuffer) {
        self.queued.lock().unwrap().push(buffer);
    }

    /// Apply any queued command buffers to the `World` and return them to the pool
    /// once drained.
    ///
    /// Queued command buffers will do nothing and will only accumulate and waste memory
    /// if this is not called! It should be called in your main update function if you
    /// use it.
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
