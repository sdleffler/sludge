use {
    crossbeam_channel::Receiver,
    hashbrown::{HashMap, HashSet},
    hibitset::BitSet,
};

use crate::ecs::{Component, Entity, Event, Flagged, World};

#[derive(Debug, Clone, Copy)]
pub struct Parent {
    id: Entity,
}

impl Component for Parent {
    type Kind = Flagged;
}

pub struct Hierarchy {
    sorted: Vec<Entity>,
    entities: HashMap<u32, Entity>,

    parents: HashMap<Entity, Entity>,
    children: HashMap<Entity, Vec<Entity>>,

    created: BitSet,
    modified: BitSet,
    destroyed: BitSet,

    scratch_set: HashSet<Entity>,

    events: Receiver<Event>,
}

impl Hierarchy {
    pub fn new(world: &mut World) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        world.subscribe::<Parent>(Box::new(sender));
        Self {
            sorted: Vec::new(),
            entities: HashMap::new(),

            parents: HashMap::new(),
            children: HashMap::new(),

            created: BitSet::new(),
            modified: BitSet::new(),
            destroyed: BitSet::new(),

            scratch_set: HashSet::new(),

            events: receiver,
        }
    }

    pub fn update(&mut self, world: &mut World) {
        self.created.clear();
        self.modified.clear();
        self.destroyed.clear();

        for event in self.events.try_iter() {
            match event {
                Event::Created(entity) => {
                    self.entities.insert(entity.id(), entity);
                    self.created.add(entity.id());
                }
                Event::Modified(entity) => {
                    self.modified.add(entity.id());
                }
                Event::Destroyed(entity) => {
                    self.entities.remove(&entity.id());
                    self.destroyed.add(entity.id());
                }
            }
        }

        self.scratch_set.clear();
    }
}
