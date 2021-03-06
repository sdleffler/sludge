extern crate sludge as sloodge;

use {
    anyhow::*,
    rand::distributions::uniform::Uniform,
    sloodge::{
        components::Transform, conf::Conf, ecs::World, event::EventHandler, graphics::*, prelude::*,
    },
};

mod sludge {
    pub use ::sludge::sludge::*;
}

#[derive(Debug, SimpleComponent)]
struct Spatial {
    pos: na::Vector2<f32>,
    vel: na::Vector2<f32>,
    acc: na::Vector2<f32>,
}

impl Spatial {
    fn new() -> Self {
        Self {
            pos: na::zero(),
            vel: na::zero(),
            acc: na::zero(),
        }
    }
}

#[derive(SimpleComponent)]
struct SpriteIndex {
    idx: SpriteId,
}

struct MainState {
    gfx: Graphics,
    space: Space,
    bullet_count: u64,
    batch: SpriteBatch,
    canvas: Canvas,
}

impl MainState {
    pub fn new(mut gfx: Graphics) -> Result<MainState> {
        let space = Space::new()?;
        let null_texture = gfx.null_texture.clone();
        let batch = SpriteBatch::with_capacity(&mut gfx, null_texture, 4096 * 4);
        let canvas = Canvas::new(&mut gfx, 320, 240);

        Ok(MainState {
            gfx,
            space,
            bullet_count: 0,
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
        const DT: f32 = 1. / 60.;
        let Self {
            space,
            bullet_count,
            batch,
            ..
        } = self;

        space.lua().context(|lua| -> Result<()> {
            //while timer::check_update_time(ctx, 60) {
            space
                .fetch_one::<Scheduler>()?
                .borrow_mut()
                .update(lua, 1.0)?;

            let world = space.fetch_one::<World>()?;

            let (w, h) = (320., 240.);
            {
                let world = &mut *world.borrow_mut();
                for _ in 0..10 {
                    let pos = na::Vector2::new(w / 2., h / 2.);
                    world.spawn((
                        Spatial {
                            pos,
                            vel: na::Vector2::from_distribution(
                                &Uniform::new(-30., 30.),
                                &mut rand::thread_rng(),
                            ),
                            ..Spatial::new()
                        },
                        SpriteIndex {
                            idx: batch.insert(InstanceParam::default().translate2(pos)),
                        },
                    ));
                    *bullet_count += 1;
                }
            }

            for (_e, (mut spatial, maybe_tx, sprite_index)) in world
                .borrow()
                .query::<(&mut Spatial, Option<&mut Transform>, &SpriteIndex)>()
                .iter()
            {
                let spatial = &mut *spatial;
                spatial.vel += spatial.acc * DT;
                spatial.pos += spatial.vel * DT;

                if let Some(mut tx) = maybe_tx {
                    let tx = &mut *tx;
                    *tx.local_mut() = na::convert(na::Translation3::from(spatial.pos.push(0.)));
                }

                batch[sprite_index.idx] = InstanceParam::default().translate2(spatial.pos);
            }
            //}

            Ok(())
        })?;

        space.maintain().unwrap();

        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let Self {
            gfx, canvas, batch, ..
        } = self;

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
