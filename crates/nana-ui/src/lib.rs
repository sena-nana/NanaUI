//! NanaUI's native Lilia-style application framework.
//!
//! [`WorkspaceController`], [`WorkspaceSlots`], and [`workspace_view`] provide
//! the reusable workspace contract. [`GalleryState`] powers the runnable
//! component gallery with real application state.

pub mod components;
pub mod dialog;
pub mod gallery;
pub mod geometry;
pub mod gpu_texture;
pub mod gpu_view;
pub mod icons;
pub mod layout;
pub mod menu;
pub mod overlay;
pub mod selection;
pub mod settings;
mod shell;
pub mod sidebar;
pub mod theme;
pub mod tooltip;
pub mod widgets;
pub mod window_chrome;
pub mod workspace;

pub use components::{
    AboutMetadata, AboutSection, ActionMenuItem, AnchoredActionMenu, AnchoredMenuPlacement,
    AnchoredMenuPosition, AppearanceEvent, AppearanceSection, Button, CalendarHeatmap,
    CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapDatum, CalendarHeatmapDayLabel,
    CalendarHeatmapEvent, CalendarHeatmapModel, CalendarHeatmapMonthLabel, CalendarHeatmapOptions,
    CalendarHeatmapState, CalendarLevelResolver, CalendarLevelStrategy, CalendarTitleFormatter,
    Card, Checkbox, ConfirmDialog, ContextMenuEvent, ContextMenuHost, ContextMenuItem, ControlSize,
    Dialog, Drawer, DrawerSide, Dropdown, DropdownEvent, DropdownOption, DropdownSelection,
    EmptyState, FormField, IconButton, ImageViewer, ImageViewerSource, Input, InteractiveCard,
    ListItem, OverlayHost, Popover, PopoverPlacement, Progress, RangeField, SearchDropdown,
    SearchDropdownOption, SearchDropdownState, SegmentedControl, Select, SelectionOption,
    SettingsCollapsibleCard, Skeleton, Spinner, StatusBadge, StatusTone, Switch, Tabs, Textarea,
    Toast, ToastTone, Tooltip, ValidationIntent, ValidationMessage, XYPad, XYPadEvent, XYPadState,
    XYPadValue, build_calendar_heatmap_model,
};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use gallery::{ContextAction, GalleryMessage, GallerySection, GalleryState, SurfaceView};
pub use geometry::{LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
pub use gpu_texture::{GpuTextureView, HostTexture};
pub use gpu_view::{GpuView, GpuViewMode, GpuViewPalette, RenderSlot};
pub use icons::{Icon, disclosure_icon, icon, spinner_icon, status_indicator};
pub use layout::{
    NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
    WorkspaceLayout, WorkspaceLayoutError,
};
pub use menu::{MenuConfirmation, MenuSelection};
pub use overlay::ExclusiveOverlay;
pub use selection::{SelectionMove, SingleSelection};
pub use settings::{
    AppearanceSettings, SettingsCard, SettingsError, SettingsModel, SettingsRow, SettingsState,
    SettingsTab, SettingsTabId, settings_page, settings_sidebar,
};
pub use shell::{
    AppTitleBar, DesktopShell, PopupShell, PopupTitleBarFrame, app_shell, app_title_bar,
};
pub use sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarRowTone,
    SidebarSection, SidebarSectionState,
};
pub use theme::{
    Colors, ThemeMetrics, ThemeMode, ThemeTokens, UI_BASE_TEXT_SIZE, UI_FONT_BOLD, UI_FONT_MEDIUM,
    UI_FONT_REGULAR, UI_FONT_SEMIBOLD, UI_METRICS, ui_font, ui_font_defaults, ui_font_sources,
};
pub use tooltip::{TooltipConfig, TooltipPlacement};
pub use widgets::{ButtonKind, CardKind};
pub use window_chrome::{
    WindowChrome, WindowChromeAction, WindowChromeEvent, WindowChromeState, WindowControlMode,
    custom_title_bar_window,
};
pub use workspace::{
    WorkspaceAction, WorkspaceController, WorkspaceRegion, WorkspaceRegions, WorkspaceSlots,
    workspace_view,
};
