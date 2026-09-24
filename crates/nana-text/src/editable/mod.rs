//! The editable text path (Issue #96): storage, editor state, IME composition
//! and the geometry an editor reads from laid-out text.
//!
//! ```text
//! EditableText ── EditState (selection, composition, goal column)
//!        │              │
//!        └── EditSession: typing, deleting, motion, IME ──┐
//!                                                          │ display text
//!                                                          ▼
//!                        EditorGeometry: one TextLayout per paragraph
//!                        hit_test / caret_rect / selection_rects / moves
//! ```
//!
//! An additional layer over [`Paragraph`](crate::TextKind::Paragraph) layout,
//! not a property every text carries: a label never holds a session or a
//! geometry, and nothing here runs for text that is not being edited.
//!
//! # Revisions
//!
//! Text, selection and composition are separate revisions
//! ([`EditRevisions`]). Only a text or composition change alters what is laid
//! out, so only those make [`EditorGeometry::sync_session`] do anything; a
//! caret move or a selection change is answered from the retained layouts, and
//! a caret blink is not an edit at all — it is paint.
//!
//! # Offsets
//!
//! Offsets are UTF-8 bytes of the committed text, always on grapheme cluster
//! boundaries once they leave a command ([`navigation`]). Geometry works in
//! *display* bytes — the committed text with the preedit spliced in —
//! and [`EditSession::display_offset`] / [`EditSession::committed_offset`]
//! convert. [`EditableText::utf16_offset`] serves platform APIs that count
//! UTF-16 units.

pub mod diff;
mod geometry;
pub mod ime;
pub mod navigation;
mod session;
mod state;
mod text;

pub use geometry::{CaretRect, CompositionMarks, EditHit, EditorGeometry, GeometrySync};
pub use session::{EditChange, EditSession, EditState, Motion, SurroundingText, collapse_edge};
pub use state::{
    Composition, EditRevisions, EditSelection, normalize_selections, remap_offset, remap_selection,
};
pub use text::{EditError, EditableText, TextEdit};
