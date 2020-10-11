use crate::{
    conf::Conf,
    graphics::Graphics,
    input::{KeyCode, KeyMods, MouseButton},
    SludgeResultExt,
};
use {anyhow::*, miniquad as mq};

pub trait EventHandler: Sized + 'static {
    fn init(ctx: Graphics) -> Result<Self>;
    fn update(&mut self) -> Result<()>;
    fn draw(&mut self) -> Result<()>;

    fn key_down_event(&mut self, _keycode: KeyCode, _keymods: KeyMods, _repeat: bool) {}
    fn key_up_event(&mut self, _keycode: KeyCode, _keymods: KeyMods) {}
    fn mouse_motion_event(&mut self, _x: f32, _y: f32) {}
    fn mouse_wheel_event(&mut self, _x: f32, _y: f32) {}
    fn mouse_button_down_event(&mut self, _button: MouseButton, _x: f32, _y: f32) {}
    fn mouse_button_up_event(&mut self, _button: MouseButton, _x: f32, _y: f32) {}
}

pub struct MqHandler<H: EventHandler> {
    handler: H,
}

impl<H: EventHandler> MqHandler<H> {
    pub fn new(ctx: mq::Context) -> Self {
        let context = Graphics::new(ctx)
            .log_error_err(module_path!())
            .expect("error creating miniquad context");
        Self {
            handler: H::init(context)
                .log_error_err(module_path!())
                .expect("error initializing event handler"),
        }
    }
}

impl<H: EventHandler> mq::EventHandlerFree for MqHandler<H> {
    fn update(&mut self) {
        self.handler.update().unwrap();
    }

    fn draw(&mut self) {
        self.handler.draw().unwrap();
    }

    fn resize_event(&mut self, _width: f32, _height: f32) {}

    fn mouse_motion_event(&mut self, x: f32, y: f32) {
        self.handler.mouse_motion_event(x, y);
    }

    fn mouse_wheel_event(&mut self, x: f32, y: f32) {
        self.handler.mouse_wheel_event(x, y);
    }

    fn mouse_button_down_event(&mut self, button: mq::MouseButton, x: f32, y: f32) {
        self.handler
            .mouse_button_down_event(MouseButton::from(button), x, y)
    }

    fn mouse_button_up_event(&mut self, button: mq::MouseButton, x: f32, y: f32) {
        self.handler
            .mouse_button_up_event(MouseButton::from(button), x, y);
    }

    fn char_event(&mut self, _character: char, _keymods: mq::KeyMods, _repeat: bool) {}

    fn key_down_event(&mut self, keycode: mq::KeyCode, keymods: mq::KeyMods, repeat: bool) {
        self.handler
            .key_down_event(KeyCode::from(keycode), KeyMods::from(keymods), repeat);
    }

    fn key_up_event(&mut self, keycode: mq::KeyCode, keymods: mq::KeyMods) {
        self.handler
            .key_up_event(KeyCode::from(keycode), KeyMods::from(keymods));
    }

    /// Default implementation emulates mouse clicks
    fn touch_event(&mut self, phase: mq::TouchPhase, _id: u64, x: f32, y: f32) {
        if phase == mq::TouchPhase::Started {
            self.mouse_button_down_event(mq::MouseButton::Left, x, y);
        }

        if phase == mq::TouchPhase::Ended {
            self.mouse_button_up_event(mq::MouseButton::Left, x, y);
        }

        if phase == mq::TouchPhase::Moved {
            self.mouse_motion_event(x, y);
        }
    }

    /// Represents raw hardware mouse motion event
    /// Note that these events are delivered regardless of input focus and not in pixels, but in
    /// hardware units instead. And those units may be different from pixels depending on the target platform
    fn raw_mouse_motion(&mut self, _dx: f32, _dy: f32) {}

    /// This event is sent when the userclicks the window's close button
    /// or application code calls the ctx.request_quit() function. The event
    /// handler callback code can handle this event by calling
    /// ctx.cancel_quit() to cancel the quit.
    /// If the event is ignored, the application will quit as usual.
    fn quit_requested_event(&mut self) {}
}

pub fn run<T: EventHandler>(conf: Conf) {
    let mq_conf = mq::conf::Conf {
        window_title: conf.window_title,
        window_width: conf.window_width as i32,
        window_height: conf.window_height as i32,
        ..mq::conf::Conf::default()
    };

    mq::start(mq_conf, |ctx| mq::UserData::free(MqHandler::<T>::new(ctx)));
}
