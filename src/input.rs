//! An abstract input state object that gets fed user
//! events and updates itself based on a set of key
//! bindings.
//!
//! The goals are:
//!
//! * Have a layer of abstract key bindings rather than
//! looking at concrete event types
//! * Use this to be able to abstract away differences
//! between keyboards, joysticks and game controllers
//! (rather based on Unity3D),
//! * Do some tweening of input axes and stuff just for
//! fun.
//! * Take ggez's event-based input API, and present event- or
//! state-based API so you can do whichever you want.

// TODO: Handle mice, game pads, joysticks

use crate::math::*;
use {hashbrown::HashMap, std::hash::Hash};

// Okay, but how does it actually work?
// Basically we have to bind input events to buttons and axes.
// Input events can be keys, mouse buttons/motion, or eventually
// joystick/controller inputs.  Mouse delta can be mapped to axes too.
//
// https://docs.unity3d.com/Manual/ConventionalGameInput.html has useful
// descriptions of the exact behavior of axes.
//
// So to think about this more clearly, here are the default bindings:
//
// W, ↑: +Y axis
// A, ←: -X axis
// S, ↓: -Y axis
// D, →: +X axis
// Enter, z, LMB: Button 1
// Shift, x, MMB: Button 2
// Ctrl,  c, RMB: Button 3
//
// Easy way?  Hash map of event -> axis/button bindings.

#[derive(Debug, Copy, Clone, PartialEq, Hash, Eq)]
#[repr(u32)]
pub enum KeyCode {
    Space,
    Apostrophe,
    Comma,
    Minus,
    Period,
    Slash,
    Key0,
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,
    Semicolon,
    Equal,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    LeftBracket,
    Backslash,
    RightBracket,
    GraveAccent,
    World1,
    World2,
    Escape,
    Enter,
    Tab,
    Backspace,
    Insert,
    Delete,
    Right,
    Left,
    Down,
    Up,
    PageUp,
    PageDown,
    Home,
    End,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    Kp0,
    Kp1,
    Kp2,
    Kp3,
    Kp4,
    Kp5,
    Kp6,
    Kp7,
    Kp8,
    Kp9,
    KpDecimal,
    KpDivide,
    KpMultiply,
    KpSubtract,
    KpAdd,
    KpEnter,
    KpEqual,
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    Menu,
    Unknown,
}

impl From<miniquad::KeyCode> for KeyCode {
    fn from(kc: miniquad::KeyCode) -> Self {
        use miniquad::KeyCode as MqKc;
        use KeyCode as SlKc;

        match kc {
            MqKc::Space => SlKc::Space,
            MqKc::Apostrophe => SlKc::Apostrophe,
            MqKc::Comma => SlKc::Comma,
            MqKc::Minus => SlKc::Minus,
            MqKc::Period => SlKc::Period,
            MqKc::Slash => SlKc::Slash,
            MqKc::Key0 => SlKc::Key0,
            MqKc::Key1 => SlKc::Key1,
            MqKc::Key2 => SlKc::Key2,
            MqKc::Key3 => SlKc::Key3,
            MqKc::Key4 => SlKc::Key4,
            MqKc::Key5 => SlKc::Key5,
            MqKc::Key6 => SlKc::Key6,
            MqKc::Key7 => SlKc::Key7,
            MqKc::Key8 => SlKc::Key8,
            MqKc::Key9 => SlKc::Key9,
            MqKc::Semicolon => SlKc::Semicolon,
            MqKc::Equal => SlKc::Equal,
            MqKc::A => SlKc::A,
            MqKc::B => SlKc::B,
            MqKc::C => SlKc::C,
            MqKc::D => SlKc::D,
            MqKc::E => SlKc::E,
            MqKc::F => SlKc::F,
            MqKc::G => SlKc::G,
            MqKc::H => SlKc::H,
            MqKc::I => SlKc::I,
            MqKc::J => SlKc::J,
            MqKc::K => SlKc::K,
            MqKc::L => SlKc::L,
            MqKc::M => SlKc::M,
            MqKc::N => SlKc::N,
            MqKc::O => SlKc::O,
            MqKc::P => SlKc::P,
            MqKc::Q => SlKc::Q,
            MqKc::R => SlKc::R,
            MqKc::S => SlKc::S,
            MqKc::T => SlKc::T,
            MqKc::U => SlKc::U,
            MqKc::V => SlKc::V,
            MqKc::W => SlKc::W,
            MqKc::X => SlKc::X,
            MqKc::Y => SlKc::Y,
            MqKc::Z => SlKc::Z,
            MqKc::LeftBracket => SlKc::LeftBracket,
            MqKc::Backslash => SlKc::Backslash,
            MqKc::RightBracket => SlKc::RightBracket,
            MqKc::GraveAccent => SlKc::GraveAccent,
            MqKc::World1 => SlKc::World1,
            MqKc::World2 => SlKc::World2,
            MqKc::Escape => SlKc::Escape,
            MqKc::Enter => SlKc::Enter,
            MqKc::Tab => SlKc::Tab,
            MqKc::Backspace => SlKc::Backspace,
            MqKc::Insert => SlKc::Insert,
            MqKc::Delete => SlKc::Delete,
            MqKc::Right => SlKc::Right,
            MqKc::Left => SlKc::Left,
            MqKc::Down => SlKc::Down,
            MqKc::Up => SlKc::Up,
            MqKc::PageUp => SlKc::PageUp,
            MqKc::PageDown => SlKc::PageDown,
            MqKc::Home => SlKc::Home,
            MqKc::End => SlKc::End,
            MqKc::CapsLock => SlKc::CapsLock,
            MqKc::ScrollLock => SlKc::ScrollLock,
            MqKc::NumLock => SlKc::NumLock,
            MqKc::PrintScreen => SlKc::PrintScreen,
            MqKc::Pause => SlKc::Pause,
            MqKc::F1 => SlKc::F1,
            MqKc::F2 => SlKc::F2,
            MqKc::F3 => SlKc::F3,
            MqKc::F4 => SlKc::F4,
            MqKc::F5 => SlKc::F5,
            MqKc::F6 => SlKc::F6,
            MqKc::F7 => SlKc::F7,
            MqKc::F8 => SlKc::F8,
            MqKc::F9 => SlKc::F9,
            MqKc::F10 => SlKc::F10,
            MqKc::F11 => SlKc::F11,
            MqKc::F12 => SlKc::F12,
            MqKc::F13 => SlKc::F13,
            MqKc::F14 => SlKc::F14,
            MqKc::F15 => SlKc::F15,
            MqKc::F16 => SlKc::F16,
            MqKc::F17 => SlKc::F17,
            MqKc::F18 => SlKc::F18,
            MqKc::F19 => SlKc::F19,
            MqKc::F20 => SlKc::F20,
            MqKc::F21 => SlKc::F21,
            MqKc::F22 => SlKc::F22,
            MqKc::F23 => SlKc::F23,
            MqKc::F24 => SlKc::F24,
            MqKc::F25 => SlKc::F25,
            MqKc::Kp0 => SlKc::Kp0,
            MqKc::Kp1 => SlKc::Kp1,
            MqKc::Kp2 => SlKc::Kp2,
            MqKc::Kp3 => SlKc::Kp3,
            MqKc::Kp4 => SlKc::Kp4,
            MqKc::Kp5 => SlKc::Kp5,
            MqKc::Kp6 => SlKc::Kp6,
            MqKc::Kp7 => SlKc::Kp7,
            MqKc::Kp8 => SlKc::Kp8,
            MqKc::Kp9 => SlKc::Kp9,
            MqKc::KpDecimal => SlKc::KpDecimal,
            MqKc::KpDivide => SlKc::KpDivide,
            MqKc::KpMultiply => SlKc::KpMultiply,
            MqKc::KpSubtract => SlKc::KpSubtract,
            MqKc::KpAdd => SlKc::KpAdd,
            MqKc::KpEnter => SlKc::KpEnter,
            MqKc::KpEqual => SlKc::KpEqual,
            MqKc::LeftShift => SlKc::LeftShift,
            MqKc::LeftControl => SlKc::LeftControl,
            MqKc::LeftAlt => SlKc::LeftAlt,
            MqKc::LeftSuper => SlKc::LeftSuper,
            MqKc::RightShift => SlKc::RightShift,
            MqKc::RightControl => SlKc::RightControl,
            MqKc::RightAlt => SlKc::RightAlt,
            MqKc::RightSuper => SlKc::RightSuper,
            MqKc::Menu => SlKc::Menu,
            MqKc::Unknown => SlKc::Unknown,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct KeyMods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl From<miniquad::KeyMods> for KeyMods {
    fn from(km: miniquad::KeyMods) -> Self {
        Self {
            shift: km.shift,
            ctrl: km.ctrl,
            alt: km.alt,
            logo: km.logo,
        }
    }
}

#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl From<miniquad::MouseButton> for MouseButton {
    fn from(mq: miniquad::MouseButton) -> Self {
        use miniquad::MouseButton as MqMb;
        use MouseButton as SlMb;

        match mq {
            MqMb::Left => SlMb::Left,
            MqMb::Right => SlMb::Right,
            MqMb::Middle => SlMb::Middle,
            MqMb::Unknown => panic!("AAAAAAAAAAAAAA"),
        }
    }
}

#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
enum InputType {
    KeyEvent(KeyCode),
    MouseButtonEvent(MouseButton),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum InputEffect<Axes, Buttons>
where
    Axes: Eq + Hash + Clone,
    Buttons: Eq + Hash + Clone,
{
    Axis(Axes, bool),
    Button(Buttons),
    Cursor(Point2<f32>),
}

#[derive(Debug, Copy, Clone)]
struct CursorState {
    // Where the cursor currently is.
    position: Point2<f32>,
    // Where the cursor was last frame.
    last_position: Point2<f32>,
    // The difference between the current position and the position last update.
    delta: Vector2<f32>,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            position: Point2::origin(),
            last_position: Point2::origin(),
            delta: Vector2::zeros(),
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct AxisState {
    // Where the axis currently is, in [-1, 1]
    position: f32,
    // Where the axis is moving towards.  Possible
    // values are -1, 0, +1
    // (or a continuous range for analog devices I guess)
    direction: f32,
    // Speed in units per second that the axis
    // moves towards the target value.
    acceleration: f32,
    // Speed in units per second that the axis will
    // fall back toward 0 if the input stops.
    gravity: f32,
}

impl Default for AxisState {
    fn default() -> Self {
        AxisState {
            position: 0.0,
            direction: 0.0,
            acceleration: 16.0,
            gravity: 12.0,
        }
    }
}

#[derive(Debug, Copy, Clone, Default)]
struct ButtonState {
    pressed: bool,
    pressed_last_frame: bool,
}

/// A struct that contains a mapping from physical input events
/// (currently just `KeyCode`s) to whatever your logical Axis/Button
/// types are.
pub struct InputBinding<Axes, Buttons>
where
    Axes: Hash + Eq + Clone,
    Buttons: Hash + Eq + Clone,
{
    // Once EnumSet is stable it should be used for these
    // instead of BTreeMap. ♥?
    // Binding of keys to input values.
    bindings: HashMap<InputType, InputEffect<Axes, Buttons>>,
}

impl<Axes, Buttons> InputBinding<Axes, Buttons>
where
    Axes: Hash + Eq + Clone,
    Buttons: Hash + Eq + Clone,
{
    pub fn new() -> Self {
        InputBinding {
            bindings: HashMap::new(),
        }
    }

    /// Adds a key binding connecting the given keycode to the given
    /// logical axis.
    pub fn bind_key_to_axis(mut self, keycode: KeyCode, axis: Axes, positive: bool) -> Self {
        self.bindings.insert(
            InputType::KeyEvent(keycode),
            InputEffect::Axis(axis.clone(), positive),
        );
        self
    }

    /// Adds a key binding connecting the given keycode to the given
    /// logical button.
    pub fn bind_key_to_button(mut self, keycode: KeyCode, button: Buttons) -> Self {
        self.bindings.insert(
            InputType::KeyEvent(keycode),
            InputEffect::Button(button.clone()),
        );
        self
    }

    pub fn bind_mouse_to_button(mut self, mouse_button: MouseButton, button: Buttons) -> Self {
        self.bindings.insert(
            InputType::MouseButtonEvent(mouse_button),
            InputEffect::Button(button.clone()),
        );
        self
    }

    /// Takes an physical input type and turns it into a logical input type (keycode -> axis/button).
    pub fn resolve(&self, keycode: KeyCode) -> Option<InputEffect<Axes, Buttons>> {
        self.bindings.get(&InputType::KeyEvent(keycode)).cloned()
    }
}

#[derive(Debug)]
pub struct InputState<Axes, Buttons>
where
    Axes: Hash + Eq + Clone,
    Buttons: Hash + Eq + Clone,
{
    // Input state for axes
    axes: HashMap<Axes, AxisState>,
    // Input states for buttons
    buttons: HashMap<Buttons, ButtonState>,
    // Input state for the mouse cursor
    mouse: CursorState,
}

impl<Axes, Buttons> InputState<Axes, Buttons>
where
    Axes: Eq + Hash + Clone,
    Buttons: Eq + Hash + Clone,
{
    pub fn new() -> Self {
        InputState {
            axes: HashMap::new(),
            buttons: HashMap::new(),
            mouse: CursorState::default(),
        }
    }

    /// Updates the logical input state based on the actual
    /// physical input state.  Should be called in your update()
    /// handler.
    /// So, it will do things like move the axes and so on.
    pub fn update(&mut self, dt: f32) {
        for (_axis, axis_status) in self.axes.iter_mut() {
            if axis_status.direction != 0.0 {
                // Accelerate the axis towards the
                // input'ed direction.
                let vel = axis_status.acceleration * dt;
                let pending_position = axis_status.position
                    + if axis_status.direction > 0.0 {
                        vel
                    } else {
                        -vel
                    };
                axis_status.position = if pending_position > 1.0 {
                    1.0
                } else if pending_position < -1.0 {
                    -1.0
                } else {
                    pending_position
                }
            } else {
                // Gravitate back towards 0.
                let abs_dx = f32::min(axis_status.gravity * dt, f32::abs(axis_status.position));
                let dx = if axis_status.position > 0.0 {
                    -abs_dx
                } else {
                    abs_dx
                };
                axis_status.position += dx;
            }
        }

        for (_button, button_status) in self.buttons.iter_mut() {
            button_status.pressed_last_frame = button_status.pressed;
        }

        self.mouse.delta = self.mouse.position - self.mouse.last_position;
        self.mouse.last_position = self.mouse.position;
    }

    /// This method should get called by your key_down_event handler.
    pub fn update_button_down(&mut self, button: Buttons) {
        self.update_effect(InputEffect::Button(button), true);
    }

    /// This method should get called by your key_up_event handler.
    pub fn update_button_up(&mut self, button: Buttons) {
        self.update_effect(InputEffect::Button(button), false);
    }

    /// This method should get called by your key_up_event handler.
    pub fn update_axis_start(&mut self, axis: Axes, positive: bool) {
        self.update_effect(InputEffect::Axis(axis, positive), true);
    }

    pub fn update_axis_stop(&mut self, axis: Axes, positive: bool) {
        self.update_effect(InputEffect::Axis(axis, positive), false);
    }

    /// This method should be called by your mouse_motion_event handler.
    pub fn update_mouse_position(&mut self, position: Point2<f32>) {
        self.update_effect(InputEffect::Cursor(position), false);
    }

    /// Takes an InputEffect and actually applies it.
    pub fn update_effect(&mut self, effect: InputEffect<Axes, Buttons>, started: bool) {
        match effect {
            InputEffect::Axis(axis, positive) => {
                let f = || AxisState::default();
                let axis_status = self.axes.entry(axis).or_insert_with(f);
                if started {
                    let direction_float = if positive { 1.0 } else { -1.0 };
                    axis_status.direction = direction_float;
                } else if (positive && axis_status.direction > 0.0)
                    || (!positive && axis_status.direction < 0.0)
                {
                    axis_status.direction = 0.0;
                }
            }
            InputEffect::Button(button) => {
                let f = || ButtonState::default();
                let button_status = self.buttons.entry(button).or_insert_with(f);
                button_status.pressed = started;
            }
            InputEffect::Cursor(position) => {
                self.mouse.position = position;
            }
        }
    }

    pub fn get_axis(&self, axis: Axes) -> f32 {
        let d = AxisState::default();
        let axis_status = self.axes.get(&axis).unwrap_or(&d);
        axis_status.position
    }

    pub fn get_axis_raw(&self, axis: Axes) -> f32 {
        let d = AxisState::default();
        let axis_status = self.axes.get(&axis).unwrap_or(&d);
        axis_status.direction
    }

    fn get_button(&self, button: Buttons) -> ButtonState {
        let d = ButtonState::default();
        let button_status = self.buttons.get(&button).unwrap_or(&d);
        *button_status
    }

    pub fn get_button_down(&self, axis: Buttons) -> bool {
        self.get_button(axis).pressed
    }

    pub fn get_button_up(&self, axis: Buttons) -> bool {
        !self.get_button(axis).pressed
    }

    /// Returns whether or not the button was pressed this frame,
    /// only returning true if the press happened this frame.
    ///
    /// Basically, `get_button_down()` and `get_button_up()` are level
    /// triggers, this and `get_button_released()` are edge triggered.
    pub fn get_button_pressed(&self, axis: Buttons) -> bool {
        let b = self.get_button(axis);
        b.pressed && !b.pressed_last_frame
    }

    pub fn get_button_released(&self, axis: Buttons) -> bool {
        let b = self.get_button(axis);
        !b.pressed && b.pressed_last_frame
    }

    pub fn mouse_position(&self) -> Point2<f32> {
        self.mouse.position
    }

    pub fn mouse_delta(&self) -> Vector2<f32> {
        self.mouse.delta
    }

    pub fn reset_input_state(&mut self) {
        for (_axis, axis_status) in self.axes.iter_mut() {
            axis_status.position = 0.0;
            axis_status.direction = 0.0;
        }

        for (_button, button_status) in self.buttons.iter_mut() {
            button_status.pressed = false;
            button_status.pressed_last_frame = false;
        }

        self.mouse.position = Point2::origin();
        self.mouse.last_position = Point2::origin();
        self.mouse.delta = Vector2::zeros();
    }
}

#[cfg(feature = "ggez")]
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Hash, Eq, PartialEq, Copy, Clone, Debug)]
    enum Buttons {
        A,
        B,
        Select,
        Start,
    }

    #[derive(Hash, Eq, PartialEq, Copy, Clone, Debug)]
    enum Axes {
        Horz,
        Vert,
    }

    fn make_input_binding() -> InputBinding<Axes, Buttons> {
        let ib = InputBinding::<Axes, Buttons>::new()
            .bind_key_to_button(KeyCode::Z, Buttons::A)
            .bind_key_to_button(KeyCode::X, Buttons::B)
            .bind_key_to_button(KeyCode::Enter, Buttons::Start)
            .bind_key_to_button(KeyCode::RightShift, Buttons::Select)
            .bind_key_to_button(KeyCode::LeftShift, Buttons::Select)
            .bind_key_to_axis(KeyCode::Up, Axes::Vert, true)
            .bind_key_to_axis(KeyCode::Down, Axes::Vert, false)
            .bind_key_to_axis(KeyCode::Left, Axes::Horz, false)
            .bind_key_to_axis(KeyCode::Right, Axes::Horz, true);
        ib
    }

    #[test]
    fn test_input_bindings() {
        let ib = make_input_binding();
        assert_eq!(
            ib.resolve(KeyCode::Z),
            Some(InputEffect::Button(Buttons::A))
        );
        assert_eq!(
            ib.resolve(KeyCode::X),
            Some(InputEffect::Button(Buttons::B))
        );
        assert_eq!(
            ib.resolve(KeyCode::Enter),
            Some(InputEffect::Button(Buttons::Start))
        );
        assert_eq!(
            ib.resolve(KeyCode::RightShift),
            Some(InputEffect::Button(Buttons::Select))
        );
        assert_eq!(
            ib.resolve(KeyCode::LeftShift),
            Some(InputEffect::Button(Buttons::Select))
        );

        assert_eq!(
            ib.resolve(KeyCode::Up),
            Some(InputEffect::Axis(Axes::Vert, true))
        );
        assert_eq!(
            ib.resolve(KeyCode::Down),
            Some(InputEffect::Axis(Axes::Vert, false))
        );
        assert_eq!(
            ib.resolve(KeyCode::Left),
            Some(InputEffect::Axis(Axes::Horz, false))
        );
        assert_eq!(
            ib.resolve(KeyCode::Right),
            Some(InputEffect::Axis(Axes::Horz, true))
        );

        assert_eq!(ib.resolve(KeyCode::Q), None);
        assert_eq!(ib.resolve(KeyCode::W), None);
    }

    #[test]
    fn test_input_events() {
        let mut im = InputState::new();
        im.update_button_down(Buttons::A);
        assert!(im.get_button_down(Buttons::A));
        im.update_button_up(Buttons::A);
        assert!(!im.get_button_down(Buttons::A));
        assert!(im.get_button_up(Buttons::A));

        // Push the 'up' button, watch the axis
        // increase to 1.0 but not beyond
        im.update_axis_start(Axes::Vert, true);
        assert!(im.get_axis_raw(Axes::Vert) > 0.0);
        while im.get_axis(Axes::Vert) < 0.99 {
            im.update(0.16);
            assert!(im.get_axis(Axes::Vert) >= 0.0);
            assert!(im.get_axis(Axes::Vert) <= 1.0);
        }
        // Release it, watch it wind down
        im.update_axis_stop(Axes::Vert, true);
        while im.get_axis(Axes::Vert) > 0.01 {
            im.update(0.16);
            assert!(im.get_axis(Axes::Vert) >= 0.0)
        }

        // Do the same with the 'down' button.
        im.update_axis_start(Axes::Vert, false);
        while im.get_axis(Axes::Vert) > -0.99 {
            im.update(0.16);
            assert!(im.get_axis(Axes::Vert) <= 0.0);
            assert!(im.get_axis(Axes::Vert) >= -1.0);
        }

        // Test the transition from 'up' to 'down'
        im.update_axis_start(Axes::Vert, true);
        while im.get_axis(Axes::Vert) < 1.0 {
            im.update(0.16);
        }
        im.update_axis_start(Axes::Vert, false);
        im.update(0.16);
        assert!(im.get_axis(Axes::Vert) < 1.0);
        im.update_axis_stop(Axes::Vert, true);
        assert!(im.get_axis_raw(Axes::Vert) < 0.0);
        im.update_axis_stop(Axes::Vert, false);
        assert_eq!(im.get_axis_raw(Axes::Vert), 0.0);
    }

    #[test]
    fn test_button_edge_transitions() {
        let mut im: InputState<Axes, Buttons> = InputState::new();

        // Push a key, confirm it's transitioned.
        assert!(!im.get_button_down(Buttons::A));
        im.update_button_down(Buttons::A);
        assert!(im.get_button_down(Buttons::A));
        assert!(im.get_button_pressed(Buttons::A));
        assert!(!im.get_button_released(Buttons::A));

        // Update, confirm it's still down but
        // wasn't pressed this frame
        im.update(0.1);
        assert!(im.get_button_down(Buttons::A));
        assert!(!im.get_button_pressed(Buttons::A));
        assert!(!im.get_button_released(Buttons::A));

        // Release it
        im.update_button_up(Buttons::A);
        assert!(im.get_button_up(Buttons::A));
        assert!(!im.get_button_pressed(Buttons::A));
        assert!(im.get_button_released(Buttons::A));
        im.update(0.1);
        assert!(im.get_button_up(Buttons::A));
        assert!(!im.get_button_pressed(Buttons::A));
        assert!(!im.get_button_released(Buttons::A));
    }
}
