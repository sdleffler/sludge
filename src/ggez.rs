use {
    anyhow::*,
    fragile::Sticky,
    ggez::{
        graphics::{spritebatch::SpriteBatch, Image},
        Context,
    },
    std::{
        cell::{Ref, RefCell, RefMut},
        io::Read,
        ops,
        sync::Arc,
    },
};

use crate::{
    filesystem::Filesystem,
    resources::{Inspect, Key, Load, Loaded, Storage},
    SharedResources,
};

#[derive(Debug, Clone)]
pub struct SharedContext {
    inner: Arc<Sticky<RefCell<Context>>>,
}

impl From<Context> for SharedContext {
    fn from(ctx: Context) -> Self {
        Self {
            inner: Arc::new(Sticky::new(RefCell::new(ctx))),
        }
    }
}

impl SharedContext {
    pub fn borrow(&self) -> Ref<Context> {
        self.inner.get().borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<Context> {
        self.inner.get().borrow_mut()
    }
}

use gilrs;
use winit::{self, dpi};

// TODO LATER: I kinda hate all these re-exports.  I kinda hate
// a lot of the details of the `EventHandler` and input now though,
// and look forward to ripping it all out and replacing it with newer winit.

/// A mouse button.
pub use winit::MouseButton;

/// An analog axis of some device (gamepad thumbstick, joystick...).
pub use gilrs::Axis;
/// A button of some device (gamepad, joystick...).
pub use gilrs::Button;

/// `winit` events; nested in a module for re-export neatness.
pub mod winit_event {
    pub use super::winit::{
        DeviceEvent, ElementState, Event, KeyboardInput, ModifiersState, MouseScrollDelta,
        TouchPhase, WindowEvent,
    };
}
pub use ggez::input::gamepad::GamepadId;
pub use ggez::input::keyboard::{KeyCode, KeyMods};

use self::winit_event::*;
/// `winit` event loop.
pub use winit::EventsLoop;

/// A trait defining event callbacks.  This is your primary interface with
/// `ggez`'s event loop.  Implement this trait for a type and
/// override at least the [`update()`](#tymethod.update) and
/// [`draw()`](#tymethod.draw) methods, then pass it to
/// [`event::run()`](fn.run.html) to run the game's mainloop.
///
/// The default event handlers do nothing, apart from
/// [`key_down_event()`](#tymethod.key_down_event), which will by
/// default exit the game if the escape key is pressed.  Just
/// override the methods you want to use.
pub trait EventHandler {
    /// Called upon each logic update to the game.
    /// This should be where the game's logic takes place.
    fn update(&mut self, _ctx: &SharedContext) -> Result<()>;

    /// Called to do the drawing of your game.
    /// You probably want to start this with
    /// [`graphics::clear()`](../graphics/fn.clear.html) and end it
    /// with [`graphics::present()`](../graphics/fn.present.html) and
    /// maybe [`timer::yield_now()`](../timer/fn.yield_now.html).
    fn draw(&mut self, _ctx: &SharedContext) -> Result<()>;

    /// A mouse button was pressed
    fn mouse_button_down_event(
        &mut self,
        _ctx: &SharedContext,
        _button: MouseButton,
        _x: f32,
        _y: f32,
    ) {
    }

    /// A mouse button was released
    fn mouse_button_up_event(
        &mut self,
        _ctx: &SharedContext,
        _button: MouseButton,
        _x: f32,
        _y: f32,
    ) {
    }

    /// The mouse was moved; it provides both absolute x and y coordinates in the window,
    /// and relative x and y coordinates compared to its last position.
    fn mouse_motion_event(&mut self, _ctx: &SharedContext, _x: f32, _y: f32, _dx: f32, _dy: f32) {}

    /// The mousewheel was scrolled, vertically (y, positive away from and negative toward the user)
    /// or horizontally (x, positive to the right and negative to the left).
    fn mouse_wheel_event(&mut self, _ctx: &SharedContext, _x: f32, _y: f32) {}

    /// A keyboard button was pressed.
    ///
    /// The default implementation of this will call `ggez::event::quit()`
    /// when the escape key is pressed.  If you override this with
    /// your own event handler you have to re-implment that
    /// functionality yourself.
    fn key_down_event(
        &mut self,
        ctx: &SharedContext,
        keycode: KeyCode,
        _keymods: KeyMods,
        _repeat: bool,
    ) {
        if keycode == KeyCode::Escape {
            quit(ctx);
        }
    }

    /// A keyboard button was released.
    fn key_up_event(&mut self, _ctx: &SharedContext, _keycode: KeyCode, _keymods: KeyMods) {}

    /// A unicode character was received, usually from keyboard input.
    /// This is the intended way of facilitating text input.
    fn text_input_event(&mut self, _ctx: &SharedContext, _character: char) {}

    /// A gamepad button was pressed; `id` identifies which gamepad.
    /// Use [`input::gamepad()`](../input/fn.gamepad.html) to get more info about
    /// the gamepad.
    fn gamepad_button_down_event(&mut self, _ctx: &SharedContext, _btn: Button, _id: GamepadId) {}

    /// A gamepad button was released; `id` identifies which gamepad.
    /// Use [`input::gamepad()`](../input/fn.gamepad.html) to get more info about
    /// the gamepad.
    fn gamepad_button_up_event(&mut self, _ctx: &SharedContext, _btn: Button, _id: GamepadId) {}

    /// A gamepad axis moved; `id` identifies which gamepad.
    /// Use [`input::gamepad()`](../input/fn.gamepad.html) to get more info about
    /// the gamepad.
    fn gamepad_axis_event(
        &mut self,
        _ctx: &SharedContext,
        _axis: Axis,
        _value: f32,
        _id: GamepadId,
    ) {
    }

    /// Called when the window is shown or hidden.
    fn focus_event(&mut self, _ctx: &SharedContext, _gained: bool) {}

    /// Called upon a quit event.  If it returns true,
    /// the game does not exit (the quit event is cancelled).
    fn quit_event(&mut self, _ctx: &SharedContext) -> bool {
        log::debug!("quit_event() callback called, quitting...");
        false
    }

    /// Called when the user resizes the window, or when it is resized
    /// via [`graphics::set_mode()`](../graphics/fn.set_mode.html).
    fn resize_event(&mut self, _ctx: &SharedContext, _width: f32, _height: f32) {}
}

/// Terminates the [`ggez::event::run()`](fn.run.html) loop by setting
/// [`Context.continuing`](struct.Context.html#structfield.continuing)
/// to `false`.
pub fn quit(ctx: &SharedContext) {
    ctx.borrow_mut().continuing = false;
}

/// Runs the game's main loop, calling event callbacks on the given state
/// object as events occur.
///
/// It does not try to do any type of framerate limiting.  See the
/// documentation for the [`timer`](../timer/index.html) module for more info.
pub fn run<S>(ctx: &SharedContext, events_loop: &mut EventsLoop, state: &mut S) -> Result<()>
where
    S: EventHandler,
{
    use ggez::input::{keyboard, mouse};

    while ctx.borrow().continuing {
        // If you are writing your own event loop, make sure
        // you include `timer_context.tick()` and
        // `ctx.process_event()` calls.  These update ggez's
        // internal state however necessary.
        ctx.borrow_mut().timer_context.tick();
        events_loop.poll_events(|event| {
            ctx.borrow_mut().process_event(&event);
            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::Resized(logical_size) => {
                        // let actual_size = logical_size;
                        state.resize_event(
                            ctx,
                            logical_size.width as f32,
                            logical_size.height as f32,
                        );
                    }
                    WindowEvent::CloseRequested => {
                        if !state.quit_event(ctx) {
                            quit(ctx);
                        }
                    }
                    WindowEvent::Focused(gained) => {
                        state.focus_event(ctx, gained);
                    }
                    WindowEvent::ReceivedCharacter(ch) => {
                        state.text_input_event(ctx, ch);
                    }
                    WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                state: ElementState::Pressed,
                                virtual_keycode: Some(keycode),
                                modifiers,
                                ..
                            },
                        ..
                    } => {
                        let repeat = keyboard::is_key_repeated(&*ctx.borrow());
                        state.key_down_event(ctx, keycode, modifiers.into(), repeat);
                    }
                    WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                state: ElementState::Released,
                                virtual_keycode: Some(keycode),
                                modifiers,
                                ..
                            },
                        ..
                    } => {
                        state.key_up_event(ctx, keycode, modifiers.into());
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let (x, y) = match delta {
                            MouseScrollDelta::LineDelta(x, y) => (x, y),
                            MouseScrollDelta::PixelDelta(dpi::LogicalPosition { x, y }) => {
                                (x as f32, y as f32)
                            }
                        };
                        state.mouse_wheel_event(ctx, x, y);
                    }
                    WindowEvent::MouseInput {
                        state: element_state,
                        button,
                        ..
                    } => {
                        let position = mouse::position(&*ctx.borrow());
                        match element_state {
                            ElementState::Pressed => {
                                state.mouse_button_down_event(ctx, button, position.x, position.y)
                            }
                            ElementState::Released => {
                                state.mouse_button_up_event(ctx, button, position.x, position.y)
                            }
                        }
                    }
                    WindowEvent::CursorMoved { .. } => {
                        let position = mouse::position(&*ctx.borrow());
                        let delta = mouse::delta(&*ctx.borrow());
                        state.mouse_motion_event(ctx, position.x, position.y, delta.x, delta.y);
                    }
                    _x => {
                        // trace!("ignoring window event {:?}", x);
                    }
                },
                Event::DeviceEvent { event, .. } => match event {
                    _ => (),
                },
                Event::Awakened => (),
                Event::Suspended(_) => (),
            }
        });

        // // Handle gamepad events if necessary.
        // while let Some(gilrs::Event { id, event, .. }) =
        //     ctx.borrow_mut().gamepad_context.next_event()
        // {
        //     match event {
        //         gilrs::EventType::ButtonPressed(button, _) => {
        //             state.gamepad_button_down_event(ctx, button, id);
        //         }
        //         gilrs::EventType::ButtonReleased(button, _) => {
        //             state.gamepad_button_up_event(ctx, button, id);
        //         }
        //         gilrs::EventType::AxisChanged(axis, value, _) => {
        //             state.gamepad_axis_event(ctx, axis, value, id);
        //         }
        //         _ => {}
        //     }
        // }

        state.update(ctx)?;
        state.draw(ctx)?;
    }

    Ok(())
}

#[derive(Debug)]
pub struct LoadedImage(Image);

impl ops::Deref for LoadedImage {
    type Target = Image;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for LoadedImage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<C> Load<C, Key> for LoadedImage
where
    LoadedImage: for<'a> Inspect<'a, C, &'a SharedResources>,
{
    type Error = Error;

    fn load(key: Key, _storage: &mut Storage<C, Key>, ctx: &mut C) -> Result<Loaded<Self, Key>> {
        match key {
            Key::Path(path) => {
                let resources = Self::inspect(ctx);
                let filesystem = &mut *resources.fetch_mut::<Filesystem>();
                let img = {
                    let mut buf = Vec::new();
                    let mut reader = filesystem.open(&path)?;
                    let _ = reader.read_to_end(&mut buf)?;
                    image::load_from_memory(&buf)?.to_rgba()
                };
                let (width, height) = img.dimensions();
                let shared_ctx = resources.fetch::<SharedContext>();
                let ggez_ctx = &mut *shared_ctx.borrow_mut();
                let img = Image::from_rgba8(ggez_ctx, width as u16, height as u16, &img)?;
                Ok(LoadedImage(img).into())
            }
        }
    }
}

#[derive(Debug)]
pub struct LoadedSpriteBatch(SpriteBatch);

impl ops::Deref for LoadedSpriteBatch {
    type Target = SpriteBatch;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for LoadedSpriteBatch {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<C> Load<C, Key> for LoadedSpriteBatch
where
    LoadedSpriteBatch: for<'a> Inspect<'a, C, &'a SharedResources>,
    LoadedImage: for<'a> Inspect<'a, C, &'a SharedResources>,
{
    type Error = Error;

    fn load(key: Key, storage: &mut Storage<C, Key>, ctx: &mut C) -> Result<Loaded<Self, Key>> {
        let image = storage
            .get::<LoadedImage>(&key, ctx)
            .map_err(|err| anyhow!("error loading image: `{}`", err))?;
        let batch = SpriteBatch::new(image.borrow().clone());
        Ok(LoadedSpriteBatch(batch).into())
    }

    /// When reloading a spritebatch, we have to reinsert all the previous indices, otherwise
    /// the spritebatch will end up cleared after being reloaded.
    fn reload(&self, key: Key, storage: &mut Storage<C, Key>, ctx: &mut C) -> Result<Self> {
        let mut batch = Self::load(key, storage, ctx)?.res.0;
        for (index, param) in self.0.iter() {
            assert_eq!(index, batch.add(param));
        }
        Ok(LoadedSpriteBatch(batch))
    }
}
