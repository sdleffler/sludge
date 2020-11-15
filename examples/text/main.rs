extern crate sludge as sloodge;

use {
    anyhow::*,
    sloodge::{
        conf::Conf, event::EventHandler, graphics::*, prelude::*, graphics::text::*
    },
};

mod sludge {
    pub use ::sludge::sludge::*;
}

struct MainState {
    gfx: Graphics,
    text: Text,
}

impl MainState {
    pub fn new(mut gfx: Graphics) -> Result<MainState> {
        let atlas = FontAtlas::new(&mut gfx, &include_bytes!("font.ttf")[..], 40.0, CharacterListType::Ascii)?;
        let text = Text::new(&mut gfx, "Hello World!", &atlas, Color::GREEN);

        Ok(MainState {
            gfx,
            text,
        })
    }
}

impl EventHandler for MainState {
    type Args = ();

    fn init(ctx: Graphics, _: ()) -> Result<Self> {
        Self::new(ctx)
    }

    fn update(&mut self) -> Result<()> {
        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let Self {
            gfx, ..
        } = self;

        gfx.set_projection(Orthographic3::new(0., 320., 240., 0., -1., 1.));
        gfx.begin_default_pass(PassAction::default());
        gfx.apply_default_pipeline();
        gfx.apply_transforms();
        gfx.draw(&self.text, InstanceParam::new().translate2(Vector2::new(20., 140.)));
        gfx.end_pass();
        gfx.commit_frame();
        Ok(())
    }
}

fn main() -> Result<()> {
    sloodge::event::run::<MainState>(
        Conf {
            window_title: "Hello world!".to_string(),
            window_width: 320 * 4,
            window_height: 240 * 4,
            ..Conf::default()
        },
        (),
    );

    Ok(())
}
