#![deny(missing_docs)]

//! A thread-safe shared type-indexed map for "global" resources.
//!
//! This module provides types which implement [`Resources`](Resources), a trait
//! which allows by-type access to singletons both local to a space
//! (for example an [ECS world](sludge::ecs::World)) and global
//! context types which are shared between all spaces in your program
//! (for example the [graphics context](sludge::graphics::Graphics)).
//!
//! There are currently three different containers for these singleton
//! resources, of which currently only two implement the `Resources`
//! trait itself:
//! - `OwnedResources`, which is the inner type-indexed map, and is at
//!   the core of the other two resource container types;
//! - `SharedResources`, which functions somewhat like an `Arc<OwnedResources>`,
//!   and allows you to share a resource container across threads and
//!   contexts which require `'static`; this type implements `Resources`.
//! - `UnifiedResources`, which combines two `SharedResources` objects
//!   into a single container. The purpose of `UnifiedResources` is to have
//!   separate `SharedResources` objects for "global" singletons like a
//!   graphics or audio context and "local" singletons like an ECS world or
//!   a singleton containing data on which entity is the player, in a game.
//!   This type implements `Resources`.
//!
//! All resource container types are capable of holding borrowed references
//! to values in their surrounding scope; if you're passing resources into
//! Lua, however, you'll probably never be able to use this feature.
//!
//! Accessing types owned by a resource container is done via the `fetch_one`
//! and `fetch` methods, which are on the `Resources` trait for `SharedResources`
//! and `UnifiedResources`, while `OwnedResources` unfortunately cannot currently
//! implement `Resources` and so it has its own `fetch` and `fetch_one` implementations.
//! `fetch_one` allows you to fetch a single type at a time, while `fetch` allows
//! you to access multiple stored global values at once:
//!
//! ```rust
//! use sludge::{
//!     prelude::*,
//!     resources::{OwnedResources, SharedResources, Resources},
//! };
//! # fn main() -> Result<()> {
//! let mut resources = OwnedResources::new();
//! resources.insert::<i32>(5);
//! resources.insert::<&'static str>("hello");
//! resources.insert::<bool>(true);
//!
//! // Using `fetch_one`, we can extract a single value at a time.
//! assert_eq!(*resources.fetch_one::<i32>()?.borrow(), 5);
//!
//! // Fetching a resource gives us a smart pointer to it, which isn't
//! // bound to the lifetime of the resources object. It's like an `Arc`.
//! let fetched_int = resources.fetch_one::<i32>()?;
//!
//! // Kind of like a thread-safe `RefCell`, we can mutably or immutably
//! // borrow a resource value. There are several variations on borrowing
//! // methods; please see the documentation on `Shared` for more info.
//! *fetched_int.borrow_mut() += 1;
//!
//! // Finally, using `fetch`, we can provide a tuple of types to fetch
//! // and we'll get them all at once (or an error if we can't find one.)
//! let (a, b, c) = resources.fetch::<(i32, &'static str, bool)>()?;
//! assert_eq!((*a.borrow(), *b.borrow(), *c.borrow()), (6, "hello", true));
//!
//! # Ok(())
//! # }
//! ```
//!
//! The type that `fetch` and `fetch_one` actually return is [`Shared`](Shared).
//! `Shared` is a smart pointer type around an `RwLock`-based construct, which
//! allows for `RefCell` and `RwLock`-like operations. It's also `Clone` and is
//! not bound to the lifetime of whatever `Resources` type it was created from.
//! You can confidently move around `Shared` instances without worrying about
//! borrowing conflicts, because nothing is borrowed until you actually call a
//! borrowing method on `Shared`.

use {
    anyhow::*,
    atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut},
    im::HashMap,
    rlua::prelude::*,
    std::{
        any::{self, Any, TypeId},
        marker::PhantomData,
        ops,
        pin::Pin,
        ptr::NonNull,
        sync::{
            Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError, TryLockResult,
        },
    },
    thiserror::Error,
};

trait LockResultExt {
    type Output;

    fn handle(self) -> Self::Output;
}

impl<T> LockResultExt for LockResult<T> {
    type Output = T;

    fn handle(self) -> T {
        let t = match self {
            Ok(t) => t,
            Err(p_err) => p_err.into_inner(),
        };

        t
    }
}

impl<T> LockResultExt for TryLockResult<T> {
    type Output = Option<T>;

    fn handle(self) -> Option<T> {
        let t = match self {
            Ok(t) => t,
            Err(TryLockError::Poisoned(p_err)) => p_err.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };

        Some(t)
    }
}

/// The error type returned when a resource type is not found in the map.
/// We return `Result<Shared<'a, T>, NotFound>` to make it more convenient
/// since normally not finding a type is going to be a panic or a crashing
/// error. With `NotFound`, the type you were trying to fetch is recorded
/// and remembered so that you don't have to look through your code trying
/// to figure out what the hell caused this.
///
/// It also implements `Into<LuaError>`, making it very simple to use inside
/// contexts like bindings for Lua code.
#[derive(Debug, Error)]
#[error("resource of type `{0}` not found")]
pub struct NotFound(String);

impl NotFound {
    fn of<T: Fetchable>() -> Self {
        Self(any::type_name::<T>().to_owned())
    }
}

impl From<NotFound> for LuaError {
    fn from(err: NotFound) -> Self {
        LuaError::external(err)
    }
}

/// The type of a shared resource.
///
/// `Shared` is `Clone` regardless of `T`; it acts like an `Arc`. The contained
/// `T` will only be dropped once all `Shared<T>` objects which reference it
/// are destroyed, including the `Shared<T>` contained in the resource type it
/// originated from.
///
/// `Shared<T>` also acts as a lock/guard, like a `RefCell` or `RwLock` (depending
/// on which methods you use to borrow its contents.) For `RefCell` behavior, use
/// `borrow` and `borrow_mut`, which will panic if a borrow would violate Rust's
/// borrowing rules. You may also use `try_borrow` and `try_borrow_mut` which return
/// `None` instead of panicking. And last but not least, `blocking_borrow` and
/// `blocking_borrow_mut` behave similarly to `RwLock`'s `read` and `write` methods,
/// and if a borrow would violate Rust's aliasing rules, it will instead block the
/// thread until it can safely borrow the contents.
#[derive(Debug)]
pub struct Shared<'a, T: 'static> {
    inner: Arc<RwLock<StoredResource<'a>>>,
    _marker: PhantomData<T>,
}

impl<'a, T: 'static> Clone for Shared<'a, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<'a, T: 'static> Shared<'a, T> {
    fn new(inner: Arc<RwLock<StoredResource<'a>>>) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }

    /// Attempt to immutably borrow the contents, panicking if the contents are already mutably
    /// borrowed somewhere.
    pub fn borrow(&self) -> Fetch<'a, '_, T> {
        match self.try_borrow() {
            Some(t) => t,
            None => panic!(
                "attempted to immutably borrow already mutably borrowed resource of type `{}`",
                any::type_name::<T>()
            ),
        }
    }

    /// Attempt to mutably borrow the contents, panicking if the contents are already mutably *or*
    /// immutably borrowed somewhere.
    pub fn borrow_mut(&self) -> FetchMut<'a, '_, T> {
        match self.try_borrow_mut() {
            Some(t) => t,
        None => panic!(
                "attempted to mutably borrow already immutably or mutably borrowed resource of type `{}`",
                any::type_name::<T>()
            ),
        }
    }

    /// Attempt to immutably borrow the contents, failing and returning `None` if the contents
    /// are already mutably borrowed somewhere.
    pub fn try_borrow(&self) -> Option<Fetch<'a, '_, T>> {
        let _borrow = self.inner.try_read().handle()?;
        let ptr = unsafe {
            let any_ref = match &*_borrow {
                StoredResource::Owned { pointer } => &**pointer,
                StoredResource::Mutable { pointer, .. }
                | StoredResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>().unwrap() as *const _ as *mut _)
        };

        Some(Fetch {
            _origin: self,
            _borrow,
            ptr,
        })
    }

    /// Attempt to mutably borrow the contents, failing and returning `None` if the contents
    /// are already mutably or immutably borrowed somewhere.
    pub fn try_borrow_mut(&self) -> Option<FetchMut<'a, '_, T>> {
        let mut _borrow = self.inner.try_write().handle()?;
        let ptr = unsafe {
            let any_ref = match &mut *_borrow {
                StoredResource::Owned { pointer } => &mut **pointer,
                StoredResource::Mutable { pointer, .. } => pointer.as_mut(),
                StoredResource::Immutable { .. } => return None,
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>().unwrap() as *mut _)
        };

        Some(FetchMut { _borrow, ptr })
    }

    /// Attempt to immutably borrow the contents, returning immediately if the contents are
    /// safe to borrow and if not, blocking the calling thread until they can be safely borrowed.
    pub fn blocking_borrow(&self) -> Fetch<'a, '_, T> {
        let _borrow = self.inner.read().handle();
        let ptr = unsafe {
            let any_ref = match &*_borrow {
                StoredResource::Owned { pointer } => &**pointer,
                StoredResource::Mutable { pointer, .. }
                | StoredResource::Immutable { pointer, .. } => pointer.as_ref(),
            };

            NonNull::new_unchecked(any_ref.downcast_ref::<T>().unwrap() as *const _ as *mut _)
        };

        Fetch {
            _origin: self,
            _borrow,
            ptr,
        }
    }

    /// Attempt to mutably borrow the contents, returning immediately if the contents are
    /// safe to borrow and if not, blocking the calling thread until they can be safely borrowed.
    pub fn blocking_borrow_mut(&self) -> FetchMut<'a, '_, T> {
        let mut _borrow = self.inner.write().handle();
        let ptr = unsafe {
            let any_ref = match &mut *_borrow {
                StoredResource::Owned { pointer } => &mut **pointer,
                StoredResource::Mutable { pointer, .. } => pointer.as_mut(),
                StoredResource::Immutable { .. } => {
                    panic!("cannot fetch resource inserted as immutable reference as mutable")
                }
            };

            NonNull::new_unchecked(any_ref.downcast_mut::<T>().unwrap() as *mut _)
        };

        FetchMut { _borrow, ptr }
    }
}

#[derive(Debug)]
enum StoredResource<'a> {
    Owned {
        pointer: Box<dyn Any + Send + Sync>,
    },
    Mutable {
        pointer: NonNull<dyn Any + Send + Sync>,
        _marker: PhantomData<&'a mut ()>,
    },
    Immutable {
        pointer: NonNull<dyn Any + Send + Sync>,
        _marker: PhantomData<&'a ()>,
    },
}

/// The type of an immutably borrowed resource. Cloneable and implements
/// `Deref` to access the inner value.
#[derive(Debug)]
pub struct Fetch<'a: 'b, 'b, T: 'static> {
    _origin: &'b Shared<'a, T>,
    _borrow: RwLockReadGuard<'b, StoredResource<'a>>,
    ptr: NonNull<T>,
}

impl<'a: 'b, 'b, T: 'static> Clone for Fetch<'a, 'b, T> {
    fn clone(&self) -> Self {
        self._origin.borrow()
    }
}

impl<'a: 'b, 'b, T: 'static> ops::Deref for Fetch<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

unsafe impl<'a: 'b, 'b, T: 'static> Sync for Fetch<'a, 'b, T> where T: Sync {}

/// The type of a mutably borrowed resource. Implements `Deref`/`DerefMut` to
/// access the inner value.
#[derive(Debug)]
pub struct FetchMut<'a: 'b, 'b, T: 'static> {
    _borrow: RwLockWriteGuard<'b, StoredResource<'a>>,
    ptr: NonNull<T>,
}

impl<'a: 'b, 'b, T: 'static> ops::Deref for FetchMut<'a, 'b, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a: 'b, 'b, T: 'static> ops::DerefMut for FetchMut<'a, 'b, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

unsafe impl<'a: 'b, 'b, T: 'static> Sync for FetchMut<'a, 'b, T> where T: Sync {}

// Implementation ripped from the `Box::downcast` method for `Box<dyn Any + 'static + Send>`
fn downcast_send_sync<T: Any>(
    this: Box<dyn Any + Send + Sync>,
) -> Result<Box<T>, Box<dyn Any + Send + Sync>> {
    <Box<dyn Any>>::downcast(this).map_err(|s| unsafe {
        // reapply the Send + Sync markers
        Box::from_raw(Box::into_raw(s) as *mut (dyn Any + Send + Sync))
    })
}

/// An owned, non-`Clone`-able resources container. For technical reasons this type does not
/// currently implement `Resources`, but it might in the future.
///
/// This is also currently the only type which can have resources inserted into it; all other
/// resource types have to have their inner `OwnedResources` borrowed in order to perform
/// insertion/removal. The general workflow for creating a `SharedResources` or `UnifiedResources`
/// type will involve first creating an empty `OwnedResources`, then inserting all the initial
/// resources into it, and then creating a `SharedResources` from that and going on to combine
/// that into a `UnifiedResources` or whatnot.
#[derive(Debug)]
pub struct OwnedResources<'a> {
    map: HashMap<TypeId, Arc<RwLock<StoredResource<'a>>>>,
}

impl<'a> OwnedResources<'a> {
    /// Create an empty `OwnedResources`.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Check whether or not this map contains a value of some type.
    pub fn has_value<T: Fetchable>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Insert a resource, allowing the map to take ownership of it.
    pub fn insert<T: Fetchable + 'static>(&mut self, res: T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.map.contains_key(&type_id));
        let entry = StoredResource::Owned {
            pointer: Box::new(res),
        };
        self.map.insert(type_id, Arc::new(RwLock::new(entry)));
    }

    /// Insert a reference to a resource owned elsewhere. The resource
    /// must live at least as long as the container, and it cannot be
    /// mutably borrowed from the container - any attempts will result in the
    /// same response as if the resource was already immutably borrowed.
    pub fn insert_ref<'b: 'a, T: Fetchable>(&mut self, res: &'b T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.map.contains_key(&type_id));
        let entry = StoredResource::Immutable {
            pointer: unsafe {
                NonNull::new_unchecked(res as &'a (dyn Any + Send + Sync) as *const _ as *mut _)
            },
            _marker: PhantomData,
        };
        self.map.insert(type_id, Arc::new(RwLock::new(entry)));
    }

    /// Insert a mutable reference to a resource owned elsewhere. The
    /// resource must live at least as long as the container.
    pub fn insert_mut<'b: 'a, T: Fetchable>(&mut self, res: &'b mut T) {
        let type_id = TypeId::of::<T>();
        assert!(!self.map.contains_key(&type_id));
        let entry = StoredResource::Mutable {
            pointer: unsafe {
                NonNull::new_unchecked(res as &'a mut (dyn Any + Send + Sync) as *mut _)
            },
            _marker: PhantomData,
        };
        self.map.insert(type_id, Arc::new(RwLock::new(entry)));
    }

    /// Remove a type from the map. This is rarely useful, but the functionality is still here.
    /// Returns `Some` with the removed value if it's found; otherwise `None`.
    pub fn remove<T: Fetchable>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|t| Arc::try_unwrap(t).ok())
            .and_then(|t| match t.into_inner().handle() {
                StoredResource::Owned { pointer } => Some(*downcast_send_sync(pointer).unwrap()),
                _ => None,
            })
    }

    /// Fetch a single resource from the container. Will return `Err(NotFound)` if the
    /// map does not contain a value of that type. The `NotFound` error implements a couple
    /// useful traits making it easy to use in sludge's usual use cases; please see its
    /// docs for more information.
    pub fn fetch_one<T: Fetchable>(&self) -> Result<Shared<'a, T>, NotFound> {
        let maybe_shared = self.map.get(&TypeId::of::<T>()).cloned().map(Shared::new);
        maybe_shared.ok_or_else(|| NotFound::of::<T>())
    }

    /// Fetch one or more resources from the container, all at once. Will return `Err(NotFound)`
    /// at the first resource it cannot find, or `Some` containing all the fetched resources.
    ///
    /// The `FetchAll` trait is implemented for tuples of length zero to 26, for Reasons. If you
    /// use `resources.fetch::<(A, B, ...)>()`, it will return
    /// `Result<(Shared<'a, A>, Shared<'a, B>, ...), NotFound>`. Hopefully this is intuitive to you.
    /// If not, you can check out the definition of `FetchAll`, though it likely won't be much help.
    ///
    /// If you like, you can implement your own `FetchAll` types. Though I think there are few cases
    /// where this would be helpful.
    pub fn fetch<T: FetchAll<'a>>(&self) -> Result<T::Fetched, NotFound> {
        T::fetch_components_owned(self)
    }

    /// Retrieve a mutable reference to some resource in the map.
    pub fn get_mut<T: Fetchable>(&mut self) -> Option<&mut T> {
        match self
            .map
            .get_mut(&TypeId::of::<T>())
            .and_then(Arc::get_mut)?
            .get_mut()
            .handle()
        {
            StoredResource::Owned { pointer } => Some(pointer.downcast_mut().unwrap()),
            StoredResource::Mutable { pointer, .. } => {
                Some(unsafe { pointer.as_mut() }.downcast_mut().unwrap())
            }
            _ => None,
        }
    }
}

unsafe impl<'a> Send for OwnedResources<'a> {}
unsafe impl<'a> Sync for OwnedResources<'a> {}

/// A shared version of `OwnedResources`. It can be easily constructed from an `OwnedResources`,
/// but unlike `OwnedResources`, it is `Clone` and internally has to check borrows to the underlying
/// `OwnedResources` type as it has interior mutability.
///
/// This type implements `Resources` unlike `OwnedResources` and like `UnifiedResources`.
#[derive(Debug, Clone)]
pub struct SharedResources<'a> {
    shared: Pin<Arc<AtomicRefCell<OwnedResources<'a>>>>,
}

impl<'a> LuaUserData for SharedResources<'a> {}

impl<'a> From<OwnedResources<'a>> for SharedResources<'a> {
    fn from(resources: OwnedResources<'a>) -> Self {
        Self {
            shared: Arc::pin(AtomicRefCell::new(resources)),
        }
    }
}

impl<'a> Default for SharedResources<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SharedResources<'a> {
    /// Create an empty `SharedResources`. This is exactly the same as
    /// `SharedResources::from(OwnedResources::new())`.
    pub fn new() -> Self {
        Self::from(OwnedResources::new())
    }
}

impl<'a> Resources<'a> for SharedResources<'a> {
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>> {
        self.shared.borrow()
    }

    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>> {
        self.shared.borrow_mut()
    }

    fn fetch_one<T: Fetchable>(&self) -> Result<Shared<'a, T>, NotFound> {
        self.shared.borrow().fetch_one()
    }
}

/// A combined pair of `SharedResources` containers, representing "local" and "global"
/// resource contexts.
#[derive(Debug, Clone)]
pub struct UnifiedResources<'a> {
    /// The "local" resources are intended to contain resources which are local to a
    /// sludge [`Space`](sludge::Space). This is stuff which will be created and
    /// destroyed alongside the space, such as an ECS world or gamestate or what have
    /// you.
    pub local: SharedResources<'a>,

    /// The "global" resources are intended to contain things which you'll need to share
    /// throughout your program for the whole lifetime of your program. For example,
    /// an audio context type or a graphics context type.
    pub global: SharedResources<'a>,
}

impl<'a> UnifiedResources<'a> {
    /// Create an empty `UnifiedResources` from a pair of fresh `SharedResources`.
    pub fn new() -> Self {
        Self {
            local: SharedResources::new(),
            global: SharedResources::new(),
        }
    }
}

impl<'a> Resources<'a> for UnifiedResources<'a> {
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>> {
        self.local.borrow()
    }

    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>> {
        self.local.borrow_mut()
    }

    fn fetch_one<T: Fetchable>(&self) -> Result<Shared<'a, T>, NotFound> {
        self.local
            .fetch_one::<T>()
            .or_else(|_| self.global.fetch_one::<T>())
    }
}

impl<'a> LuaUserData for UnifiedResources<'a> {}

/// This trait is just shorthand for `Any + Send + Sync`. It's automatically implemented
/// for all such types.
pub trait Fetchable: Any + Send + Sync {}
impl<T> Fetchable for T where T: Any + Send + Sync {}

/// A trait which generalizes operations over resource containers, so that you can implement
/// functions which can operate on any resource container type.
pub trait Resources<'a> {
    /// Borrow the underlying "most-local" `OwnedResources`. This method will likely
    /// be removed soon, as there's little use for it.
    fn borrow(&self) -> AtomicRef<OwnedResources<'a>>;

    /// Borrow the underlying "most-local" `OwnedResources`. This method is useful for
    /// when you need to insert a resource but you have a `SharedResources` or
    /// `UnifiedResources` instead of an `OwnedResources`. There are a number of
    /// shortcomings with this method which need to be resolved and its type and
    /// semantics will likely change and it will probably be removed in favor of some
    /// sort of insertion/removal method in the future. It returns the "most-local"
    /// `OwnedResources` to be modified, which just means that if you run this on a
    /// `UnifiedResources`, it will return the corresponding `OwnedResources` of its
    /// `local` field. This is another reason the method is dubious; we don't give a
    /// way to access the global resources...
    fn borrow_mut(&self) -> AtomicRefMut<OwnedResources<'a>>;

    /// Fetch a single resource from the container.
    fn fetch_one<T: Fetchable>(&self) -> Result<Shared<'a, T>, NotFound>;

    /// Fetch one or more resources from the container, all at once. Will return `Err(NotFound)`
    /// at the first resource it cannot find, or `Some` containing all the fetched resources.
    ///
    /// The `FetchAll` trait is implemented for tuples of length zero to 26, for Reasons. If you
    /// use `resources.fetch::<(A, B, ...)>()`, it will return
    /// `Result<(Shared<'a, A>, Shared<'a, B>, ...), NotFound>`. Hopefully this is intuitive to you.
    /// If not, you can check out the definition of `FetchAll`, though it likely won't be much help.
    ///
    /// If you like, you can implement your own `FetchAll` types. Though I think there are few cases
    /// where this would be helpful.
    fn fetch<T: FetchAll<'a>>(&self) -> Result<T::Fetched, NotFound> {
        T::fetch_components(self)
    }
}

/// A trait marking a type which represents a bundle of resources to be fetched from a resource
/// container, all at once. This is implemented for tuples from size 0 to 26 by default, but you
/// can implement it for your own types if you like.
pub trait FetchAll<'a> {
    /// Where `Self` is the type of the bundle, `Self::Fetched` is the equivalent with all those
    /// bundle elements converted to `Shared` references.
    type Fetched;

    /// Fetch all components of the bundle at once.
    fn fetch_components<R>(resources: &R) -> Result<Self::Fetched, NotFound>
    where
        R: Resources<'a> + ?Sized;

    /// Fetch all components of the bundle at once, but from an `OwnedResources`, since we
    /// currently don't have an implementation of `Resources` for `OwnedResources`.
    fn fetch_components_owned(resources: &OwnedResources<'a>) -> Result<Self::Fetched, NotFound>;
}

macro_rules! impl_tuple {
    ($($id:ident),*) => {
        #[allow(non_snake_case)]
        impl<'a, $($id: Fetchable),*> FetchAll<'a> for ($($id,)*) {
            type Fetched = ($(Shared<'a, $id>,)*);
            fn fetch_components<Res>(_resources: &Res) -> Result<Self::Fetched, NotFound>
                where Res: Resources<'a> + ?Sized
            {
                $(let $id = _resources.fetch_one()?;)*
                Ok(($($id,)*))
            }

            fn fetch_components_owned(_resources: &OwnedResources<'a>) -> Result<Self::Fetched, NotFound> {
                $(let $id = _resources.fetch_one()?;)*
                Ok(($($id,)*))
            }
        }
    };
}

macro_rules! impl_all_tuples {
    ($t:ident $(, $ts:ident)*) => {
        impl_tuple!($t $(, $ts)*);
        impl_all_tuples!($($ts),*);
    };
    () => {
        impl_tuple!();
    };
}

impl_all_tuples!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_resources() -> Result<()> {
        let mut a = 5i32;
        let mut b: &'static str = "hello";
        let c = true;

        {
            let mut borrowed_resources = OwnedResources::new();
            borrowed_resources.insert_mut(&mut a);
            borrowed_resources.insert_mut(&mut b);
            borrowed_resources.insert_ref(&c);

            let shared_a = borrowed_resources.fetch_one::<i32>()?;
            let shared_b = borrowed_resources.fetch_one::<&'static str>()?;
            let shared_c = borrowed_resources.fetch_one::<bool>()?;

            {
                assert_eq!(*shared_a.borrow(), 5i32);
                *shared_b.borrow_mut() = "world";
                assert_eq!(*shared_c.borrow(), true);
            }

            let _ = borrowed_resources;
        }

        assert_eq!(b, "world");

        Ok(())
    }
}
