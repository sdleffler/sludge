use ::{
    easer::functions::*,
    ncollide2d as nc,
    sludge::{
        api::{LuaComponent, LuaComponentInterface},
        prelude::*,
    },
    sludge_2d::math::*,
    smallbox::SmallBox,
    stack_dst::Value as StackDst,
    std::f32,
};

use crate::bullet::BulletTypeId;

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct Projectile {
    pub(crate) position: Isometry2<f32>,
    pub(crate) next_position: Isometry2<f32>,
    pub(crate) origin: Isometry2<f32>,
    pub(crate) id: BulletTypeId,
}

impl Projectile {
    pub fn origin(id: BulletTypeId) -> Self {
        Self::new(id, Isometry2::identity())
    }

    pub fn new(id: BulletTypeId, origin: Isometry2<f32>) -> Self {
        Self {
            position: origin,
            next_position: origin,
            origin,
            id,
        }
    }

    pub fn position(&self) -> &Isometry2<f32> {
        &self.position
    }

    pub fn next_position(&self) -> &Isometry2<f32> {
        &self.next_position
    }

    pub fn next_position_mut(&mut self) -> &mut Isometry2<f32> {
        &mut self.next_position
    }

    pub fn bullet_type(&self) -> BulletTypeId {
        self.id
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProjectileAccessor(Entity);

impl LuaUserData for ProjectileAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("position", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
            let projectile = world.get::<Projectile>(this.0).to_lua_err()?;
            let v = projectile.position.translation.vector;
            Ok((v.x, v.y, projectile.position.rotation.angle()))
        });
    }
}

impl LuaComponentInterface for Projectile {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        ProjectileAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        _lua: LuaContext<'lua>,
        _args: LuaValue<'lua>,
        _builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        todo!()
    }
}

inventory::submit! {
    LuaComponent::new::<Projectile>("Projectile")
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct QuadraticMotion {
    pub integrated: Isometry2<f32>,
    pub velocity: Velocity2<f32>,
    pub acceleration: Velocity2<f32>,
}

impl QuadraticMotion {
    pub fn zero() -> Self {
        Self::new(Velocity2::zero(), Velocity2::zero())
    }

    pub fn with_velocity(velocity: Velocity2<f32>) -> Self {
        Self::new(velocity, Velocity2::zero())
    }

    pub fn new(velocity: Velocity2<f32>, acceleration: Velocity2<f32>) -> Self {
        Self {
            integrated: Isometry2::identity(),
            velocity,
            acceleration,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QuadraticMotionAccessor(Entity);

impl LuaUserData for QuadraticMotionAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("integrateed", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
            let projectile = world.get::<QuadraticMotion>(this.0).to_lua_err()?;
            let v = projectile.integrated.translation.vector;
            Ok((v.x, v.y, projectile.integrated.rotation.angle()))
        });

        methods.add_method("velocity", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
            let projectile = world.get::<QuadraticMotion>(this.0).to_lua_err()?;
            let v = projectile.velocity.linear;
            Ok((v.x, v.y, projectile.velocity.angular))
        });

        methods.add_method("acceleration", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
            let projectile = world.get::<QuadraticMotion>(this.0).to_lua_err()?;
            let v = projectile.acceleration.linear;
            Ok((v.x, v.y, projectile.acceleration.angular))
        });
    }
}

impl LuaComponentInterface for QuadraticMotion {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        QuadraticMotionAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        _lua: LuaContext<'lua>,
        _args: LuaValue<'lua>,
        _builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        todo!()
    }
}

inventory::submit! {
    LuaComponent::new::<QuadraticMotion>("Quadratic")
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct DirectionalMotion {
    pub integrated: Isometry2<f32>,
    pub velocity: Velocity2<f32>,
    pub acceleration: Velocity2<f32>,
}

pub trait ParametricMotionFunction: Send + Sync {
    fn set_endpoints(&mut self, start: &Isometry2<f32>, end: &Isometry2<f32>);
    fn transform_by(&mut self, tx: &Isometry2<f32>);
    fn calculate(&self, t: f32) -> Isometry2<f32>;
}

trait PmfClone: ParametricMotionFunction {
    fn as_pmf(&self) -> &dyn ParametricMotionFunction;
    fn clone_smallboxed(&self) -> SmallBox<dyn PmfClone, ParametricMotionSpace>;
}

impl<T: ParametricMotionFunction + Clone + 'static> PmfClone for T {
    fn as_pmf(&self) -> &dyn ParametricMotionFunction {
        self
    }

    fn clone_smallboxed(&self) -> SmallBox<dyn PmfClone, ParametricMotionSpace> {
        SmallBox::new(self.clone())
    }
}

#[derive(Clone, Copy)]
pub struct ParametricEased {
    pub origin: Isometry2<f32>,
    pub displacement: Velocity2<f32>,
    pub duration: f32,
    pub easer: fn(f32, f32, f32, f32) -> f32,
}

impl ParametricEased {
    pub fn new(
        duration: f32,
        start: &Isometry2<f32>,
        end: &Isometry2<f32>,
        easer: fn(f32, f32, f32, f32) -> f32,
    ) -> Self {
        let linear = (end.translation / start.translation).vector;
        let angular = start.rotation.angle_to(&end.rotation);
        Self {
            origin: *start,
            displacement: Velocity2::new(linear, angular),
            duration,
            easer,
        }
    }
}

impl ParametricMotionFunction for ParametricEased {
    fn set_endpoints(&mut self, start: &Isometry2<f32>, end: &Isometry2<f32>) {
        self.origin = *start;
        self.displacement = Velocity2::between_positions(start, end, 1.);
    }

    fn transform_by(&mut self, tx: &Isometry2<f32>) {
        self.origin = tx * self.origin;
        self.displacement = self.displacement.transformed(tx);
    }

    fn calculate(&self, t: f32) -> Isometry2<f32> {
        let eased = (self.easer)(t, 0., 1., self.duration);
        let mut interpolated = self.origin;
        let integrated = self.displacement.integrate(eased);
        interpolated.translation *= integrated.translation;
        interpolated.rotation *= integrated.rotation;
        interpolated
    }
}

// Assume that the vast majority of parametric motion functions will be
// under 256 bytes in size.
//
// 8 * u64 = 16 * u32 = 16 * f32 = four Isometry2<f32>s (x + y + re + im)
type ParametricMotionSpace = [u64; 8];

#[derive(SimpleComponent)]
pub struct ParametricMotion {
    time: f32,
    pub(crate) despawn_after_duration: bool,
    function: SmallBox<dyn PmfClone, ParametricMotionSpace>,
}

impl Clone for ParametricMotion {
    fn clone(&self) -> Self {
        Self {
            time: self.time,
            despawn_after_duration: self.despawn_after_duration,
            function: self.function.clone_smallboxed(),
        }
    }
}

impl ParametricMotion {
    pub fn new<F>(despawn_after_duration: bool, function: F) -> Self
    where
        F: ParametricMotionFunction + Clone + 'static,
    {
        Self {
            time: 0.,
            despawn_after_duration,
            function: SmallBox::new(function),
        }
    }

    // pub fn transformed(&self, tx: &Isometry2<f32>) -> Self {
    //     let mut this = self.clone();
    //     this.function.transform_by(tx);
    //     this
    // }

    // pub fn transform_by(&mut self, tx: &Isometry2<f32>) {
    //     self.function.transform_by(tx);
    // }

    pub fn lerp_expo_out(
        despawn_after_duration: bool,
        duration: f32,
        start: &Isometry2<f32>,
        end: &Isometry2<f32>,
    ) -> Self {
        Self::lerp_eased(despawn_after_duration, duration, start, end, Expo::ease_out)
    }

    pub fn lerp_eased(
        despawn_after_duration: bool,
        duration: f32,
        start: &Isometry2<f32>,
        end: &Isometry2<f32>,
        easer: fn(f32, f32, f32, f32) -> f32,
    ) -> Self {
        Self {
            time: 0.,
            despawn_after_duration,
            function: SmallBox::new(ParametricEased::new(duration, start, end, easer)),
        }
    }

    pub fn update(&mut self, dt: f32) -> Isometry2<f32> {
        let calculated = self.function.calculate(self.time);
        self.time += dt;
        calculated
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proximity {
    Intersecting,
    WithinMargin,
    Disjoint,
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub enum Collision {
    Circle { radius: f32 },
    Rectangle { radii: Vector2<f32> },
}

impl Collision {
    pub fn circle(radius: f32) -> Self {
        Self::Circle { radius }
    }

    pub fn rectangle(radii: Vector2<f32>) -> Self {
        Self::Rectangle { radii }
    }

    pub(crate) fn to_shape(&self) -> StackDst<dyn nc::shape::Shape<f32>> {
        match *self {
            Self::Circle { radius } => StackDst::new(nc::shape::Ball::new(radius)).unwrap(),
            Self::Rectangle { radii } => StackDst::new(nc::shape::Cuboid::new(radii)).unwrap(),
        }
    }

    pub fn proximity(
        m1: &Isometry2<f32>,
        c1: &Collision,
        m2: &Isometry2<f32>,
        c2: &Collision,
        margin: f32,
    ) -> Proximity {
        let s1 = c1.to_shape();
        let s2 = c2.to_shape();

        use nc::query::Proximity as NcProximity;
        match nc::query::proximity(m1, &*s1, m2, &*s2, margin) {
            NcProximity::Intersecting => Proximity::Intersecting,
            NcProximity::WithinMargin => Proximity::WithinMargin,
            NcProximity::Disjoint => Proximity::Disjoint,
        }
    }
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct MaximumVelocity {
    pub linear: f32,
    pub angular: f32,
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct DespawnOutOfBounds;

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct DespawnAfterTimeLimit {
    pub ttl: f32,
}
