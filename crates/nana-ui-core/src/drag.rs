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

use std::ops::{BitOr, BitOrAssign};
use std::sync::Arc;

use crate::data_transfer::{DataFormat, DataTransfer};

/// Platform file-drag phase. Hosts map window hover/drop/cancel onto this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileDragKind {
    Hover,
    Drop,
    Cancel,
}

/// Unique identifier for an active drag-and-drop session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DragSessionId(pub u64);

/// Why an internal drag session ended without completing a drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DragCancelReason {
    NoTarget,
    Escape,
    PointerCancelled,
    WindowBlur,
    WindowClosed,
    SourceDestroyed,
    Programmatic,
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
    /// A link or reference to the payload is created here.
    Link,
}

/// Alias matching the Issue #120 / #121 payload vocabulary.
pub type TransferAction = DropEffect;

impl DropEffect {
    /// Negotiate an effective action between source-allowed and target-allowed sets.
    pub fn negotiate(
        source: DropEffects,
        target: DropEffects,
        preferred_target: DropEffect,
        requested: Option<DropEffect>,
    ) -> Option<DropEffect> {
        let allowed = source.intersection(target);
        if allowed.is_empty() {
            return None;
        }
        if let Some(req) = requested {
            return allowed.contains(req).then_some(req);
        }
        if allowed.contains(preferred_target) {
            Some(preferred_target)
        } else {
            allowed.first()
        }
    }
}

/// Bitmask of allowed [`DropEffect`] actions for negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DropEffects(u8);

impl DropEffects {
    pub const NONE: Self = Self(0);
    pub const COPY: Self = Self(1 << 0);
    pub const MOVE: Self = Self(1 << 1);
    pub const LINK: Self = Self(1 << 2);
    pub const ALL: Self = Self(0b111);

    pub const fn from_effect(effect: DropEffect) -> Self {
        match effect {
            DropEffect::Copy => Self::COPY,
            DropEffect::Move => Self::MOVE,
            DropEffect::Link => Self::LINK,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, effect: DropEffect) -> bool {
        (self.0 & Self::from_effect(effect).0) != 0
    }

    pub fn insert(&mut self, effect: DropEffect) {
        self.0 |= Self::from_effect(effect).0;
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn first(self) -> Option<DropEffect> {
        if self.contains(DropEffect::Copy) {
            Some(DropEffect::Copy)
        } else if self.contains(DropEffect::Move) {
            Some(DropEffect::Move)
        } else if self.contains(DropEffect::Link) {
            Some(DropEffect::Link)
        } else {
            None
        }
    }
}

impl Default for DropEffects {
    fn default() -> Self {
        Self::COPY
    }
}

impl BitOr for DropEffects {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl BitOrAssign for DropEffects {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// What one node accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropAccepts {
    kinds: Vec<DropKind>,
    formats: Vec<DataFormat>,
    effect: DropEffect,
    effects: DropEffects,
    explicit_effects: bool,
}

impl DropAccepts {
    /// Accepts the listed kinds. An empty list accepts nothing, which is how a
    /// node temporarily refuses drops without being unregistered.
    pub fn new(kinds: impl IntoIterator<Item = DropKind>) -> Self {
        let effect = DropEffect::default();
        Self {
            kinds: kinds.into_iter().collect(),
            formats: Vec::new(),
            effect,
            effects: DropEffects::from_effect(effect),
            explicit_effects: false,
        }
    }

    /// Accepts platform file drops.
    pub fn files() -> Self {
        Self::new([DropKind::Files])
    }

    /// Accepts a single named custom drop kind.
    pub fn custom(name: impl Into<Arc<str>>) -> Self {
        Self::new([DropKind::custom(name)])
    }

    /// Accepts drops carrying any of the specified [`DataFormat`]s.
    pub fn formats(formats: impl IntoIterator<Item = DataFormat>) -> Self {
        let effect = DropEffect::default();
        Self {
            kinds: Vec::new(),
            formats: formats.into_iter().collect(),
            effect,
            effects: DropEffects::from_effect(effect),
            explicit_effects: false,
        }
    }

    /// Also require/accept the listed [`DataFormat`]s.
    pub fn with_formats(mut self, formats: impl IntoIterator<Item = DataFormat>) -> Self {
        self.formats.extend(formats);
        self
    }

    pub fn effect(mut self, effect: DropEffect) -> Self {
        self.effect = effect;
        if !self.explicit_effects {
            self.effects = DropEffects::from_effect(effect);
        } else {
            self.effects.insert(effect);
        }
        self
    }

    /// Declare the full set of [`DropEffects`] this target supports.
    pub fn effects(mut self, effects: DropEffects) -> Self {
        self.effects = effects;
        self.explicit_effects = true;
        if !effects.contains(self.effect)
            && let Some(first) = effects.first()
        {
            self.effect = first;
        }
        self
    }

    pub fn accepts(&self, kind: &DropKind) -> bool {
        self.kinds.iter().any(|accepted| accepted == kind)
    }

    /// Whether this target accepts a typed drag session with `kind` and `transfer`.
    ///
    /// When `kinds` is non-empty, `kind` must match; when `formats` is
    /// non-empty, `transfer` must advertise at least one accepted format.
    /// If both `kinds` and `formats` are empty, nothing is accepted.
    pub fn accepts_transfer(&self, kind: &DropKind, transfer: &DataTransfer) -> bool {
        if self.kinds.is_empty() && self.formats.is_empty() {
            return false;
        }
        let kind_ok = self.kinds.is_empty() || self.accepts(kind);
        let format_ok =
            self.formats.is_empty() || self.formats.iter().any(|fmt| transfer.has_format(fmt));
        kind_ok && format_ok
    }

    pub fn declared_effect(&self) -> DropEffect {
        self.effect
    }

    pub fn allowed_effects(&self) -> DropEffects {
        self.effects
    }

    pub fn accepted_formats(&self) -> &[DataFormat] {
        &self.formats
    }

    pub fn negotiate_effect(
        &self,
        source: DropEffects,
        requested: Option<DropEffect>,
    ) -> Option<DropEffect> {
        DropEffect::negotiate(source, self.effects, self.effect, requested)
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
        assert_eq!(target.allowed_effects(), DropEffects::MOVE);
    }

    #[test]
    fn an_empty_target_refuses_everything() {
        let target = DropAccepts::new([]);
        assert!(!target.accepts(&DropKind::Files));
        assert!(!target.accepts_transfer(&DropKind::Files, &DataTransfer::from_text("x")));
    }

    #[test]
    fn action_negotiation_handles_copy_move_link_and_disjoint_sets() {
        let target = DropAccepts::custom("node")
            .effect(DropEffect::Move)
            .effects(DropEffects::COPY | DropEffects::MOVE | DropEffects::LINK);
        assert_eq!(
            target.negotiate_effect(DropEffects::COPY | DropEffects::LINK, None),
            Some(DropEffect::Copy)
        );
        assert_eq!(
            target.negotiate_effect(DropEffects::ALL, Some(DropEffect::Link)),
            Some(DropEffect::Link)
        );
        assert_eq!(
            target.negotiate_effect(DropEffects::COPY, Some(DropEffect::Move)),
            None
        );

        let mut transfer = DataTransfer::internal();
        transfer.set_text("hello").unwrap();
        let fmt_target = DropAccepts::formats([DataFormat::TextPlain]);
        assert!(fmt_target.accepts_transfer(&DropKind::custom("anything"), &transfer));
        let html_target = DropAccepts::formats([DataFormat::TextHtml]);
        assert!(!html_target.accepts_transfer(&DropKind::custom("anything"), &transfer));
    }
}
