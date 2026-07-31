//! NanaUI's native Lilia-style application framework.
//!
//! [`WorkspaceController`], [`WorkspaceSlots`], and [`workspace_view`] provide
//! the reusable workspace contract.

pub mod components;
pub mod dialog;
mod drag_handle;
pub mod geometry;
#[cfg(feature = "gpu")]
pub mod gpu_texture;
#[cfg(feature = "gpu")]
pub mod gpu_view;
pub mod icons;
pub mod layout;
pub mod menu;
pub mod overlay;
pub mod selection;
pub mod settings;
mod shell;
pub mod sidebar;
pub mod split_pane;
pub mod theme;
pub mod tooltip;
pub mod widgets;
pub mod window_chrome;
pub mod workspace;

pub use components::actions::{Button, ControlSize, IconButton};
#[cfg(feature = "calendar")]
pub use components::calendar::{
    CalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapCell, CalendarHeatmapDatum,
    CalendarHeatmapDayLabel, CalendarHeatmapEvent, CalendarHeatmapModel, CalendarHeatmapMonthLabel,
    CalendarHeatmapOptions, CalendarHeatmapState, CalendarLevelResolver, CalendarLevelStrategy,
    CalendarTitleFormatter, build_calendar_heatmap_model,
};
#[cfg(feature = "controls")]
pub use components::controls::{
    Checkbox, Input, RangeField, SegmentedControl, Select, SelectionOption, Switch, Tabs, Textarea,
};
#[cfg(feature = "feedback")]
pub use components::feedback::{
    Progress, Skeleton, Spinner, StatusBadge, StatusTone, Toast, ToastTone, ValidationIntent,
    ValidationMessage,
};
#[cfg(feature = "image-viewer")]
pub use components::image_viewer::{ImageViewer, ImageViewerSource};
pub use components::menus::{
    ActionMenuItem, AnchoredActionMenu, AnchoredMenuPlacement, AnchoredMenuPosition,
    ContextMenuEvent, ContextMenuHost, ContextMenuItem, OverlayHost,
};
#[cfg(feature = "overlays")]
pub use components::overlays::{ConfirmDialog, Dialog, Drawer, DrawerSide, Tooltip};
#[cfg(feature = "popover")]
pub use components::popover::{Popover, PopoverPlacement};
#[cfg(feature = "selects")]
pub use components::selects::{
    Dropdown, DropdownEvent, DropdownOption, DropdownSelection, SearchDropdown,
    SearchDropdownOption, SearchDropdownState,
};
#[cfg(feature = "settings-components")]
pub use components::settings_sections::{
    AboutMetadata, AboutSection, AppearanceEvent, AppearanceSection, SettingsCollapsibleCard,
};
#[cfg(feature = "surfaces")]
pub use components::surfaces::{Card, EmptyState, FormField, InteractiveCard, ListItem};
#[cfg(feature = "xy-pad")]
pub use components::xy_pad::{XYPad, XYPadEvent, XYPadState, XYPadValue};
pub use dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
pub use geometry::{LogicalRect, PhysicalRect, RegionRect, WorkspaceGeometry};
#[cfg(feature = "gpu")]
pub use gpu_texture::{GpuTextureView, HostTexture};
#[cfg(feature = "gpu")]
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
pub use split_pane::{SplitAxis, SplitPaneAction, SplitPaneController, split_pane};
pub use theme::{
    Colors, ThemeMetrics, ThemeMode, ThemeTokens, UI_BASE_TEXT_SIZE, UI_METRICS, ui_font,
    ui_font_defaults,
};
#[cfg(feature = "bundled-fonts")]
pub use theme::{UI_FONT_BOLD, UI_FONT_MEDIUM, UI_FONT_REGULAR, UI_FONT_SEMIBOLD, ui_font_sources};
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
