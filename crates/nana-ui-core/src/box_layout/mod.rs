//! Style Model **Layout** — box flex intent (not a CSSOM, not workspace regions).
//!
//! Pure data consumed by L3 draw (`nana-ui` Scene painter) and the view
//! compilers. CSS declaration / class parsing stays in `nana-ui-css`.
//! Workspace region layout lives in [`crate::layout`].
//!
//! Shared geometry helpers used by `measure` (pre-paint / parity) and Runtime
//! layout: [`LayoutStyle::resolve_content_box`], [`LayoutStyle::resolve_inset`],
//! `resolved_padding_against` / `resolved_margin_against` / gap helpers.
//! Prefer extending these over duplicating box math in adapters.

mod calc;
mod grid;
mod paint;
mod style;
#[cfg(test)]
mod tests;
mod units;

pub use calc::{CalcBinOp, CalcExpr, CalcExprRef, intern_calc};
pub use grid::*;
pub use paint::*;
pub use style::*;
pub use units::*;

crate::shared::shared_default!(
    PaintStyle,
    LogicalEdges,
    GridPlacement,
    GridRepeatAuto,
    GridStyle,
    TransformStyle,
    TextStyleExt,
    BorderSides,
    LogicalBoxEdges,
    ContainerStyle,
);
