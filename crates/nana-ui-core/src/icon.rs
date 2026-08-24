/// Compact line icons used by NanaUI navigation surfaces.
///
/// This is the semantic identity only. Backend crates own glyph rendering.
/// Vue / Lucide icons should render as SVG subtree geometry, not by
/// stretching this enum with incorrect Lucide→glyph aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    About,
    Add,
    Appearance,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Bot,
    ChevronDown,
    ChevronRight,
    Chart,
    Close,
    Eye,
    File,
    Folder,
    GitBranch,
    Maximize,
    MessageSquarePlus,
    Minimize,
    Moon,
    Nodes,
    Paperclip,
    Restore,
    Search,
    Settings,
    ShieldCheck,
    Sidebar,
    Sparkles,
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
            "arrow-right" | "arrowright" => Self::ArrowRight,
            "arrow-up" | "arrowup" | "chevron-up" => Self::ArrowUp,
            "bot" | "bot-message-square" | "robot" => Self::Bot,
            "chevron-right" => Self::ChevronRight,
            "chevron-down" | "arrow-down" => Self::ChevronDown,
            "chart" | "line-chart" | "linechart" => Self::Chart,
            "close" | "x" | "x-mark" | "x-circle" | "circle-x" | "xcircle" => Self::Close,
            "eye" | "visibility" | "eye-off" | "eyeoff" => Self::Eye,
            "file" | "document" => Self::File,
            "folder" | "directory" | "folder-open" | "folderopen" => Self::Folder,
            "git-branch" | "gitbranch" | "branch" => Self::GitBranch,
            "maximize" | "square" => Self::Maximize,
            "message-square-plus" | "messagesquareplus" | "square-plus" => {
                Self::MessageSquarePlus
            }
            "minimize" | "minus" => Self::Minimize,
            "moon" | "dark" => Self::Moon,
            "nodes" | "graph" | "network" | "layout-grid" | "grid" => Self::Nodes,
            "paperclip" | "attachment" | "paper-clip" => Self::Paperclip,
            "restore" | "refresh-cw" | "refreshcw" | "rotate-cw" | "reload" | "sync" => {
                Self::Restore
            }
            "search" | "magnifier" | "magnifying-glass" => Self::Search,
            "settings" | "gear" | "cog" | "sliders" | "sliders-horizontal" => Self::Settings,
            "shield-check" | "shieldcheck" | "shield" => Self::ShieldCheck,
            "sidebar" | "panel-left" | "layout-panel-left" | "panel-left-open"
            | "panelleftopen" | "panel-left-close" | "panelleftclose" => Self::Sidebar,
            "sparkles" | "sparkle" | "wand-sparkles" => Self::Sparkles,
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
            Some(Icon::ChevronRight)
        );
        assert_eq!(Icon::parse_name("chevron-right"), Some(Icon::ChevronRight));
        assert_eq!(Icon::parse_name("arrow-right"), Some(Icon::ArrowRight));
        assert_eq!(Icon::parse_name("chevron-down"), Some(Icon::ChevronDown));
        assert_eq!(Icon::parse_name("arrow-down"), Some(Icon::ChevronDown));
        assert_eq!(
            Icon::parse_name("lucide-chevron-down"),
            Some(Icon::ChevronDown)
        );
        assert_eq!(Icon::parse_name("lucide-minus"), Some(Icon::Minimize));
        assert_eq!(Icon::parse_name("lucide-square"), Some(Icon::Maximize));
        assert_eq!(Icon::parse_name("lucide-refresh-cw"), Some(Icon::Restore));
        assert_eq!(Icon::parse_name("lucide-arrow-up"), Some(Icon::ArrowUp));
        assert_eq!(Icon::parse_name("lucide-bot"), Some(Icon::Bot));
        assert_eq!(
            Icon::parse_name("lucide-git-branch"),
            Some(Icon::GitBranch)
        );
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
    }

    #[test]
    fn parse_name_rejects_lucide_business_aliases() {
        // These used to map to wrong shell glyphs; Vue must use SVG instead.
        assert_eq!(Icon::parse_name("lucide-trash-2"), None);
        assert_eq!(Icon::parse_name("lucide-pin"), None);
        assert_eq!(Icon::parse_name("lucide-pencil"), None);
        assert_eq!(Icon::parse_name("lucide-user-round"), None);
        assert_eq!(Icon::parse_name("lucide-map-pin"), None);
        assert_eq!(Icon::parse_name("lucide-copy"), None);
        assert_eq!(Icon::parse_name("lucide-share-2"), None);
        assert_eq!(Icon::parse_name("lucide-filter"), None);
        assert_eq!(Icon::parse_name("lucide-list-checks"), None);
        assert_eq!(Icon::parse_name("lucide-unknown"), None);
        assert_eq!(Icon::parse_name("lucide-send"), None);
    }
}
