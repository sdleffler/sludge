use ::{
    rand::{RngCore, SeedableRng},
    rand_xorshift::XorShiftRng,
    sludge::{prelude::*, resources::Shared},
    sludge_2d::math::*,
    std::{f32, marker::PhantomData},
};

use crate::{
    bullet::{BulletTypeId, Bundler},
    pattern::Pattern,
    DanmakuResourceExt, SharedRng, RNG_REGISTRY_KEY,
};

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
    pub fn to_velocity(&self) -> Velocity2<f32> {
        self.speed.transformed(&self.position)
    }

    #[inline]
    pub fn to_acceleration(&self) -> Velocity2<f32> {
        self.accel.transformed(&self.position)
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
    BulletType(BulletTypeId),
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
            Op::BulletType(bt) => ("bullet_type", bt.to_lua(lua)).to_lua_multi(lua),
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
            "bullet_type" => Ok(Op::BulletType(BulletTypeId::from_lua(
                vec.next().unwrap(),
                lua,
            )?)),
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

pub trait Bullet: Send + Sync {
    type Bundled: Bundle;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled;

    fn on_batched(&self, _lua: LuaContext) -> Result<()> {
        Ok(())
    }
}

pub struct Batch<'lua> {
    parameter_stack: Vec<Parameters>,
    bullet_type_stack: Vec<BulletTypeId>,
    bundler: Bundler,
    entities: Vec<Entity>,
    lua: LuaContext<'lua>,
    rng: SharedRng<XorShiftRng>,
}

impl<'lua> Batch<'lua> {
    pub fn new(lua: LuaContext<'lua>) -> Result<Self> {
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
            parameter_stack: vec![Parameters::default()],
            bullet_type_stack: Vec::new(),
            bundler: lua.bundler()?.detach(),
            entities: Vec::new(),
            lua,
            rng,
        })
    }

    pub fn spawn(
        &mut self,
        resources: &UnifiedResources,
        world: &Shared<'static, World>,
    ) -> Result<impl Iterator<Item = Entity> + '_> {
        self.bundler.flush(resources, world, &mut self.entities)?;
        Ok(self.entities.drain(..))
    }
}

impl<'lua> PatternBuilder<'lua> for Batch<'lua> {
    #[inline]
    fn op(&mut self, op: Op) -> Result<()> {
        match op {
            Op::Push(maybe_ps) => {
                let new_top = maybe_ps.unwrap_or_else(|| *self.parameter_stack.last().unwrap());
                self.parameter_stack.push(new_top);

                if let Some(last) = self.bullet_type_stack.last().copied() {
                    self.bullet_type_stack.push(last);
                }
            }
            Op::Transform(tx) => {
                let top = self.parameter_stack.last_mut().unwrap();
                *top = top.transformed(&tx);
            }
            Op::Translate(v) => {
                let top = self.parameter_stack.last_mut().unwrap();
                *top = top.translated(&v);
            }
            Op::Rotate(r) => {
                let top = self.parameter_stack.last_mut().unwrap();
                *top = top.rotated_wrt_center(&r);
            }
            Op::RotateVelocity(r) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.speed = top.speed.rotated(&r.to_rotation_matrix());
            }
            Op::AddVelocity(v) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.speed += v;
            }
            Op::MulVelocity(m) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.speed *= m;
            }
            Op::RotateAcceleration(r) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.accel = top.accel.rotated(&r.to_rotation_matrix());
            }
            Op::AddAcceleration(v) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.accel += v;
            }
            Op::MulAcceleration(m) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.accel *= m;
            }
            Op::AimAt(p0) => {
                let ps = self.parameter_stack.last_mut().unwrap();
                let p1 = Point2::from(ps.position.translation.vector);
                let v = p0 - p1;
                let u = ps.position.transform_vector(&Vector2::x());
                let rot = UnitComplex::scaled_rotation_between(&u, &v, 1.);
                *ps = ps.rotated_wrt_center(&rot);
            }
            Op::Destination(iso) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.destination = iso;
            }
            Op::Duration(t) => {
                let top = self.parameter_stack.last_mut().unwrap();
                top.duration = t;
            }
            Op::Pop => {
                self.parameter_stack.pop().unwrap();
                self.bullet_type_stack.pop();
            }
            Op::BulletType(bt_id) => {
                match self.bullet_type_stack.last_mut() {
                    Some(top) => *top = bt_id,
                    None => self.bullet_type_stack.push(bt_id),
                }
                self.bundler.set_id(bt_id);
            }
            Op::Fire => {
                self.bundler.push(*self.parameter_stack.last().unwrap());
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

        methods.add_function(
            "bullet_type",
            |_lua, (this, t): (LuaAnyUserData, BulletTypeId)| {
                this.get_user_value::<LuaFunction>()?
                    .call::<_, ()>(("bullet_type", t))
            },
        );

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
