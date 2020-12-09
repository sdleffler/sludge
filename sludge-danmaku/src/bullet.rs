use {
    dynamic_pool::DynamicReset,
    hashbrown::HashMap,
    sludge::{prelude::*, resources::Shared},
    std::sync::{Arc, RwLock},
    thunderdome::{Arena, Index},
};

use crate::{builder::Parameters, DanmakuResourceExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BulletTypeId(pub(crate) Index);

impl<'lua> ToLua<'lua> for BulletTypeId {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        self.0.to_bits().to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for BulletTypeId {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        Ok(Self(Index::from_bits(FromLua::from_lua(lua_value, lua)?)))
    }
}

pub trait BulletData: Send + Sync + 'static {
    type Bundled: Bundle + Send + Sync;

    fn bundle(
        &self,
        resources: &UnifiedResources,
        parameters: &[Parameters],
        bullet_type: BulletTypeId,
        bundles: &mut Vec<Self::Bundled>,
    ) -> Result<()>;
}

struct BulletDataWrapper<T: BulletData> {
    data: Arc<T>,
}

trait ErasedBulletData: Send + Sync + 'static {
    fn construct_erased_bundler(&self, id: BulletTypeId) -> Box<dyn ErasedMonoBundler>;
}

impl<T: BulletData> ErasedBulletData for BulletDataWrapper<T> {
    fn construct_erased_bundler(&self, id: BulletTypeId) -> Box<dyn ErasedMonoBundler> {
        let data = self.data.clone();
        Box::new(MonoBundler {
            data,
            id,
            params: Vec::new(),
            bundles: Vec::new(),
        })
    }
}

pub struct MonoBundler<T: BulletData> {
    data: Arc<T>,
    id: BulletTypeId,
    params: Vec<Parameters>,
    bundles: Vec<T::Bundled>,
}

trait ErasedMonoBundler: Send + Sync {
    fn extend_erased(&mut self, params: &[Parameters]);

    fn bundle_erased(
        &mut self,
        resources: &UnifiedResources,
        world: &Shared<'static, World>,
        entities: &mut Vec<Entity>,
    ) -> Result<()>;
}

impl<T: BulletData> ErasedMonoBundler for MonoBundler<T> {
    fn extend_erased(&mut self, params: &[Parameters]) {
        self.params.extend_from_slice(params);
    }

    fn bundle_erased(
        &mut self,
        resources: &UnifiedResources,
        world: &Shared<'static, World>,
        entities: &mut Vec<Entity>,
    ) -> Result<()> {
        self.data
            .bundle(resources, &self.params, self.id, &mut self.bundles)?;
        world
            .borrow_mut()
            .spawn_batch_into_buf(self.bundles.drain(..), entities);
        Ok(())
    }
}

pub struct Bundler {
    current: Option<(BulletTypeId, Box<dyn ErasedMonoBundler>)>,
    buf: Vec<Parameters>,
    monos: HashMap<BulletTypeId, Box<dyn ErasedMonoBundler>>,
    bullet_types: Arc<RwLock<BulletTypes>>,
}

impl Bundler {
    pub(crate) fn new(bullet_types: Arc<RwLock<BulletTypes>>) -> Self {
        let monos = bullet_types
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .types
            .iter()
            .map(|(id, ty)| {
                (
                    BulletTypeId(id),
                    ty.erased.construct_erased_bundler(BulletTypeId(id)),
                )
            })
            .collect();
        Self {
            current: None,
            buf: Vec::new(),
            monos,
            bullet_types,
        }
    }

    #[inline]
    fn reset(&mut self) {
        if let Some((id, mut bundler)) = self.current.take() {
            bundler.extend_erased(&self.buf);
            self.monos.insert(id, bundler);
        }
        self.buf.clear();
    }

    #[inline]
    pub fn set_id(&mut self, new_id: BulletTypeId) {
        self.reset();
        let bundler = self
            .monos
            .remove(&new_id)
            .or_else(|| {
                self.bullet_types
                    .read()
                    .unwrap_or_else(|p| p.into_inner())
                    .types
                    .get(new_id.0)
                    .map(|bt| bt.erased.construct_erased_bundler(new_id))
            })
            .unwrap();
        self.current = Some((new_id, bundler));
    }

    #[inline]
    pub fn push(&mut self, params: Parameters) {
        self.buf.push(params);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
        self.current = None;
    }

    pub fn flush(
        &mut self,
        resources: &UnifiedResources,
        world: &Shared<'static, World>,
        entities: &mut Vec<Entity>,
    ) -> Result<()> {
        self.reset();
        for (_, mono) in self.monos.iter_mut() {
            mono.bundle_erased(resources, world, entities)?;
        }
        Ok(())
    }
}

impl DynamicReset for Bundler {
    fn reset(&mut self) {
        self.clear();
    }
}

pub(crate) struct BulletType {
    erased: Box<dyn ErasedBulletData>,
}

pub struct BulletTypes {
    pub(crate) types: Arena<BulletType>,
    pub(crate) named: HashMap<String, BulletTypeId>,
}

impl BulletTypes {
    pub fn new() -> Self {
        Self {
            types: Arena::new(),
            named: HashMap::new(),
        }
    }

    pub fn insert<T>(&mut self, bullet: T) -> BulletTypeId
    where
        T: BulletData,
    {
        let bt = BulletType {
            erased: Box::new(BulletDataWrapper {
                data: Arc::new(bullet),
            }),
        };
        let id = self.types.insert(bt);
        BulletTypeId(id)
    }

    pub fn insert_with_name<S, T>(&mut self, bullet: T, name: &S) -> BulletTypeId
    where
        S: AsRef<str> + ?Sized,
        T: BulletData,
    {
        let id = self.insert(bullet);
        self.named.insert(name.as_ref().to_owned(), id);
        id
    }
}

#[derive(Clone, Copy)]
pub struct BulletMetatype {
    pub(crate) name: &'static str,
    pub(crate) insert: for<'lua> fn(
        LuaValue<'lua>,
        Option<LuaString<'lua>>,
        LuaContext<'lua>,
    ) -> Result<BulletTypeId>,
}

impl BulletMetatype {
    pub fn new<T>(name: &'static str) -> Self
    where
        T: BulletData + for<'lua> FromLua<'lua>,
    {
        Self {
            name,
            insert: Self::decode::<T>,
        }
    }

    fn decode<'lua, T>(
        bullet_type: LuaValue<'lua>,
        name: Option<LuaString<'lua>>,
        lua: LuaContext<'lua>,
    ) -> Result<BulletTypeId>
    where
        T: BulletData + for<'lua2> FromLua<'lua2>,
    {
        let bullet_type = T::from_lua(bullet_type, lua)?;
        if let Some(name) = name {
            let s = name.to_str()?;
            lua.insert_bullet_type_with_name(bullet_type, s)
        } else {
            lua.insert_bullet_type(bullet_type)
        }
    }
}

inventory::collect!(BulletMetatype);

#[derive(Debug)]
pub struct UserdataValue {
    key: LuaRegistryKey,
}

impl<'a, 'lua> ToLua<'lua> for &'a UserdataValue {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        lua.registry_value(&self.key)
    }
}

impl<'lua> FromLua<'lua> for UserdataValue {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let key = lua.create_registry_value(lua_value)?;
        Ok(Self { key })
    }
}

// #[derive(Debug, Clone, Copy, Bundle)]
// pub struct DirectionalShot {
//     pub projectile: Projectile,
//     pub motion: DirectionalMotion,
// }

// #[derive(Debug, Clone, Copy)]
// pub enum Shot {
//     Quadratic(QuadraticShot),
//     Directional(DirectionalShot),
// }

// impl Shot {
//     pub fn linear(id: BulletTypeId, at: Isometry2<f32>, vel: Velocity2<f32>) -> Self {
//         Self::quadratic(id, at, vel, Velocity2::zero())
//     }

//     pub fn quadratic(
//         id: BulletTypeId,
//         at: Isometry2<f32>,
//         vel: Velocity2<f32>,
//         acc: Velocity2<f32>,
//     ) -> Self {
//         Self::Quadratic(QuadraticShot {
//             projectile: Projectile::new(id, at),
//             motion: QuadraticMotion {
//                 integrated: Isometry2::identity(),
//                 velocity: vel,
//                 acceleration: acc,
//             },
//         })
//     }
// }
