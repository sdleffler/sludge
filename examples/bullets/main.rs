use {
    anyhow::*,
    miniquad as mq,
    rand::distributions::uniform::Uniform,
    sludge::{
        components::Transform,
        ecs::World,
        event::EventHandler,
        graphics::{Context, *},
        prelude::*,
    },
};

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
    idx: SpriteIdx,
}

struct MainState {
    context: Context,
    space: Space,
    bullet_count: u64,
    batch: SpriteBatch,
    canvas: Canvas,
}

impl MainState {
    pub fn new(mut ctx: Context) -> Result<MainState> {
        let mut space = Space::new()?;

        space.refresh()?;

        // graphics::set_screen_coordinates(ctx, graphics::Rect::new(0., 0., 320., 240.))?;

        // let mut canvas = graphics::Canvas::new(ctx, 320, 240, ggez::conf::NumSamples::One)?;
        // canvas.set_filter(graphics::FilterMode::Nearest);

        let null_texture = ctx.null_texture.clone();
        let batch = SpriteBatch::with_capacity(&mut ctx, null_texture, 4096 * 4);
        let canvas = Canvas::new(&mut ctx, 320, 240);

        Ok(MainState {
            context: ctx,
            space,
            bullet_count: 0,
            batch,
            canvas,
            // batch: graphics::spritebatch::SpriteBatch::new(graphics::Image::solid(
            //     ctx,
            //     1,
            //     graphics::WHITE,
            // )?),
            // canvas,
        })
    }
}

impl EventHandler for MainState {
    fn init(ctx: Context) -> Result<Self> {
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

        space.lua().context(|lua| {
            //while timer::check_update_time(ctx, 60) {
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
                            idx: batch.insert(InstanceParam::default().translate(pos)),
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
                    *tx.local_mut() = na::convert(na::Translation3::from(spatial.pos.push(0.)));
                }

                batch[sprite_index.idx] = InstanceParam::default().translate(spatial.pos);
            }
            //}
        });

        space.update().unwrap();

        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        self.context
            .set_projection(Orthographic3::new(0., 320., 0., 240., -1., 1.));

        self.context
            .begin_pass(&self.canvas.render_pass, PassAction::default());
        self.context.apply_default_pipeline();
        self.context.apply_transforms();
        self.batch.draw(&mut self.context);
        self.context.end_pass();

        self.context.begin_default_pass(PassAction::default());
        self.context.apply_default_pipeline();
        self.context.apply_transforms();
        self.canvas.draw(
            &mut self.context,
            InstanceParam::new().scale(Vector2::new(320., 240.)),
        );
        self.context.end_pass();
        self.context.commit_frame();
        // let fps = timer::fps(ctx);
        // let fps_display = graphics::Text::new(format!(
        //     "FPS: {:2.1}, #bullets: {:04}",
        //     fps, self.bullet_count
        // ));

        // graphics::set_canvas(ctx, Some(&self.canvas));
        // graphics::clear(ctx, graphics::BLACK);
        // graphics::draw(ctx, &self.batch, (na::Point2::origin(),))?;
        // graphics::draw(ctx, &fps_display, (na::Point2::origin(), graphics::WHITE))?;
        // graphics::set_canvas(ctx, None);

        // graphics::clear(ctx, graphics::BLACK);
        // graphics::draw(ctx, &self.canvas, graphics::DrawParam::new())?;
        // graphics::present(ctx)
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

    sludge::event::run::<MainState>(mq::conf::Conf {
        window_title: "Bullets!".to_string(),
        window_width: 320 * 4,
        window_height: 240 * 4,
        ..mq::conf::Conf::default()
    });

    // let (mut ctx, mut event_loop) = ggez::ContextBuilder::new("bullets", "Sean Leffler")
    //     .window_setup(ggez::conf::WindowSetup::default().title("Bullets!"))
    //     .window_mode(ggez::conf::WindowMode::default().dimensions(1280., 960.))
    //     .build()?;

    // let mut main_state = MainState::new(&mut ctx)?;

    // match event::run(&mut ctx, &mut event_loop, &mut main_state) {
    //     Ok(_) => println!("Exited cleanly."),
    //     Err(e) => println!("Error occured: {}", e),
    // }

    Ok(())
}
