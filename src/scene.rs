//! This is a modified version of the `scene` module from `ggez-goodies`. Please
//! see the licensing information below in the source file.
//!
//! The Scene system is basically for transitioning between
//! *completely* different states that have entirely different game
//! loops and but which all share a state.  It operates as a stack, with new
//! scenes getting pushed to the stack (while the old ones stay in
//! memory unchanged).  Apparently this is basically a push-down automata.
//!
//! Also there's no reason you can't have a Scene contain its own
//! Scene subsystem to do its own indirection.  With a different state
//! type, as well!  What fun!  Though whether you want to go that deep
//! down the rabbit-hole is up to you.  I haven't found it necessary
//! yet.
//!
//! This is basically identical in concept to the Amethyst engine's scene
//! system, the only difference is the details of how the pieces are put
//! together.

/*
 * MIT License
 *
 * Copyright (c) 2016-2018 the ggez developers
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

use {
    anyhow::*,
    atomic_refcell::AtomicRefCell,
    std::{borrow::Cow, fmt, sync::Arc},
};

pub struct DynamicScene<C, Ev>(Arc<AtomicRefCell<dyn Scene<C, Ev>>>);

impl<C, Ev> Clone for DynamicScene<C, Ev> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C, Ev> fmt::Debug for DynamicScene<C, Ev> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let borrowed = self.0.borrow();
        let name = borrowed.name();
        f.debug_tuple("DynamicScene").field(&name).finish()
    }
}

impl<C, Ev> DynamicScene<C, Ev> {
    pub fn new<T>(scene: T) -> Self
    where
        T: Scene<C, Ev> + 'static,
    {
        Self(Arc::new(AtomicRefCell::new(scene)))
    }

    fn map_mut_inner<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut dyn Scene<C, Ev>) -> R,
    {
        match Arc::get_mut(&mut self.0) {
            Some(m) => f(m.get_mut()),
            None => f(&mut *self.0.borrow_mut()),
        }
    }
}

impl<C, Ev> Scene<C, Ev> for DynamicScene<C, Ev> {
    fn update(&mut self, scene_stack: &mut SceneStack<C, Ev>, ctx: &mut C) -> Result<()> {
        self.map_mut_inner(|s| s.update(scene_stack, ctx))
    }

    fn draw(&mut self, ctx: &mut C) -> Result<()> {
        self.map_mut_inner(|s| s.draw(ctx))
    }

    fn event(&mut self, ctx: &mut C, event: Ev) {
        self.map_mut_inner(|s| s.event(ctx, event))
    }

    fn name(&self) -> Cow<'_, str> {
        self.0.borrow().name().into_owned().into()
    }

    fn draw_previous(&self) -> bool {
        self.0.borrow().draw_previous()
    }
}

/// A trait for you to implement on a scene.
/// Defines the callbacks the scene uses:
/// a common context type `C`, and an input event type `Ev`.
pub trait Scene<C, Ev> {
    fn update(&mut self, scene_stack: &mut SceneStack<C, Ev>, ctx: &mut C) -> Result<()>;
    fn draw(&mut self, ctx: &mut C) -> Result<()>;
    fn event(&mut self, ctx: &mut C, event: Ev);
    /// Only used for human-readable convenience (or not at all, tbh)
    fn name(&self) -> Cow<'_, str>;
    /// This returns whether or not to draw the next scene down on the
    /// stack as well; this is useful for layers or GUI stuff that
    /// only partially covers the screen.
    fn draw_previous(&self) -> bool {
        false
    }
}

/// A stack of `Scene`'s, together with a context object.
pub struct SceneStack<C, Ev> {
    scenes: Vec<DynamicScene<C, Ev>>,
}

impl<C, Ev> SceneStack<C, Ev> {
    pub fn new() -> Self {
        Self { scenes: Vec::new() }
    }

    /// Add a new scene to the top of the stack.
    pub fn push(&mut self, scene: DynamicScene<C, Ev>) {
        self.scenes.push(scene)
    }

    /// Remove the top scene from the stack and returns it;
    /// panics if there is none.
    pub fn pop(&mut self) -> DynamicScene<C, Ev> {
        self.scenes
            .pop()
            .expect("ERROR: Popped an empty scene stack.")
    }

    /// Replace the top scene on the stack by popping and then
    /// pushing a new scene. Will panic if the stack is empty.
    /// Returns the replaced scene.
    pub fn replace(&mut self, scene: DynamicScene<C, Ev>) -> DynamicScene<C, Ev> {
        let replaced = self.pop();
        self.push(scene);
        replaced
    }

    /// Returns the current scene; panics if there is none.
    pub fn current(&self) -> &DynamicScene<C, Ev> {
        self.scenes
            .last()
            .expect("ERROR: Tried to get current scene of an empty scene stack.")
    }

    // These functions must be on the SceneStack because otherwise
    // if you try to get the current scene and the world to call
    // update() on the current scene it causes a double-borrow.  :/
    pub fn update(&mut self, ctx: &mut C) -> Result<()> {
        let mut current_scene = self
            .scenes
            .last()
            .cloned()
            .expect("Tried to update empty scene stack");
        current_scene.update(self, ctx)
    }

    /// We walk down the scene stack until we find a scene where we aren't
    /// supposed to draw the previous one, then draw them from the bottom up.
    ///
    /// This allows for layering GUI's and such.
    fn draw_scenes(scenes: &mut [DynamicScene<C, Ev>], ctx: &mut C) -> Result<()> {
        assert!(scenes.len() > 0);
        if let Some((current, rest)) = scenes.split_last_mut() {
            if current.draw_previous() {
                SceneStack::draw_scenes(rest, ctx)?;
            }
            current.draw(ctx)
        } else {
            Ok(())
        }
    }

    /// Draw the current scene.
    pub fn draw(&mut self, ctx: &mut C) -> Result<()> {
        SceneStack::draw_scenes(&mut self.scenes, ctx)
    }

    /// Feeds the given event to the current scene.
    pub fn event(&mut self, ctx: &mut C, event: Ev) {
        let current_scene = self
            .scenes
            .last_mut()
            .expect("Tried to do input for empty scene stack");
        current_scene.event(ctx, event);
    }
}
