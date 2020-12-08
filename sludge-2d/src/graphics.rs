use {
    sludge::{
        graphics::{Drawable, Sprite, SpriteBatch, Mesh, Texture},
        prelude::*,
    },
    std::any::Any,
};

pub mod drawable_graph;
pub mod particle_system;
pub mod text;

pub trait Drawable2: Drawable {
    fn aabb(&self) -> Box2<f32>;
}

/// Shorthand trait for types that are `Drawable` and `Any`, as well as
/// `Send + Sync`. This is blanket-impl'd and you should never have to implement
/// it manually.
pub trait AnyDrawable2: Drawable2 + Any + Send + Sync {
    #[doc(hidden)]
    fn as_any(&self) -> &dyn Any;

    #[doc(hidden)]
    fn as_any_mut(&mut self) -> &mut dyn Any;

    #[doc(hidden)]
    fn to_box_any(self: Box<Self>) -> Box<dyn Any>;

    #[doc(hidden)]
    fn as_drawable(&self) -> &dyn Drawable;

    #[doc(hidden)]
    fn as_drawable2(&self) -> &dyn Drawable2;
}

impl<T: Drawable2 + Any + Send + Sync> AnyDrawable2 for T {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self as &mut dyn Any
    }

    fn to_box_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }

    fn as_drawable(&self) -> &dyn Drawable {
        self as &dyn Drawable
    }

    fn as_drawable2(&self) -> &dyn Drawable2 {
        self as &dyn Drawable2
    }
}

impl dyn AnyDrawable2 {
    #[doc(hidden)]
    pub fn downcast<T: Any>(self: Box<Self>) -> Option<T> {
        Box::<dyn Any>::downcast(self.to_box_any())
            .map(|boxed| *boxed)
            .ok()
    }
}

impl Drawable2 for () {
    fn aabb(&self) -> Box2<f32> {
        Box2::invalid()
    }
}

impl Drawable2 for Texture {
    fn aabb(&self) -> Box2<f32> {
        Box2::from_corners(
            Point2::origin(),
            Point2::new(self.width() as f32, self.height() as f32),
        )
    }
}

impl Drawable2 for Sprite {
    fn aabb(&self) -> Box2<f32> {
        let texture_aabb = self.texture.load().aabb();
        let extents = self.params.src.extents();
        self.params.transform_aabb(&Box2::from_corners(
            texture_aabb.mins,
            Point2::new(
                texture_aabb.maxs.x * extents.x,
                texture_aabb.maxs.y * extents.y,
            ),
        ))
    }
}

impl Drawable2 for SpriteBatch {
    fn aabb(&self) -> Box2<f32> {
        let mut initial = Box2::invalid();
        let texture = self.texture().load();
        let image_aabb = texture.aabb();
        for (_, param) in self.iter() {
            initial.merge(
                &param
                    .scale2(param.src.extents())
                    .transform_aabb(&image_aabb),
            );
        }

        initial
    }
}

impl Drawable2 for Mesh {
    fn aabb(&self) -> Box2<f32> {
        self.aabb
    }
}
