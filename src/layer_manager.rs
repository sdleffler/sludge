use crate::ecs::*;
use {
    hibitset::BitSet,
    shrev::ReaderId,
    std::{any::TypeId, collections::BTreeMap},
};

#[derive(Debug, Copy, Clone)]
pub struct LayerIndex {
    pub layer: i32,
}

impl<'a> SmartComponent<ScContext<'a>> for LayerIndex {
    fn on_borrow_mut(&mut self, entity: Entity, context: ScContext<'a>) {
        context[&TypeId::of::<Self>()].emit_modified_atomic(entity);
    }
}

inventory::submit! {
    FlaggedComponent::of::<LayerIndex>()
}

#[derive(Debug)]
pub struct LayerManager {
    layers: BTreeMap<i32, Vec<Entity>>,
    events: ReaderId<ComponentEvent>,

    to_update: BitSet,
    to_remove: BitSet,
}

impl LayerManager {
    pub fn new(world: &mut World) -> Self {
        let events = world.track::<LayerIndex>();
        let mut layers = BTreeMap::<_, Vec<_>>::new();

        world
            .query::<(&LayerIndex,)>()
            .iter()
            .for_each(|(e, (layer_index,))| {
                layers.entry(layer_index.layer).or_default().push(e);
            });

        Self {
            layers,
            events,

            to_update: BitSet::new(),
            to_remove: BitSet::new(),
        }
    }

    // pub fn update(&mut self, world: &World) {
    //     for event in world.poll::<LayerIndex>(&mut self.events) {
    //         match event {
    //             ComponentEvent::Inserted(entity) | ComponentEvent::Modified(entity) => {
    //                 self.to_update.add(entity.id());
    //                 self.to_remove.remove(entity.id());
    //             }
    //             ComponentEvent::Removed(entity) => {
    //                 self.to_remove.add(entity.id());
    //                 self.to_update.removed(entity.id());
    //             }
    //         }
    //     }
    // }
}
