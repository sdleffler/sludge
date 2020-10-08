use crate::{ecs::*, SludgeResultExt};
use {
    hashbrown::HashMap,
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
    current_layers: HashMap<Entity, i32>,
    events: ReaderId<ComponentEvent>,
}

impl LayerManager {
    pub fn new(world: &mut World) -> Self {
        let events = world.track::<LayerIndex>();
        let mut layers = BTreeMap::<_, Vec<_>>::new();
        let mut current_layers = HashMap::new();

        world
            .query::<(&LayerIndex,)>()
            .iter()
            .for_each(|(e, (layer_index,))| {
                layers.entry(layer_index.layer).or_default().push(e);
                current_layers.insert(e, layer_index.layer);
            });

        Self {
            layers,
            current_layers,
            events,
        }
    }

    pub fn update(&mut self, world: &World) {
        let Self {
            layers,
            current_layers,
            events,
        } = self;

        for &event in world.poll::<LayerIndex>(events) {
            match event {
                ComponentEvent::Inserted(entity) => {
                    if let Ok(layer_index) =
                        world.get::<LayerIndex>(entity).log_warn_err(module_path!())
                    {
                        layers.entry(layer_index.layer).or_default().push(entity);
                        current_layers.insert(entity, layer_index.layer);
                    }
                }
                ComponentEvent::Modified(entity) => {
                    let layer_index =
                        match world.get::<LayerIndex>(entity).log_warn_err(module_path!()) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };

                    if let Some(old_index) = world
                        .get::<LayerIndex>(entity)
                        .log_warn_err(module_path!())
                        .ok()
                        .and_then(|layer_index| {
                            current_layers
                                .get(&entity)
                                .copied()
                                .filter(|&old_index| old_index != layer_index.layer)
                        })
                    {
                        let layer = layers.get_mut(&old_index).unwrap();
                        if let Some(i) = layer.iter().position(|&e| e == entity) {
                            layer.swap_remove(i);
                        }
                    }

                    layers.entry(layer_index.layer).or_default().push(entity);
                    current_layers.insert(entity, layer_index.layer);
                }
                ComponentEvent::Removed(entity) => {
                    let layer_index =
                        match world.get::<LayerIndex>(entity).log_warn_err(module_path!()) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };

                    let layer = layers.get_mut(&layer_index.layer).unwrap();
                    if let Some(i) = layer.iter().position(|&e| e == entity) {
                        layer.swap_remove(i);
                    }

                    current_layers.remove(&entity);
                }
            }
        }
    }
}
