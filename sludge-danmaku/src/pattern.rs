use ::{
    im::Vector,
    sludge::prelude::*,
    sludge_2d::math::*,
    std::{f32, sync},
};

use crate::{
    builder::{LuaPatternBuilder, Op, PatternBuilder},
    components::Projectile,
};

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

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub(crate) entities: Vector<Entity>,
}

impl Group {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LuaUserData for Group {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method_mut("cancel", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
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

impl Pattern for Group {
    fn build<'lua>(&self, builder: &mut dyn PatternBuilder<'lua>) -> Result<()> {
        let tmp = builder.lua().fetch_one::<World>()?;
        let world = tmp.borrow();

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
