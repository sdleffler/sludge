use {
    crossbeam_channel::Sender,
    hashbrown::HashMap,
    hecs::{self, *},
    hibitset::{AtomicBitSet, BitSetLike},
    std::{any::TypeId, vec::Drain},
};

pub mod hierarchy;

pub use hecs::Entity;

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Created(Entity),
    Modified(Entity),
    Destroyed(Entity),
}

pub trait EventSender: Send + Sync + 'static {
    fn send_event(&self, event: Event) -> bool;
}

impl EventSender for Sender<Event> {
    fn send_event(&self, event: Event) -> bool {
        self.try_send(event).is_ok()
    }
}

pub trait DynamicBundle: Into<<Self as DynamicBundle>::Hecs> {
    type Hecs: hecs::DynamicBundle;

    #[doc(hidden)]
    fn init_flag_sets(&self, flags: &mut HashMap<TypeId, AtomicBitSet>);
}

pub struct BuiltEntity<'a> {
    built: hecs::BuiltEntity<'a>,
    flagged_types: Drain<'a, TypeId>,
}

impl<'a> DynamicBundle for BuiltEntity<'a> {
    type Hecs = hecs::BuiltEntity<'a>;

    fn init_flag_sets(&self, flags: &mut HashMap<TypeId, AtomicBitSet>) {
        for typeid in self.flagged_types.as_slice() {
            flags.entry(*typeid).or_default();
        }
    }
}

impl<'a> From<BuiltEntity<'a>> for hecs::BuiltEntity<'a> {
    fn from(built: BuiltEntity<'a>) -> Self {
        built.built
    }
}

pub struct EntityBuilder {
    builder: hecs::EntityBuilder,
    flagged_types: Vec<TypeId>,
}

impl EntityBuilder {
    pub fn new() -> Self {
        Self {
            builder: hecs::EntityBuilder::new(),
            flagged_types: Vec::new(),
        }
    }

    pub fn add<T: Component>(&mut self, component: T) -> &mut Self {
        self.builder.add(component);

        // if T::Kind::IS_FLAGGED {
        //     self.flagged_types.push(TypeId::of::<T>());
        // }

        self
    }

    pub fn build(&mut self) -> BuiltEntity {
        BuiltEntity {
            built: self.builder.build(),
            flagged_types: self.flagged_types.drain(..),
        }
    }

    pub fn clear(&mut self) {
        self.builder.clear()
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
        components.init_flag_sets(&mut self.flags);
        let e = self.ecs.spawn(components.into());

        for typeid in self.ecs.entity(e).expect("just created").component_types() {
            if let Some(channel) = self.channels.get(&typeid) {
                for subscriber in channel {
                    subscriber.send_event(Event::Created(e));
                }
            }
        }

        e
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
                                subscriber.send_event(Event::Modified(e));
                            }
                        }
                    }
                }

                set.clear();
            }
        }
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<$($name: Component),*> DynamicBundle for ($($name,)*) {
            type Hecs = Self;

            #[allow(unused_variables)]
            fn init_flag_sets(&self, flags: &mut HashMap<TypeId, AtomicBitSet>) {
                $(flags.entry(TypeId::of::<$name>()).or_default();)*
            }
        }
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
