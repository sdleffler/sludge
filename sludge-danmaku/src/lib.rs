#![feature(exact_size_is_empty)]

use ::{
    atomic_refcell::AtomicRefCell,
    dynamic_pool::{DynamicPool, DynamicPoolItem},
    hashbrown::HashMap,
    hibitset::{BitSet, DrainableBitSet},
    rand::RngCore,
    sludge::{api::Module, prelude::*},
    sludge_2d::math::*,
    std::{
        f32,
        ops::Deref,
        sync::{Arc, RwLock, RwLockReadGuard},
    },
};

mod builder;
mod bullet;
mod components;
pub mod pattern;

#[doc(inline)]
pub use crate::{
    builder::{LuaPatternBuilder, Op, Parameters, PatternBuilder},
    bullet::{BulletData, BulletMetatype, BulletTypeId, Bundler},
    components::{
        Collision, DespawnAfterTimeLimit, DespawnOutOfBounds, DirectionalMotion, MaximumVelocity,
        ParametricMotion, Projectile, Proximity, QuadraticMotion,
    },
};

pub use sludge::inventory;

use crate::{
    builder::Batch,
    bullet::BulletTypes,
    pattern::{Group, LuaPattern, RustPattern},
};

const RNG_REGISTRY_KEY: &'static str = "danmaku.rng";

#[derive(Clone)]
pub struct SharedRng<R: RngCore> {
    rng: Arc<AtomicRefCell<R>>,
}

impl<R: RngCore> SharedRng<R> {
    pub fn new(rng: R) -> Self {
        Self {
            rng: Arc::new(AtomicRefCell::new(rng)),
        }
    }
}

impl<R: RngCore> LuaUserData for SharedRng<R> {}

impl<R: RngCore> RngCore for SharedRng<R> {
    fn next_u32(&mut self) -> u32 {
        self.rng.borrow_mut().next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.rng.borrow_mut().next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.rng.borrow_mut().fill_bytes(dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.rng.borrow_mut().try_fill_bytes(dest)
    }
}

// pub trait Bullets: Send + Sync + 'static {}

// struct BulletType<T: Bullets> {
//     bullet: sync::Arc<dyn ErasedBullet>,
//     data: T,
// }

// #[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
// pub struct DanmakuId(Entity);

pub struct BulletTypesRef<'a> {
    inner: RwLockReadGuard<'a, BulletTypes>,
}

impl<'a> Deref for BulletTypesRef<'a> {
    type Target = BulletTypes;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct Danmaku {
    bounds: Option<Box2<f32>>,
    to_despawn: BitSet,
    bullet_metatypes: HashMap<String, BulletMetatype>,
    bullet_types: Arc<RwLock<BulletTypes>>,
    bundler_pool: DynamicPool<Bundler>,
}

impl Danmaku {
    pub fn new() -> Self {
        let bullet_types = Arc::new(RwLock::new(BulletTypes::new()));
        let bullet_metatypes = inventory::iter::<BulletMetatype>
            .into_iter()
            .map(|bmt| (bmt.name.to_owned(), *bmt))
            .collect();
        let bundler_pool = {
            let bt_cloned = bullet_types.clone();
            DynamicPool::new(4, 32, move || Bundler::new(bt_cloned.clone()))
        };
        Self {
            bounds: None,
            to_despawn: BitSet::new(),
            bullet_metatypes,
            bullet_types,
            bundler_pool,
        }
    }

    pub fn with_bounds(bounds: Box2<f32>) -> Self {
        Self {
            bounds: Some(bounds),
            ..Self::new()
        }
    }

    pub fn insert_bullet_type<T>(&mut self, bullet_type: T) -> BulletTypeId
    where
        T: BulletData,
    {
        self.bullet_types
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(bullet_type)
    }

    pub fn insert_bullet_type_with_name<S, T>(&mut self, bullet_type: T, name: &S) -> BulletTypeId
    where
        S: AsRef<str> + ?Sized,
        T: BulletData,
    {
        self.bullet_types
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert_with_name(bullet_type, name)
    }

    pub fn bullet_types(&self) -> BulletTypesRef<'_> {
        BulletTypesRef {
            inner: self.bullet_types.read().unwrap_or_else(|p| p.into_inner()),
        }
    }

    pub fn bundler(&self) -> DynamicPoolItem<Bundler> {
        self.bundler_pool.take()
    }

    pub fn update(&mut self, world: &mut World, dt: f32) {
        for (_e, (mut proj, mut quadratic, maximum)) in world
            .query::<(
                &mut Projectile,
                &mut QuadraticMotion,
                Option<&MaximumVelocity>,
            )>()
            .iter()
        {
            let quadratic = &mut *quadratic;
            quadratic.velocity += quadratic.acceleration * dt;

            if let Some(max_vel) = maximum {
                let cur_vel = quadratic.velocity.linear.norm();
                if cur_vel > max_vel.linear {
                    quadratic.velocity.linear *= max_vel.linear / cur_vel;
                }

                let cur_ang = quadratic.velocity.angular.abs();
                if cur_ang > max_vel.angular {
                    quadratic.velocity.angular *= max_vel.angular / cur_ang;
                }
            }

            let delta = quadratic.velocity.integrate(dt);
            quadratic.integrated.translation *= delta.translation;
            quadratic.integrated.rotation *= delta.rotation;

            let proj = &mut *proj;
            proj.next_position.translation *= quadratic.integrated.translation;
            proj.next_position.rotation *= quadratic.integrated.rotation;
        }

        for (_e, (mut proj, mut directional, maximum)) in world
            .query::<(
                &mut Projectile,
                &mut DirectionalMotion,
                Option<&MaximumVelocity>,
            )>()
            .iter()
        {
            let directional = &mut *directional;
            directional.velocity += directional.acceleration * dt;

            if let Some(max_vel) = maximum {
                let cur_vel = directional.velocity.linear.norm();
                if cur_vel > max_vel.linear {
                    directional.velocity.linear *= max_vel.linear / cur_vel;
                }

                let cur_ang = directional.velocity.angular.abs();
                if cur_ang > max_vel.angular {
                    directional.velocity.angular *= max_vel.angular / cur_ang;
                }
            }

            directional.integrated *= directional.velocity.integrate(dt);

            let proj = &mut *proj;
            proj.next_position.translation *= directional.integrated.translation;
            proj.next_position.rotation *= directional.integrated.rotation;
        }

        for (e, (mut proj, mut motion)) in world
            .query::<(&mut Projectile, &mut ParametricMotion)>()
            .iter()
        {
            let (proj, motion) = (&mut *proj, &mut *motion);
            let iso = motion.update(dt);
            proj.next_position.translation *= iso.translation;
            proj.next_position.rotation *= iso.rotation;

            if motion.despawn_after_duration {
                self.to_despawn.add(e.id());
            }
        }

        for (_e, (mut proj,)) in world.query::<(&mut Projectile,)>().iter() {
            let proj = &mut *proj;
            proj.position = proj.next_position;
            proj.next_position = proj.origin;
        }

        if let Some(bounds) = self.bounds {
            for (e, (proj, collision, _)) in world
                .query::<(&Projectile, &Collision, &DespawnOutOfBounds)>()
                .iter()
            {
                let bb = match *collision {
                    Collision::Circle { radius } => Box2::from_half_extents(
                        Point2::from(proj.position.translation.vector),
                        Vector2::repeat(radius),
                    ),
                    Collision::Rectangle { radii } => {
                        let homogeneous = homogeneous_mat3_to_mat4(&proj.position.to_homogeneous());
                        Box2::from_half_extents(Point2::origin(), radii)
                            .transformed_by(&homogeneous)
                    }
                };

                if !bounds.intersects(&bb) {
                    self.to_despawn.add(e.id());
                }
            }
        }

        for (e, (_, mut time_limit)) in world
            .query::<(&Projectile, &mut DespawnAfterTimeLimit)>()
            .iter()
        {
            time_limit.ttl -= dt;
            if time_limit.ttl <= 0. {
                self.to_despawn.add(e.id());
            }
        }

        for id in self.to_despawn.drain() {
            let entity = unsafe { world.find_entity_from_id(id) };
            world.despawn(entity).unwrap();
        }
    }
}

pub trait DanmakuResourceExt {
    fn bundler(&self) -> Result<DynamicPoolItem<Bundler>>;
    fn insert_bullet_type<T>(&self, bullet_type: T) -> Result<BulletTypeId>
    where
        T: BulletData;
    fn insert_bullet_type_with_name<S, T>(&self, bullet_type: T, name: &S) -> Result<BulletTypeId>
    where
        S: AsRef<str> + ?Sized,
        T: BulletData;
    fn get_bullet_type<S>(&self, name: &S) -> Result<BulletTypeId>
    where
        S: AsRef<str> + ?Sized;
}

impl<'a, R: Resources<'a>> DanmakuResourceExt for R {
    fn bundler(&self) -> Result<DynamicPoolItem<Bundler>> {
        Ok(self.fetch_one::<Danmaku>()?.borrow().bundler())
    }

    fn insert_bullet_type<T>(&self, bullet_type: T) -> Result<BulletTypeId>
    where
        T: BulletData,
    {
        Ok(self
            .fetch_one::<Danmaku>()?
            .borrow_mut()
            .insert_bullet_type(bullet_type))
    }

    fn insert_bullet_type_with_name<S, T>(&self, bullet_type: T, name: &S) -> Result<BulletTypeId>
    where
        S: AsRef<str> + ?Sized,
        T: BulletData,
    {
        Ok(self
            .fetch_one::<Danmaku>()?
            .borrow_mut()
            .insert_bullet_type_with_name(bullet_type, name))
    }

    fn get_bullet_type<S>(&self, name: &S) -> Result<BulletTypeId>
    where
        S: AsRef<str> + ?Sized,
    {
        self.fetch_one::<Danmaku>()?
            .borrow()
            .bullet_types
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .named
            .get(name.as_ref())
            .copied()
            .ok_or_else(|| anyhow!("no such bullet type `{}`", name.as_ref()))
    }
}

// pub trait Bullet: Send + Sync {
//     type Bundled: Bundle;

//     fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled;
//     fn on_batched(&self, _lua: LuaContext) -> Result<()> {
//         Ok(())
//     }
// }

// impl Bullet for QuadraticShot {
//     type Bundled = Self;

//     fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
//         let position = parameters.apply_to_position(&self.projectile.position);
//         let velocity = parameters.apply_to_velocity(&self.motion.velocity);
//         let acceleration = parameters.apply_to_acceleration(&self.motion.acceleration);

//         Self {
//             projectile: Projectile::new(self.projectile.id, position),
//             motion: QuadraticMotion::new(velocity, acceleration),
//         }
//     }
// }

// impl Bullet for DirectionalShot {
//     type Bundled = Self;

//     fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
//         let position = parameters.apply_to_position(&self.projectile.position);
//         let velocity = parameters.apply_to_velocity(&self.motion.velocity);
//         let acceleration = parameters.apply_to_acceleration(&self.motion.acceleration);

//         Self {
//             projectile: Projectile::new(self.projectile.id, position),
//             motion: DirectionalMotion {
//                 integrated: Isometry2::identity(),
//                 velocity,
//                 acceleration,
//             },
//         }
//     }
// }

// struct SingleTypeBatch<B: Bullet> {
//     bullet: B,
//     data: Vec<B::Bundled>,
// }

// #[derive(Clone)]
// pub struct BulletType {
//     name: &'static str,
//     bullet: sync::Arc<dyn ErasedBullet>,
// }

// trait ErasedBullet: Send + Sync {
//     fn batch_me<'lua>(
//         &self,
//         lua: LuaContext<'lua>,
//         closure: LuaFunction<'lua>,
//     ) -> LuaResult<Vec<Entity>>;
// }

// struct BulletSlug<B: Bullet + Clone> {
//     bullet: B,
// }

// impl<B: Bullet + Clone> ErasedBullet for BulletSlug<B> {
//     fn batch_me<'lua>(
//         &self,
//         lua: LuaContext<'lua>,
//         closure: LuaFunction<'lua>,
//     ) -> LuaResult<Vec<Entity>> {
//         let mut batch = Batch::new(lua, self.bullet.clone()).to_lua_err()?;
//         lua.scope(|scope| -> LuaResult<()> {
//             let emit_closure =
//                 scope.create_function_mut(|_lua, op: Op| batch.op(op).to_lua_err())?;
//             let lua_builder = LuaPatternBuilder::new(lua, emit_closure)?;
//             LuaFunction::call(&closure, lua_builder)?;
//             Ok(())
//         })?;

//         self.bullet.on_batched(lua).to_lua_err()?;

//         let tmp = lua.fetch_one::<World>()?;
//         let world = &mut *tmp.borrow_mut();
//         Ok(world.spawn_batch(batch.to_vec()))
//     }
// }

// impl BulletType {
//     pub fn new<B: Bullet + Clone + 'static>(name: &'static str, bullet: B) -> Self {
//         Self {
//             name,
//             bullet: sync::Arc::new(BulletSlug { bullet }),
//         }
//     }
// }

// inventory::collect!(BulletType);

// #[derive(Clone)]
// pub struct LuaBullet {
//     erased: sync::Arc<dyn ErasedBullet>,
// }

// impl LuaUserData for LuaBullet {}

pub struct DanmakuSystem;

impl System for DanmakuSystem {
    fn init(
        &self,
        _lua: LuaContext,
        local: &mut OwnedResources,
        _global: Option<&SharedResources>,
    ) -> Result<()> {
        if !local.has_value::<Danmaku>() {
            local.insert(Danmaku::new());
        }

        Ok(())
    }

    fn update(&self, _lua: LuaContext, resources: &UnifiedResources) -> Result<()> {
        let (world, danmaku) = resources.fetch::<(World, Danmaku)>()?;
        danmaku
            .borrow_mut()
            .update(&mut *world.borrow_mut(), 1. / 60.);

        Ok(())
    }
}

pub mod api {
    use super::*;

    fn wrap<'lua2, A, R, F>(lua: LuaContext<'lua2>, f: F) -> LuaResult<LuaValue<'lua2>>
    where
        A: FromLuaMulti<'lua2>,
        R: ToLuaMulti<'lua2>,
        F: 'static + Send + Fn(LuaContext<'lua2>, A) -> LuaResult<R>,
    {
        lua.create_function(f)?.to_lua(lua)
    }

    pub fn new_group<'lua>(_lua: LuaContext<'lua>, _: ()) -> LuaResult<Group> {
        Ok(Group::new())
    }

    pub fn spawn<'lua>(
        lua: LuaContext<'lua>,
        (closure, maybe_lua_group): (LuaFunction<'lua>, Option<LuaAnyUserData<'lua>>),
    ) -> LuaResult<()> {
        let mut maybe_group = maybe_lua_group
            .as_ref()
            .map(LuaAnyUserData::borrow_mut::<Group>)
            .transpose()?;
        let resources = lua.resources();
        let world = resources.fetch_one::<World>()?;

        let mut batch = Batch::new(lua).to_lua_err()?;
        lua.scope(|scope| -> LuaResult<()> {
            let emit_closure =
                scope.create_function_mut(|_lua, op: Op| batch.op(op).to_lua_err())?;
            let lua_builder = LuaPatternBuilder::new(lua, emit_closure)?;
            LuaFunction::call(&closure, lua_builder)?;
            Ok(())
        })?;

        let entities = batch.spawn(&resources, &world).to_lua_err()?;
        if let Some(group) = maybe_group.as_deref_mut() {
            group.entities.extend(entities);
        }

        Ok(())
    }

    pub mod bullet {
        use super::*;

        pub fn new<'lua>(lua: LuaContext<'lua>, table: LuaTable<'lua>) -> LuaResult<BulletTypeId> {
            let metatype = table.get::<_, LuaString>("metatype")?;
            let bullet_type = table.get("bullet")?;
            let name = table.get("name")?;
            let danmaku = lua.fetch_one::<Danmaku>()?;
            let insert = danmaku
                .borrow()
                .bullet_metatypes
                .get(metatype.to_str()?)
                .ok_or_else(|| anyhow!("no such bullet metatype"))
                .to_lua_err()?
                .insert;
            insert(bullet_type, name, lua).to_lua_err()
        }

        pub fn get_type_by_name<'lua>(
            lua: LuaContext<'lua>,
            name: LuaString<'lua>,
        ) -> LuaResult<BulletTypeId> {
            lua.get_bullet_type(name.to_str()?).to_lua_err()
        }

        pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
            let t = lua.create_table_from(vec![
                ("new", wrap(lua, new)?),
                ("get_type_by_name", wrap(lua, get_type_by_name)?),
            ])?;
            Ok(LuaValue::Table(t))
        }
    }

    pub mod pattern {
        use super::*;
        use crate::pattern::{Aimed, Arc, Destination, Ring, Stack};

        pub fn aimed<'lua>(_lua: LuaContext<'lua>, (x, y): (f32, f32)) -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Aimed {
                target: Point2::new(x, y),
            }))
        }

        pub fn arc<'lua>(
            _lua: LuaContext<'lua>,
            (radius, angle, count): (f32, f32, u32),
        ) -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Arc {
                radius,
                angle,
                count,
            }))
        }

        pub fn destination<'lua>(
            _lua: LuaContext<'lua>,
            (duration, x, y, angle): (f32, f32, f32, Option<f32>),
        ) -> LuaResult<RustPattern> {
            let destination = match angle {
                Some(angle) => Isometry2::new(Vector2::new(x, y), angle),
                None => Isometry2::translation(x, y),
            };
            Ok(RustPattern::new(Destination {
                destination,
                duration,
            }))
        }

        pub fn new<'lua>(_lua: LuaContext<'lua>, pattern: LuaPattern) -> LuaResult<RustPattern> {
            Ok(RustPattern::new(pattern))
        }

        pub fn ring<'lua>(
            _lua: LuaContext<'lua>,
            (radius, count): (f32, u32),
        ) -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Ring { radius, count }))
        }

        pub fn stack<'lua>(
            _lua: LuaContext<'lua>,
            (x, y, angular, count): (f32, f32, f32, u32),
        ) -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Stack {
                delta: Velocity2::new(Vector2::new(x, y), angular),
                count,
            }))
        }

        pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
            let t = lua.create_table_from(vec![
                ("aimed", wrap(lua, aimed)?),
                ("arc", wrap(lua, arc)?),
                ("destination", wrap(lua, destination)?),
                ("new", wrap(lua, new)?),
                ("ring", wrap(lua, ring)?),
                ("stack", wrap(lua, stack)?),
            ])?;
            Ok(LuaValue::Table(t))
        }
    }

    pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
        let t = lua.create_table_from(vec![
            ("pattern", pattern::load(lua)?),
            ("bullet", bullet::load(lua)?),
            ("new_group", wrap(lua, new_group)?),
            ("spawn", wrap(lua, spawn)?),
        ])?;
        Ok(LuaValue::Table(t))
    }
}

// pub fn load<'lua, T: Bullets>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
//     let bullets = inventory::iter::<BulletType>
//         .into_iter()
//         .map(|bullet| {
//             let name = bullet.name;
//             let erased = bullet.bullet.clone();
//             (name, erased)
//         })
//         .collect::<HashMap<_, _>>();

//     let table = lua.create_table()?;
//     let key = lua.create_registry_value(table.clone())?;
//     table.set(
//         "group",
//         lua.create_function(|_, ()| {
//             Ok(Group {
//                 entities: Vector::new(),
//             })
//         })?,
//     )?;
//     table.set(
//         "pattern",
//         lua.create_function(|_, pattern: LuaPattern| Ok(RustPattern::new(pattern)))?,
//     )?;
//     table.set(
//         "aimed",
//         lua.create_function(|_, (x, y)| {
//             Ok(RustPattern::new(Aimed {
//                 target: Point2::new(x, y),
//             }))
//         })?,
//     )?;
//     table.set(
//         "destination",
//         lua.create_function(|_, (duration, x, y, angle): (f32, f32, f32, Option<f32>)| {
//             let destination = match angle {
//                 Some(angle) => Isometry2::new(Vector2::new(x, y), angle),
//                 None => Isometry2::translation(x, y),
//             };
//             Ok(RustPattern::new(Destination {
//                 destination,
//                 duration,
//             }))
//         })?,
//     )?;
//     table.set(
//         "ring",
//         lua.create_function(|_, (radius, count)| -> LuaResult<RustPattern> {
//             Ok(RustPattern::new(Ring { radius, count }))
//         })?,
//     )?;
//     table.set(
//         "arc",
//         lua.create_function(|_, (radius, angle, count)| -> LuaResult<RustPattern> {
//             Ok(RustPattern::new(Arc {
//                 radius,
//                 angle,
//                 count,
//             }))
//         })?,
//     )?;
//     table.set(
//         "stack",
//         lua.create_function(|_, (x, y, angular, count)| -> LuaResult<RustPattern> {
//             Ok(RustPattern::new(Stack {
//                 delta: Velocity2::new(Vector2::new(x, y), angular),
//                 count,
//             }))
//         })?,
//     )?;
//     table.set(
//         "spawn",
//         lua.create_function(
//             move |lua,
//                   (bullet_ty, closure, maybe_lua_group): (
//                 LuaValue,
//                 LuaFunction,
//                 Option<LuaAnyUserData>,
//             )| {
//                 let mut maybe_group = maybe_lua_group
//                     .as_ref()
//                     .map(LuaAnyUserData::borrow_mut::<Group>)
//                     .transpose()?;

//                 let entities = match &bullet_ty {
//                     LuaValue::String(ty_string) => {
//                         bullets[ty_string.to_str()?].batch_me(lua, closure)?
//                     }
//                     LuaValue::UserData(ty_ud) => {
//                         ty_ud.borrow::<LuaBullet>()?.erased.batch_me(lua, closure)?
//                     }
//                     _ => {
//                         return Err(LuaError::FromLuaConversionError {
//                             from: "lua value",
//                             to: "string or userdata",
//                             message: None,
//                         });
//                     }
//                 };

//                 let table = lua.registry_value::<LuaTable>(&key)?;
//                 if let Some(on_spawn) = table.get::<_, Option<LuaFunction>>("on_spawn")? {
//                     on_spawn.call::<_, ()>(bullet_ty)?;
//                 }

//                 if let Some(group) = maybe_group.as_deref_mut() {
//                     group.entities.extend(entities);
//                 }

//                 Ok(())
//             },
//         )?,
//     )?;
//     table.set(
//         "set_bounds",
//         lua.create_function(|lua, bounds: Option<Box2<f32>>| {
//             let danmaku = lua.fetch_one::<Danmaku<T>>()?;
//             danmaku.borrow_mut().bounds = bounds;
//             Ok(())
//         })?,
//     )?;

//     Ok(LuaValue::Table(table))
// }

// pub fn module<T: Bullets>(name: &'static str) -> Module {
//     Module::parse(name, load::<T>)
// }

inventory::submit! {
    Module::parse("danmaku", api::load)
}
