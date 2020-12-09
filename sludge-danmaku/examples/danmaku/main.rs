#![feature(exact_size_is_empty)]

use ::{
    anyhow::*,
    hashbrown::HashMap,
    sludge::{
        assets::DefaultCache, conf::Conf, dispatcher::Dispatcher, event::EventHandler,
        filesystem::Filesystem, graphics::*, prelude::*,
    },
    sludge_danmaku::*,
    std::{env, path::PathBuf},
};

pub struct TestResource {
    batch: SpriteBatch,
}

#[derive(Debug, Clone, Copy, SimpleComponent)]
pub struct SpriteIndex {
    idx: SpriteId,
}

#[derive(Clone, Bundle)]
pub struct EasedBullet;

impl BulletData for EasedBullet {
    type Bundled = (Projectile, ParametricMotion, SpriteIndex, Collision);

    fn bundle(
        &self,
        resources: &UnifiedResources,
        parameters: &[Parameters],
        bullet_type: BulletTypeId,
        bundles: &mut Vec<Self::Bundled>,
    ) -> Result<()> {
        let tmp = resources.fetch_one::<TestResource>()?;
        let test_resource = &mut tmp.borrow_mut();
        let batch = &mut test_resource.batch;

        bundles.extend(parameters.iter().map(|ps| {
            let instance = InstanceParam::default().translate2(ps.position.translation.vector);
            let idx = SpriteIndex {
                idx: batch.insert(instance),
            };
            let projectile = Projectile::origin(bullet_type);
            let motion =
                ParametricMotion::lerp_expo_out(false, ps.duration, &ps.position, &ps.destination);
            let collision = Collision::Circle { radius: 1.0 };
            (projectile, motion, idx, collision)
        }));

        Ok(())
    }
}

impl<'lua> ToLua<'lua> for EasedBullet {
    fn to_lua(self, _: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        Ok(LuaValue::Nil)
    }
}

impl<'lua> FromLua<'lua> for EasedBullet {
    fn from_lua(_: LuaValue<'lua>, _: LuaContext<'lua>) -> LuaResult<Self> {
        Ok(Self)
    }
}

inventory::submit! {
    BulletMetatype::new::<EasedBullet>("Eased")
}

#[derive(Clone, Bundle)]
pub struct TestBullet;

impl BulletData for TestBullet {
    type Bundled = (Projectile, QuadraticMotion, SpriteIndex, Collision);

    fn bundle(
        &self,
        resources: &UnifiedResources,
        parameters: &[Parameters],
        bullet_type: BulletTypeId,
        bundles: &mut Vec<Self::Bundled>,
    ) -> Result<()> {
        let tmp = resources.fetch_one::<TestResource>()?;
        let test_resource = &mut tmp.borrow_mut();
        let batch = &mut test_resource.batch;

        bundles.extend(parameters.iter().map(|ps| {
            let instance = InstanceParam::default().translate2(ps.position.translation.vector);
            let idx = SpriteIndex {
                idx: batch.insert(instance),
            };
            let projectile = Projectile::new(bullet_type, ps.position);
            let motion = QuadraticMotion::new(ps.to_velocity(), ps.to_acceleration());
            let collision = Collision::Circle { radius: 1.0 };
            (projectile, motion, idx, collision)
        }));

        Ok(())
    }
}

impl<'lua> ToLua<'lua> for TestBullet {
    fn to_lua(self, _: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        Ok(LuaValue::Nil)
    }
}

impl<'lua> FromLua<'lua> for TestBullet {
    fn from_lua(_: LuaValue<'lua>, _: LuaContext<'lua>) -> LuaResult<Self> {
        Ok(Self)
    }
}

inventory::submit! {
    BulletMetatype::new::<TestBullet>("Test")
}

struct MainState {
    space: Space,
    dispatcher: Dispatcher<'static>,
    events: ComponentSubscriber<SpriteIndex>,
    indices: HashMap<Entity, SpriteId>,
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
            resources.insert(TestResource { batch });

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
            ..
        } = self;

        let (scheduler, world, test_resource) =
            space.fetch::<(Scheduler, World, TestResource)>()?;
        space
            .lua()
            .context(|lua| scheduler.borrow_mut().update(lua, 1.0))?;

        space.dispatch(dispatcher)?;

        let tr = &mut *test_resource.borrow_mut();

        for (_, (proj, sprite_index)) in world
            .borrow()
            .query::<(&Projectile, &mut SpriteIndex)>()
            .iter()
        {
            let param = InstanceParam::default().translate2(proj.position().translation.vector);
            tr.batch[sprite_index.idx] = param;
        }

        for &event in world.borrow().poll::<SpriteIndex>(events) {
            match event {
                ComponentEvent::Inserted(e) => {
                    let idx = world.borrow().get::<SpriteIndex>(e)?.idx;
                    indices.insert(e, idx);
                }
                ComponentEvent::Removed(e) => {
                    let idx = indices.remove(&e).unwrap();
                    tr.batch.remove(idx);
                }
                _ => {}
            }
        }

        space.maintain().unwrap();

        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let Self { space, canvas, .. } = self;

        let (gfx_tmp, test_resource_tmp) = space.fetch::<(Graphics, TestResource)>()?;
        let gfx = &mut *gfx_tmp.borrow_mut();
        let test_resource = &*test_resource_tmp.borrow();

        gfx.set_projection(Orthographic3::new(0., 320., 0., 240., -1., 1.));

        gfx.begin_pass(&canvas.render_pass, PassAction::default());
        gfx.apply_default_pipeline();
        gfx.apply_transforms();
        gfx.draw(&test_resource.batch, None);
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

    sludge::event::run::<MainState>(
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
