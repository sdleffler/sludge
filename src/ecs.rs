use {
    crossbeam_channel::Sender,
    hashbrown::HashMap,
    hecs,
    hibitset::{AtomicBitSet, BitSetLike},
    std::{any::TypeId, ops, vec::Drain},
};

pub mod hierarchy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity(hecs::Entity);

impl Entity {
    pub fn id(&self) -> u32 {
        self.0.id()
    }
}

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

pub enum Normal {}

pub enum Flagged {}

pub trait WrappedMut<'q, T, U>
where
    T: ?Sized + Component,
    U: ops::DerefMut<Target = T> + 'q,
{
    const IS_FLAGGED: bool;
    type Output;

    fn transform(this: U, id: u32, flags: &'q HashMap<TypeId, AtomicBitSet>) -> Self::Output;
}

pub struct FlaggedRefMut<'a, T, U>
where
    T: ?Sized + Component,
    U: ops::DerefMut<Target = T> + 'a,
{
    id: u32,
    flags: &'a HashMap<TypeId, AtomicBitSet>,
    inner: U,
}

impl<'a, T, U> ops::Deref for FlaggedRefMut<'a, T, U>
where
    T: ?Sized + Component,
    U: ops::DerefMut<Target = T> + 'a,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl<'a, T, U> ops::DerefMut for FlaggedRefMut<'a, T, U>
where
    T: ?Sized + Component,
    U: ops::DerefMut<Target = T> + 'a,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.flags[&TypeId::of::<T>()].add_atomic(self.id);
        &mut *self.inner
    }
}

impl<'a, T, U> WrappedMut<'a, T, U> for Flagged
where
    T: ?Sized + Component,
    U: ops::DerefMut<Target = T> + 'a,
{
    const IS_FLAGGED: bool = true;
    type Output = FlaggedRefMut<'a, T, U>;

    fn transform(this: U, id: u32, flags: &'a HashMap<TypeId, AtomicBitSet>) -> Self::Output {
        FlaggedRefMut {
            flags,
            id,
            inner: this,
        }
    }
}

impl<'a, T, U> WrappedMut<'a, T, U> for Normal
where
    T: ?Sized + Component,
    U: ops::DerefMut<Target = T> + 'a,
{
    const IS_FLAGGED: bool = false;
    type Output = U;

    fn transform(this: U, _id: u32, _flags: &'a HashMap<TypeId, AtomicBitSet>) -> Self::Output {
        this
    }
}

pub trait Component: hecs::Component {
    type Kind: for<'a> WrappedMut<'a, Self, &'a mut Self>;
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

        if T::Kind::IS_FLAGGED {
            self.flagged_types.push(TypeId::of::<T>());
        }

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
                    subscriber.send_event(Event::Created(Entity(e)));
                }
            }
        }

        Entity(e)
    }

    pub fn query<Q: hecs::Query>(&self) -> QueryBorrow<Q> {
        QueryBorrow {
            flags: &self.flags,
            borrow: self.ecs.query(),
        }
    }

    pub fn query_one<Q: hecs::Query>(
        &self,
        entity: Entity,
    ) -> Result<QueryOne<Q>, hecs::NoSuchEntity> {
        Ok(QueryOne {
            id: entity.id(),
            flags: &self.flags,
            borrow: self.ecs.query_one(entity.0)?,
        })
    }

    pub fn get<C: Component>(&self, entity: Entity) -> Result<hecs::Ref<C>, hecs::ComponentError> {
        self.ecs.get(entity.0)
    }

    pub fn get_mut<'a, C, R>(&'a self, entity: Entity) -> Result<R, hecs::ComponentError>
    where
        C: Component,
        C::Kind: WrappedMut<'a, C, hecs::RefMut<'a, C>, Output = R>,
    {
        self.ecs
            .get_mut::<C>(entity.0)
            .map(|p| C::Kind::transform(p, entity.id(), &self.flags))
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
                                subscriber.send_event(Event::Modified(Entity(e)));
                            }
                        }
                    }
                }

                set.clear();
            }
        }
    }
}

pub struct QueryBorrow<'w, Q: hecs::Query> {
    flags: &'w HashMap<TypeId, AtomicBitSet>,
    borrow: hecs::QueryBorrow<'w, Q>,
}

impl<'w, Q: hecs::Query> QueryBorrow<'w, Q> {
    pub fn iter<'q>(&'q mut self) -> QueryIter<'q, 'w, Q> {
        QueryIter {
            flags: self.flags,
            iter: self.borrow.iter(),
        }
    }

    pub fn with<T: Component>(self) -> QueryBorrow<'w, hecs::With<T, Q>> {
        QueryBorrow {
            flags: self.flags,
            borrow: self.borrow.with::<T>(),
        }
    }

    pub fn without<T: Component>(self) -> QueryBorrow<'w, hecs::Without<T, Q>> {
        QueryBorrow {
            flags: self.flags,
            borrow: self.borrow.without::<T>(),
        }
    }
}

pub struct QueryIter<'q, 'w, Q: hecs::Query> {
    flags: &'w HashMap<TypeId, AtomicBitSet>,
    iter: hecs::QueryIter<'q, 'w, Q>,
}

impl<'q, 'w, Q> Iterator for QueryIter<'q, 'w, Q>
where
    Q: hecs::Query,
    <Q::Fetch as hecs::Fetch<'q>>::Item: TransformFetched<'q>,
{
    type Item = (
        Entity,
        <<Q::Fetch as hecs::Fetch<'q>>::Item as TransformFetched<'q>>::Output,
    );

    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .next()
            .map(|(e, t)| (Entity(e), t.transform(e.id(), self.flags)))
    }
}

pub struct QueryOne<'w, Q: hecs::Query> {
    id: u32,
    flags: &'w HashMap<TypeId, AtomicBitSet>,
    borrow: hecs::QueryOne<'w, Q>,
}

impl<'w, Q: hecs::Query> QueryOne<'w, Q>
where
    Q: hecs::Query,
    for<'q> <Q::Fetch as hecs::Fetch<'q>>::Item: TransformFetched<'q>,
{
    pub fn get(&mut self) -> Option<<<Q::Fetch as hecs::Fetch>::Item as TransformFetched>::Output> {
        let Self { id, flags, borrow } = self;

        borrow
            .get()
            .map(|fetched| TransformFetched::transform(fetched, *id, flags))
    }

    pub fn with<T: Component>(self) -> QueryOne<'w, hecs::With<T, Q>> {
        QueryOne {
            id: self.id,
            flags: self.flags,
            borrow: self.borrow.with(),
        }
    }

    pub fn without<T: Component>(self) -> QueryOne<'w, hecs::Without<T, Q>> {
        QueryOne {
            id: self.id,
            flags: self.flags,
            borrow: self.borrow.without(),
        }
    }
}

pub trait TransformFetched<'q> {
    type Output;

    fn transform(self, id: u32, flags: &'q HashMap<TypeId, AtomicBitSet>) -> Self::Output;
}

impl<'q, T: Component> TransformFetched<'q> for &'q T {
    type Output = &'q T;

    fn transform(self, _id: u32, _flags: &'q HashMap<TypeId, AtomicBitSet>) -> Self::Output {
        self
    }
}

impl<'q, T: Component> TransformFetched<'q> for &'q mut T {
    type Output = <T::Kind as WrappedMut<'q, T, &'q mut T>>::Output;

    fn transform(self, id: u32, flags: &'q HashMap<TypeId, AtomicBitSet>) -> Self::Output {
        <T::Kind as WrappedMut<'q, T, &'q mut T>>::transform(self, id, flags)
    }
}

impl<'q, T: TransformFetched<'q>> TransformFetched<'q> for Option<T> {
    type Output = Option<T::Output>;

    fn transform(self, id: u32, flags: &'q HashMap<TypeId, AtomicBitSet>) -> Self::Output {
        match self {
            Some(t) => Some(t.transform(id, flags)),
            None => None,
        }
    }
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<'a, $($name: TransformFetched<'a>),*> TransformFetched<'a> for ($($name,)*) {
            type Output = ($($name::Output,)*);

            #[allow(unused_variables, non_snake_case)]
            fn transform(self, id: u32, flags: &'a HashMap<TypeId, AtomicBitSet>) -> Self::Output {
                let ($($name,)*) = self;
                ($($name.transform(id, flags),)*)
            }
        }

        impl<$($name: Component),*> Component for ($($name,)*) {
            type Kind = Normal;
        }

        impl<$($name: Component),*> DynamicBundle for ($($name,)*) {
            type Hecs = Self;

            #[allow(unused_variables)]
            fn init_flag_sets(&self, flags: &mut HashMap<TypeId, AtomicBitSet>) {
                $(if $name::Kind::IS_FLAGGED { flags.entry(TypeId::of::<$name>()).or_default(); })*
            }
        }
    };
}

//smaller_tuples_too!(tuple_impl, B, A);
smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
