//! Fixed-size records carried by the producer rings. Building one never
//! allocates or formats.

use std::time::Duration;

use crate::schema::{EventDescriptor, FieldKind, MAX_FIELDS};

/// One structured event as it sits in a producer ring.
#[derive(Clone, Copy)]
pub struct Record {
    /// Nanoseconds since the session's monotonic start.
    pub ts_ns: u64,
    pub event: &'static EventDescriptor,
    pub thread: u32,
    pub len: u8,
    /// Raw bits, reinterpreted by the descriptor's [`FieldKind`]s.
    pub values: [u64; MAX_FIELDS],
}

/// A fault: an event plus an optional message. The message is the only heap
/// allocation on any producer path, and faults are rare by definition.
pub struct FaultRecord {
    pub record: Record,
    pub message: Option<Box<str>>,
}

/// Conversion into a typed field value. Implemented for the primitive
/// numbers, `bool`, and [`Duration`] (nanoseconds).
pub trait FieldValue {
    const KIND: FieldKind;
    fn to_bits(self) -> u64;
}

macro_rules! unsigned {
    ($($ty:ty),*) => {$(
        impl FieldValue for $ty {
            const KIND: FieldKind = FieldKind::U64;
            #[inline(always)]
            fn to_bits(self) -> u64 {
                self as u64
            }
        }
    )*};
}
macro_rules! signed {
    ($($ty:ty),*) => {$(
        impl FieldValue for $ty {
            const KIND: FieldKind = FieldKind::I64;
            #[inline(always)]
            fn to_bits(self) -> u64 {
                (self as i64) as u64
            }
        }
    )*};
}
unsigned!(u8, u16, u32, u64, usize);
signed!(i8, i16, i32, i64, isize);

impl FieldValue for f64 {
    const KIND: FieldKind = FieldKind::F64;
    #[inline(always)]
    fn to_bits(self) -> u64 {
        self.to_bits()
    }
}
impl FieldValue for f32 {
    const KIND: FieldKind = FieldKind::F64;
    #[inline(always)]
    fn to_bits(self) -> u64 {
        f64::from(self).to_bits()
    }
}
impl FieldValue for bool {
    const KIND: FieldKind = FieldKind::Bool;
    #[inline(always)]
    fn to_bits(self) -> u64 {
        self as u64
    }
}
impl FieldValue for Duration {
    const KIND: FieldKind = FieldKind::U64;
    #[inline(always)]
    fn to_bits(self) -> u64 {
        saturating_ns(self)
    }
}

/// `duration` in nanoseconds, saturating at `u64::MAX` (~584 years).
#[inline(always)]
pub(crate) fn saturating_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// One argument as the macros hand it over: the call-site name (checked
/// against the descriptor in debug builds only), wire kind, and bits.
#[derive(Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub kind: FieldKind,
    pub bits: u64,
}

impl Field {
    #[inline(always)]
    pub fn new<V: FieldValue>(name: &'static str, value: V) -> Self {
        Self {
            name,
            kind: V::KIND,
            bits: value.to_bits(),
        }
    }
}

/// Pack call-site fields into a record's value array.
#[inline(always)]
pub(crate) fn pack(event: &'static EventDescriptor, fields: &[Field]) -> (u8, [u64; MAX_FIELDS]) {
    #[cfg(debug_assertions)]
    check_fields(event, fields);
    #[cfg(not(debug_assertions))]
    let _ = event;
    let mut values = [0u64; MAX_FIELDS];
    let len = fields.len().min(MAX_FIELDS);
    for (slot, field) in values.iter_mut().zip(&fields[..len]) {
        *slot = field.bits;
    }
    (len as u8, values)
}

#[cfg(debug_assertions)]
fn check_fields(event: &'static EventDescriptor, fields: &[Field]) {
    debug_assert_eq!(
        fields.len(),
        event.fields.len(),
        "event `{}` declares {} fields, call site passed {}",
        event.name,
        event.fields.len(),
        fields.len()
    );
    for (declared, passed) in event.fields.iter().zip(fields) {
        debug_assert!(
            declared.name == passed.name && declared.kind == passed.kind,
            "event `{}`: field `{}` ({:?}) passed as `{}` ({:?})",
            event.name,
            declared.name,
            declared.kind,
            passed.name,
            passed.kind
        );
    }
}
