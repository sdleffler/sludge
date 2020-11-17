#![feature(exact_size_is_empty)]

use ::{
    atomic_refcell::AtomicRefCell,
    easer::functions::*,
    hashbrown::HashMap,
    hibitset::{BitSet, DrainableBitSet},
    im::Vector,
    ncollide2d as nc,
    rand::{RngCore, SeedableRng},
    rand_xorshift::XorShiftRng,
    sludge::{
        api::{LuaComponent, LuaComponentInterface, Module},
        prelude::*,
    },
    sludge_2d::math::*,
    smallbox::SmallBox,
    stack_dst::Value as StackDst,
    std::{f32, marker::PhantomData, sync},
};

const RNG_REGISTRY_KEY: &'static str = "danmaku.rng";

#[derive(Clone)]
pub struct SharedRng<R: RngCore> {
    rng: sync::Arc<AtomicRefCell<R>>,
}

impl<R: RngCore> SharedRng<R> {
    pub fn new(rng: R) -> Self {
        Self {
            rng: sync::Arc::new(AtomicRefCell::new(rng)),
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

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct Projectile {
    pub position: Isometry2<f32>,
    pub velocity: Velocity2<f32>,
    pub acceleration: Velocity2<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct ProjectileAccessor(Entity);

impl LuaUserData for ProjectileAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("position", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let projectile = world.get::<Projectile>(this.0).to_lua_err()?;
            let v = projectile.position.translation.vector;
            Ok((v.x, v.y, projectile.position.rotation.angle()))
        });

        methods.add_method("velocity", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let projectile = world.get::<Projectile>(this.0).to_lua_err()?;
            let v = projectile.velocity.linear;
            Ok((v.x, v.y, projectile.velocity.angular))
        });

        methods.add_method("acceleration", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let projectile = world.get::<Projectile>(this.0).to_lua_err()?;
            let v = projectile.acceleration.linear;
            Ok((v.x, v.y, projectile.acceleration.angular))
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
pub struct QuadraticMotion;

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct DirectionalMotion;

pub trait ParametricMotionFunction: Send + Sync {
    fn set_endpoints(&mut self, start: &Isometry2<f32>, end: &Isometry2<f32>);
    fn transform_by(&mut self, tx: &Isometry2<f32>);
    fn calculate(&self, t: f32) -> Option<Isometry2<f32>>;
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

    fn calculate(&self, t: f32) -> Option<Isometry2<f32>> {
        if t < self.duration {
            let eased = (self.easer)(t, 0., 1., self.duration);
            let mut interpolated = self.origin;
            let integrated = self.displacement.integrate(eased);
            interpolated.translation *= integrated.translation;
            interpolated.rotation *= integrated.rotation;
            Some(interpolated)
        } else {
            None
        }
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
    despawn_after_duration: bool,
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

    pub fn update(&mut self, dt: f32) -> Option<Isometry2<f32>> {
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

#[derive(Debug, Clone, Copy, Bundle)]
pub struct QuadraticShot {
    pub projectile: Projectile,
    pub motion: QuadraticMotion,
}

impl QuadraticShot {
    pub fn linear(at: Isometry2<f32>, vel: Velocity2<f32>) -> Self {
        Self::new(at, vel, Velocity2::zero())
    }

    pub fn new(at: Isometry2<f32>, vel: Velocity2<f32>, acc: Velocity2<f32>) -> Self {
        QuadraticShot {
            projectile: Projectile {
                position: at,
                velocity: vel,
                acceleration: acc,
            },
            motion: QuadraticMotion,
        }
    }
}

#[derive(Debug, Clone, Copy, Bundle)]
pub struct DirectionalShot {
    pub projectile: Projectile,
    pub motion: DirectionalMotion,
}

#[derive(Debug, Clone, Copy)]
pub enum Shot {
    Quadratic(QuadraticShot),
    Directional(DirectionalShot),
}

impl Shot {
    pub fn linear(at: Isometry2<f32>, vel: Velocity2<f32>) -> Self {
        Self::quadratic(at, vel, Velocity2::zero())
    }

    pub fn quadratic(at: Isometry2<f32>, vel: Velocity2<f32>, acc: Velocity2<f32>) -> Self {
        Self::Quadratic(QuadraticShot {
            projectile: Projectile {
                position: at,
                velocity: vel,
                acceleration: acc,
            },
            motion: QuadraticMotion,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Parameters {
    /// Position should be used to position the fired bullets.
    pub position: Isometry2<f32>,

    /// Speed should be used to adjust the velocity of fired bullets which
    /// rely on linear/angular velocity to update themselves, for example
    /// bullets with the `QuadraticMotion` or `DirectionalMotion` components.
    pub speed: Velocity2<f32>,

    /// Acceleration should be used similarly to speed.
    pub accel: Velocity2<f32>,

    /// Destination is a parameter intended for working with bullets
    /// with parameterized movement, likely with the `ParametricMotion`
    /// component. It should be transformed according to `position` as
    /// the parameters are manipulated, allowing it to function similarly
    /// in usage to `aim_at`; if destination is set before transforms,
    /// then those transforms should correctly manipulate the destination.
    /// If it is set afterwards, they should not.
    pub destination: Isometry2<f32>,

    /// Duration is a parameter intended for working with bullets with
    /// parameterized movement or other movement which requires duration
    /// information. For something like a `ParametricMotion` component,
    /// duration will be interpreted as the total time of the parameterized
    /// motion, for example.
    ///
    /// Duration is in seconds.
    pub duration: f32,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            position: Isometry2::identity(),
            speed: Velocity2::zero(),
            accel: Velocity2::zero(),
            destination: Isometry2::identity(),
            duration: 0.,
        }
    }
}

impl Parameters {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn transformed(mut self, tx: &Isometry2<f32>) -> Self {
        self.position = self.position * tx;
        self.destination = self.destination * tx;
        self
    }

    #[inline]
    pub fn translated(self, v: &Vector2<f32>) -> Self {
        self.transformed(&Isometry2::from_parts(Translation2::from(*v), na::one()))
    }

    #[inline]
    pub fn rotated(mut self, rot: &UnitComplex<f32>) -> Self {
        self.position.append_rotation_mut(rot);
        self.destination.append_rotation_mut(rot);
        self
    }

    #[inline]
    pub fn rotated_wrt_center(mut self, rot: &UnitComplex<f32>) -> Self {
        self.position.append_rotation_wrt_center_mut(rot);
        self.destination
            .append_rotation_wrt_point_mut(rot, &Point2::from(self.position.translation.vector));
        self
    }

    #[inline]
    pub fn destination(mut self, destination: &Isometry2<f32>) -> Self {
        self.destination = *destination;
        self
    }

    #[inline]
    pub fn duration(mut self, duration: f32) -> Self {
        self.duration = duration;
        self
    }

    #[inline]
    pub fn apply_to_position(&self, iso: &Isometry2<f32>) -> Isometry2<f32> {
        self.position * iso
    }

    #[inline]
    pub fn apply_to_velocity(&self, dx: &Velocity2<f32>) -> Velocity2<f32> {
        (*dx + self.speed).transformed(&self.position)
    }

    #[inline]
    pub fn apply_to_acceleration(&self, dv: &Velocity2<f32>) -> Velocity2<f32> {
        (*dv + self.accel).transformed(&self.position)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Push(Option<Parameters>),
    Transform(Isometry2<f32>),
    Translate(Vector2<f32>),
    Rotate(UnitComplex<f32>),
    RotateVelocity(UnitComplex<f32>),
    AddVelocity(Velocity2<f32>),
    MulVelocity(f32),
    RotateAcceleration(UnitComplex<f32>),
    AddAcceleration(Velocity2<f32>),
    MulAcceleration(f32),
    AimAt(Point2<f32>),
    Destination(Isometry2<f32>),
    Duration(f32),
    Pop,
    Fire,
}

impl<'lua> ToLuaMulti<'lua> for Op {
    fn to_lua_multi(self, lua: LuaContext<'lua>) -> LuaResult<LuaMultiValue<'lua>> {
        match self {
            Op::Push(Some(ps)) => (
                "push",
                ps.position.translation.x,
                ps.position.translation.y,
                ps.position.rotation.re,
                ps.position.rotation.im,
                ps.speed.linear.x,
                ps.speed.linear.y,
                ps.speed.angular,
                ps.accel.linear.x,
                ps.accel.linear.y,
                ps.accel.angular,
                ps.destination.translation.x,
                ps.destination.translation.y,
                ps.destination.rotation.re,
                ps.destination.rotation.im,
                ps.duration,
            )
                .to_lua_multi(lua),
            Op::Push(None) => ("push",).to_lua_multi(lua),
            Op::Transform(iso) => (
                "transform",
                iso.translation.x,
                iso.translation.y,
                iso.rotation.re,
                iso.rotation.im,
            )
                .to_lua_multi(lua),
            Op::Translate(v) => ("translate", v.x, v.y).to_lua_multi(lua),
            Op::Rotate(r) => ("rotate", r.re, r.im).to_lua_multi(lua),
            Op::RotateVelocity(r) => ("rotate_velocity", r.re, r.im).to_lua_multi(lua),
            Op::AddVelocity(v) => {
                ("add_velocity", v.linear.x, v.linear.y, v.angular).to_lua_multi(lua)
            }
            Op::MulVelocity(m) => ("mul_velocity", m).to_lua_multi(lua),
            Op::RotateAcceleration(r) => ("rotate_acceleration", r.re, r.im).to_lua_multi(lua),
            Op::AddAcceleration(v) => {
                ("add_acceleration", v.linear.x, v.linear.y, v.angular).to_lua_multi(lua)
            }
            Op::MulAcceleration(m) => ("mul_acceleration", m).to_lua_multi(lua),
            Op::AimAt(pt) => ("aim_at", pt.x, pt.y).to_lua_multi(lua),
            Op::Destination(iso) => (
                "destination",
                iso.translation.x,
                iso.translation.y,
                iso.rotation.re,
                iso.rotation.im,
            )
                .to_lua_multi(lua),
            Op::Duration(t) => ("duration", t).to_lua_multi(lua),
            Op::Pop => ("pop",).to_lua_multi(lua),
            Op::Fire => ("fire",).to_lua_multi(lua),
        }
    }
}

impl<'lua> FromLuaMulti<'lua> for Op {
    fn from_lua_multi(values: LuaMultiValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let mut vec = values.into_iter();
        let op_name = LuaString::from_lua(vec.next().unwrap(), lua)?;

        match op_name.to_str()? {
            "push" => {
                if !vec.is_empty() {
                    let position = {
                        let x = f32::from_lua(vec.next().unwrap(), lua)?;
                        let y = f32::from_lua(vec.next().unwrap(), lua)?;
                        let re = f32::from_lua(vec.next().unwrap(), lua)?;
                        let im = f32::from_lua(vec.next().unwrap(), lua)?;
                        Isometry2::from_parts(
                            Translation2::new(x, y),
                            Unit::new_unchecked(Complex::new(re, im)),
                        )
                    };
                    let speed = {
                        let x = f32::from_lua(vec.next().unwrap(), lua)?;
                        let y = f32::from_lua(vec.next().unwrap(), lua)?;
                        let angular = f32::from_lua(vec.next().unwrap(), lua)?;
                        Velocity2 {
                            linear: Vector2::new(x, y),
                            angular,
                        }
                    };
                    let accel = {
                        let x = f32::from_lua(vec.next().unwrap(), lua)?;
                        let y = f32::from_lua(vec.next().unwrap(), lua)?;
                        let angular = f32::from_lua(vec.next().unwrap(), lua)?;
                        Velocity2 {
                            linear: Vector2::new(x, y),
                            angular,
                        }
                    };
                    let destination = {
                        let x = f32::from_lua(vec.next().unwrap(), lua)?;
                        let y = f32::from_lua(vec.next().unwrap(), lua)?;
                        let re = f32::from_lua(vec.next().unwrap(), lua)?;
                        let im = f32::from_lua(vec.next().unwrap(), lua)?;
                        Isometry2::from_parts(
                            Translation2::new(x, y),
                            Unit::new_unchecked(Complex::new(re, im)),
                        )
                    };
                    let duration = f32::from_lua(vec.next().unwrap(), lua)?;
                    Ok(Op::Push(Some(Parameters {
                        position,
                        speed,
                        accel,
                        destination,
                        duration,
                    })))
                } else {
                    Ok(Op::Push(None))
                }
            }
            "transform" => {
                let x = f32::from_lua(vec.next().unwrap(), lua)?;
                let y = f32::from_lua(vec.next().unwrap(), lua)?;
                let re = f32::from_lua(vec.next().unwrap(), lua)?;
                let im = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::Transform(Isometry2::from_parts(
                    Translation2::new(x, y),
                    Unit::new_unchecked(Complex::new(re, im)),
                )))
            }
            "translate" => {
                let x = f32::from_lua(vec.next().unwrap(), lua)?;
                let y = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::Translate(Vector2::new(x, y)))
            }
            "rotate" => {
                let re = f32::from_lua(vec.next().unwrap(), lua)?;
                let im = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::Rotate(Unit::new_unchecked(Complex::new(re, im))))
            }
            "rotate_velocity" => {
                let re = f32::from_lua(vec.next().unwrap(), lua)?;
                let im = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::RotateVelocity(Unit::new_unchecked(Complex::new(
                    re, im,
                ))))
            }
            "add_velocity" => {
                let x = f32::from_lua(vec.next().unwrap(), lua)?;
                let y = f32::from_lua(vec.next().unwrap(), lua)?;
                let angular = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::AddVelocity(Velocity2::new(Vector2::new(x, y), angular)))
            }
            "mul_velocity" => {
                let m = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::MulVelocity(m))
            }
            "rotate_acceleration" => {
                let re = f32::from_lua(vec.next().unwrap(), lua)?;
                let im = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::RotateAcceleration(Unit::new_unchecked(Complex::new(
                    re, im,
                ))))
            }
            "add_acceleration" => {
                let x = f32::from_lua(vec.next().unwrap(), lua)?;
                let y = f32::from_lua(vec.next().unwrap(), lua)?;
                let angular = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::AddAcceleration(Velocity2::new(
                    Vector2::new(x, y),
                    angular,
                )))
            }
            "mul_acceleration" => {
                let m = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::MulAcceleration(m))
            }
            "aim_at" => {
                let x = f32::from_lua(vec.next().unwrap(), lua)?;
                let y = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::AimAt(Point2::new(x, y)))
            }
            "destination" => {
                let destination = {
                    let x = f32::from_lua(vec.next().unwrap(), lua)?;
                    let y = f32::from_lua(vec.next().unwrap(), lua)?;
                    let rot = if !vec.is_empty() {
                        let re = f32::from_lua(vec.next().unwrap(), lua)?;
                        let im = f32::from_lua(vec.next().unwrap(), lua)?;
                        UnitComplex::new_unchecked(Complex::new(re, im))
                    } else {
                        UnitComplex::identity()
                    };

                    Isometry2::from_parts(Translation2::new(x, y), rot)
                };
                Ok(Op::Destination(destination))
            }
            "duration" => {
                let duration = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::Duration(duration))
            }
            "pop" => Ok(Op::Pop),
            "fire" => Ok(Op::Fire),
            bad_op => return Err(anyhow!("invalid op `{}`", bad_op)).to_lua_err(),
        }
    }
}

pub trait PatternBuilder<'lua> {
    #[inline]
    fn push(&mut self, ps: Option<Parameters>) -> Result<()> {
        self.op(Op::Push(ps))
    }

    #[inline]
    fn transform(&mut self, tx: Isometry2<f32>) -> Result<()> {
        self.op(Op::Transform(tx))
    }

    #[inline]
    fn translate(&mut self, v: Vector2<f32>) -> Result<()> {
        self.op(Op::Translate(v))
    }

    #[inline]
    fn rotate(&mut self, angle: f32) -> Result<()> {
        self.op(Op::Rotate(UnitComplex::new(angle)))
    }

    #[inline]
    fn rotate_velocity(&mut self, angle: f32) -> Result<()> {
        self.op(Op::RotateVelocity(UnitComplex::new(angle)))
    }

    #[inline]
    fn add_linear_velocity(&mut self, v: Vector2<f32>) -> Result<()> {
        self.add_velocity(Velocity2::new(v, 0.))
    }

    #[inline]
    fn add_angular_velocity(&mut self, theta: f32) -> Result<()> {
        self.add_velocity(Velocity2::angular(theta))
    }

    #[inline]
    fn add_velocity(&mut self, velocity: Velocity2<f32>) -> Result<()> {
        self.op(Op::AddVelocity(velocity))
    }

    #[inline]
    fn mul_velocity(&mut self, m: f32) -> Result<()> {
        self.op(Op::MulVelocity(m))
    }

    #[inline]
    fn rotate_acceleration(&mut self, angle: f32) -> Result<()> {
        self.op(Op::RotateAcceleration(UnitComplex::new(angle)))
    }

    #[inline]
    fn add_linear_acceleration(&mut self, v: Vector2<f32>) -> Result<()> {
        self.add_acceleration(Velocity2::new(v, 0.))
    }

    #[inline]
    fn add_angular_acceleration(&mut self, theta: f32) -> Result<()> {
        self.add_acceleration(Velocity2::angular(theta))
    }

    #[inline]
    fn add_acceleration(&mut self, acceleration: Velocity2<f32>) -> Result<()> {
        self.op(Op::AddAcceleration(acceleration))
    }

    #[inline]
    fn mul_accel(&mut self, m: f32) -> Result<()> {
        self.op(Op::MulAcceleration(m))
    }

    #[inline]
    fn aim_at(&mut self, pt: Point2<f32>) -> Result<()> {
        self.op(Op::AimAt(pt))
    }

    #[inline]
    fn destination(&mut self, dest: Isometry2<f32>) -> Result<()> {
        self.op(Op::Destination(dest))
    }

    #[inline]
    fn duration(&mut self, duration: f32) -> Result<()> {
        self.op(Op::Duration(duration))
    }

    #[inline]
    fn pop(&mut self) -> Result<()> {
        self.op(Op::Pop)
    }

    #[inline]
    fn fire(&mut self) -> Result<()> {
        self.op(Op::Fire)
    }

    fn op(&mut self, op: Op) -> Result<()>;
    fn lua(&self) -> LuaContext<'lua>;
    fn rng(&mut self) -> &mut dyn RngCore;

    #[inline]
    fn compose_with_pattern<P: Pattern>(self, pattern: P) -> Composed<'lua, P, Self>
    where
        Self: Sized,
    {
        Composed {
            pattern,
            builder: self,
            _marker: PhantomData,
        }
    }
}

impl<'lua, B: PatternBuilder<'lua> + ?Sized> PatternBuilder<'lua> for &'_ mut B {
    #[inline]
    fn op(&mut self, op: Op) -> Result<()> {
        (**self).op(op)
    }

    #[inline]
    fn lua(&self) -> LuaContext<'lua> {
        (**self).lua()
    }

    #[inline]
    fn rng(&mut self) -> &mut dyn RngCore {
        (**self).rng()
    }
}

impl<'lua, B: PatternBuilder<'lua> + ?Sized> PatternBuilder<'lua> for Box<B> {
    #[inline]
    fn op(&mut self, op: Op) -> Result<()> {
        (**self).op(op)
    }

    #[inline]
    fn lua(&self) -> LuaContext<'lua> {
        (**self).lua()
    }

    #[inline]
    fn rng(&mut self) -> &mut dyn RngCore {
        (**self).rng()
    }
}

pub trait Pattern: Send + Sync {
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()>;

    #[inline]
    fn of<Q>(self, subpattern: Q) -> Of<Self, Q>
    where
        Self: Sized,
        Q: Pattern,
    {
        Of {
            pattern: self,
            subpattern,
        }
    }
}

impl<P: Pattern + ?Sized> Pattern for &'_ P {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        (**self).build(builder)
    }
}

impl<P: Pattern + ?Sized> Pattern for Box<P> {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        (**self).build(builder)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Composed<'lua, P: Pattern, B: PatternBuilder<'lua>> {
    pattern: P,
    builder: B,
    _marker: PhantomData<&'lua ()>,
}

impl<'lua, P: Pattern, B: PatternBuilder<'lua>> PatternBuilder<'lua> for Composed<'lua, P, B> {
    #[inline]
    fn op(&mut self, op: Op) -> Result<()> {
        match op {
            Op::Fire => self.pattern.build(&mut self.builder),
            other => self.builder.op(other),
        }
    }

    #[inline]
    fn lua(&self) -> LuaContext<'lua> {
        self.builder.lua()
    }

    #[inline]
    fn rng(&mut self) -> &mut dyn RngCore {
        self.builder.rng()
    }
}

pub struct Of<P: Pattern, Q: Pattern> {
    pattern: P,
    subpattern: Q,
}

impl<P, Q> Pattern for Of<P, Q>
where
    P: Pattern,
    Q: Pattern,
{
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()>
    where
        Self: Sized,
    {
        self.pattern
            .build(&mut builder.compose_with_pattern(&self.subpattern))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LuaPatternBuilderUserData;

impl LuaUserData for LuaPatternBuilderUserData {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_function(
            "op",
            |_lua, (this, args): (LuaAnyUserData, LuaMultiValue)| {
                this.get_user_value::<LuaFunction>()?.call::<_, ()>(args)
            },
        );

        methods.add_function("push", |_lua, this: LuaAnyUserData| {
            this.get_user_value::<LuaFunction>()?.call::<_, ()>("push")
        });

        methods.add_function(
            "translate",
            |_lua, (this, x, y): (LuaAnyUserData, f32, f32)| {
                this.get_user_value::<LuaFunction>()?
                    .call::<_, ()>(("translate", x, y))
            },
        );

        methods.add_function("rotate", |_lua, (this, angle): (LuaAnyUserData, f32)| {
            let rot = UnitComplex::new(angle);
            this.get_user_value::<LuaFunction>()?
                .call::<_, ()>(("rotate", rot.re, rot.im))
        });

        methods.add_function(
            "add_linear_velocity",
            |_lua, (this, x, y): (LuaAnyUserData, f32, f32)| {
                this.get_user_value::<LuaFunction>()?
                    .call::<_, ()>(("add_velocity", x, y, 0.))
            },
        );

        methods.add_function(
            "add_linear_acceleration",
            |_lua, (this, x, y): (LuaAnyUserData, f32, f32)| {
                this.get_user_value::<LuaFunction>()?
                    .call::<_, ()>(("add_acceleration", x, y, 0.))
            },
        );

        methods.add_function(
            "aim_at",
            |_lua, (this, x, y): (LuaAnyUserData, f32, f32)| {
                this.get_user_value::<LuaFunction>()?
                    .call::<_, ()>(("aim_at", x, y))
            },
        );

        methods.add_function(
            "destination",
            |_lua, (this, x, y, angle): (LuaAnyUserData, f32, f32, Option<f32>)| {
                let rot = angle
                    .map(UnitComplex::new)
                    .unwrap_or(UnitComplex::identity());
                this.get_user_value::<LuaFunction>()?.call::<_, ()>((
                    "destination",
                    x,
                    y,
                    rot.re,
                    rot.im,
                ))
            },
        );

        methods.add_function("duration", |_lua, (this, t): (LuaAnyUserData, f32)| {
            this.get_user_value::<LuaFunction>()?
                .call::<_, ()>(("duration", t))
        });

        methods.add_function("pop", |_lua, this: LuaAnyUserData| {
            this.get_user_value::<LuaFunction>()?.call::<_, ()>("pop")
        });

        methods.add_function("fire", |_lua, this: LuaAnyUserData| {
            this.get_user_value::<LuaFunction>()?.call::<_, ()>("fire")
        });

        methods.add_meta_method(
            LuaMetaMethod::Index,
            |_lua, _this, key: LuaString| -> LuaResult<()> {
                Err(anyhow!(
                    "no such method `{}` for PatternBuilder",
                    key.to_str()?
                ))
                .to_lua_err()
            },
        );
    }
}

#[derive(Clone)]
pub struct LuaPatternBuilder<'lua> {
    lua: LuaContext<'lua>,
    closure: LuaFunction<'lua>,
    rng: SharedRng<XorShiftRng>,
}

impl<'lua> ToLua<'lua> for LuaPatternBuilder<'lua> {
    #[inline]
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        let ud = lua.create_userdata(LuaPatternBuilderUserData)?;
        ud.set_user_value(self.closure)?;
        ud.to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for LuaPatternBuilder<'lua> {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let ud = LuaAnyUserData::from_lua(lua_value, lua)?;
        let closure = ud.get_user_value()?;
        LuaPatternBuilder::new(lua, closure)
    }
}

impl<'lua> PatternBuilder<'lua> for LuaPatternBuilder<'lua> {
    fn op(&mut self, op: Op) -> Result<()> {
        Ok(self.closure.call(op)?)
    }

    fn lua(&self) -> LuaContext<'lua> {
        self.lua
    }

    fn rng(&mut self) -> &mut dyn RngCore {
        &mut self.rng
    }
}

impl<'lua> LuaPatternBuilder<'lua> {
    #[inline]
    pub fn new(lua: LuaContext<'lua>, closure: LuaFunction<'lua>) -> LuaResult<Self> {
        let rng = lua.named_registry_value(RNG_REGISTRY_KEY)?;
        Ok(Self { lua, closure, rng })
    }
}

#[derive(Debug)]
pub struct LuaPattern {
    key: LuaRegistryKey,
}

impl Pattern for LuaPattern {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()>
    where
        Self: Sized,
    {
        let lua = builder.lua();
        lua.scope(|scope| {
            let portal = scope.create_function_mut(|_lua, op: Op| {
                builder.op(op).to_lua_err()?;
                Ok(())
            })?;
            let closure = lua.registry_value::<LuaFunction>(&self.key)?;
            closure.call(LuaPatternBuilder::new(lua, portal)?)?;
            Ok(())
        })
    }
}

impl<'lua> ToLua<'lua> for LuaPattern {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        let f = lua.registry_value::<LuaFunction>(&self.key)?;
        lua.remove_registry_value(self.key)?;
        f.to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for LuaPattern {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let f = LuaFunction::from_lua(lua_value, lua)?;
        let key = lua.create_registry_value(f)?;
        Ok(Self { key })
    }
}

#[derive(Clone)]
pub struct RustPattern(sync::Arc<dyn Pattern>);

impl RustPattern {
    pub fn new<P: Pattern + 'static>(pattern: P) -> Self {
        Self(sync::Arc::new(pattern))
    }
}

impl Pattern for RustPattern {
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        self.0.build(builder)
    }
}

impl LuaUserData for RustPattern {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T)
    where
        Self: Sized + LuaUserData,
    {
        methods.add_method("build", |_lua, this, mut builder: LuaPatternBuilder| {
            this.0.build(&mut builder).to_lua_err()?;
            Ok(())
        });

        methods.add_function(
            "of",
            |_lua, (pattern, subpattern): (RustPattern, RustPattern)| {
                Ok(RustPattern::new(pattern.of(subpattern)))
            },
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Ring {
    pub radius: f32,
    pub count: u32,
}

impl Ring {
    pub fn new(radius: f32, count: u32) -> Self {
        Self { radius, count }
    }
}

impl Pattern for Ring {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        builder.push(None)?;
        builder.translate(Vector2::x() * self.radius)?;
        builder.fire()?;
        let step = f32::consts::TAU / (self.count as f32);
        for _ in 1..self.count {
            builder.translate(-Vector2::x() * self.radius)?;
            builder.rotate(step)?;
            builder.translate(Vector2::x() * self.radius)?;
            builder.fire()?;
        }
        builder.pop()?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Arc {
    pub radius: f32,
    pub angle: f32,
    pub count: u32,
}

impl Arc {
    pub fn new(radius: f32, angle: f32, count: u32) -> Self {
        Self {
            radius,
            angle,
            count,
        }
    }
}

impl Pattern for Arc {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        builder.push(None)?;
        let half_angle = self.angle / 2.;
        let step = self.angle / (self.count as f32 - 1.);
        builder.rotate(-half_angle)?;
        builder.translate(Vector2::x() * self.radius)?;
        builder.fire()?;
        for _ in 1..self.count {
            builder.translate(-Vector2::x() * self.radius)?;
            builder.rotate(step)?;
            builder.translate(Vector2::x() * self.radius)?;
            builder.fire()?;
        }
        builder.pop()?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Stack {
    pub delta: Velocity2<f32>,
    pub count: u32,
}

impl Stack {
    pub fn new(delta: Velocity2<f32>, count: u32) -> Self {
        Self { delta, count }
    }
}

impl Pattern for Stack {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        builder.push(None)?;
        builder.fire()?;
        for _ in 1..self.count {
            builder.add_linear_velocity(self.delta.linear)?;
            builder.add_angular_velocity(self.delta.angular)?;
            builder.fire()?;
        }
        builder.pop()?;

        Ok(())
    }
}

pub struct Aimed {
    pub target: Point2<f32>,
}

impl Pattern for Aimed {
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        builder.push(None)?;
        builder.aim_at(self.target)?;
        builder.fire()?;
        builder.pop()?;

        Ok(())
    }
}

pub struct Destination {
    pub destination: Isometry2<f32>,
    pub duration: f32,
}

impl Pattern for Destination {
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        builder.push(None)?;
        builder.destination(self.destination)?;
        builder.duration(self.duration)?;
        builder.fire()?;
        builder.pop()?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct Recorder<'lua> {
    ops: Vec<Op>,
    fire_count: u32,
    lua: LuaContext<'lua>,
    rng: SharedRng<XorShiftRng>,
}

impl<'lua> PatternBuilder<'lua> for Recorder<'lua> {
    fn op(&mut self, op: Op) -> Result<()> {
        if matches!(op, Op::Fire) {
            self.fire_count += 1;
        }

        self.ops.push(op);
        Ok(())
    }

    fn lua(&self) -> LuaContext<'lua> {
        self.lua
    }

    fn rng(&mut self) -> &mut dyn RngCore {
        &mut self.rng
    }
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct DanmakuId(Entity);

pub struct Danmaku {
    bounds: Option<Box2<f32>>,
    to_despawn: BitSet,
}

impl Danmaku {
    pub fn new() -> Self {
        Self {
            bounds: None,
            to_despawn: BitSet::new(),
        }
    }

    pub fn with_bounds(bounds: Box2<f32>) -> Self {
        Self {
            bounds: Some(bounds),
            ..Self::new()
        }
    }

    pub fn update(&mut self, world: &mut World, dt: f32) {
        for (_e, (mut proj, maximum)) in world
            .query::<(&mut Projectile, Option<&MaximumVelocity>)>()
            .with::<QuadraticMotion>()
            .iter()
        {
            let proj = &mut *proj;
            proj.velocity += proj.acceleration * dt;

            if let Some(max_vel) = maximum {
                let cur_vel = proj.velocity.linear.norm();
                if cur_vel > max_vel.linear {
                    proj.velocity.linear *= max_vel.linear / cur_vel;
                }

                let cur_ang = proj.velocity.angular.abs();
                if cur_ang > max_vel.angular {
                    proj.velocity.angular *= max_vel.angular / cur_ang;
                }
            }

            let integrated = proj.velocity.integrate(dt);
            proj.position.translation.vector += integrated.translation.vector;
            proj.position.rotation *= integrated.rotation;
        }

        for (_e, (mut proj, maximum)) in world
            .query::<(&mut Projectile, Option<&MaximumVelocity>)>()
            .with::<DirectionalMotion>()
            .iter()
        {
            let proj = &mut *proj;
            proj.velocity += proj.acceleration * dt;

            if let Some(max_vel) = maximum {
                let cur_vel = proj.velocity.linear.norm();
                if cur_vel > max_vel.linear {
                    proj.velocity.linear *= max_vel.linear / cur_vel;
                }

                let cur_ang = proj.velocity.angular.abs();
                if cur_ang > max_vel.angular {
                    proj.velocity.angular *= max_vel.angular / cur_ang;
                }
            }

            proj.position *= proj.velocity.integrate(dt);
        }

        for (e, (mut proj, mut motion)) in world
            .query::<(&mut Projectile, &mut ParametricMotion)>()
            .iter()
        {
            let (proj, motion) = (&mut *proj, &mut *motion);
            if let Some(iso) = motion.update(dt) {
                proj.position = iso;
            } else if motion.despawn_after_duration {
                self.to_despawn.add(e.id());
            }
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

pub trait Bullet: Send + Sync {
    type Bundled: Bundle;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled;
}

impl Bullet for QuadraticShot {
    type Bundled = Self;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
        let position = parameters.apply_to_position(&self.projectile.position);
        let velocity = parameters.apply_to_velocity(&self.projectile.velocity);
        let acceleration = parameters.apply_to_acceleration(&self.projectile.acceleration);

        Self {
            projectile: Projectile {
                position,
                velocity,
                acceleration,
            },
            motion: QuadraticMotion,
        }
    }
}

impl Bullet for DirectionalShot {
    type Bundled = Self;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
        let position = parameters.apply_to_position(&self.projectile.position);
        let velocity = parameters.apply_to_velocity(&self.projectile.velocity);
        let acceleration = parameters.apply_to_acceleration(&self.projectile.acceleration);

        Self {
            projectile: Projectile {
                position,
                velocity,
                acceleration,
            },
            motion: DirectionalMotion,
        }
    }
}

#[derive(Clone)]
pub struct Batch<'lua, B>
where
    B: Bullet,
{
    bullet: B,
    batched: Vec<B::Bundled>,
    stack: Vec<Parameters>,
    lua: LuaContext<'lua>,
    rng: SharedRng<XorShiftRng>,
}

impl<'lua, B> Batch<'lua, B>
where
    B: Bullet,
{
    pub fn new(lua: LuaContext<'lua>, bullet: B) -> Result<Self> {
        let rng = match lua
            .named_registry_value::<_, Option<SharedRng<XorShiftRng>>>(RNG_REGISTRY_KEY)?
        {
            Some(rng) => rng,
            None => {
                let rng = SharedRng::new(XorShiftRng::from_rng(rand::thread_rng())?);
                lua.set_named_registry_value(RNG_REGISTRY_KEY, rng.clone())?;
                rng
            }
        };
        Ok(Self {
            bullet,
            batched: Vec::new(),
            stack: vec![Parameters::default()],
            lua,
            rng,
        })
    }

    pub fn to_vec(self) -> Vec<B::Bundled> {
        self.batched
    }
}

impl<'lua, B> PatternBuilder<'lua> for Batch<'lua, B>
where
    B: Bullet,
{
    #[inline]
    fn op(&mut self, op: Op) -> Result<()> {
        match op {
            Op::Push(Some(ps)) => {
                self.stack.push(ps);
            }
            Op::Push(None) => {
                let top = *self.stack.last().unwrap();
                self.stack.push(top);
            }
            Op::Transform(tx) => {
                let top = self.stack.last_mut().unwrap();
                *top = top.transformed(&tx);
            }
            Op::Translate(v) => {
                let top = self.stack.last_mut().unwrap();
                *top = top.translated(&v);
            }
            Op::Rotate(r) => {
                let top = self.stack.last_mut().unwrap();
                *top = top.rotated_wrt_center(&r);
            }
            Op::RotateVelocity(r) => {
                let top = self.stack.last_mut().unwrap();
                top.speed = top.speed.rotated(&r.to_rotation_matrix());
            }
            Op::AddVelocity(v) => {
                let top = self.stack.last_mut().unwrap();
                top.speed += v;
            }
            Op::MulVelocity(m) => {
                let top = self.stack.last_mut().unwrap();
                top.speed *= m;
            }
            Op::RotateAcceleration(r) => {
                let top = self.stack.last_mut().unwrap();
                top.accel = top.accel.rotated(&r.to_rotation_matrix());
            }
            Op::AddAcceleration(v) => {
                let top = self.stack.last_mut().unwrap();
                top.accel += v;
            }
            Op::MulAcceleration(m) => {
                let top = self.stack.last_mut().unwrap();
                top.accel *= m;
            }
            Op::AimAt(p0) => {
                let ps = self.stack.last_mut().unwrap();
                let p1 = Point2::from(ps.position.translation.vector);
                let v = p0 - p1;
                let u = ps.position.transform_vector(&Vector2::x());
                let rot = UnitComplex::scaled_rotation_between(&u, &v, 1.);
                *ps = ps.rotated_wrt_center(&rot);
            }
            Op::Destination(iso) => {
                let top = self.stack.last_mut().unwrap();
                top.destination = iso;
            }
            Op::Duration(t) => {
                let top = self.stack.last_mut().unwrap();
                top.duration = t;
            }
            Op::Pop => {
                self.stack.pop().unwrap();
            }
            Op::Fire => {
                self.batched
                    .push(self.bullet.to_bundled(self.stack.last().unwrap()));
            }
        }

        Ok(())
    }

    #[inline]
    fn lua(&self) -> LuaContext<'lua> {
        self.lua
    }

    #[inline]
    fn rng(&mut self) -> &mut dyn RngCore {
        &mut self.rng
    }
}

#[derive(Clone)]
pub struct BulletType {
    name: &'static str,
    bullet: sync::Arc<dyn ErasedBullet>,
}

trait ErasedBullet: Send + Sync {
    fn batch_me<'lua>(
        &self,
        lua: LuaContext<'lua>,
        closure: LuaFunction<'lua>,
    ) -> LuaResult<Vec<Entity>>;
}

struct BulletSlug<B: Bullet + Clone> {
    bullet: B,
}

impl<B: Bullet + Clone> ErasedBullet for BulletSlug<B> {
    fn batch_me<'lua>(
        &self,
        lua: LuaContext<'lua>,
        closure: LuaFunction<'lua>,
    ) -> LuaResult<Vec<Entity>> {
        let mut batch = Batch::new(lua, self.bullet.clone()).to_lua_err()?;
        lua.scope(|scope| -> LuaResult<()> {
            let emit_closure =
                scope.create_function_mut(|_lua, op: Op| batch.op(op).to_lua_err())?;
            let lua_builder = LuaPatternBuilder::new(lua, emit_closure)?;
            LuaFunction::call(&closure, lua_builder)?;
            Ok(())
        })?;

        let resources = lua.resources();
        let world = &mut *resources.fetch_mut::<World>();
        Ok(world.spawn_batch(batch.to_vec()))
    }
}

impl BulletType {
    pub fn new<B: Bullet + Clone + 'static>(name: &'static str, bullet: B) -> Self {
        Self {
            name,
            bullet: sync::Arc::new(BulletSlug { bullet }),
        }
    }
}

inventory::collect!(BulletType);

#[derive(Clone)]
pub struct LuaBullet {
    erased: sync::Arc<dyn ErasedBullet>,
}

impl LuaUserData for LuaBullet {}

#[derive(Debug, Clone)]
pub struct LuaGroup {
    entities: Vector<Entity>,
}

impl LuaUserData for LuaGroup {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method_mut("cancel", |lua, this, ()| {
            let resources = lua.resources();
            let world = resources.fetch::<World>();
            let mut buf = world.get_buffer();

            for &e in &this.entities {
                buf.despawn(e);
            }

            world.queue_buffer(buf);
            this.entities.clear();

            Ok(())
        });

        methods.add_method("to_pattern", |_lua, this, ()| {
            Ok(RustPattern::new(this.clone()))
        });
    }
}

impl Pattern for LuaGroup {
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        let resources = builder.lua().resources();
        let world = resources.fetch::<World>();

        for &entity in &self.entities {
            let proj = match world.get::<Projectile>(entity) {
                Ok(p) => p,
                Err(_) => continue,
            };

            builder.push(None)?;
            builder.transform(proj.position)?;
            builder.fire()?;
            builder.pop()?;
        }

        Ok(())
    }
}

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
        let mut world = resources.fetch_mut::<World>();
        let mut danmaku = resources.fetch_mut::<Danmaku>();

        danmaku.update(&mut *world, 1. / 60.);

        Ok(())
    }
}

pub fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    let bullets = inventory::iter::<BulletType>
        .into_iter()
        .map(|bullet| {
            let name = bullet.name;
            let erased = bullet.bullet.clone();
            (name, erased)
        })
        .collect::<HashMap<_, _>>();

    let table = lua.create_table()?;
    let key = lua.create_registry_value(table.clone())?;
    table.set(
        "group",
        lua.create_function(|_, ()| {
            Ok(LuaGroup {
                entities: Vector::new(),
            })
        })?,
    )?;
    table.set(
        "pattern",
        lua.create_function(|_, pattern: LuaPattern| Ok(RustPattern::new(pattern)))?,
    )?;
    table.set(
        "aimed",
        lua.create_function(|_, (x, y)| {
            Ok(RustPattern::new(Aimed {
                target: Point2::new(x, y),
            }))
        })?,
    )?;
    table.set(
        "destination",
        lua.create_function(|_, (duration, x, y, angle): (f32, f32, f32, Option<f32>)| {
            let destination = match angle {
                Some(angle) => Isometry2::new(Vector2::new(x, y), angle),
                None => Isometry2::translation(x, y),
            };
            Ok(RustPattern::new(Destination {
                destination,
                duration,
            }))
        })?,
    )?;
    table.set(
        "ring",
        lua.create_function(|_, (radius, count)| -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Ring { radius, count }))
        })?,
    )?;
    table.set(
        "arc",
        lua.create_function(|_, (radius, angle, count)| -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Arc {
                radius,
                angle,
                count,
            }))
        })?,
    )?;
    table.set(
        "stack",
        lua.create_function(|_, (x, y, angular, count)| -> LuaResult<RustPattern> {
            Ok(RustPattern::new(Stack {
                delta: Velocity2::new(Vector2::new(x, y), angular),
                count,
            }))
        })?,
    )?;
    table.set(
        "spawn",
        lua.create_function(
            move |lua,
                  (bullet_ty, closure, maybe_lua_group): (
                LuaValue,
                LuaFunction,
                Option<LuaAnyUserData>,
            )| {
                let mut maybe_group = maybe_lua_group
                    .as_ref()
                    .map(LuaAnyUserData::borrow_mut::<LuaGroup>)
                    .transpose()?;

                let entities = match &bullet_ty {
                    LuaValue::String(ty_string) => {
                        bullets[ty_string.to_str()?].batch_me(lua, closure)?
                    }
                    LuaValue::UserData(ty_ud) => {
                        ty_ud.borrow::<LuaBullet>()?.erased.batch_me(lua, closure)?
                    }
                    _ => {
                        return Err(LuaError::FromLuaConversionError {
                            from: "lua value",
                            to: "string or userdata",
                            message: None,
                        });
                    }
                };

                let table = lua.registry_value::<LuaTable>(&key)?;
                if let Some(on_spawn) = table.get::<_, Option<LuaFunction>>("on_spawn")? {
                    on_spawn.call::<_, ()>(bullet_ty)?;
                }

                if let Some(group) = maybe_group.as_deref_mut() {
                    group.entities.extend(entities);
                }

                Ok(())
            },
        )?,
    )?;
    table.set(
        "set_bounds",
        lua.create_function(|lua, bounds: Option<Box2<f32>>| {
            let resources = lua.resources();
            let mut danmaku = resources.fetch_mut::<Danmaku>();
            danmaku.bounds = bounds;
            Ok(())
        })?,
    )?;

    Ok(LuaValue::Table(table))
}

inventory::submit! {
    Module::parse("danmaku", load)
}
