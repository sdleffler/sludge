use {
    anyhow::*,
    ggez::{
        event::{self, EventHandler},
        graphics::{
            self,
            spritebatch::{SpriteBatch, SpriteIdx},
        },
        timer,
    },
    serde::{Deserialize, Serialize},
    sludge::{
        api::Template,
        ecs::{Entity, Flags, SmartComponent, World},
        prelude::*,
        SludgeLuaContextExt,
    },
};

struct BulletTemplate;

impl Template for BulletTemplate {
    fn constructor<'lua>(
        &self,
        lua: LuaContext<'lua>,
        args: LuaMultiValue<'lua>,
    ) -> Result<Entity> {
        let (x, y, vx, vy, ax, ay): (f32, f32, f32, f32, Option<f32>, Option<f32>) =
            FromLuaMulti::from_lua_multi(args, lua)?;

        let table = lua.create_table_from(vec![(
            "Spatial",
            lua.create_table_from(vec![
                ("pos", vec![x, y]),
                ("vel", vec![vx, vy]),
                ("acc", vec![ax.unwrap_or(0.), ay.unwrap_or(0.)]),
            ])?,
        )])?;

        self.from_table(lua, table)
    }

    fn to_table<'lua>(
        &self,
        lua: LuaContext<'lua>,
        instance: Entity,
    ) -> Result<Option<LuaTable<'lua>>> {
        let resources = lua.resources();
        let world = resources.fetch::<World>();

        let spatial = rlua_serde::to_value(lua, &*world.get::<Spatial>(instance)?)?;
        Ok(Some(lua.create_table_from(vec![("Spatial", spatial)])?))
    }

    fn from_table<'lua>(&self, lua: LuaContext<'lua>, table: LuaTable<'lua>) -> Result<Entity> {
        let spatial: Spatial = rlua_serde::from_value(table.get::<_, LuaValue<'lua>>("Spatial")?)?;
        let sprite_idx = SpriteIndex {
            idx: lua
                .resources()
                .fetch_mut::<SpriteBatch>()
                .add((na::Point2::from(spatial.pos),)),
        };
        Ok(lua
            .resources()
            .fetch_mut::<World>()
            .spawn((spatial, sprite_idx)))
    }
}

inventory::submit! {
    sludge::api::StaticTemplate::new("Bullet", BulletTemplate)
}

#[derive(Debug, Serialize, Deserialize)]
struct Spatial {
    pos: na::Vector2<f32>,
    vel: na::Vector2<f32>,
    acc: na::Vector2<f32>,
}

impl<'a> SmartComponent<&'a Flags> for Spatial {}

struct SpriteIndex {
    idx: SpriteIdx,
}

impl<'a> SmartComponent<&'a Flags> for SpriteIndex {}

struct MainState {
    space: Space,
    bullet_count: u64,
    canvas: graphics::Canvas,
}

impl MainState {
    pub fn new(ctx: &mut ggez::Context) -> Result<MainState> {
        let mut space = Space::new()?;

        space.refresh()?;

        graphics::set_screen_coordinates(ctx, graphics::Rect::new(0., 0., 320., 240.))?;

        let mut canvas = graphics::Canvas::new(ctx, 320, 240, ggez::conf::NumSamples::One)?;
        canvas.set_filter(graphics::FilterMode::Nearest);

        let batch = graphics::spritebatch::SpriteBatch::new(graphics::Image::solid(
            ctx,
            1,
            graphics::WHITE,
        )?);

        space.resources().borrow_mut().insert(batch);

        space.lua().context(|lua| -> Result<_> {
            lua.load(include_str!("main.lua")).exec()?;
            Ok(())
        })?;

        Ok(MainState {
            space,
            bullet_count: 0,
            canvas,
        })
    }
}

impl EventHandler for MainState {
    fn update(&mut self, ctx: &mut ggez::Context) -> ggez::GameResult<()> {
        const DT: f32 = 1. / 60.;
        let Self { space, .. } = self;

        space.lua().context(|lua| {
            while timer::check_update_time(ctx, 60) {
                space
                    .fetch_mut::<Scheduler>()
                    .with_context(lua)
                    .update(1.0)
                    .unwrap();

                let world = space.fetch::<World>();
                let mut batch = space.fetch_mut::<SpriteBatch>();

                for (_e, (mut spatial, sprite_index)) in
                    world.query::<(&mut Spatial, &SpriteIndex)>().iter()
                {
                    let spatial = &mut *spatial;
                    spatial.vel += spatial.acc * DT;
                    spatial.pos += spatial.vel * DT;

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
        graphics::draw(
            ctx,
            &*self.space.fetch::<SpriteBatch>(),
            (na::Point2::origin(),),
        )?;
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

    let (mut ctx, mut event_loop) = ggez::ContextBuilder::new("templates", "Sean Leffler")
        .window_setup(ggez::conf::WindowSetup::default().title("Templates!"))
        .window_mode(ggez::conf::WindowMode::default().dimensions(1280., 960.))
        .build()?;

    let mut main_state = MainState::new(&mut ctx)?;

    match event::run(&mut ctx, &mut event_loop, &mut main_state) {
        Ok(_) => println!("Exited cleanly."),
        Err(e) => println!("Error occured: {}", e),
    }

    Ok(())
}
