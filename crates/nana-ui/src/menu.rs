/// Result of choosing an action from a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuSelection<T> {
    /// The action needs to be chosen again before it is executed.
    Pending(T),
    /// The action can be executed immediately.
    Confirmed(T),
}

/// Tracks the two-step confirmation used by destructive LiliaUI menu actions.
///
/// Rendering remains the caller's responsibility. This controller only keeps
/// the pending action so menus, popovers and host integrations can share the
/// same interaction semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuConfirmation<T> {
    pending: Option<T>,
}

impl<T> Default for MenuConfirmation<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> MenuConfirmation<T> {
    pub const fn new() -> Self {
        Self { pending: None }
    }

    pub const fn pending(&self) -> Option<&T> {
        self.pending.as_ref()
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

impl<T> MenuConfirmation<T>
where
    T: Copy + PartialEq,
{
    pub fn select(&mut self, action: T, requires_confirmation: bool) -> MenuSelection<T> {
        if requires_confirmation && self.pending != Some(action) {
            self.pending = Some(action);
            MenuSelection::Pending(action)
        } else {
            self.pending = None;
            MenuSelection::Confirmed(action)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MenuConfirmation, MenuSelection};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Action {
        Rename,
        Remove,
    }

    #[test]
    fn destructive_action_requires_a_second_selection() {
        let mut confirmation = MenuConfirmation::new();

        assert_eq!(
            confirmation.select(Action::Remove, true),
            MenuSelection::Pending(Action::Remove)
        );
        assert_eq!(confirmation.pending(), Some(&Action::Remove));
        assert_eq!(
            confirmation.select(Action::Remove, true),
            MenuSelection::Confirmed(Action::Remove)
        );
        assert_eq!(confirmation.pending(), None);
    }

    #[test]
    fn regular_action_clears_pending_confirmation() {
        let mut confirmation = MenuConfirmation::new();
        confirmation.select(Action::Remove, true);

        assert_eq!(
            confirmation.select(Action::Rename, false),
            MenuSelection::Confirmed(Action::Rename)
        );
        assert_eq!(confirmation.pending(), None);
    }
}
