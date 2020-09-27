use {
    anyhow::*,
    ggez::{
        event::{self, EventHandler},
        graphics, timer,
    },
    rand::distributions::uniform::Uniform,
    sludge::{
        ecs::{
            components::{Parent, Transform},
            Flags, SmartComponent, World,
        },
        modules::ecs::ArchetypeRegistry,
        prelude::*,
    },
};

#[derive(Debug)]
struct Spatial {
    pos: na::Vector2<f32>,
    vel: na::Vector2<f32>,
    acc: na::Vector2<f32>,
}

impl<'a> SmartComponent<&'a Flags> for Spatial {}

impl Spatial {
    fn new() -> Self {
        Self {
            pos: na::zero(),
            vel: na::zero(),
            acc: na::zero(),
        }
    }
}

struct SpriteIndex {
    idx: graphics::spritebatch::SpriteIdx,
}

impl<'a> SmartComponent<&'a Flags> for SpriteIndex {}

struct MainState {
    space: Space,
    bullet_count: u64,
    batch: graphics::spritebatch::SpriteBatch,
    canvas: graphics::Canvas,
}

impl MainState {
    pub fn new(ctx: &mut ggez::Context) -> Result<MainState> {
        let mut space = Space::new()?;

        space.lua().context(|lua| -> Result<()> {
            space.resources().borrow_mut().insert({
                let mut registry = ArchetypeRegistry::new(lua)?;
                registry.register::<Transform>(lua)?;
                registry.register::<Parent>(lua)?;
                registry
            });

            Ok(())
        })?;

        space.refresh()?;

        graphics::set_screen_coordinates(ctx, graphics::Rect::new(0., 0., 320., 240.))?;

        let mut canvas = graphics::Canvas::new(ctx, 320, 240, ggez::conf::NumSamples::One)?;
        canvas.set_filter(graphics::FilterMode::Nearest);

        Ok(MainState {
            space,
            bullet_count: 0,
            batch: graphics::spritebatch::SpriteBatch::new(graphics::Image::solid(
                ctx,
                1,
                graphics::WHITE,
            )?),
            canvas,
        })
    }
}

impl EventHandler for MainState {
    fn update(&mut self, ctx: &mut ggez::Context) -> ggez::GameResult<()> {
        const DT: f32 = 1. / 60.;
        let Self {
            space,
            bullet_count,
            batch,
            ..
        } = self;

        space.lua().context(|lua| {
            while timer::check_update_time(ctx, 60) {
                space
                    .fetch_mut::<Scheduler>()
                    .with_context(lua)
                    .update(1.0)
                    .unwrap();

                let (w, h) = (320., 240.);
                {
                    let mut world = space.fetch_mut::<World>();
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
                                idx: batch.add((na::Point2::from(pos),)),
                            },
                        ));
                        *bullet_count += 1;
                    }
                }

                let world = space.fetch::<World>();

                for (_e, (mut spatial, maybe_tx, sprite_index)) in world
                    .query::<(&mut Spatial, Option<&mut Transform>, &SpriteIndex)>()
                    .iter()
                {
                    let spatial = &mut *spatial;
                    spatial.vel += spatial.acc * DT;
                    spatial.pos += spatial.vel * DT;

                    if let Some(mut tx) = maybe_tx {
                        let tx = &mut *tx;
                        *tx.local_mut() = na::convert(na::Translation2::from(spatial.pos));
                    }

                    batch
                        .set(sprite_index.idx, (na::Point2::from(spatial.pos),))
                        .unwrap();
                }
            }
        });

        space.update().unwrap();

        Ok(())
    }

    fn draw(&mut self, ctx: &mut ggez::Context) -> ggez::GameResult<()> {
        let fps = timer::fps(ctx);
        let fps_display = graphics::Text::new(format!(
            "FPS: {:2.1}, #bullets: {:04}",
            fps, self.bullet_count
        ));

        graphics::set_canvas(ctx, Some(&self.canvas));
        graphics::clear(ctx, graphics::BLACK);
        graphics::draw(ctx, &self.batch, (na::Point2::origin(),))?;
        graphics::draw(ctx, &fps_display, (na::Point2::origin(), graphics::WHITE))?;
        graphics::set_canvas(ctx, None);

        graphics::clear(ctx, graphics::BLACK);
        graphics::draw(ctx, &self.canvas, graphics::DrawParam::new())?;
        graphics::present(ctx)
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

    let (mut ctx, mut event_loop) = ggez::ContextBuilder::new("bullets", "Sean Leffler")
        .window_setup(ggez::conf::WindowSetup::default().title("Bullets!"))
        .window_mode(ggez::conf::WindowMode::default().dimensions(1280., 960.))
        .build()?;

    let mut main_state = MainState::new(&mut ctx)?;

    match event::run(&mut ctx, &mut event_loop, &mut main_state) {
        Ok(_) => println!("Exited cleanly."),
        Err(e) => println!("Error occured: {}", e),
    }

    Ok(())
}
