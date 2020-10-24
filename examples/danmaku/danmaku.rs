use {
    atomic_refcell::AtomicRefCell,
    hashbrown::HashMap,
    hibitset::{BitSet, DrainableBitSet},
    rand::{RngCore, SeedableRng},
    rand_xorshift::XorShiftRng,
    sludge::{anyhow::*, api::Module, inventory, prelude::*},
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
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct QuadraticMotion {
    pub velocity: Velocity2<f32>,
    pub acceleration: Velocity2<f32>,
}

impl QuadraticMotion {
    pub fn linear(vel: Velocity2<f32>) -> Self {
        Self::new(vel, Velocity2::zero())
    }

    pub fn new(vel: Velocity2<f32>, acc: Velocity2<f32>) -> Self {
        QuadraticMotion {
            velocity: vel,
            acceleration: acc,
        }
    }
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct DirectionalMotion {
    pub velocity: Velocity2<f32>,
    pub acceleration: Velocity2<f32>,
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct Circle {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct Rectangle {
    pub half_extents: Vector2<f32>,
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct MaximumVelocity {
    pub linear: f32,
    pub angular: f32,
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
            projectile: Projectile { position: at },
            motion: QuadraticMotion {
                velocity: vel,
                acceleration: acc,
            },
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
            projectile: Projectile { position: at },
            motion: QuadraticMotion {
                velocity: vel,
                acceleration: acc,
            },
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SpeedParameters {
    pub coeff: f32,
    pub linear_bias: f32,
    pub angular_bias: f32,
}

impl Default for SpeedParameters {
    fn default() -> Self {
        Self {
            coeff: 1.,
            linear_bias: 0.,
            angular_bias: 0.,
        }
    }
}

impl SpeedParameters {
    pub fn new(coeff: f32, linear_bias: f32, angular_bias: f32) -> Self {
        Self {
            coeff,
            linear_bias,
            angular_bias,
        }
    }

    pub fn linear_bias(linear_bias: f32) -> Self {
        Self {
            linear_bias,
            ..Self::default()
        }
    }

    pub fn angular_bias(angular_bias: f32) -> Self {
        Self {
            angular_bias,
            ..Self::default()
        }
    }

    pub fn coeff(coeff: f32) -> Self {
        Self {
            coeff,
            ..Self::default()
        }
    }

    pub fn apply_linear(&self, speed: f32) -> f32 {
        self.coeff * speed + self.linear_bias
    }

    pub fn apply_angular(&self, speed: f32) -> f32 {
        self.coeff * speed + self.angular_bias
    }

    // Application:
    //    coeff1 * (coeff2 * x + bias2) + bias1
    // => coeff1 * coeff2 * x + (coeff1 * bias2 + bias1)
    pub fn after(&self, of: &SpeedParameters) -> SpeedParameters {
        SpeedParameters {
            coeff: self.coeff * of.coeff,
            linear_bias: self.coeff * of.linear_bias + self.linear_bias,
            angular_bias: self.coeff * of.angular_bias + self.angular_bias,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Parameters {
    pub position: Isometry2<f32>,
    pub speed: SpeedParameters,
    pub accel: SpeedParameters,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            position: Isometry2::identity(),
            speed: SpeedParameters::default(),
            accel: SpeedParameters::default(),
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
        self
    }

    #[inline]
    pub fn translated(self, v: &Vector2<f32>) -> Self {
        self.transformed(&Isometry2::from_parts(Translation2::from(*v), na::one()))
    }

    #[inline]
    pub fn rotated(mut self, rot: &UnitComplex<f32>) -> Self {
        self.position.append_rotation_mut(rot);
        self
    }

    #[inline]
    pub fn rotated_wrt_center(mut self, rot: &UnitComplex<f32>) -> Self {
        self.position.append_rotation_wrt_center_mut(rot);
        self
    }

    #[inline]
    pub fn composed(mut self, sp: &SpeedParameters) -> Self {
        self.speed = self.speed.after(sp);
        self
    }

    #[inline]
    pub fn apply_to_position(&self, iso: &Isometry2<f32>) -> Isometry2<f32> {
        self.position * iso
    }

    #[inline]
    pub fn apply_to_derivative(&self, dx: &Velocity2<f32>) -> Velocity2<f32> {
        let mut dx = dx.transformed(&self.position);

        let speed = dx.linear.norm();
        if speed != 0. {
            let adjusted_speed = self.speed.apply_linear(speed);
            dx.linear *= adjusted_speed / speed;
        }
        dx.angular = self.speed.apply_angular(dx.angular);

        dx
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Push(Option<Parameters>),
    Transform(Isometry2<f32>),
    Translate(Vector2<f32>),
    Rotate(UnitComplex<f32>),
    Compose(SpeedParameters),
    AimAt(Point2<f32>),
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
                ps.speed.coeff,
                ps.speed.linear_bias,
                ps.speed.angular_bias,
                ps.accel.coeff,
                ps.accel.linear_bias,
                ps.accel.angular_bias,
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
            Op::Compose(sp) => {
                ("compose", sp.coeff, sp.linear_bias, sp.angular_bias).to_lua_multi(lua)
            }
            Op::AimAt(pt) => ("aim_at", pt.x, pt.y).to_lua_multi(lua),
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
                    let x = f32::from_lua(vec.next().unwrap(), lua)?;
                    let y = f32::from_lua(vec.next().unwrap(), lua)?;
                    let re = f32::from_lua(vec.next().unwrap(), lua)?;
                    let im = f32::from_lua(vec.next().unwrap(), lua)?;
                    let speed = {
                        let coeff = f32::from_lua(vec.next().unwrap(), lua)?;
                        let linear_bias = f32::from_lua(vec.next().unwrap(), lua)?;
                        let angular_bias = f32::from_lua(vec.next().unwrap(), lua)?;
                        SpeedParameters {
                            coeff,
                            linear_bias,
                            angular_bias,
                        }
                    };
                    let accel = {
                        let coeff = f32::from_lua(vec.next().unwrap(), lua)?;
                        let linear_bias = f32::from_lua(vec.next().unwrap(), lua)?;
                        let angular_bias = f32::from_lua(vec.next().unwrap(), lua)?;
                        SpeedParameters {
                            coeff,
                            linear_bias,
                            angular_bias,
                        }
                    };
                    Ok(Op::Push(Some(Parameters {
                        position: Isometry2::from_parts(
                            Translation2::new(x, y),
                            Unit::new_unchecked(Complex::new(re, im)),
                        ),
                        speed,
                        accel,
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
            "compose" => {
                let speed = {
                    let coeff = f32::from_lua(vec.next().unwrap(), lua)?;
                    let linear_bias = f32::from_lua(vec.next().unwrap(), lua)?;
                    let angular_bias = f32::from_lua(vec.next().unwrap(), lua)?;
                    SpeedParameters {
                        coeff,
                        linear_bias,
                        angular_bias,
                    }
                };
                Ok(Op::Compose(speed))
            }
            "aim_at" => {
                let x = f32::from_lua(vec.next().unwrap(), lua)?;
                let y = f32::from_lua(vec.next().unwrap(), lua)?;
                Ok(Op::AimAt(Point2::new(x, y)))
            }
            "pop" => Ok(Op::Pop),
            "fire" => Ok(Op::Fire),
            _ => panic!("invalid op"),
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
    fn compose(&mut self, sp: SpeedParameters) -> Result<()> {
        self.op(Op::Compose(sp))
    }

    #[inline]
    fn mul_speed(&mut self, coeff: f32) -> Result<()> {
        self.compose(SpeedParameters::coeff(coeff))
    }

    #[inline]
    fn add_linear_speed(&mut self, linear_bias: f32) -> Result<()> {
        self.compose(SpeedParameters::linear_bias(linear_bias))
    }

    #[inline]
    fn add_angular_speed(&mut self, angular_bias: f32) -> Result<()> {
        self.compose(SpeedParameters::angular_bias(angular_bias))
    }

    #[inline]
    fn aim_at(&mut self, pt: Point2<f32>) -> Result<()> {
        self.op(Op::AimAt(pt))
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

        methods.add_function("pop", |_lua, this: LuaAnyUserData| {
            this.get_user_value::<LuaFunction>()?.call::<_, ()>("pop")
        });

        methods.add_function("fire", |_lua, this: LuaAnyUserData| {
            this.get_user_value::<LuaFunction>()?.call::<_, ()>("fire")
        });
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

impl LuaUserData for Ring {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_meta_method(
            LuaMetaMethod::Call,
            |_lua, this, mut builder: LuaPatternBuilder| {
                this.build(&mut builder).to_lua_err()?;
                Ok(())
            },
        );
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
        let step = self.angle / (self.count as f32);
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

impl LuaUserData for Arc {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_meta_method(
            LuaMetaMethod::Call,
            |_lua, this, mut builder: LuaPatternBuilder| {
                this.build(&mut builder).to_lua_err()?;
                Ok(())
            },
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Stack {
    pub delta: SpeedParameters,
    pub count: u32,
}

impl Stack {
    pub fn new(delta: SpeedParameters, count: u32) -> Self {
        Self { delta, count }
    }
}

impl Pattern for Stack {
    #[inline]
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        builder.push(None)?;
        builder.fire()?;
        for _ in 1..self.count {
            builder.compose(self.delta)?;
            builder.fire()?;
        }
        builder.pop()?;

        Ok(())
    }
}

impl LuaUserData for Stack {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_meta_method(
            LuaMetaMethod::Call,
            |_lua, this, mut builder: LuaPatternBuilder| {
                this.build(&mut builder).to_lua_err()?;
                Ok(())
            },
        );
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
        for (_e, (mut proj, mut motion, maximum)) in world
            .query::<(
                &mut Projectile,
                &mut QuadraticMotion,
                Option<&MaximumVelocity>,
            )>()
            .iter()
        {
            let (proj, motion) = (&mut *proj, &mut *motion);
            motion.velocity += motion.acceleration * dt;

            if let Some(max_vel) = maximum {
                let cur_vel = motion.velocity.linear.norm();
                if cur_vel > max_vel.linear {
                    motion.velocity.linear *= max_vel.linear / cur_vel;
                }

                let cur_ang = motion.velocity.angular.abs();
                if cur_ang > max_vel.angular {
                    motion.velocity.angular *= max_vel.angular / cur_ang;
                }
            }

            proj.position = motion.velocity.integrate(dt) * proj.position;
        }

        for (_e, (mut proj, mut motion, maximum)) in world
            .query::<(
                &mut Projectile,
                &mut DirectionalMotion,
                Option<&MaximumVelocity>,
            )>()
            .iter()
        {
            let (proj, motion) = (&mut *proj, &mut *motion);
            motion.velocity += motion.acceleration * dt;

            if let Some(max_vel) = maximum {
                let cur_vel = motion.velocity.linear.norm();
                if cur_vel > max_vel.linear {
                    motion.velocity.linear *= max_vel.linear / cur_vel;
                }

                let cur_ang = motion.velocity.angular.abs();
                if cur_ang > max_vel.angular {
                    motion.velocity.angular *= max_vel.angular / cur_ang;
                }
            }

            proj.position =
                motion.velocity.transformed(&proj.position).integrate(dt) * proj.position;
        }

        if let Some(bounds) = self.bounds {
            for (e, (proj, circle)) in world.query::<(&Projectile, &Circle)>().iter() {
                let circle_bb = Box2::from_half_extents(
                    Point2::from(proj.position.translation.vector),
                    Vector2::repeat(circle.radius),
                );

                if !bounds.intersects(&circle_bb) {
                    self.to_despawn.add(e.id());
                }
            }

            for (e, (proj, rect)) in world.query::<(&Projectile, &Rectangle)>().iter() {
                let homogeneous =
                    sludge::math::homogeneous_mat3_to_mat4(&proj.position.to_homogeneous());
                let rect_bb = Box2::from_half_extents(Point2::origin(), rect.half_extents)
                    .transformed_by(&homogeneous);

                if !bounds.intersects(&rect_bb) {
                    self.to_despawn.add(e.id());
                }
            }
        }

        for id in self.to_despawn.drain() {
            let entity = unsafe { world.resolve_unknown_gen(id) }.unwrap();
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
        let velocity = parameters.apply_to_derivative(&self.motion.velocity);
        let acceleration = parameters.apply_to_derivative(&self.motion.acceleration);

        Self {
            projectile: Projectile { position },
            motion: QuadraticMotion {
                velocity,
                acceleration,
            },
        }
    }
}

impl Bullet for DirectionalShot {
    type Bundled = Self;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
        let position = parameters.apply_to_position(&self.projectile.position);
        let velocity = parameters.apply_to_derivative(&self.motion.velocity);
        let acceleration = parameters.apply_to_derivative(&self.motion.acceleration);

        Self {
            projectile: Projectile { position },
            motion: DirectionalMotion {
                velocity,
                acceleration,
            },
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
            Op::Compose(sp) => {
                let top = self.stack.last_mut().unwrap();
                *top = top.composed(&sp);
            }
            Op::AimAt(p0) => {
                let ps = self.stack.last_mut().unwrap();
                let p1 = Point2::from(ps.position.translation.vector);
                let v = p0 - p1;
                let u = ps.position.transform_vector(&Vector2::x());
                let rot = UnitComplex::scaled_rotation_between(&u, &v, 1.);
                *ps = ps.rotated_wrt_center(&rot);
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
    fn batch_me<'lua>(&self, lua: LuaContext<'lua>, closure: LuaFunction<'lua>) -> LuaResult<()>;
}

struct BulletSlug<B: Bullet + Clone> {
    bullet: B,
}

impl<B: Bullet + Clone> ErasedBullet for BulletSlug<B> {
    fn batch_me<'lua>(&self, lua: LuaContext<'lua>, closure: LuaFunction<'lua>) -> LuaResult<()> {
        let mut batch = Batch::new(lua, self.bullet.clone()).to_lua_err()?;
        lua.scope(|scope| -> LuaResult<()> {
            let emit_closure =
                scope.create_function_mut(|_lua, op: Op| batch.op(op).to_lua_err())?;
            let lua_builder = LuaPatternBuilder::new(lua, emit_closure)?;
            LuaFunction::call(&closure, lua_builder)
        })?;

        let resources = lua.resources();
        let world = &mut *resources.fetch_mut::<World>();
        world.spawn_batch(batch.to_vec()).for_each(|_| {});

        Ok(())
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

    let table = lua.create_table_from(vec![
        (
            "ring",
            lua.create_function(|_, (radius, count)| -> LuaResult<RustPattern> {
                Ok(RustPattern::new(Ring { radius, count }))
            })?,
        ),
        (
            "arc",
            lua.create_function(|_, (radius, angle, count)| -> LuaResult<RustPattern> {
                Ok(RustPattern::new(Arc {
                    radius,
                    angle,
                    count,
                }))
            })?,
        ),
        (
            "stack",
            lua.create_function(
                |_, (coeff, linear_bias, angular_bias, count)| -> LuaResult<RustPattern> {
                    Ok(RustPattern::new(Stack {
                        delta: SpeedParameters::new(coeff, linear_bias, angular_bias),
                        count,
                    }))
                },
            )?,
        ),
        (
            "spawn",
            lua.create_function(move |lua, (bullet_ty, closure): (LuaString, LuaFunction)| {
                bullets[bullet_ty.to_str()?].batch_me(lua, closure)
            })?,
        ),
    ])?;

    Ok(LuaValue::Table(table))
}

inventory::submit! {
    Module::parse("danmaku", load)
}
