//! Static descriptors. Call sites pass `&'static` descriptors, never names:
//! the name, field names and units reach the `.nlog` file once per file as a
//! schema chunk, so a reader recovers them without the producing binary.

/// Owner of a family of event and metric IDs.
///
/// NanaUI reserves `0x0000..=0x00FF` ([`Domain::FRAMEWORK_MAX`]); applications
/// pick their own domains from [`Domain::APPLICATION_MIN`] upward. An ID is
/// unique only inside its domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Domain(pub u16);

impl Domain {
    pub const RUNTIME: Self = Self(0x0001);
    pub const LAYOUT: Self = Self(0x0002);
    pub const TEXT: Self = Self(0x0003);
    pub const GPU: Self = Self(0x0004);
    pub const WINDOW: Self = Self(0x0005);
    pub const HOST: Self = Self(0x0006);
    /// The diagnostics runtime's own bookkeeping.
    pub const DIAGNOSTICS: Self = Self(0x0007);
    /// Packaged resource reads (`.nrpack` mounts, Issue #226).
    pub const RESOURCE: Self = Self(0x0008);
    /// The package manifest and the packaged-application self-check.
    pub const PACKAGE: Self = Self(0x0009);
    pub const FRAMEWORK_MAX: u16 = 0x00FF;
    pub const APPLICATION_MIN: u16 = 0x0100;

    pub const fn is_framework(self) -> bool {
        self.0 <= Self::FRAMEWORK_MAX
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Severity {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Fatal = 5,
}

impl Severity {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Trace,
            1 => Self::Debug,
            2 => Self::Info,
            3 => Self::Warn,
            4 => Self::Error,
            5 => Self::Fatal,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }
}

/// Wire type of one event field. Values travel as raw `u64` bits and are
/// reinterpreted by kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FieldKind {
    U64 = 0,
    I64 = 1,
    F64 = 2,
    Bool = 3,
}

impl FieldKind {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::U64,
            1 => Self::I64,
            2 => Self::F64,
            3 => Self::Bool,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub name: &'static str,
    pub kind: FieldKind,
}

impl FieldDescriptor {
    pub const fn u64(name: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::U64,
        }
    }
    pub const fn i64(name: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::I64,
        }
    }
    pub const fn f64(name: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::F64,
        }
    }
    pub const fn bool(name: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::Bool,
        }
    }
}

/// Most typed fields one record carries. Keeps [`crate::Record`] fixed-size.
pub const MAX_FIELDS: usize = 4;

/// A structured event type. Declare it once as a `static`:
///
/// ```
/// use nana_diagnostics::{Domain, EventDescriptor, FieldDescriptor, Severity};
/// pub static MODEL_LOADED: EventDescriptor = EventDescriptor::new(
///     Domain(0x0100), 1, "live.model_loaded", Severity::Info,
///     &[FieldDescriptor::u64("bytes"), FieldDescriptor::f64("ms")],
/// );
/// ```
#[derive(Debug)]
pub struct EventDescriptor {
    pub domain: Domain,
    pub id: u32,
    pub name: &'static str,
    pub severity: Severity,
    pub fields: &'static [FieldDescriptor],
}

impl EventDescriptor {
    /// Panics at compile time (in a `static`) when more than [`MAX_FIELDS`]
    /// fields are declared.
    pub const fn new(
        domain: Domain,
        id: u32,
        name: &'static str,
        severity: Severity,
        fields: &'static [FieldDescriptor],
    ) -> Self {
        assert!(
            fields.len() <= MAX_FIELDS,
            "an event carries at most MAX_FIELDS typed fields"
        );
        Self {
            domain,
            id,
            name,
            severity,
            fields,
        }
    }

    pub const fn key(&self) -> SchemaKey {
        SchemaKey {
            domain: self.domain,
            id: self.id,
        }
    }
}

/// `(domain, id)` — the identity written to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SchemaKey {
    pub domain: Domain,
    pub id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MetricKind {
    /// Monotonic total; snapshots report the total and the delta.
    Counter = 0,
    /// Last written value.
    Gauge = 1,
    /// Log2-bucketed distribution; snapshots report the interval's samples.
    Histogram = 2,
}

impl MetricKind {
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Counter,
            1 => Self::Gauge,
            2 => Self::Histogram,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDescriptor {
    pub domain: Domain,
    pub id: u32,
    pub name: &'static str,
    pub kind: MetricKind,
    /// Free-form unit label for readers, e.g. `"ns"`, `"bytes"`, `"count"`.
    pub unit: &'static str,
}

impl MetricDescriptor {
    pub const fn key(&self) -> SchemaKey {
        SchemaKey {
            domain: self.domain,
            id: self.id,
        }
    }
}
