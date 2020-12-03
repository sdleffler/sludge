#![feature(exact_size_is_empty)]

extern crate sludge as sloodge;

use ::{
    anyhow::*,
    hashbrown::HashMap,
    sloodge::{
        assets::DefaultCache, conf::Conf, dispatcher::Dispatcher, event::EventHandler,
        filesystem::Filesystem, graphics::*, prelude::*,
    },
    sludge_danmaku::*,
    std::{env, path::PathBuf},
};

mod sludge {
    pub use ::sludge::sludge::*;
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
struct SpriteIndex {
    idx: Option<SpriteId>,
}

#[derive(Clone, Bundle)]
pub struct EasedBullet {
    projectile: Projectile,
    motion: ParametricMotion,
    sprite_idx: SpriteIndex,
    collision: Collision,
}

impl Bullet for EasedBullet {
    type Bundled = Self;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
        let position = parameters.apply_to_position(self.projectile.position());

        Self {
            projectile: Projectile::origin(),
            motion: ParametricMotion::lerp_expo_out(
                false,
                parameters.duration,
                &position,
                &parameters.destination,
            ),
            ..*self
        }
    }
}

#[derive(Debug, Clone, Copy, Bundle)]
pub struct TestBullet {
    projectile: Projectile,
    motion: QuadraticMotion,
    sprite_idx: SpriteIndex,
    collision: Collision,
}

impl Bullet for TestBullet {
    type Bundled = Self;

    fn to_bundled(&self, parameters: &Parameters) -> Self::Bundled {
        let position = parameters.apply_to_position(self.projectile.position());
        let velocity = parameters.apply_to_velocity(&self.motion.velocity);
        let acceleration = parameters.apply_to_acceleration(&self.motion.acceleration);

        Self {
            projectile: Projectile::new(position),
            motion: QuadraticMotion::new(velocity, acceleration),
            ..*self
        }
    }
}

inventory::submit! {
    BulletType::new::<TestBullet>("TestBullet", TestBullet {
        projectile: Projectile::origin(),
        motion: QuadraticMotion::zero(),
        sprite_idx: SpriteIndex { idx: None },
        collision: Collision::Circle { radius: 1.0 },
    })
}

inventory::submit! {
    BulletType::new::<EasedBullet>("EasedBullet", EasedBullet {
        projectile: Projectile::origin(),
        motion: ParametricMotion::lerp_expo_out(
            false,
            5.,
            &Isometry2::identity(),
            &Isometry2::identity(),
        ),
        sprite_idx: SpriteIndex { idx: None },
        collision: Collision::Circle { radius: 1.0 },
    })
}

struct MainState {
    space: Space,
    dispatcher: Dispatcher<'static>,
    events: ComponentSubscriber<SpriteIndex>,
    indices: HashMap<Entity, SpriteId>,
    batch: SpriteBatch,
    canvas: Canvas,
}

impl MainState {
    pub fn new(mut gfx: Graphics) -> Result<MainState> {
        let null_texture = gfx.null_texture.clone();
        let batch = SpriteBatch::with_capacity(&mut gfx, null_texture, 4096 * 4);
        let canvas = Canvas::new(&mut gfx, 320, 240);

        let global = {
            let mut resources = OwnedResources::new();

            let mut fs = Filesystem::new("ascendant", "Sean Leffler, juneflower")?;
            if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
                let mut path = PathBuf::from(manifest_dir);
                path.push("resources");
                log::info!("Adding resource path {}", path.display());
                fs.mount(&path, true);
            }

            resources.insert(fs);
            resources.insert(gfx);

            SharedResources::from(resources)
        };

        let space = Space::with_global_resources(global)?;
        let cache = DefaultCache::new(space.resources().clone());

        {
            let mut res_mut = space.resources().borrow_mut();
            res_mut.insert(cache);
            res_mut.insert(Danmaku::with_bounds(Box2::new(0., 0., 320., 240.)));
        }

        let mut dispatcher = Dispatcher::new();
        dispatcher.register(DanmakuSystem, "Danmaku", &[])?;

        space.refresh(&mut dispatcher)?;

        let events = space.world()?.borrow_mut().track::<SpriteIndex>();

        space.lua().context(|lua| -> Result<_> {
            lua.load(include_str!("main.lua")).exec()?;
            Ok(())
        })?;

        Ok(MainState {
            space,
            dispatcher,
            events,
            indices: HashMap::new(),
            batch,
            canvas,
        })
    }
}

impl EventHandler for MainState {
    type Args = ();

    fn init(ctx: Graphics, _: ()) -> Result<Self> {
        Self::new(ctx)
    }

    fn update(&mut self) -> Result<()> {
        let Self {
            space,
            dispatcher,
            events,
            indices,
            batch,
            ..
        } = self;

        let (scheduler, world) = space.fetch::<(Scheduler, World)>()?;
        space
            .lua()
            .context(|lua| scheduler.borrow_mut().update(lua, 1.0))?;

        space.dispatch(dispatcher)?;

        for (e, (proj, mut sprite_index)) in world
            .borrow()
            .query::<(&Projectile, &mut SpriteIndex)>()
            .iter()
        {
            let param = InstanceParam::default().translate2(proj.position().translation.vector);
            match sprite_index.idx {
                Some(idx) => {
                    batch[idx] = param;
                }
                None => {
                    let idx = batch.insert(param);
                    indices.insert(e, idx);
                    sprite_index.idx = Some(idx);
                }
            };
        }

        for event in world.borrow().poll::<SpriteIndex>(events) {
            if let ComponentEvent::Removed(e) = event {
                let idx = indices.remove(e).unwrap();
                batch.remove(idx);
            }
        }

        space.maintain().unwrap();

        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let Self {
            space,
            canvas,
            batch,
            ..
        } = self;

        let tmp = space.fetch_one::<Graphics>()?;
        let gfx = &mut *tmp.borrow_mut();

        gfx.set_projection(Orthographic3::new(0., 320., 0., 240., -1., 1.));

        gfx.begin_pass(&canvas.render_pass, PassAction::default());
        gfx.apply_default_pipeline();
        gfx.apply_transforms();
        gfx.draw(batch, None);
        gfx.end_pass();

        gfx.begin_default_pass(PassAction::default());
        gfx.apply_default_pipeline();
        gfx.apply_transforms();
        gfx.draw(canvas, None);
        gfx.end_pass();
        gfx.commit_frame();
        Ok(())
    }
}

fn main() -> Result<()> {
    use fern::colors::{Color, ColoredLevelConfig};
    let colors = ColoredLevelConfig::default()
        .info(Color::Green)
        .debug(Color::BrightMagenta)
        .trace(Color::BrightBlue);

    // This sets up a `fern` logger and initializes `log`.
    fern::Dispatch::new()
        // Formats logs
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}][{:<5}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                colors.color(record.level()),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .level_for("winit", log::LevelFilter::Warn)
        .level_for("gfx_device_gl", log::LevelFilter::Warn)
        .chain(std::io::stdout())
        .apply()?;

    sloodge::event::run::<MainState>(
        Conf {
            window_title: "Bullets!".to_string(),
            window_width: 320 * 4,
            window_height: 240 * 4,
            ..Conf::default()
        },
        (),
    );

    Ok(())
}
