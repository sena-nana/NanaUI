use crate::box_layout::LengthSpec;

/// How wide a dialog card is: a preset shared with LiliaUI dialogs, or a
/// length of its own.
///
/// The card is never wider than the room its scrim leaves; see
/// [`Self::width_in`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum DialogSize {
    Compact,
    #[default]
    Default,
    Medium,
    Wide,
    Workspace,
    /// A width of the dialog's own, with CSS length semantics: `Px(560.0)`
    /// is 560px; `Min2(Px(560.0), Viewport { Width, 92.0 })` is CSS
    /// `min(560px, 92vw)`. Percentages and viewport units resolve against
    /// the scrim, which covers the window. A length that names no size
    /// (`auto`, `fill`) asks for the whole width.
    Width(LengthSpec),
}

impl DialogSize {
    /// CSS `min(px, viewport%)`: `px` wide, but never more than `viewport`
    /// percent of the window's width.
    pub const fn capped(px: f32, viewport: f32) -> Self {
        Self::Width(LengthSpec::Min2(
            crate::box_layout::LengthAtom::Px(px),
            crate::box_layout::LengthAtom::Viewport {
                axis: crate::box_layout::ViewportAxis::Width,
                value: viewport,
            },
        ))
    }

    /// The card width this size asks for over a scrim of `scrim_width` ×
    /// `scrim_height` logical px, before the scrim's own margin caps it.
    pub fn width_in(self, scrim_width: f32, scrim_height: f32) -> f32 {
        let width = match self {
            Self::Compact => 420.0,
            Self::Default => 520.0,
            Self::Medium => 600.0,
            Self::Wide => 680.0,
            Self::Workspace => 1080.0,
            Self::Width(length) => length
                .resolve_with(Some(scrim_width), Some((scrim_width, scrim_height)))
                .unwrap_or(scrim_width),
        };
        if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogCloseTrigger {
    Escape,
    Outside,
    CloseButton,
}

/// Controls which user gestures may dismiss a dialog. Every gesture also
/// reaches the dialog as a request (the runtime's `DialogCloseRequested`),
/// whether or not this lets it close the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogClosePolicy {
    pub close_on_escape: bool,
    pub close_on_outside: bool,
    pub close_disabled: bool,
}

impl Default for DialogClosePolicy {
    fn default() -> Self {
        Self {
            close_on_escape: true,
            close_on_outside: true,
            close_disabled: false,
        }
    }
}

impl DialogClosePolicy {
    /// Close on no gesture: Escape, a press outside and the close button
    /// each only ask. The dialog hears the request, and the application
    /// closes it when, and if, it decides to.
    pub const fn requests_only() -> Self {
        Self {
            close_on_escape: false,
            close_on_outside: false,
            close_disabled: true,
        }
    }

    pub const fn allows(self, trigger: DialogCloseTrigger) -> bool {
        if self.close_disabled {
            return false;
        }

        match trigger {
            DialogCloseTrigger::Escape => self.close_on_escape,
            DialogCloseTrigger::Outside => self.close_on_outside,
            DialogCloseTrigger::CloseButton => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
    use crate::box_layout::{LengthSpec, ViewportAxis};

    #[test]
    fn dialog_sizes_match_the_shared_contract() {
        assert_eq!(DialogSize::Compact.width_in(2000.0, 1000.0), 420.0);
        assert_eq!(DialogSize::Default.width_in(2000.0, 1000.0), 520.0);
        assert_eq!(DialogSize::Medium.width_in(2000.0, 1000.0), 600.0);
        assert_eq!(DialogSize::Wide.width_in(2000.0, 1000.0), 680.0);
        assert_eq!(DialogSize::Workspace.width_in(2000.0, 1000.0), 1080.0);
    }

    /// A width of the dialog's own follows CSS: a px length is that wide,
    /// `min(px, vw)` gives way to the window, `%` and `vh` read the scrim.
    #[test]
    fn a_width_of_its_own_resolves_like_css() {
        assert_eq!(
            DialogSize::Width(LengthSpec::Px(560.0)).width_in(400.0, 300.0),
            560.0
        );
        let capped = DialogSize::capped(560.0, 92.0);
        assert_eq!(capped.width_in(1200.0, 800.0), 560.0);
        assert!((capped.width_in(400.0, 800.0) - 368.0).abs() < 1e-3);
        assert_eq!(
            DialogSize::Width(LengthSpec::Percent(50.0)).width_in(800.0, 600.0),
            400.0
        );
        assert_eq!(
            DialogSize::Width(LengthSpec::Viewport {
                axis: ViewportAxis::Height,
                value: 50.0,
            })
            .width_in(800.0, 600.0),
            300.0
        );
        assert_eq!(
            DialogSize::Width(LengthSpec::Fill).width_in(800.0, 600.0),
            800.0
        );
        assert_eq!(
            DialogSize::Width(LengthSpec::Px(-20.0)).width_in(800.0, 600.0),
            0.0
        );
    }

    #[test]
    fn close_policy_honors_each_dismissal_guard() {
        let outside_locked = DialogClosePolicy {
            close_on_outside: false,
            ..DialogClosePolicy::default()
        };
        assert!(outside_locked.allows(DialogCloseTrigger::Escape));
        assert!(!outside_locked.allows(DialogCloseTrigger::Outside));
        assert!(outside_locked.allows(DialogCloseTrigger::CloseButton));

        let disabled = DialogClosePolicy {
            close_disabled: true,
            ..DialogClosePolicy::default()
        };
        assert!(!disabled.allows(DialogCloseTrigger::Escape));
        assert!(!disabled.allows(DialogCloseTrigger::Outside));
        assert!(!disabled.allows(DialogCloseTrigger::CloseButton));

        let requests = DialogClosePolicy::requests_only();
        assert!(!requests.allows(DialogCloseTrigger::Escape));
        assert!(!requests.allows(DialogCloseTrigger::Outside));
        assert!(!requests.allows(DialogCloseTrigger::CloseButton));
    }
}
