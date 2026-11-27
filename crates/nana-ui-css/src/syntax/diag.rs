//! Structured tiered diagnostics for CSS lexing, parsing, variable expansion,
//! selector matching, at-rules, and value parsing.

use std::fmt;

use super::span::Span;

/// Diagnostic severity tier.
///
/// - [`DiagSeverity::Error`]: syntax breakage, invalid value, unsupported
///   selector/combinator or at-rule, variable cycle, nesting overflow.
/// - [`DiagSeverity::Warning`]: safely-ignorable vendor-prefixed properties
///   (e.g. `-moz-*`, `-ms-*`, `-webkit-font-smoothing`) or minor recoverable
///   issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagSeverity {
    Error,
    Warning,
}

/// Stable machine-readable diagnostic classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagCode {
    SyntaxError,
    NestingLimitExceeded,
    UnterminatedComment,
    UnterminatedString,
    BadString,
    BadUrl,
    InvalidDeclaration,
    InvalidSelector,
    UnsupportedSelector,
    UnsupportedCombinator,
    InvalidAtRule,
    UnsupportedAtRule,
    InvalidValue,
    VarCycle,
    VarDepthExceeded,
    InvalidVarSyntax,
    MissingVariable,
    VendorPrefixedProperty,
    UnknownProperty,
}

/// One diagnostic attached to a byte [`Span`] in the input stylesheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssDiag {
    pub severity: DiagSeverity,
    pub span: Span,
    pub message: String,
    pub code: DiagCode,
}

impl CssDiag {
    /// Create an [`DiagSeverity::Error`] diagnostic.
    pub fn error(span: Span, code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            severity: DiagSeverity::Error,
            span,
            message: message.into(),
            code,
        }
    }

    /// Create a [`DiagSeverity::Warning`] diagnostic.
    pub fn warning(span: Span, code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            severity: DiagSeverity::Warning,
            span,
            message: message.into(),
            code,
        }
    }

    /// Format this diagnostic with 1-based `line:col` against `src`.
    pub fn format_with_source(&self, src: &str) -> String {
        let (line, col) = self.span.line_col(src);
        let sev = match self.severity {
            DiagSeverity::Error => "error",
            DiagSeverity::Warning => "warning",
        };
        format!("{sev}[{:?}] at {line}:{col}: {}", self.code, self.message)
    }
}

impl fmt::Display for CssDiag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            DiagSeverity::Error => "error",
            DiagSeverity::Warning => "warning",
        };
        write!(
            f,
            "{sev}[{:?}] ({}..{}): {}",
            self.code, self.span.start, self.span.end, self.message
        )
    }
}

/// Returns `true` for vendor-prefixed CSS property names that are safely
/// ignorable in NanaUI (emitted as [`DiagSeverity::Warning`] instead of
/// [`DiagSeverity::Error`]).
///
/// Supported `-webkit-*` properties (`-webkit-backdrop-filter`,
/// `-webkit-mask-image`, `-webkit-line-clamp`, `-webkit-text-stroke*`,
/// `-webkit-text-emphasis*`) are NOT considered ignorable because the engine
/// actively maps them.
pub fn is_ignorable_vendor_property(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("-moz-") || lower.starts_with("-ms-") || lower.starts_with("-o-") {
        return true;
    }
    if lower.starts_with("-webkit-") {
        return !matches!(
            lower.as_str(),
            "-webkit-backdrop-filter"
                | "-webkit-mask-image"
                | "-webkit-line-clamp"
                | "-webkit-text-stroke"
                | "-webkit-text-stroke-width"
                | "-webkit-text-stroke-color"
                | "-webkit-text-emphasis"
                | "-webkit-text-emphasis-style"
                | "-webkit-text-emphasis-position"
                | "-webkit-text-emphasis-color"
        );
    }
    false
}

/// Classify an unrecognized property name into either a vendor-prefix
/// [`DiagSeverity::Warning`] or an unknown-property [`DiagSeverity::Error`].
pub fn diagnose_unknown_property(name: &str, span: Span) -> CssDiag {
    if is_ignorable_vendor_property(name) {
        CssDiag::warning(
            span,
            DiagCode::VendorPrefixedProperty,
            format!("ignoring vendor-prefixed property `{name}`"),
        )
    } else {
        CssDiag::error(
            span,
            DiagCode::UnknownProperty,
            format!("unknown CSS property `{name}`"),
        )
    }
}
