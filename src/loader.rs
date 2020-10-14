//! Hot-reloadable resources using `warmy`.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::UnifiedResources;

/// Reexport everything from warmy except for the `Key` trait and `SimpleKey` as we expect
/// users to use our `Key` type.
pub use warmy::{
    Discovery, Inspect, Load, Loaded, Res, Storage, StoreError, StoreErrorOr, StoreOpt,
};

pub type Store = warmy::Store<UnifiedResources<'static>, Key>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    Path(PathBuf),
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl From<&Path> for Key {
    fn from(p: &Path) -> Self {
        Key::Path(p.to_owned())
    }
}

impl Key {
    pub fn from_path<P>(p: P) -> Self
    where
        P: AsRef<Path>,
    {
        Self::Path(p.as_ref().to_owned())
    }
}

impl warmy::Key for Key {
    fn prepare_key(self, _root: &Path) -> Self {
        self
    }
}
