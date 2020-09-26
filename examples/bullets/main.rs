use {
    anyhow::*,
    ggez::{
        event::{self, EventHandler},
        graphics, timer, Context, ContextBuilder, GameResult,
    },
    sludge::{
        ecs::{
            components::{Parent, Transform},
            World,
        },
        module::ecs::ArchetypeRegistry,
        prelude::*,
    },
};

fn main() {
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
        .apply()
        .expect("unable to configure logging");

    let (mut ctx, mut event_loop) = ContextBuilder::new("ascendant", "Sean Leffler, juneflower")
        .build()
        .expect("aieee, could not create ggez context!");

    let mut main_state = MainState::new(&mut ctx);

    match event::run(&mut ctx, &mut event_loop, &mut main_state) {
        Ok(_) => println!("Exited cleanly."),
        Err(e) => println!("Error occured: {}", e),
    }
}

struct MainState {
    space: Space,
}

impl MainState {
    pub fn new(_ctx: &mut Context) -> MainState {
        let space = Space::new().unwrap();

        space.lua().context(|lua| {
            let mut resources = space.resources().borrow_mut();

            resources.insert(World::new());
            resources.insert({
                let mut registry = ArchetypeRegistry::new(lua).unwrap();
                registry.register::<Transform>(lua).unwrap();
                registry.register::<Parent>(lua).unwrap();
                registry
            });
        });

        MainState { space }
    }
}

impl EventHandler for MainState {
    fn update(&mut self, ctx: &mut Context) -> GameResult<()> {
        self.space.lua().context(|lua| {
            while timer::check_update_time(ctx, 60) {
                self.space
                    .fetch_mut::<Scheduler>()
                    .with_context(lua)
                    .update(1.0)
                    .unwrap();
            }
        });

        Ok(())
    }

    fn draw(&mut self, ctx: &mut Context) -> GameResult<()> {
        graphics::clear(ctx, graphics::WHITE);
        // Draw code here...
        graphics::present(ctx)
    }
}
