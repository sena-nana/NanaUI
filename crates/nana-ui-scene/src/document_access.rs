//! Failure to acquire a synchronous document access scope.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentAccessError {
    Busy,
    Poisoned,
}

impl std::fmt::Display for DocumentAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "document is already borrowed",
            Self::Poisoned => "document access was poisoned by a failed callback",
        })
    }
}

impl std::error::Error for DocumentAccessError {}
