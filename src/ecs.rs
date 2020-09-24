use {
    crossbeam_channel::Sender,
    hashbrown::HashMap,
    hecs::{self, *},
    hibitset::{AtomicBitSet, BitSetLike},
    std::any::TypeId,
};

pub mod hierarchy;
pub mod transform_graph;

pub use hecs::Entity;

#[derive(Debug, Clone, Copy)]
pub enum ComponentEvent {
    Inserted(Entity),
    Modified(Entity),
    Removed(Entity),
}

pub trait EventSender: Send + Sync + 'static {
    fn send_event(&self, event: ComponentEvent) -> bool;
}

impl EventSender for Sender<ComponentEvent> {
    fn send_event(&self, event: ComponentEvent) -> bool {
        self.try_send(event).is_ok()
    }
}

pub struct World {
    ecs: hecs::World,
    flags: HashMap<TypeId, AtomicBitSet>,
    channels: HashMap<TypeId, Vec<Box<dyn EventSender>>>,
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

            if let Some(channel) = self.channels.get(&typeid) {
                for subscriber in channel {
                    subscriber.send_event(ComponentEvent::Inserted(entity));
                }
            }
        }

        entity
    }

    pub fn despawn(&mut self, entity: Entity) -> Result<(), hecs::NoSuchEntity> {
        for typeid in self.ecs.entity(entity)?.component_types() {
            if let Some(channel) = self.channels.get(&typeid) {
                for subscriber in channel {
                    subscriber.send_event(ComponentEvent::Removed(entity));
                }
            }
        }

        self.ecs.despawn(entity)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.ecs.contains(entity)
    }

    pub fn query<'w, Q>(&'w self) -> hecs::QueryBorrow<'w, Q, &'w Flags>
    where
        Q: hecs::Query<'w, &'w Flags>,
    {
        self.ecs.query_with_context(&self.flags)
    }

    pub fn query_one<'w, Q>(
        &'w self,
        entity: Entity,
    ) -> Result<hecs::QueryOne<'w, Q, &'w Flags>, hecs::NoSuchEntity>
    where
        Q: hecs::Query<'w, &'w Flags>,
    {
        self.ecs.query_one_with_context(entity, &self.flags)
    }

    pub fn get<'w, C: SmartComponent<&'w Flags>>(
        &'w self,
        entity: Entity,
    ) -> Result<hecs::Ref<'w, C, &'w Flags>, hecs::ComponentError> {
        self.ecs.get_with_context(entity, &self.flags)
    }

    pub fn get_mut<'w, C: SmartComponent<&'w Flags>>(
        &'w self,
        entity: Entity,
    ) -> Result<hecs::RefMut<'w, C, &'w Flags>, hecs::ComponentError> {
        self.ecs.get_mut_with_context(entity, &self.flags)
    }

    pub fn get_raw<C: Component>(
        &self,
        entity: Entity,
    ) -> Result<hecs::Ref<C>, hecs::ComponentError> {
        self.ecs.get_with_context(entity, ())
    }

    pub fn get_mut_raw<C: Component>(
        &self,
        entity: Entity,
    ) -> Result<hecs::RefMut<C>, hecs::ComponentError> {
        self.ecs.get_mut_with_context(entity, ())
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

                if let Some(channel) = self.channels.get(&typeid) {
                    for subscriber in channel {
                        subscriber.send_event(ComponentEvent::Inserted(entity));
                    }
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
        if let Some(channel) = self.channels.get(&typeid) {
            for subscriber in channel {
                subscriber.send_event(ComponentEvent::Inserted(entity));
            }
        }

        self.ecs.insert_one(entity, component)
    }

    pub fn remove<T: Bundle>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        T::with_static_ids(|typeids| {
            for typeid in typeids.iter().copied() {
                if let Some(channel) = self.channels.get(&typeid) {
                    for subscriber in channel {
                        subscriber.send_event(ComponentEvent::Removed(entity));
                    }
                }
            }
        });

        self.ecs.remove(entity)
    }

    pub fn remove_one<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        if let Some(channel) = self.channels.get(&TypeId::of::<T>()) {
            for subscriber in channel {
                subscriber.send_event(ComponentEvent::Removed(entity));
            }
        }

        self.ecs.remove_one(entity)
    }

    pub fn subscribe<T: Component>(&mut self, sender: Box<dyn EventSender>) {
        self.channels
            .entry(TypeId::of::<T>())
            .or_default()
            .push(sender);
    }

    pub fn flush_events(&mut self) {
        for (typeid, set) in self.flags.iter_mut() {
            if !set.is_empty() {
                if let Some(channels) = self.channels.get(&typeid) {
                    for id in set.iter() {
                        if let Some(e) = unsafe { self.ecs.resolve_unknown_gen(id) } {
                            for subscriber in channels {
                                subscriber.send_event(ComponentEvent::Modified(e));
                            }
                        }
                    }
                }

                set.clear();
            }
        }
    }
}
