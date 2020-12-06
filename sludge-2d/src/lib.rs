use {
    serde::{Deserialize, Serialize},
    sludge::{
        api::{LuaComponent, LuaComponentInterface},
        ecs::*,
        math::*,
        prelude::*,
    },
    std::ops,
};

pub use ncollide2d::{
    self as nc,
    bounding_volume::{self, BoundingVolume, HasBoundingVolume},
    query::{self, DefaultTOIDispatcher, Proximity},
    shape::{Ball, Cuboid, ShapeHandle},
};

pub mod math;
pub mod spatial_hash;

pub mod prelude {
    pub use crate::math::*;
    pub use sludge::prelude::*;
}

use crate::math::Velocity2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "PositionProxy", into = "PositionProxy")]
pub struct Position(pub Isometry2<f32>);

impl Default for Position {
    fn default() -> Self {
        Self(Isometry2::identity())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename = "Position")]
struct PositionProxy {
    x: f32,
    y: f32,

    #[serde(default)]
    angle: f32,
}

impl Default for PositionProxy {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            angle: 0.,
        }
    }
}

impl From<PositionProxy> for Position {
    fn from(de: PositionProxy) -> Self {
        Self(Isometry2::from_parts(
            Translation2::new(de.x, de.y),
            UnitComplex::new(de.angle),
        ))
    }
}

impl From<Position> for PositionProxy {
    fn from(Position(ser): Position) -> Self {
        Self {
            x: ser.translation.vector.x,
            y: ser.translation.vector.y,
            angle: ser.rotation.angle(),
        }
    }
}

impl<'a> SmartComponent<ScContext<'a>> for Position {}

impl ops::Deref for Position {
    type Target = Isometry2<f32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Position {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Position {
    pub fn center(&self) -> Point2<f32> {
        Point2::from(self.0.translation.vector)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PositionAccessor(Entity);

impl LuaUserData for PositionAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_meta_method(LuaMetaMethod::Index, |lua, this, key: LuaString| {
            let world = lua.fetch_one::<World>()?;
            let pos = *world.borrow().get::<Position>(this.0).to_lua_err()?;
            match key.to_str()? {
                "x" => pos.translation.vector.x.to_lua(lua),
                "y" => pos.translation.vector.y.to_lua(lua),
                "angle" => pos.rotation.angle().to_lua(lua),
                _ => LuaValue::Nil.to_lua(lua),
            }
        });

        methods.add_meta_method(
            LuaMetaMethod::NewIndex,
            |lua, this, (key, value): (LuaString, f32)| {
                let tmp = lua.fetch_one::<World>()?;
                let world = tmp.borrow();
                let pos = &mut *world.get_mut::<Position>(this.0).to_lua_err()?;
                match key.to_str()? {
                    "x" => pos.translation.vector.x = value,
                    "y" => pos.translation.vector.y = value,
                    "angle" => pos.rotation = UnitComplex::new(value),
                    other => {
                        return Err(anyhow!("no such field {} for Position", other).to_lua_err())
                    }
                }
                Ok(())
            },
        );

        // Separate method from index because index cannot return multiple
        // values.
        methods.add_method("coords", |lua, this, ()| {
            let world = lua.fetch_one::<World>()?;
            let pos = *world.borrow().get::<Position>(this.0).to_lua_err()?;
            let x = pos.translation.vector.x;
            let y = pos.translation.vector.y;
            (x, y).to_lua_multi(lua)
        });

        methods.add_method("set_coords", |lua, this, (x, y): (f32, f32)| {
            let world = lua.fetch_one::<World>()?;
            world
                .borrow()
                .get_mut::<Position>(this.0)
                .to_lua_err()?
                .translation
                .vector = Vector2::new(x, y);
            Ok(())
        });

        methods.add_method("to_table", |lua, this, ()| {
            let world = lua.fetch_one::<World>()?;
            let position = *world.borrow().get::<Position>(this.0).to_lua_err()?;
            rlua_serde::to_value(lua, *position)
        });
    }
}

impl LuaComponentInterface for Position {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        PositionAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        _lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let position = rlua_serde::from_value::<Position>(args)?;
        builder.add(position);
        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<Position>("Position")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(from = "VelocityProxy", into = "VelocityProxy")]
pub struct Velocity(pub Velocity2<f32>);

impl Default for Velocity {
    fn default() -> Self {
        Self(Velocity2::zero())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename = "Velocity")]
#[serde(default)]
struct VelocityProxy {
    x: f32,
    y: f32,
    angular: f32,
}

impl Default for VelocityProxy {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            angular: 0.,
        }
    }
}

impl From<VelocityProxy> for Velocity {
    fn from(de: VelocityProxy) -> Self {
        Self(Velocity2::new(Vector2::new(de.x, de.y), de.angular))
    }
}

impl From<Velocity> for VelocityProxy {
    fn from(Velocity(ser): Velocity) -> Self {
        Self {
            x: ser.linear.x,
            y: ser.linear.y,
            angular: ser.angular,
        }
    }
}

impl<'a> SmartComponent<ScContext<'a>> for Velocity {}

impl ops::Deref for Velocity {
    type Target = Velocity2<f32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Velocity {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VelocityAccessor(Entity);

impl LuaUserData for VelocityAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_meta_method(LuaMetaMethod::Index, |lua, this, key: LuaString| {
            let world = lua.fetch_one::<World>()?;
            let velocity = *world.borrow().get::<Velocity>(this.0).to_lua_err()?;
            match key.to_str()? {
                "x" => velocity.linear.x.to_lua(lua),
                "y" => velocity.linear.y.to_lua(lua),
                "angular" => velocity.angular.to_lua(lua),
                _ => LuaValue::Nil.to_lua(lua),
            }
        });

        methods.add_meta_method(
            LuaMetaMethod::Index,
            |lua, this, (key, value): (LuaString, f32)| {
                let tmp = lua.fetch_one::<World>()?;
                let world = tmp.borrow();
                let velocity = &mut *world.get_mut::<Velocity>(this.0).to_lua_err()?;
                match key.to_str()? {
                    "x" => velocity.linear.x = value,
                    "y" => velocity.linear.y = value,
                    "angular" => velocity.angular = value,
                    other => {
                        return Err(anyhow!("no such field {} for Velocity", other).to_lua_err())
                    }
                }
                Ok(())
            },
        );

        methods.add_method("linear", |lua, this, ()| {
            let world = lua.fetch_one::<World>()?;
            let velocity = *world.borrow().get::<Velocity>(this.0).to_lua_err()?;
            let x = velocity.linear.x;
            let y = velocity.linear.y;
            (x, y).to_lua_multi(lua)
        });

        methods.add_method("set_linear", |lua, this, (x, y)| {
            let world = lua.fetch_one::<World>()?;
            world
                .borrow()
                .get_mut::<Velocity>(this.0)
                .to_lua_err()?
                .linear = Vector2::new(x, y);
            Ok(())
        });

        methods.add_method("to_table", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
            let velocity = world.get::<Velocity>(this.0).to_lua_err()?;
            rlua_serde::to_value(lua, *velocity)
        });
    }
}

impl LuaComponentInterface for Velocity {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        VelocityAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        _lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let velocity = rlua_serde::from_value::<Velocity>(args)?;
        builder.add(velocity);
        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<Velocity>("Velocity")
}

#[derive(Clone)]
pub struct Shape {
    pub local: Isometry2<f32>,
    pub handle: ShapeHandle<f32>,
}

impl<'a> SmartComponent<ScContext<'a>> for Shape {}

impl Shape {
    pub fn new(local: Isometry2<f32>, handle: ShapeHandle<f32>) -> Self {
        Self { local, handle }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShapeTable {
    Box {
        position: Position,
        width: f32,
        height: f32,
    },
    Circle {
        position: Position,
        radius: f32,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ShapeAccessor(Entity);

impl LuaUserData for ShapeAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("to_table", |lua, this, ()| {
            let tmp = lua.fetch_one::<World>()?;
            let world = tmp.borrow();
            let shape = world.get::<Shape>(this.0).to_lua_err()?;

            if let Some(cuboid) = shape.handle.as_shape::<Cuboid<f32>>() {
                let extents = cuboid.half_extents * 2.;
                rlua_serde::to_value(
                    lua,
                    ShapeTable::Box {
                        position: Position(shape.local),
                        width: extents.x,
                        height: extents.y,
                    },
                )
            } else if let Some(ball) = shape.handle.as_shape::<Ball<f32>>() {
                let radius = ball.radius;
                rlua_serde::to_value(
                    lua,
                    ShapeTable::Circle {
                        position: Position(shape.local),
                        radius,
                    },
                )
            } else {
                Err(format_err!("unsupported shape")).to_lua_err()
            }
        });
    }
}

impl LuaComponentInterface for Shape {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        ShapeAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        _lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let shape_table = rlua_serde::from_value::<ShapeTable>(args)?;
        match shape_table {
            ShapeTable::Box {
                position,
                width,
                height,
            } => {
                let local = *position;
                let cuboid = Cuboid::new(Vector2::new(width / 2., height / 2.));
                builder.add(Shape {
                    local,
                    handle: ShapeHandle::new(cuboid),
                });
            }
            ShapeTable::Circle { position, radius } => {
                let local = *position;
                let ball = Ball::new(radius);
                builder.add(Shape {
                    local,
                    handle: ShapeHandle::new(ball),
                });
            }
        }

        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<Shape>("Shape")
}
