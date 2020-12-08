use crate::{graphics::Drawable2, Velocity2};
use sludge::{
    assets::Cached,
    graphics::{Drawable, Graphics, InstanceParam, SpriteBatch, SpriteId, Texture},
    prelude::*,
    sprite::{Frame, SpriteFrame, SpriteSheet, SpriteTag, TagId},
};

pub trait ParticleState {
    fn update(
        &mut self,
        particle: &mut Particle,
        sheet: &SpriteSheet,
        dt: f32,
    ) -> Option<InstanceParam>;
}

#[derive(Debug, Clone, Copy)]
pub struct PixelPerfectQuadraticState {
    pub position: Isometry2<f32>,
    pub velocity: Velocity2<f32>,
    pub acceleration: Velocity2<f32>,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub time_multiplier: f32,
}

impl Default for PixelPerfectQuadraticState {
    fn default() -> Self {
        Self {
            position: Isometry2::identity(),
            velocity: Velocity2::zero(),
            acceleration: Velocity2::zero(),
            linear_damping: 1.,
            angular_damping: 1.,
            time_multiplier: 1.,
        }
    }
}

impl PixelPerfectQuadraticState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn position(self, position: Isometry2<f32>) -> Self {
        Self { position, ..self }
    }

    pub fn velocity(self, velocity: Velocity2<f32>) -> Self {
        Self { velocity, ..self }
    }

    pub fn acceleration(self, acceleration: Velocity2<f32>) -> Self {
        Self {
            acceleration,
            ..self
        }
    }

    pub fn linear_damping(self, linear_damping: f32) -> Self {
        Self {
            linear_damping,
            ..self
        }
    }

    pub fn angular_damping(self, angular_damping: f32) -> Self {
        Self {
            angular_damping,
            ..self
        }
    }
}

impl ParticleState for PixelPerfectQuadraticState {
    fn update(
        &mut self,
        particle: &mut Particle,
        sheet: &SpriteSheet,
        dt: f32,
    ) -> Option<InstanceParam> {
        let dt = dt * self.time_multiplier;
        let frame = particle.update_animation(sheet, dt);

        if particle.is_animation_ended() {
            None
        } else {
            self.velocity += self.acceleration * dt;
            self.velocity.linear *= self.linear_damping;
            self.velocity.angular *= self.angular_damping;
            let integrated = self.velocity.integrate(dt);
            self.position.translation *= integrated.translation;
            self.position.rotation *= integrated.rotation;

            let pixel_pos = crate::math::smooth_subpixels(
                Point2::from(self.position.translation.vector),
                self.velocity.linear,
            );

            Some(
                InstanceParam::new()
                    .translate2(pixel_pos.coords)
                    .rotate2(self.position.rotation.angle())
                    .translate2(frame.offset)
                    .src(frame.uvs),
            )
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Particle {
    sprite: Option<SpriteId>,
    tag: SpriteTag,
    frame: SpriteFrame,
}

impl Particle {
    pub fn is_animation_ended(&self) -> bool {
        self.tag.is_paused && !self.tag.should_loop
    }

    pub fn update_animation(&mut self, sheet: &SpriteSheet, dt: f32) -> Frame {
        sheet.update_animation(dt, &mut self.tag, &mut self.frame);
        sheet[self.frame]
    }
}

#[derive(Debug)]
pub struct ParticleSystem<T: ParticleState> {
    sheet: Cached<SpriteSheet>,
    particles: Vec<(Particle, T)>,
    batch: SpriteBatch,
}

const DEFAULT_PARTICLE_SYSTEM_CAPACITY: usize = 64;

impl<T: ParticleState> ParticleSystem<T> {
    pub fn new<U, V>(gfx: &mut Graphics, texture: U, sheet: V) -> Self
    where
        U: Into<Cached<Texture>>,
        V: Into<Cached<SpriteSheet>>,
    {
        Self {
            sheet: sheet.into(),
            particles: Vec::new(),
            batch: SpriteBatch::with_capacity(gfx, texture, DEFAULT_PARTICLE_SYSTEM_CAPACITY),
        }
    }

    pub fn spawn(&mut self, state: T, tag: TagId, should_loop: bool) {
        let sheet = self.sheet.load_cached();
        let (frame, tag) = sheet.at_tag(tag, should_loop);
        let sprite = None;
        let particle = Particle { frame, tag, sprite };
        self.particles.push((particle, state));
    }

    pub fn update(&mut self, dt: f32) -> Result<()> {
        let sheet = self.sheet.load_cached();
        let mut i = 0;
        while i < self.particles.len() {
            let (ref mut particle, ref mut state) = &mut self.particles[i];
            if let Some(param) = state.update(particle, sheet, dt) {
                match particle.sprite {
                    None => particle.sprite = Some(self.batch.insert(param)),
                    Some(id) => self.batch[id] = param,
                }
                i += 1;
            } else {
                if let Some(id) = particle.sprite {
                    self.batch.remove(id);
                }
                self.particles.swap_remove(i);
            }
        }

        Ok(())
    }

    pub fn sheet(&self) -> &Cached<SpriteSheet> {
        &self.sheet
    }
}

impl<T: ParticleState> Drawable for ParticleSystem<T> {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        self.batch.draw(ctx, instance)
    }
}

impl<T: ParticleState> Drawable2 for ParticleSystem<T> {
    fn aabb(&self) -> Box2<f32> {
        self.batch.aabb()
    }
}
