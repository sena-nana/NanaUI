//! Copy-on-write storage for style groups most nodes leave at their default.
//!
//! A [`crate::LayoutStyle`] is copied per node and per resolution, and most of
//! its bytes are groups a node never sets (paint layers, logical edges, grid
//! placement). Holding those as [`Shared`] makes an unset group one pointer
//! to a process-wide default, and a set one a single allocation that copies
//! share until one of them writes.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A value with one shared default instance per type.
pub trait SharedDefault: Clone + Default + Send + Sync + 'static {
    fn shared_default() -> &'static Arc<Self>;
}

/// Implement [`SharedDefault`] with a lazily built process-wide default.
macro_rules! shared_default {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::shared::SharedDefault for $ty {
            fn shared_default() -> &'static ::std::sync::Arc<Self> {
                static DEFAULT: ::std::sync::LazyLock<::std::sync::Arc<$ty>> =
                    ::std::sync::LazyLock::new(|| ::std::sync::Arc::new(<$ty>::default()));
                &DEFAULT
            }
        }
    )*};
}
pub(crate) use shared_default;

/// Reads through to the value; a write copies it first if anything else
/// holds it ([`Arc::make_mut`]).
pub struct Shared<T: SharedDefault>(Arc<T>);

impl<T: SharedDefault> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// Whether both hold the same instance, which makes them equal without
    /// comparing.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: SharedDefault> Default for Shared<T> {
    fn default() -> Self {
        Self(Arc::clone(T::shared_default()))
    }
}

impl<T: SharedDefault> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: SharedDefault> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: SharedDefault> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: SharedDefault + PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || *self.0 == *other.0
    }
}

impl<T: SharedDefault + Eq> Eq for Shared<T> {}

impl<T: SharedDefault + fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<T: SharedDefault> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: SharedDefault + Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: SharedDefault + Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::new)
    }
}
