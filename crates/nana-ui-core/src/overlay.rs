/// Tracks one transient overlay at a time.
///
/// Dialogs, menus and popovers are mutually exclusive in the application shell:
/// opening a new surface replaces the previous one, while dismiss always returns
/// the overlay that was active. The controller deliberately contains no renderer
/// state so it can be shared across hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExclusiveOverlay<T> {
    active: Option<T>,
}

impl<T> Default for ExclusiveOverlay<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ExclusiveOverlay<T> {
    pub const fn new() -> Self {
        Self { active: None }
    }

    pub const fn active(&self) -> Option<&T> {
        self.active.as_ref()
    }

    pub const fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn open(&mut self, overlay: T) {
        self.active = Some(overlay);
    }

    pub fn dismiss(&mut self) -> Option<T> {
        self.active.take()
    }
}

impl<T> ExclusiveOverlay<T>
where
    T: PartialEq,
{
    pub fn contains(&self, overlay: &T) -> bool {
        self.active.as_ref() == Some(overlay)
    }

    pub fn toggle(&mut self, overlay: T) {
        if self.contains(&overlay) {
            self.active = None;
        } else {
            self.active = Some(overlay);
        }
    }
}

/// Where a floating overlay surface mounts visually in the scene tree.
///
/// Logical ownership, events, focus, and theme tokens always stay with the
/// component's declared parent; only visual inheritance (ancestor opacity,
/// scroll edge fades, ancestor clips, and paint stacking) follows the mount.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum OverlayMount {
    /// Escape local cards, lists, scroll fades, and clips up to the nearest
    /// enclosing modal dialog (`ModalFrame`) when inside one, or the document
    /// root otherwise.
    #[default]
    NearestModal,
    /// Escape all ancestors—including any enclosing modal dialog—to the
    /// top-level document overlay layer.
    TopLayer,
    /// Remain visually nested inside the declaring parent hierarchy and
    /// inherit its clips, opacity groups, and scroll fades.
    Inline,
}

#[cfg(test)]
mod tests {
    use super::ExclusiveOverlay;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Surface {
        Menu,
        Dialog,
    }

    #[test]
    fn opening_an_overlay_replaces_the_active_surface() {
        let mut overlay = ExclusiveOverlay::new();
        overlay.open(Surface::Menu);
        overlay.open(Surface::Dialog);

        assert!(overlay.contains(&Surface::Dialog));
        assert!(!overlay.contains(&Surface::Menu));
        assert_eq!(overlay.dismiss(), Some(Surface::Dialog));
        assert!(!overlay.is_open());
    }

    #[test]
    fn toggling_the_active_overlay_dismisses_it() {
        let mut overlay = ExclusiveOverlay::new();
        overlay.toggle(Surface::Menu);
        assert!(overlay.contains(&Surface::Menu));

        overlay.toggle(Surface::Menu);
        assert!(!overlay.is_open());
    }
}
