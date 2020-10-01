use {
    hashbrown::{HashMap, HashSet},
    hecs::SmartComponent,
    hibitset::{BitSet, BitSetLike},
    shrev::{EventChannel, ReaderId},
    std::{any::TypeId, marker::PhantomData},
};

use crate::{
    ecs::{ComponentEvent, Entity, Flags, World},
    resources::SharedResources,
};

#[derive(Debug, Clone, Copy)]
pub struct Parent {
    pub parent_entity: Entity,
}

impl Parent {
    pub fn new(parent_entity: Entity) -> Self {
        Self { parent_entity }
    }
}

impl<'a> SmartComponent<&'a Flags> for Parent {
    fn on_borrow_mut(&mut self, entity: hecs::Entity, context: &'a Flags) {
        context[&TypeId::of::<Self>()].add_atomic(entity.id());
    }
}

impl ParentComponent for Parent {
    fn parent_entity(&self) -> Entity {
        self.parent_entity
    }
}

pub trait ParentComponent: for<'a> SmartComponent<&'a Flags> {
    fn parent_entity(&self) -> Entity;
}

#[derive(Debug, Clone, Copy)]
pub enum HierarchyEvent {
    ModifiedOrCreated(Entity),
    Removed(Entity),
}

pub struct Hierarchy<P: ParentComponent> {
    sorted: Vec<Entity>,
    entities: HashMap<u32, usize>,

    roots: HashSet<Entity>,
    current_parent: HashMap<Entity, Entity>,
    children: HashMap<Entity, Vec<Entity>>,

    inserted: BitSet,
    modified: BitSet,
    removed: BitSet,

    scratch_set: HashSet<Entity>,

    events: ReaderId<ComponentEvent>,
    changed: EventChannel<HierarchyEvent>,

    _marker: PhantomData<P>,
}

impl<P: ParentComponent> Hierarchy<P> {
    pub fn new(world: &mut World) -> Self {
        let events = world.track::<P>();
        Self {
            sorted: Vec::new(),
            entities: HashMap::new(),

            roots: HashSet::new(),
            current_parent: HashMap::new(),
            children: HashMap::new(),

            inserted: BitSet::new(),
            modified: BitSet::new(),
            removed: BitSet::new(),

            scratch_set: HashSet::new(),

            events,
            changed: EventChannel::new(),

            _marker: PhantomData,
        }
    }

    /// Get all entities that contain parents, in sorted order, where parents are guaranteed to
    /// be before their children.
    ///
    /// Note: This does not include entities that **are** parents.
    pub fn all(&self) -> &[Entity] {
        self.sorted.as_slice()
    }

    /// Get the immediate children of a specific entity.
    pub fn children(&self, entity: Entity) -> &[Entity] {
        self.children
            .get(&entity)
            .map(|vec| vec.as_slice())
            .unwrap_or(&[])
    }

    /// Get all children of this entity recursively as a `BitSet`
    ///
    /// This does not include the parent entity you pass in.
    pub fn all_children(&self, entity: Entity) -> BitSet {
        let mut entities = BitSet::new();
        self.add_children_to_set(entity, &mut entities);
        entities
    }

    fn add_children_to_set(&self, entity: Entity, set: &mut BitSet) {
        if let Some(children) = self.children.get(&entity) {
            for child in children {
                set.add(child.id());
                self.add_children_to_set(*child, set);
            }
        }
    }

    /// Returns an iterator over all of the recursive children of this entity.
    ///
    /// This does not include the parent entity you pass in. Parents are guaranteed to be
    /// prior to their children.
    pub fn all_children_iter(&self, entity: Entity) -> SubHierarchyIterator<'_, P> {
        SubHierarchyIterator::new(self, entity)
    }

    /// Get the parent of a specific entity
    pub fn parent(&self, entity: Entity) -> Option<Entity> {
        self.current_parent.get(&entity).cloned()
    }

    pub fn track(&mut self) -> ReaderId<HierarchyEvent> {
        self.changed.register_reader()
    }

    pub fn changed(&self) -> &EventChannel<HierarchyEvent> {
        &self.changed
    }

    pub fn update(&mut self, resources: &SharedResources) {
        self.inserted.clear();
        self.modified.clear();
        self.removed.clear();

        let world = &mut *resources.fetch_mut::<World>();

        for event in world.poll::<P>(&mut self.events) {
            match event {
                ComponentEvent::Inserted(entity) => {
                    self.inserted.add(entity.id());
                }
                ComponentEvent::Modified(entity) => {
                    self.modified.add(entity.id());
                }
                ComponentEvent::Removed(entity) => {
                    self.removed.add(entity.id());
                }
            }
        }

        self.scratch_set.clear();

        for id in (&self.removed).iter() {
            if let Some(index) = self.entities.get(&id) {
                self.scratch_set.insert(self.sorted[*index]);
            }
        }

        for entity in &self.roots {
            if !world.contains(*entity) {
                self.scratch_set.insert(*entity);
            }
        }

        if !self.scratch_set.is_empty() {
            let mut i = 0;
            let mut min_index = std::usize::MAX;
            while i < self.sorted.len() {
                let entity = self.sorted[i];
                let remove = self.scratch_set.contains(&entity)
                    || self
                        .current_parent
                        .get(&entity)
                        .map(|parent_entity| self.scratch_set.contains(&parent_entity))
                        .unwrap_or(false);

                if remove {
                    if i < min_index {
                        min_index = i;
                    }

                    self.scratch_set.insert(entity);
                    self.sorted.remove(i);

                    if let Some(children) = self
                        .current_parent
                        .get(&entity)
                        .cloned()
                        .and_then(|parent_entity| self.children.get_mut(&parent_entity))
                    {
                        if let Some(pos) = children.iter().position(|e| *e == entity) {
                            children.swap_remove(pos);
                        }
                    }

                    self.current_parent.remove(&entity);
                    self.children.remove(&entity);
                    self.entities.remove(&entity.id());
                } else {
                    i += 1;
                }
            }
            for i in min_index..self.sorted.len() {
                self.entities.insert(self.sorted[i].id(), i);
            }
            for entity in &self.scratch_set {
                self.changed.single_write(HierarchyEvent::Removed(*entity));
                self.roots.remove(entity);
            }
        }

        // insert new components in hierarchy
        let inserted = &self.inserted;
        self.scratch_set.clear();
        for (entity, parent) in world
            .query_raw::<&P>()
            .iter()
            .filter(|(e, _)| inserted.contains(e.id()))
        {
            let parent_entity = parent.parent_entity();

            // if we insert a parent component on an entity that have children, we need to make
            // sure the parent is inserted before the children in the sorted list
            let insert_index = self
                .children
                .get(&entity)
                .and_then(|children| {
                    children
                        .iter()
                        .map(|child_entity| self.entities.get(&child_entity.id()).unwrap())
                        .min()
                        .cloned()
                })
                .unwrap_or_else(|| self.sorted.len());

            self.entities.insert(entity.id(), insert_index);

            if insert_index >= self.sorted.len() {
                self.sorted.push(entity);
            } else {
                self.sorted.insert(insert_index, entity);
                for i in insert_index..self.sorted.len() {
                    self.entities.insert(self.sorted[i].id(), i);
                }
            }

            {
                let children = self.children.entry(parent_entity).or_default();

                children.push(entity);
            }

            self.current_parent.insert(entity, parent_entity);
            self.scratch_set.insert(entity);
            if !self.current_parent.contains_key(&parent_entity) {
                self.roots.insert(parent_entity);
            }
            self.roots.remove(&entity);
        }

        let modified = &self.modified;
        for (entity, parent) in world
            .query_raw::<&P>()
            .iter()
            .filter(|(e, _)| modified.contains(e.id()))
        {
            let parent_entity = parent.parent_entity();
            // if theres an old parent
            if let Some(old_parent) = self.current_parent.get(&entity).cloned() {
                // if the parent entity was not changed, ignore event
                if old_parent == parent_entity {
                    continue;
                }
                // remove entity from old parents children
                if let Some(children) = self.children.get_mut(&old_parent) {
                    if let Some(pos) = children.iter().position(|e| *e == entity) {
                        children.remove(pos);
                    }
                }
            }

            // insert in new parents children
            self.children
                .entry(parent_entity)
                .or_insert_with(Vec::default)
                .push(entity);

            // move entity in sorted if needed
            let entity_index = self.entities.get(&entity.id()).cloned().unwrap();
            if let Some(parent_index) = self.entities.get(&parent_entity.id()).cloned() {
                let mut offset = 0;
                let mut process_index = parent_index;
                while process_index > entity_index {
                    let move_entity = self.sorted.remove(process_index);
                    self.sorted.insert(entity_index, move_entity);
                    offset += 1;
                    process_index = self
                        .current_parent
                        .get(&move_entity)
                        .and_then(|p_entity| self.entities.get(&p_entity.id()))
                        .map(|p_index| p_index + offset)
                        .unwrap_or(0);
                }

                // fix indexes
                if parent_index > entity_index {
                    for i in entity_index..parent_index {
                        self.entities.insert(self.sorted[i].id(), i);
                    }
                }
            }

            self.current_parent.insert(entity, parent_entity);
            self.scratch_set.insert(entity);

            if !self.current_parent.contains_key(&parent_entity) {
                self.roots.insert(parent_entity);
            }
        }

        if !self.scratch_set.is_empty() {
            for i in 0..self.sorted.len() {
                let entity = self.sorted[i];
                let notify = self.scratch_set.contains(&entity)
                    || self
                        .current_parent
                        .get(&entity)
                        .map(|parent_entity| self.scratch_set.contains(&parent_entity))
                        .unwrap_or(false);
                if notify {
                    self.scratch_set.insert(entity);
                    self.changed
                        .single_write(HierarchyEvent::ModifiedOrCreated(entity));
                }
            }
        }

        self.scratch_set.clear();
        for entity in &self.roots {
            if !self.children.contains_key(entity) {
                self.scratch_set.insert(*entity);
            }
        }

        for entity in &self.scratch_set {
            self.roots.remove(entity);
        }
    }
}

pub struct SubHierarchyIterator<'a, P: ParentComponent> {
    current_index: usize,
    end_index: usize,
    hierarchy: &'a Hierarchy<P>,
    entities: BitSet,
}

impl<'a, P: ParentComponent> SubHierarchyIterator<'a, P> {
    fn new(hierarchy: &'a Hierarchy<P>, root: Entity) -> Self {
        let max = hierarchy.sorted.len();
        let root_index = hierarchy
            .children
            .get(&root)
            .map(|children| {
                children
                    .iter()
                    .map(|c| hierarchy.entities.get(&c.id()).cloned().unwrap_or(max))
                    .min()
                    .unwrap_or(max)
            })
            .unwrap_or(max);
        let mut iter = SubHierarchyIterator {
            hierarchy,
            current_index: root_index,
            end_index: 0,
            entities: BitSet::new(),
        };
        iter.process_entity(root);
        if root_index != max {
            iter.process_entity(hierarchy.sorted[root_index]);
        }
        iter
    }

    fn process_entity(&mut self, child: Entity) {
        if let Some(children) = self.hierarchy.children.get(&child) {
            for child in children {
                self.entities.add(child.id());
                if let Some(index) = self.hierarchy.entities.get(&child.id()) {
                    if *index > self.end_index {
                        self.end_index = *index;
                    }
                }
            }
        }
    }
}

impl<'a, P: ParentComponent> Iterator for SubHierarchyIterator<'a, P> {
    type Item = Entity;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        if self.current_index >= self.hierarchy.sorted.len() || self.current_index > self.end_index
        {
            None
        } else {
            let entity = self.hierarchy.sorted[self.current_index];
            let mut found_next = false;
            while !found_next {
                self.current_index += 1;
                if self.current_index > self.end_index
                    || self.current_index >= self.hierarchy.sorted.len()
                {
                    found_next = true;
                } else if self
                    .entities
                    .contains(self.hierarchy.sorted[self.current_index].id())
                {
                    found_next = true;
                    self.process_entity(self.hierarchy.sorted[self.current_index]);
                }
            }
            Some(entity)
        }
    }
}
