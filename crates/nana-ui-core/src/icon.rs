/// Compact line icons used by NanaUI navigation surfaces.
///
/// This is the semantic identity only. Backend crates own glyph rendering.
/// Vue / Lucide icons should render via iced SVG (subtree geometry), not by
/// stretching this enum with incorrect Lucide→glyph aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    About,
    Add,
    Appearance,
    ArrowLeft,
    ArrowRight,
    Chart,
    Close,
    Eye,
    File,
    Folder,
    Maximize,
    Minimize,
    Moon,
    Nodes,
    Restore,
    Search,
    Settings,
    Sidebar,
    Workspace,
}

impl Icon {
    /// Parse a stable shell icon name (`search`, `settings`, …).
    ///
    /// Only accepts names that genuinely match this enum. Lucide business icons
    /// (`trash-2`, `pin`, `pencil`, …) return `None` — Vue renders those as SVG.
    pub fn parse_name(raw: &str) -> Option<Self> {
        let s = raw.trim().to_ascii_lowercase();
        let s = s
            .strip_prefix("icon-")
            .or_else(|| s.strip_prefix("nana-"))
            .unwrap_or(s.as_str());
        // Strip common Lucide / SVG prefixes.
        let s = s
            .strip_prefix("lucide-")
            .or_else(|| s.strip_prefix("icon:"))
            .unwrap_or(s);
        // Lucide Vue emits both `lucide-search` and `lucide-search-icon`.
        let s = s.strip_suffix("-icon").unwrap_or(s);
        Some(match s {
            "about" | "info" | "circle-info" | "info-circle" => Self::About,
            "add" | "plus" | "plus-circle" => Self::Add,
            "appearance" | "palette" | "sun" | "sun-medium" | "paintbrush" => Self::Appearance,
            "arrow-left" | "arrowleft" | "back" | "chevron-left" => Self::ArrowLeft,
            "arrow-right" | "arrowright" | "chevron-right" => Self::ArrowRight,
            "chart" | "line-chart" | "linechart" => Self::Chart,
            "close" | "x" | "x-mark" | "x-circle" | "circle-x" | "xcircle" => Self::Close,
            "eye" | "visibility" | "eye-off" | "eyeoff" => Self::Eye,
            "file" | "document" => Self::File,
            "folder" | "directory" | "folder-open" | "folderopen" => Self::Folder,
            "maximize" | "square" => Self::Maximize,
            "minimize" | "minus" => Self::Minimize,
            "moon" | "dark" => Self::Moon,
            "nodes" | "graph" | "network" | "layout-grid" | "grid" => Self::Nodes,
            "restore" | "refresh-cw" | "refreshcw" | "rotate-cw" | "reload" | "sync" => {
                Self::Restore
            }
            "search" | "magnifier" | "magnifying-glass" => Self::Search,
            "settings" | "gear" | "cog" | "sliders" | "sliders-horizontal" => Self::Settings,
            "sidebar" | "panel-left" | "layout-panel-left" | "panel-left-open"
            | "panelleftopen" | "panel-left-close" | "panelleftclose" => Self::Sidebar,
            "workspace" | "layout-dashboard" | "home" | "house" => Self::Workspace,
            _ => return None,
        })
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
        assert_eq!(Icon::parse_name("lucide-folder-open"), Some(Icon::Folder));
        assert_eq!(
            Icon::parse_name("lucide-panel-left-open"),
            Some(Icon::Sidebar)
        );
        assert_eq!(
            Icon::parse_name("lucide-chevron-right"),
            Some(Icon::ArrowRight)
        );
        assert_eq!(Icon::parse_name("lucide-minus"), Some(Icon::Minimize));
        assert_eq!(Icon::parse_name("lucide-square"), Some(Icon::Maximize));
        assert_eq!(Icon::parse_name("lucide-refresh-cw"), Some(Icon::Restore));
    }

    #[test]
    fn parse_name_rejects_lucide_business_aliases() {
        // These used to map to wrong shell glyphs; Vue must use SVG instead.
        assert_eq!(Icon::parse_name("lucide-trash-2"), None);
        assert_eq!(Icon::parse_name("lucide-pin"), None);
        assert_eq!(Icon::parse_name("lucide-pencil"), None);
        assert_eq!(Icon::parse_name("lucide-git-branch"), None);
        assert_eq!(Icon::parse_name("lucide-user-round"), None);
        assert_eq!(Icon::parse_name("lucide-map-pin"), None);
        assert_eq!(Icon::parse_name("lucide-copy"), None);
        assert_eq!(Icon::parse_name("lucide-share-2"), None);
        assert_eq!(Icon::parse_name("lucide-filter"), None);
        assert_eq!(Icon::parse_name("lucide-list-checks"), None);
        assert_eq!(Icon::parse_name("lucide-unknown"), None);
        assert_eq!(Icon::parse_name("lucide-send"), None);
        assert_eq!(Icon::parse_name("lucide-sparkles"), None);
    }
}
