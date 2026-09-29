//! Named slots of the shell composites, as views:
//!
//! ```ignore
//! widget(DesktopShell::from_model(model).title("Gallery"))
//!     .title_trailing(row().gap(6.0).children((search, theme)))
//!     .navigation(sidebar())
//!     .primary(page())
//! ```
//!
//! Each slot is built before the shell and placed by its assembler once the
//! tree commits (`TypeBehavior::slot_assembler`), so a view never calls
//! `assemble_desktop_shell`. A slot's node stays for the view's life: what
//! changes inside it is a `when` / `each` / `dynamic` there, and whether a
//! region shows is the workspace model's.

use nana_ui_core::RegionId;

use super::{El, IntoView};
use crate::{AppTitleBar, DesktopShell};

impl<K> El<DesktopShell, K> {
    pub fn title_leading(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::title_leading)
    }

    pub fn title_center(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::title_center)
    }

    pub fn title_trailing(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::title_trailing)
    }

    pub fn primary(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::primary)
    }

    pub fn navigation(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::navigation)
    }

    pub fn navigation_footer(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::navigation_footer)
    }

    /// See [`DesktopShell::inspector`].
    pub fn inspector(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::inspector)
    }

    pub fn bottom(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::bottom)
    }

    /// See [`DesktopShell::region`].
    pub fn region(self, id: RegionId, view: impl IntoView) -> Self {
        self.slot(view, move |shell, content| shell.region(id, content))
    }

    /// See [`DesktopShell::overlay`].
    pub fn overlay(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::overlay)
    }

    /// See [`DesktopShell::status`].
    pub fn status(self, view: impl IntoView) -> Self {
        self.slot(view, DesktopShell::status)
    }
}

impl<K> El<AppTitleBar, K> {
    pub fn leading(self, view: impl IntoView) -> Self {
        self.slot(view, AppTitleBar::leading)
    }

    pub fn center(self, view: impl IntoView) -> Self {
        self.slot(view, AppTitleBar::center)
    }

    pub fn trailing(self, view: impl IntoView) -> Self {
        self.slot(view, AppTitleBar::trailing)
    }
}
