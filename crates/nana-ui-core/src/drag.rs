//! Vocabulary for dropping something onto a node.
//!
//! The framework answers *where* a drop would land and, for platform file
//! drags, which registered target is hovered. An application registers which
//! nodes accept which kinds. What a drop then means — opening the file, moving
//! the record, rejecting it — stays the application's, the same way
//! `SecondaryPress` reports a right-click without deciding what menu to show.
//!
//! Dragging a tab, a dock pane or a `ReorderList` row keeps its own typed
//! contract: those move framework-owned structure and the framework does
//! reconcile them. This module is for payloads that come from outside those
//! families, the platform's file drops above all.

use std::sync::Arc;

/// Platform file-drag phase. Hosts map window hover/drop/cancel onto this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileDragKind {
    Hover,
    Drop,
    Cancel,
}

/// What a drop carries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DropKind {
    /// Paths from the platform's file drag.
    Files,
    /// An application-defined payload, named by the application.
    Custom(Arc<str>),
}

impl DropKind {
    pub fn custom(name: impl Into<Arc<str>>) -> Self {
        Self::Custom(name.into())
    }
}

/// What accepting a drop would do, so a host can pick the pointer feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DropEffect {
    /// The payload is taken and left in place at the source.
    #[default]
    Copy,
    /// The payload moves here.
    Move,
}

/// What one node accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropAccepts {
    kinds: Vec<DropKind>,
    effect: DropEffect,
}

impl DropAccepts {
    /// Accepts the listed kinds. An empty list accepts nothing, which is how a
    /// node temporarily refuses drops without being unregistered.
    pub fn new(kinds: impl IntoIterator<Item = DropKind>) -> Self {
        Self {
            kinds: kinds.into_iter().collect(),
            effect: DropEffect::default(),
        }
    }

    /// Accepts platform file drops.
    pub fn files() -> Self {
        Self::new([DropKind::Files])
    }

    pub fn effect(mut self, effect: DropEffect) -> Self {
        self.effect = effect;
        self
    }

    pub fn accepts(&self, kind: &DropKind) -> bool {
        self.kinds.iter().any(|accepted| accepted == kind)
    }

    pub fn declared_effect(&self) -> DropEffect {
        self.effect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_accepts_only_the_kinds_it_lists() {
        let target = DropAccepts::files().effect(DropEffect::Move);
        assert!(target.accepts(&DropKind::Files));
        assert!(!target.accepts(&DropKind::custom("record")));
        assert_eq!(target.declared_effect(), DropEffect::Move);
    }

    #[test]
    fn an_empty_target_refuses_everything() {
        let target = DropAccepts::new([]);
        assert!(!target.accepts(&DropKind::Files));
    }
}
