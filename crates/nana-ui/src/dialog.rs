/// Width presets shared with LiliaUI dialogs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DialogSize {
    Compact,
    #[default]
    Default,
    Medium,
    Wide,
    Workspace,
}

impl DialogSize {
    pub const fn max_width(self) -> f32 {
        match self {
            Self::Compact => 420.0,
            Self::Default => 520.0,
            Self::Medium => 600.0,
            Self::Wide => 680.0,
            Self::Workspace => 1080.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogCloseTrigger {
    Escape,
    Outside,
    CloseButton,
}

/// Controls which user gestures may dismiss a dialog.
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

    #[test]
    fn dialog_sizes_match_the_shared_contract() {
        assert_eq!(DialogSize::Compact.max_width(), 420.0);
        assert_eq!(DialogSize::Default.max_width(), 520.0);
        assert_eq!(DialogSize::Medium.max_width(), 600.0);
        assert_eq!(DialogSize::Wide.max_width(), 680.0);
        assert_eq!(DialogSize::Workspace.max_width(), 1080.0);
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
    }
}
