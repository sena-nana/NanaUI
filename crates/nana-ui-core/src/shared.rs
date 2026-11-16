//! Copy-on-write storage for style groups most nodes leave at their default.
//!
//! A [`crate::LayoutStyle`] is copied per node and per resolution, and most of
//! its bytes are groups a node never sets (paint layers, logical edges, grid
//! placement). Holding those as [`Shared`] makes an unset group `None` (`0`
//! allocations, `0` atomic refcount operations on clone/drop) falling back to a
//! process-wide static default on read, and a set one a single [`Arc`] that
//! copies share until one of them writes.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// A value with one shared static default instance per type.
pub trait SharedDefault: Clone + Default + PartialEq + Send + Sync + 'static {
    fn shared_default() -> &'static Self;
}

/// Implement [`SharedDefault`] with a lazily built process-wide default.
#[macro_export]
macro_rules! shared_default {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::shared::SharedDefault for $ty {
            fn shared_default() -> &'static Self {
                static DEFAULT: ::std::sync::LazyLock<$ty> =
                    ::std::sync::LazyLock::new(<$ty>::default);
                &DEFAULT
            }
        }
    )*};
}
pub use crate::shared_default;

/// Reads through to the value; a write materializes and copies it if anything
/// else holds it ([`Arc::make_mut`]).
pub struct Shared<T: SharedDefault>(Option<Arc<T>>);

const _: () =
    assert!(std::mem::size_of::<Shared<crate::LogicalEdges>>() == std::mem::size_of::<usize>());

impl<T: SharedDefault> Shared<T> {
    pub const fn empty() -> Self {
        Self(None)
    }

    pub fn new(value: T) -> Self {
        if value == *T::shared_default() {
            Self::empty()
        } else {
            Self(Some(Arc::new(value)))
        }
    }

    /// Whether both hold the same instance (or both are the unset shared
    /// default), which makes them equal without comparing fields.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Whether this is the unset shared default, which a reader that only cares
    /// about non-default values can skip.
    pub fn is_shared_default(&self) -> bool {
        self.0.is_none()
    }
}

impl<T: SharedDefault> Default for Shared<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T: SharedDefault> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: SharedDefault> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        match &self.0 {
            Some(arc) => arc,
            None => T::shared_default(),
        }
    }
}

impl<T: SharedDefault> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        let arc = self.0.get_or_insert_with(|| Arc::new(T::default()));
        Arc::make_mut(arc)
    }
}

impl<T: SharedDefault + PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || **self == **other
    }
}

impl<T: SharedDefault + Eq> Eq for Shared<T> {}

impl<T: SharedDefault + fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(formatter)
    }
}

impl<T: SharedDefault> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
