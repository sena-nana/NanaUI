//! Compact line icons used by NanaUI navigation surfaces.
//!
//! [`Icon`] is a `Copy` identity that holds a pointer to static Lucide geometry.
//! Unused catalog constants are not referenced by a central match table, so fat
//! LTO can drop them. [`parse_name`] only resolves shell chrome names.

use std::fmt;
use std::hash::{Hash, Hasher};

/// Backend-neutral 24×24 stroke geometry. Tests and hit-testing use these shapes;
/// painters rasterize [`IconData::svg`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconGeometry {
    pub shapes: &'static [IconShape],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconShape {
    Path(&'static [IconPathCommand]),
    Circle {
        center: [f32; 2],
        radius: f32,
    },
    Rect {
        origin: [f32; 2],
        size: [f32; 2],
        filled: bool,
    },
    RoundedRect {
        origin: [f32; 2],
        size: [f32; 2],
        radius: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconPathCommand {
    MoveTo([f32; 2]),
    LineTo([f32; 2]),
    CubicTo {
        control_a: [f32; 2],
        control_b: [f32; 2],
        to: [f32; 2],
    },
    Close,
}

#[derive(Debug)]
pub struct IconData {
    pub name: &'static str,
    pub shapes: &'static [IconShape],
    pub svg: &'static str,
}

/// Semantic icon identity. Compare by geometry pointer, not by display name.
#[derive(Clone, Copy)]
pub struct Icon(pub(crate) &'static IconData);

impl Icon {
    pub fn name(self) -> &'static str {
        self.0.name
    }

    pub fn shapes(self) -> &'static [IconShape] {
        self.0.shapes
    }

    pub fn geometry(self) -> IconGeometry {
        IconGeometry {
            shapes: self.0.shapes,
        }
    }

    /// Lucide 24×24 source. Painters rasterize this with `currentColor`.
    pub fn svg(self) -> &'static str {
        self.0.svg
    }

    pub fn as_ptr(self) -> *const IconData {
        self.0
    }

    /// Parse a stable shell icon name (`search`, `settings`, …).
    ///
    /// Catalog icons (`puzzle`, `palette`, `atom`, …) are typed constants and
    /// return `None` here so a name table cannot keep unused geometry live.
    pub fn parse_name(raw: &str) -> Option<Self> {
        let s = raw.trim().to_ascii_lowercase();
        let s = s
            .strip_prefix("icon-")
            .or_else(|| s.strip_prefix("nana-"))
            .unwrap_or(s.as_str());
        let s = s
            .strip_prefix("lucide-")
            .or_else(|| s.strip_prefix("icon:"))
            .unwrap_or(s);
        let s = s.strip_suffix("-icon").unwrap_or(s);
        Some(match s {
            "about" | "info" | "circle-info" | "info-circle" => Self::About,
            "add" | "plus" => Self::Add,
            "appearance" | "sun" => Self::Appearance,
            "arrow-left" | "arrowleft" | "back" => Self::ArrowLeft,
            "arrow-right" | "arrowright" => Self::ArrowRight,
            "arrow-up" | "arrowup" => Self::ArrowUp,
            "bot" | "robot" => Self::Bot,
            "chevron-right" => Self::ChevronRight,
            "chevron-up" => Self::ChevronUp,
            "chevron-down" => Self::ChevronDown,
            "chart" | "line-chart" | "linechart" | "chart-line" => Self::Chart,
            "close" | "x" | "x-mark" => Self::Close,
            "eye" | "visibility" => Self::Eye,
            "file" | "document" => Self::File,
            "folder" | "directory" => Self::Folder,
            "git-branch" | "gitbranch" | "branch" => Self::GitBranch,
            "maximize" | "square" => Self::Maximize,
            "message-square-plus" | "messagesquareplus" | "square-plus" => Self::MessageSquarePlus,
            "minimize" | "minus" => Self::Minimize,
            "moon" | "dark" => Self::Moon,
            "nodes" | "graph" | "network" => Self::Nodes,
            "paperclip" | "attachment" | "paper-clip" => Self::Paperclip,
            "restore" => Self::Restore,
            "search" | "magnifier" | "magnifying-glass" => Self::Search,
            "settings" | "gear" | "cog" => Self::Settings,
            "shield-check" | "shieldcheck" | "shield" => Self::ShieldCheck,
            "sidebar" | "panel-left" | "layout-panel-left" | "panel-left-open"
            | "panelleftopen" | "panel-left-close" | "panelleftclose" => Self::Sidebar,
            "sparkles" | "sparkle" => Self::Sparkles,
            "workspace" | "layout-dashboard" => Self::Workspace,
            _ => return None,
        })
    }
}

impl PartialEq for Icon {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Icon {}

impl Hash for Icon {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ptr().hash(state);
    }
}

impl fmt::Debug for Icon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Icon").field(&self.0.name).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Icon;

    #[test]
    fn parse_name_accepts_shell_aliases() {
        assert_eq!(Icon::parse_name("search"), Some(Icon::Search));
        assert_eq!(Icon::parse_name("icon-settings"), Some(Icon::Settings));
        assert_eq!(Icon::parse_name("nana-close"), Some(Icon::Close));
        assert_eq!(Icon::parse_name("plus"), Some(Icon::Add));
        assert_eq!(Icon::parse_name("lucide-search-icon"), Some(Icon::Search));
        assert_eq!(Icon::parse_name("lucide-folder"), Some(Icon::Folder));
        assert_eq!(
            Icon::parse_name("lucide-panel-left-open"),
            Some(Icon::Sidebar)
        );
        assert_eq!(
            Icon::parse_name("lucide-chevron-right"),
            Some(Icon::ChevronRight)
        );
        assert_eq!(Icon::parse_name("chevron-right"), Some(Icon::ChevronRight));
        assert_eq!(Icon::parse_name("arrow-right"), Some(Icon::ArrowRight));
        assert_eq!(Icon::parse_name("chevron-down"), Some(Icon::ChevronDown));
        assert_eq!(
            Icon::parse_name("lucide-chevron-down"),
            Some(Icon::ChevronDown)
        );
        assert_eq!(Icon::parse_name("lucide-minus"), Some(Icon::Minimize));
        assert_eq!(Icon::parse_name("lucide-square"), Some(Icon::Maximize));
        assert_eq!(Icon::parse_name("lucide-arrow-up"), Some(Icon::ArrowUp));
        assert_eq!(Icon::parse_name("lucide-bot"), Some(Icon::Bot));
        assert_eq!(Icon::parse_name("lucide-git-branch"), Some(Icon::GitBranch));
        assert_eq!(
            Icon::parse_name("lucide-message-square-plus"),
            Some(Icon::MessageSquarePlus)
        );
        assert_eq!(Icon::parse_name("lucide-paperclip"), Some(Icon::Paperclip));
        assert_eq!(
            Icon::parse_name("lucide-shield-check"),
            Some(Icon::ShieldCheck)
        );
        assert_eq!(Icon::parse_name("lucide-sparkles"), Some(Icon::Sparkles));
        assert_eq!(Icon::parse_name("sun"), Some(Icon::Appearance));
        assert_eq!(Icon::Sun, Icon::Appearance);
    }

    #[test]
    fn parse_name_rejects_catalog_and_wrong_aliases() {
        assert_eq!(Icon::parse_name("palette"), None);
        assert_eq!(Icon::parse_name("paintbrush"), None);
        assert_eq!(Icon::parse_name("puzzle"), None);
        assert_eq!(Icon::parse_name("lucide-puzzle"), None);
        assert_eq!(Icon::parse_name("atom"), None);
        assert_eq!(Icon::parse_name("package"), None);
        assert_eq!(Icon::parse_name("lucide-refresh-cw"), None);
        assert_eq!(Icon::parse_name("lucide-folder-open"), None);
        assert_eq!(Icon::parse_name("lucide-trash-2"), None);
        assert_eq!(Icon::parse_name("lucide-pin"), None);
        assert_eq!(Icon::parse_name("lucide-pencil"), None);
        assert_eq!(Icon::parse_name("lucide-unknown"), None);
        assert_eq!(Icon::parse_name("sliders"), None);
        assert_eq!(Icon::parse_name("home"), None);
    }

    #[test]
    fn shell_icons_have_geometry() {
        for icon in [
            Icon::About,
            Icon::Add,
            Icon::Appearance,
            Icon::ArrowLeft,
            Icon::ArrowRight,
            Icon::ArrowUp,
            Icon::Bot,
            Icon::ChevronDown,
            Icon::ChevronRight,
            Icon::ChevronUp,
            Icon::Chart,
            Icon::Close,
            Icon::Eye,
            Icon::File,
            Icon::Folder,
            Icon::GitBranch,
            Icon::Maximize,
            Icon::MessageSquarePlus,
            Icon::Minimize,
            Icon::Moon,
            Icon::Nodes,
            Icon::Paperclip,
            Icon::Restore,
            Icon::Search,
            Icon::Settings,
            Icon::ShieldCheck,
            Icon::Sidebar,
            Icon::Sparkles,
            Icon::Workspace,
        ] {
            assert!(!icon.shapes().is_empty(), "{icon:?}");
            assert!(
                icon.svg().contains("viewBox=\"0 0 24 24\""),
                "{icon:?} svg missing viewBox"
            );
            assert!(
                icon.svg().contains("currentColor"),
                "{icon:?} svg missing currentColor stroke"
            );
        }
    }
}
