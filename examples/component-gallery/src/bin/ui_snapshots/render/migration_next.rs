#[path = "migration_next/selection.rs"]
mod selection;
use selection::*;
#[path = "migration_next/workspace.rs"]
mod workspace;
use workspace::*;
#[path = "migration_next/feedback.rs"]
mod feedback;
use feedback::*;
#[path = "migration_next/evidence.rs"]
mod evidence;
use evidence::*;
#[path = "migration_next/fixture_values.rs"]
mod fixture_values;
use fixture_values::*;
#[path = "migration_next/catalog.rs"]
mod catalog;
use catalog::*;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_ui::runtime::{
    AboutMetadata as RuntimeAboutMetadata, AboutSection as RuntimeAboutSection,
    AccessibilityAction, AccessibilityActionRequest, ActionMenu as RuntimeActionMenu,
    ActionMenuItem as RuntimeActionMenuItem, Activate,
    AnchoredActionMenu as RuntimeAnchoredActionMenu, AppShell as RuntimeAppShell,
    AppTitleBar as RuntimeAppTitleBar, AppearanceSection as RuntimeAppearanceSection,
    Button as RuntimeButton, CalendarHeatmap as RuntimeCalendarHeatmap,
    CalendarHeatmapDatum as RuntimeCalendarDatum, Card as RuntimeCard, Checkbox as RuntimeCheckbox,
    CommandPalette as RuntimeCommandPalette, ConfirmDialog as RuntimeConfirmDialog, ConfirmSlots,
    ContextMenu as RuntimeContextMenu, ContextMenuItem as RuntimeContextMenuItem,
    DesktopShell as RuntimeDesktopShell, Dialog as RuntimeDialog, Dock as RuntimeDock,
    DockNode as RuntimeDockNode, DockPanel as RuntimeDockPanel, DocumentId,
    Drawer as RuntimeDrawer, Dropdown as RuntimeDropdown, DropdownOption as RuntimeDropdownOption,
    EmptyState as RuntimeEmptyState, Entity, FormField as RuntimeFormField,
    GpuTextureView as RuntimeGpuTextureView, GpuView as RuntimeGpuView,
    GpuViewPalette as RuntimeGpuViewPalette, GraphCanvas as RuntimeGraphCanvas,
    HostedTextarea as RuntimeHostedTextarea, IconButton as RuntimeIconButton,
    ImageViewer as RuntimeImageViewer, ImageViewerContent,
    InteractiveCard as RuntimeInteractiveCard, KeyCaptureLayer as RuntimeKeyCaptureLayer,
    KeymapLayer as RuntimeKeymapLayer, LabeledValue as RuntimeLabeledValue, LayoutViewport,
    LevelMeter as RuntimeLevelMeter, List as RuntimeList, ListItem as RuntimeListItem,
    ListItemSlots, MarkdownBlock, MarkdownBlockKind, MarkdownSpan, ModalSlots, MountState,
    MutationQueue, NativeMarkdown as RuntimeNativeMarkdown, NodeStyle,
    OverlayHost as RuntimeOverlayHost, PaneChrome as RuntimePaneChrome,
    PaneTree as RuntimePaneTree, PaneTreeNode as RuntimePaneTreeNode, Popover as RuntimePopover,
    Progress as RuntimeProgress, QrCode as RuntimeQrCode, RangeField as RuntimeRangeField,
    ReorderItem as RuntimeReorderItem, ReorderList as RuntimeReorderList, RichSpan,
    RuntimeDocument, SearchDropdown as RuntimeSearchDropdown,
    SearchDropdownOption as RuntimeSearchDropdownOption,
    SegmentedControl as RuntimeSegmentedControl, SegmentedOption as RuntimeSegmentedOption,
    SegmentedSelectionRequested, Select as RuntimeSelect, SelectOption as RuntimeSelectOption,
    SelectableRichText as RuntimeSelectableRichText, SettingsCard as RuntimeSettingsCard,
    SettingsCollapsibleCard as RuntimeSettingsCollapsibleCard, SettingsPage as RuntimeSettingsPage,
    SettingsSidebar as RuntimeSettingsSidebar, SidebarFooter as RuntimeSidebarFooter,
    SidebarFooterButton as RuntimeSidebarFooterButton, SidebarFrame as RuntimeSidebarFrame,
    SidebarRow as RuntimeSidebarRow, SidebarSection as RuntimeSidebarSection,
    Skeleton as RuntimeSkeleton, Spinner as RuntimeSpinner, SplitPane as RuntimeSplitPane,
    StableNodeId, StatusBadge as RuntimeStatusBadge, Switch as RuntimeSwitch,
    TabOption as RuntimeTabOption, Tabs as RuntimeTabs, Text as RuntimeText,
    TextArea as RuntimeTextArea, TextHorizontalAlignment, TextInput as RuntimeTextInput,
    TextSelection, TextVerticalAlignment, Thumbnail as RuntimeThumbnail,
    TimeSeriesChart as RuntimeTimeSeriesChart, Toast as RuntimeToast, TreeView as RuntimeTreeView,
    ValidationMessage as RuntimeValidationMessage, ValueEmphasis, Workspace as RuntimeWorkspace,
    WorkspaceRegionSlot, XYPad as RuntimeXYPad,
};
use nana_ui::{
    ActionId, AppearanceSettings, CardKind, CommandPaletteItem, ComponentId, ControlSize,
    GraphEdge, GraphEndpoint, GraphModel, GraphNode, GraphPoint, GraphPort, GraphPortKind,
    GraphPortSide, GraphSize, Icon, NanaTextShaper, RegionId, RegionRole, RegionState,
    RuntimeInputAdapter, SettingsModel, SettingsState, SettingsTab, SettingsTabId, SplitAxis,
    ThemeMode, ThemeModeExt, TooltipConfig, TooltipPlacement, TreeNode, WindowMaterialMode,
    WorkspaceLayout, XYPadValue, component_catalog, component_ids,
};
use nana_ui_core::{
    DialogSize, DrawerSide, LengthSpec, SemanticColorRole, SplitPaneModel, StatusTone,
    SwitchControlPosition, ToastTone, ValidationIntent, WorkspaceModel,
};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};
use nana_ui_scene::ScenePrimitiveKind;

use crate::write::{self, Size};

use super::gpu::{self, SnapshotGpu};
use super::{pixel_difference, side_by_side};

const SIZE: Size<u32> = Size::new(420, 120);
const GAP: u32 = 8;
const SLOT_INSET: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Text,
    Button,
    TextInput,
    Textarea,
    HostedTextarea,
    Checkbox,
    IconButton,
    Switch,
    Card,
    ListItem,
    RangeField,
    SegmentedControl,
    Tabs,
    StatusBadge,
    ValidationMessage,
    EmptyState,
    LabeledValue,
    Progress,
    Spinner,
    Skeleton,
    LevelMeter,
    FormField,
    InteractiveCard,
    Tooltip,
    Dialog,
    ConfirmDialog,
    Drawer,
    Toast,
    XYPad,
    QrCode,
    Select,
    Popover,
    ActionMenu,
    ActionMenuItem,
    AnchoredActionMenu,
    ContextMenu,
    SidebarFrame,
    SidebarSection,
    SidebarFooter,
    AppearanceSection,
    AboutSection,
    SettingsCollapsibleCard,
    CommandPalette,
    OverlayHost,
    Dropdown,
    SearchDropdown,
    TreeView,
    SidebarRow,
    Settings,
    SettingsSidebar,
    SettingsPage,
    Workspace,
    Dock,
    DockPanel,
    SplitPane,
    PaneChrome,
    PaneTree,
    AppShell,
    DesktopShell,
    AppTitleBar,
    CalendarHeatmap,
    TimeSeriesChart,
    ReorderList,
    NativeMarkdown,
    SelectableRichText,
    ImageViewer,
    GraphCanvas,
    GraphMinimap,
    KeyCaptureLayer,
    KeymapLayer,
    GpuTextureView,
    GpuView,
    Thumbnail,
}

impl Component {
    const fn id(self) -> ComponentId {
        match self {
            Self::Text => component_ids::TEXT,
            Self::Button => component_ids::BUTTON,
            Self::TextInput => component_ids::TEXT_INPUT,
            Self::Textarea => component_ids::TEXTAREA,
            Self::HostedTextarea => component_ids::HOSTED_TEXTAREA,
            Self::Checkbox => component_ids::CHECKBOX,
            Self::IconButton => component_ids::ICON_BUTTON,
            Self::Switch => component_ids::SWITCH,
            Self::Card => component_ids::CARD,
            Self::ListItem => component_ids::LIST_ITEM,
            Self::RangeField => component_ids::RANGE_FIELD,
            Self::SegmentedControl => component_ids::SEGMENTED_CONTROL,
            Self::Tabs => component_ids::TABS,
            Self::StatusBadge => component_ids::STATUS_BADGE,
            Self::ValidationMessage => component_ids::VALIDATION_MESSAGE,
            Self::EmptyState => component_ids::EMPTY_STATE,
            Self::LabeledValue => component_ids::LABELED_VALUE,
            Self::Progress => component_ids::PROGRESS,
            Self::Spinner => component_ids::SPINNER,
            Self::Skeleton => component_ids::SKELETON,
            Self::LevelMeter => component_ids::LEVEL_METER,
            Self::FormField => component_ids::FORM_FIELD,
            Self::InteractiveCard => component_ids::INTERACTIVE_CARD,
            Self::Tooltip => component_ids::TOOLTIP,
            Self::Dialog => component_ids::DIALOG,
            Self::ConfirmDialog => component_ids::CONFIRM_DIALOG,
            Self::Drawer => component_ids::DRAWER,
            Self::Toast => component_ids::TOAST,
            Self::XYPad => component_ids::XY_PAD,
            Self::QrCode => component_ids::QR_CODE,
            Self::Select => component_ids::SELECT,
            Self::Popover => component_ids::POPOVER,
            Self::ActionMenu => component_ids::ACTION_MENU,
            Self::ActionMenuItem => component_ids::ACTION_MENU_ITEM,
            Self::AnchoredActionMenu => component_ids::ANCHORED_ACTION_MENU,
            Self::ContextMenu => component_ids::CONTEXT_MENU,
            Self::SidebarFrame => component_ids::SIDEBAR_FRAME,
            Self::SidebarSection => component_ids::SIDEBAR_SECTION,
            Self::SidebarFooter => component_ids::SIDEBAR_FOOTER,
            Self::AppearanceSection => component_ids::APPEARANCE_SECTION,
            Self::AboutSection => component_ids::ABOUT_SECTION,
            Self::SettingsCollapsibleCard => component_ids::SETTINGS_COLLAPSIBLE_CARD,
            Self::CommandPalette => component_ids::COMMAND_PALETTE,
            Self::OverlayHost => component_ids::OVERLAY_HOST,
            Self::Dropdown => component_ids::DROPDOWN,
            Self::SearchDropdown => component_ids::SEARCH_DROPDOWN,
            Self::TreeView => component_ids::TREE_VIEW,
            Self::SidebarRow => component_ids::SIDEBAR_ROW,
            Self::Settings => component_ids::SETTINGS,
            Self::SettingsSidebar => component_ids::SETTINGS,
            Self::SettingsPage => component_ids::SETTINGS,
            Self::Workspace => component_ids::WORKSPACE,
            Self::Dock => component_ids::DOCK,
            Self::DockPanel => component_ids::DOCK_PANEL,
            Self::SplitPane => component_ids::SPLIT_PANE,
            Self::PaneChrome => component_ids::PANE_CHROME,
            Self::PaneTree => component_ids::PANE_TREE,
            Self::AppShell => component_ids::APP_SHELL,
            Self::DesktopShell => component_ids::APP_SHELL,
            Self::AppTitleBar => component_ids::APP_TITLE_BAR,
            Self::CalendarHeatmap => component_ids::CALENDAR_HEATMAP,
            Self::TimeSeriesChart => component_ids::TIME_SERIES_CHART,
            Self::ReorderList => component_ids::REORDER_LIST,
            Self::NativeMarkdown => component_ids::NATIVE_MARKDOWN,
            Self::SelectableRichText => component_ids::SELECTABLE_RICH_TEXT,
            Self::ImageViewer => component_ids::IMAGE_VIEWER,
            Self::GraphCanvas => component_ids::GRAPH_CANVAS,
            Self::GraphMinimap => component_ids::GRAPH_MINIMAP,
            Self::KeyCaptureLayer => component_ids::KEY_CAPTURE_LAYER,
            Self::KeymapLayer => component_ids::KEYMAP_LAYER,
            Self::GpuTextureView => component_ids::GPU_TEXTURE_VIEW,
            Self::GpuView => component_ids::GPU_VIEW,
            Self::Thumbnail => component_ids::THUMBNAIL,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Fixture {
    id: ComponentId,
    component: Component,
    state: &'static str,
    expected: &'static str,
    reference_contract: &'static str,
    runtime_contract: &'static str,
    divergence: &'static str,
}

fn tooltip_fixture_config(state: &str) -> TooltipConfig {
    TooltipConfig {
        placement: if matches!(state, "edge" | "tooltip-edge") {
            TooltipPlacement::Left
        } else {
            TooltipPlacement::FollowCursor
        },
        delay_ms: if state == "delay" { 350 } else { 0 },
        gap: 6.0,
        viewport_padding: 4.0,
        max_width: 280.0,
    }
}

pub(super) fn generate_registered(
    snapshots: &mut super::offscreen::OffscreenSnapshots,
    output: &Path,
    theme: ThemeMode,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    validate_fixture_registry().map_err(std::io::Error::other)?;

    let colors = theme.colors();
    let gpu = gpu::create_snapshot_gpu(
        &snapshots.device,
        &snapshots.queue,
        colors.background,
        colors.accent_strong,
    );
    let mut paths = Vec::with_capacity(FIXTURE_REGISTRY.len() * 5);
    for fixture in FIXTURE_REGISTRY {
        paths.extend(render_fixture(snapshots, output, theme, *fixture, &gpu)?);
    }
    Ok(paths)
}

fn validate_fixture_registry() -> Result<(), String> {
    use std::collections::BTreeSet;

    let catalog_ids = component_catalog()
        .iter()
        .map(|support| support.id)
        .collect::<BTreeSet<_>>();
    let mut registered_ids = BTreeSet::new();
    let mut registered_states = BTreeSet::new();

    for fixture in FIXTURE_REGISTRY {
        let id = fixture.id;
        if !catalog_ids.contains(&id) {
            return Err(format!(
                "snapshot fixture references component `{id}` absent from the catalog"
            ));
        }
        if !registered_states.insert((id, fixture.state)) {
            return Err(format!(
                "duplicate snapshot fixture for component `{id}` state `{}`",
                fixture.state
            ));
        }
        registered_ids.insert(id);
    }

    let missing = component_catalog()
        .iter()
        .filter(|support| support.compiled && !registered_ids.contains(&support.id))
        .map(|support| support.id.as_str())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "compiled components lack snapshot fixtures: {}",
            missing.join(", ")
        ))
    }
}

fn render_fixture(
    snapshots: &mut super::offscreen::OffscreenSnapshots,
    output: &Path,
    theme: ThemeMode,
    fixture: Fixture,
    gpu: &SnapshotGpu,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let size = fixture_size(fixture);
    let theme_name = match theme {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    };
    let directory = output
        .join("component-migration")
        .join(fixture.id.as_str())
        .join(theme_name)
        .join(fixture.state);

    let runtime = runtime_fixture(theme, fixture, size)?;
    let (host_textures, gpu_renderers) = if is_gpu_fixture(fixture) {
        (Some(&gpu.textures), Some(&gpu.renderers))
    } else {
        (None, None)
    };
    let runtime_pixels = snapshots.paint(
        runtime.document.scene(),
        size,
        [
            theme.colors().background.r,
            theme.colors().background.g,
            theme.colors().background.b,
            theme.colors().background.a,
        ],
        host_textures,
        gpu_renderers,
    )?;
    let runtime_path = directory.join("runtime.png");
    write::png(&runtime_path, size, &runtime_pixels)?;

    let reference_path = directory.join("reference.png");
    let reference_pixels = if let Some((png_size, pixels)) = write::read_png(&reference_path) {
        if png_size == size && pixels.len() == runtime_pixels.len() {
            pixels
        } else {
            runtime_pixels.clone()
        }
    } else {
        write::png(&reference_path, size, &runtime_pixels)?;
        runtime_pixels.clone()
    };

    let side_size = Size::new(size.width * 2 + GAP, size.height);
    let side = side_by_side(&reference_pixels, &runtime_pixels, size, GAP);
    let side_path = directory.join("side-by-side.png");
    write::png(&side_path, side_size, &side)?;
    let difference = pixel_difference(&reference_pixels, &runtime_pixels);
    let difference_path = directory.join("difference.png");
    write::png(&difference_path, size, &difference)?;

    let evidence_path = directory.join("evidence.txt");
    write_evidence(&evidence_path, fixture, &runtime)?;
    Ok(vec![
        reference_path,
        runtime_path,
        side_path,
        difference_path,
        evidence_path,
    ])
}

fn fixture_size(fixture: Fixture) -> Size<u32> {
    match (fixture.component, fixture.state) {
        (Component::EmptyState, "complete-action") => Size::new(420, 190),
        (Component::EmptyState, "narrow-cjk") => Size::new(220, 220),
        (Component::EmptyState, "extreme-clip") => Size::new(92, 180),
        (Component::FormField, _) => Size::new(420, 160),
        (Component::InteractiveCard, _) => Size::new(420, 140),
        (Component::Tooltip, _) => Size::new(420, 140),
        (Component::Dialog, _) => Size::new(560, 280),
        (Component::ConfirmDialog, _) => Size::new(560, 260),
        (Component::Drawer, _) => Size::new(420, 240),
        (Component::Toast, _) => Size::new(420, 88),
        (Component::XYPad, _) => Size::new(420, 88),
        (Component::QrCode, _) => Size::new(280, 280),
        (Component::Select, "opened") => Size::new(420, 220),
        (Component::Popover | Component::ActionMenu, _) => Size::new(420, 200),
        (Component::AnchoredActionMenu | Component::ContextMenu, _) => Size::new(420, 220),
        (Component::SidebarFrame, _) => Size::new(240, 280),
        (Component::SidebarSection, _) => Size::new(240, 180),
        (Component::SidebarFooter, _) => Size::new(240, 72),
        (Component::AppearanceSection, _) => Size::new(420, 560),
        (Component::AboutSection, _) => Size::new(420, 180),
        (Component::SettingsCollapsibleCard, _) => Size::new(420, 160),
        (Component::CommandPalette, _) => Size::new(560, 320),
        (Component::OverlayHost, _) => Size::new(420, 160),
        (Component::TreeView, _) => Size::new(280, 160),
        (Component::Settings, _) => Size::new(420, 140),
        (Component::SettingsSidebar, _) => Size::new(220, 400),
        (Component::SettingsPage, _) => Size::new(420, 360),
        (Component::Workspace, _) => Size::new(720, 400),
        (Component::Dock, _) => Size::new(640, 320),
        (Component::DockPanel, _) => Size::new(280, 120),
        (Component::SplitPane, _) => Size::new(480, 200),
        (Component::PaneChrome, _) => Size::new(420, 180),
        (Component::PaneTree, _) => Size::new(480, 240),
        (Component::AppShell, _) => Size::new(560, 280),
        (Component::DesktopShell, _) => Size::new(560, 360),
        (Component::AppTitleBar, _) => Size::new(560, 80),
        (Component::CalendarHeatmap, _) => Size::new(280, 180),
        (Component::TimeSeriesChart, _) => Size::new(420, 180),
        (Component::NativeMarkdown, _) => Size::new(420, 140),
        (Component::ImageViewer, _) => Size::new(420, 240),
        (Component::GraphCanvas, _) => Size::new(420, 180),
        (Component::Thumbnail, "wide") => Size::new(80, 40),
        (Component::Thumbnail, _) => Size::new(40, 40),
        _ => SIZE,
    }
}

fn textarea_is_focused(state: &str) -> bool {
    matches!(
        state,
        "focused" | "selection" | "multiline-selection" | "invalid-focused" | "scroll"
    )
}

fn is_gpu_fixture(fixture: Fixture) -> bool {
    matches!(
        fixture.component,
        Component::GpuTextureView | Component::GpuView
    ) || (fixture.component == Component::Thumbnail && fixture.state == "ready")
}

struct RuntimeEvidence {
    document: RuntimeDocument,
    target: StableNodeId,
    first_passes: usize,
    first_accessibility_updates: usize,
    final_passes: usize,
    final_accessibility_updates: usize,
    idle: bool,
    action_applied: bool,
    feedback_contract_ok: bool,
    segmented_contract_ok: bool,
    segmented_options: Vec<StableNodeId>,
    segmented_requests: usize,
    next_deadline: Option<Duration>,
}

struct FeedbackActionFixture {
    action: Entity<RuntimeButton>,
    replacement: Entity<RuntimeButton>,
    activations: Arc<Mutex<usize>>,
}

struct SegmentedFixture {
    control: Entity<RuntimeSegmentedControl>,
    options: Vec<Entity<RuntimeSegmentedOption>>,
    requests: Arc<Mutex<Vec<StableNodeId>>>,
}

fn runtime_fixture(
    theme: ThemeMode,
    fixture: Fixture,
    size: Size<u32>,
) -> Result<RuntimeEvidence, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(9).expect("migration fixture document id");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let mut root_style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut root_style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.height = Some(LengthSpec::Percent(100.0));
        layout.padding_left = Some(LengthSpec::Px(20.0));
        layout.padding_right = Some(LengthSpec::Px(20.0));
        let vertical_padding = if fixture.component == Component::Textarea {
            12.0
        } else {
            20.0
        };
        layout.padding_top = Some(LengthSpec::Px(vertical_padding));
        layout.padding_bottom = Some(LengthSpec::Px(vertical_padding));
    }
    let root = document
        .context_mut()
        .create_component(document_id, RuntimeList::new().style(root_style))?;
    let feedback_action = if matches!(
        (fixture.component, fixture.state),
        (Component::EmptyState, "complete-action") | (Component::LabeledValue, "action")
    ) {
        let (label, replacement_label, kind) = if fixture.component == Component::EmptyState {
            (
                "Create project",
                "Import project",
                nana_ui::ButtonKind::Primary,
            )
        } else {
            ("View revision", "Open revision", nana_ui::ButtonKind::Ghost)
        };
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        let (action, replacement) = document.context_mut().build_detached(document_id, |ui| {
            let action = ui.leaf(RuntimeButton::new(label).kind(kind));
            let replacement = ui.leaf(RuntimeButton::new(replacement_label).kind(kind));
            ui.on(action, move |_button, _event: &Activate, _context| {
                *observed.lock().expect("feedback activation count") += 1;
            });
            (action, replacement)
        })?;
        Some(FeedbackActionFixture {
            action,
            replacement,
            activations,
        })
    } else {
        None
    };

    let mut segmented_fixture = None;
    let target = match fixture.component {
        Component::Text => {
            let mut style = NodeStyle {
                foreground: Some(if fixture.state == "muted" {
                    SemanticColorRole::Muted
                } else {
                    SemanticColorRole::Text
                }),
                text_horizontal_alignment: if fixture.state == "centered" {
                    TextHorizontalAlignment::Center
                } else {
                    TextHorizontalAlignment::Start
                },
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            };
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(if matches!(fixture.state, "wrap" | "ellipsis") {
                LengthSpec::Px(180.0)
            } else {
                LengthSpec::Percent(100.0)
            });
            layout.height = Some(LengthSpec::Px(if fixture.state == "wrap" {
                44.0
            } else {
                32.0
            }));
            layout.font_size = Some(13.0);
            layout.white_space_nowrap = fixture.state == "ellipsis";
            layout.text_overflow_ellipsis = fixture.state == "ellipsis";
            document
                .context_mut()
                .create_component(
                    document_id,
                    RuntimeText::new(if matches!(fixture.state, "wrap" | "ellipsis") {
                        "A deliberately long migration label that must respect its authored content box."
                    } else {
                        "Migration text 文本"
                    })
                    .style(style),
                )?
                .stable_id()
        }
        Component::Button => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeButton::new("Run build")
                    .kind(button_kind(fixture.state))
                    .size(button_control_size(fixture.state))
                    .disabled(fixture.state == "disabled")
                    .loading(fixture.state == "loading"),
            )?
            .stable_id(),
        Component::TextInput => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTextInput::new(if fixture.state == "placeholder" {
                    ""
                } else {
                    "release/next"
                })
                .label("Branch name")
                .placeholder("Branch name")
                .size(text_input_control_size(fixture.state))
                .disabled(fixture.state == "disabled")
                .loading(fixture.state == "loading")
                .read_only(fixture.state == "read-only")
                .invalid(fixture.state == "invalid")
                .secure(fixture.state == "secure"),
            )?
            .stable_id(),
        Component::Textarea => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTextArea::new(textarea_value(fixture.state))
                    .label("Issue description")
                    .placeholder("Describe the issue")
                    .height(96.0)
                    .invalid(fixture.state == "invalid-focused")
                    .disabled(fixture.state == "disabled"),
            )?
            .stable_id(),
        Component::HostedTextarea => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeHostedTextarea::new(hosted_textarea_value(fixture.state), "rs")
                    .label("Highlighted source")
                    .placeholder("fn main")
                    .height(96.0)
                    .disabled(fixture.state == "disabled"),
            )?
            .stable_id(),
        Component::CalendarHeatmap => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCalendarHeatmap::new([
                    RuntimeCalendarDatum::<()>::new("2026-06-01", 1.0),
                    RuntimeCalendarDatum::<()>::new("2026-06-02", 4.0),
                    RuntimeCalendarDatum::<()>::new("2026-06-03", 8.0),
                ]),
            )?
            .stable_id(),
        Component::TimeSeriesChart => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTimeSeriesChart::new([2.0, 5.0, 3.0, 8.0]),
            )?
            .stable_id(),
        Component::ReorderList => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeReorderList::new([
                    RuntimeReorderItem::new("alpha", "Alpha").selected(true),
                    RuntimeReorderItem::new("beta", "Beta"),
                    RuntimeReorderItem::new("gamma", "Gamma"),
                ]),
            )?
            .stable_id(),
        Component::NativeMarkdown => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeNativeMarkdown::from_blocks([
                    MarkdownBlock::Text {
                        kind: MarkdownBlockKind::Heading(1),
                        spans: vec![MarkdownSpan::plain("Title")],
                    },
                    MarkdownBlock::Text {
                        kind: MarkdownBlockKind::Paragraph,
                        spans: vec![MarkdownSpan::plain("Body copy.")],
                    },
                ]),
            )?
            .stable_id(),
        Component::SelectableRichText => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeSelectableRichText::new([RichSpan::plain("See "), RichSpan::plain("docs")]),
            )?
            .stable_id(),
        Component::ImageViewer => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeImageViewer::new(ImageViewerContent::None).name("Preview"),
            )?
            .stable_id(),
        Component::GraphMinimap => document
            .context_mut()
            .create_component(
                document_id,
                nana_ui::runtime::GraphMinimap::new(snapshot_graph())
                    .canvas_size(nana_ui_core::GraphSize::new(200.0, 100.0))
                    .style({
                        let mut style = NodeStyle::default();
                        let layout = Arc::make_mut(&mut style.layout);
                        layout.width = Some(LengthSpec::Fill);
                        layout.height = Some(LengthSpec::Px(100.0));
                        style
                    }),
            )?
            .stable_id(),
        Component::GraphCanvas => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeGraphCanvas::new("snapshot", snapshot_graph()),
            )?
            .stable_id(),
        Component::KeyCaptureLayer => document
            .context_mut()
            .create_component(document_id, RuntimeKeyCaptureLayer::new().recording(true))?
            .stable_id(),
        Component::KeymapLayer => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeKeymapLayer::new(
                    nana_ui::runtime::Keymap::new([]),
                    nana_ui_core::KeyContext::default(),
                    nana_ui::runtime::ActionRegistry::new(),
                ),
            )?
            .stable_id(),
        Component::Checkbox => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCheckbox::new(
                    "Notifications",
                    matches!(
                        fixture.state,
                        "on" | "pointer-toggle" | "space-toggle" | "accessibility-toggle"
                    ),
                )
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid"),
            )?
            .stable_id(),
        Component::IconButton => {
            let tooltip = TooltipConfig {
                placement: if fixture.state == "tooltip-edge" {
                    TooltipPlacement::Left
                } else {
                    TooltipPlacement::FollowCursor
                },
                ..TooltipConfig::default()
            };
            let component = RuntimeIconButton::new(Icon::Add, "Add source")
                .selected(fixture.state == "selected")
                .disabled(fixture.state == "disabled")
                .tooltip("Add source", tooltip);
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Switch => {
            let mut component = RuntimeSwitch::new("Auto build", fixture.state == "on")
                .hint("Run when sources change")
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid");
            component.control_position = if fixture.state == "control-start" {
                SwitchControlPosition::Start
            } else {
                SwitchControlPosition::End
            };
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Card => {
            let mut component = RuntimeCard::new()
                .title("Pipeline")
                .kind(card_kind(fixture.state))
                .loading(fixture.state == "loading")
                .padding(if fixture.state == "padding" {
                    28.0
                } else {
                    14.0
                });
            if fixture.state == "fixed-height" {
                component = component.height(92.0);
            }
            set_full_width(&mut component.style);
            document.context_mut().build(document_id, |ui| {
                let body = ui.leaf(RuntimeText::new(if fixture.state == "long-content" {
                    "A deliberately long body that must remain inside the card content region even when space is constrained."
                } else {
                    "Build status: ready"
                }));
                let card = ui.child("card", component);
                ui.nest(card, |ui| ui.adopt(body));
                card.stable_id()
            })?
        }
        Component::ListItem => {
            let mut component = RuntimeListItem::new(if fixture.state == "auto-height" {
                "Primary line\nSupporting line"
            } else {
                "Camera source"
            })
            .selected(fixture.state.starts_with("selected"))
            .disabled(fixture.state == "disabled")
            .size(control_size(fixture.state))
            .auto_height(fixture.state == "auto-height");
            set_full_width(&mut component.style);
            if fixture.state == "three-slots" {
                let item = document.context_mut().build(document_id, |ui| {
                    let leading = ui.leaf(RuntimeText::new("●"));
                    let content = ui.leaf(RuntimeText::new("Camera source"));
                    let trailing = ui.leaf(RuntimeText::new("⌘1"));
                    let item = ui.child("item", component);
                    (item, leading, content, trailing)
                })?;
                let (item, leading, content, trailing) = item;
                document.context_mut().set_list_item_slots(
                    item,
                    ListItemSlots {
                        leading: Some(leading.stable_id()),
                        content: Some(content.stable_id()),
                        trailing: Some(trailing.stable_id()),
                    },
                )?;
                item.stable_id()
            } else {
                document
                    .context_mut()
                    .create_component(document_id, component)?
                    .stable_id()
            }
        }
        Component::RangeField => {
            let mut component = RuntimeRangeField::new(range_value(fixture.state), 0.0, 1.0, 0.1)?
                .label("Opacity")
                .unit("×")
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid");
            set_full_width(&mut component.style);
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::SegmentedControl => {
            let segmented = create_segmented_fixture(&mut document, fixture)?;
            let target = segmented.control.stable_id();
            segmented_fixture = Some(segmented);
            target
        }
        Component::StatusBadge => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeStatusBadge::new(
                    status_badge_label(fixture.state),
                    status_tone(fixture.state),
                ),
            )?
            .stable_id(),
        Component::ValidationMessage => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeValidationMessage::new(
                    validation_message(fixture.state),
                    validation_intent(fixture.state),
                ),
            )?
            .stable_id(),
        Component::EmptyState => {
            let mut empty = RuntimeEmptyState::new(empty_title(fixture.state));
            if fixture.state != "title-only" {
                empty = empty
                    .icon(Icon::Folder)
                    .message(empty_message(fixture.state));
            }
            if fixture.state == "compact" {
                empty = empty.compact(true);
            }
            if fixture.state == "extreme-clip" {
                let mut style = NodeStyle::default();
                Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(100.0));
                empty = empty.style(style);
            }
            let empty = document
                .context_mut()
                .create_component(document_id, empty)?;
            if let Some(action) = feedback_action.as_ref() {
                document
                    .context_mut()
                    .set_empty_state_action(empty, Some(action.action.stable_id()))?;
            }
            empty.stable_id()
        }
        Component::LabeledValue => {
            let summary = document.context_mut().create_component(
                document_id,
                RuntimeLabeledValue::new("Revision", "42").emphasis(if fixture.state == "strong" {
                    ValueEmphasis::Strong
                } else {
                    ValueEmphasis::Normal
                }),
            )?;
            if let Some(action) = feedback_action.as_ref() {
                document
                    .context_mut()
                    .set_labeled_value_action(summary, Some(action.action.stable_id()))?;
            }
            summary.stable_id()
        }
        Component::Progress => {
            let value = if fixture.state == "empty" { 0.0 } else { 42.0 };
            let mut progress = RuntimeProgress::new(value, 100.0);
            if fixture.state == "labeled" {
                progress = progress.label("Copying");
            }
            set_full_width(&mut progress.style);
            document
                .context_mut()
                .create_component(document_id, progress)?
                .stable_id()
        }
        Component::Spinner => document
            .context_mut()
            .create_component(document_id, RuntimeSpinner::new("Loading").phase(0.25))?
            .stable_id(),
        Component::Tabs => create_tabs_fixture(&mut document, fixture)?.stable_id(),
        Component::Skeleton => document
            .context_mut()
            .create_component(document_id, RuntimeSkeleton::fill_width(16.0))?
            .stable_id(),
        Component::LevelMeter => {
            let mut meter = RuntimeLevelMeter::new(0.65).tone(StatusTone::Success);
            set_full_width(&mut meter.style);
            document
                .context_mut()
                .create_component(document_id, meter)?
                .stable_id()
        }
        Component::FormField => {
            let (field, control) = document.context_mut().build(document_id, |ui| {
                let control = ui.leaf(
                    RuntimeTextInput::new("name@studio.local").placeholder("name@studio.local"),
                );
                let field = ui.child("field", RuntimeFormField::new("Email").error("Required"));
                (field, control)
            })?;
            document
                .context_mut()
                .set_form_field_control(field, Some(control.stable_id()))?;
            field.stable_id()
        }
        Component::InteractiveCard => document.context_mut().build(document_id, |ui| {
            let label = ui.leaf(RuntimeText::new("Interactive surface"));
            let card = ui.child("card", RuntimeInteractiveCard::new().selected(true));
            ui.nest(card, |ui| ui.adopt(label));
            card.stable_id()
        })?,
        Component::Tooltip => {
            let component = RuntimeIconButton::new(Icon::Add, "Add source")
                .tooltip("Add source", tooltip_fixture_config(fixture.state));
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Dialog => {
            let (dialog, body, close) = document.context_mut().build(document_id, |ui| {
                let body = ui.leaf(RuntimeText::new("Camera A"));
                let close = ui.leaf(RuntimeIconButton::new(Icon::Close, "Close"));
                let dialog = ui.child(
                    "dialog",
                    RuntimeDialog::new("Rename scene")
                        .description("This updates the workspace label.")
                        .size(DialogSize::Default),
                );
                (dialog, body, close)
            })?;
            document.context_mut().set_modal_slots(
                dialog,
                ModalSlots {
                    body: Some(body.stable_id()),
                    close_action: Some(close.stable_id()),
                    ..ModalSlots::default()
                },
            )?;
            dialog.stable_id()
        }
        Component::ConfirmDialog => {
            let mut confirm = RuntimeConfirmDialog::new("Delete take", "This cannot be undone.");
            confirm.danger = fixture.state == "danger";
            confirm.busy = fixture.state == "busy";
            let (confirm, cancel, accept, close) =
                document.context_mut().build(document_id, |ui| {
                    let cancel = ui.leaf(RuntimeButton::new("取消"));
                    let accept = ui.leaf(
                        RuntimeButton::new(if fixture.state == "busy" {
                            "处理中"
                        } else {
                            "确认"
                        })
                        .kind(if fixture.state == "danger" {
                            nana_ui::ButtonKind::Danger
                        } else {
                            nana_ui::ButtonKind::Primary
                        })
                        .loading(fixture.state == "busy"),
                    );
                    let close = (fixture.state != "busy")
                        .then(|| ui.leaf(RuntimeIconButton::new(Icon::Close, "Close")));
                    let confirm = ui.child("confirm", confirm);
                    (confirm, cancel, accept, close)
                })?;
            document.context_mut().set_confirm_slots(
                confirm,
                ConfirmSlots {
                    body: None,
                    close_action: close.map(|close| close.stable_id()),
                    cancel: cancel.stable_id(),
                    secondary: None,
                    confirm: accept.stable_id(),
                },
            )?;
            confirm.stable_id()
        }
        Component::Drawer => {
            let (drawer, body, close) = document.context_mut().build(document_id, |ui| {
                let body = ui.leaf(RuntimeText::new("Properties"));
                let close = ui.leaf(RuntimeIconButton::new(Icon::Close, "Close"));
                let drawer = ui.child(
                    "drawer",
                    RuntimeDrawer::new("Inspector").side(if fixture.state == "left" {
                        DrawerSide::Left
                    } else {
                        DrawerSide::Right
                    }),
                );
                (drawer, body, close)
            })?;
            document.context_mut().set_modal_slots(
                drawer,
                ModalSlots {
                    body: Some(body.stable_id()),
                    close_action: Some(close.stable_id()),
                    ..ModalSlots::default()
                },
            )?;
            drawer.stable_id()
        }
        Component::Toast => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeToast::new(
                    if fixture.state == "dismissible" {
                        "Export complete"
                    } else {
                        "Listening"
                    },
                    if fixture.state == "dismissible" {
                        ToastTone::Success
                    } else {
                        ToastTone::Info
                    },
                )
                .description(if fixture.state == "dismissible" {
                    "Master sent to disk."
                } else {
                    "Program follow is armed."
                })
                .dismissible(fixture.state == "dismissible"),
            )?
            .stable_id(),
        Component::XYPad => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeXYPad::new(XYPadValue::new(0.35, 0.7)).invalid(fixture.state == "invalid"),
            )?
            .stable_id(),
        Component::QrCode => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeQrCode::encode("nana-ui://pair", 224.0).expect("fixture qr encodes"),
            )?
            .stable_id(),
        Component::Select => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeSelect::new(Some("code"))
                    .placeholder("Choose view")
                    .options([
                        RuntimeSelectOption::new("code", "Code"),
                        RuntimeSelectOption::new("split", "Split").disabled(true),
                        RuntimeSelectOption::new("preview", "Preview"),
                    ])
                    .invalid(fixture.state == "invalid")
                    .opened(fixture.state == "opened"),
            )?
            .stable_id(),
        Component::Popover => document.context_mut().build(document_id, |ui| {
            let body = ui.leaf(RuntimeText::new("Inspector content"));
            let popover = ui.child(
                "popover",
                RuntimePopover::new().trigger("Details").open(true),
            );
            ui.nest(popover, |ui| ui.adopt(body));
            popover.stable_id()
        })?,
        Component::ActionMenu => document.context_mut().build(document_id, |ui| {
            let rename = ui.leaf(RuntimeActionMenuItem::new("Rename"));
            let delete = ui.leaf(RuntimeActionMenuItem::new("Delete").danger(true));
            let menu = ui.child(
                "menu",
                RuntimeActionMenu::new().trigger("Actions").open(true),
            );
            ui.nest(menu, |ui| {
                ui.adopt(rename);
                ui.adopt(delete);
            });
            menu.stable_id()
        })?,
        Component::ActionMenuItem => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeActionMenuItem::new("Delete").hint("⌫").danger(true),
            )?
            .stable_id(),
        Component::AnchoredActionMenu => document.context_mut().build(document_id, |ui| {
            let rename = ui.leaf(RuntimeActionMenuItem::new("Rename"));
            let delete = ui.leaf(RuntimeActionMenuItem::new("Delete").danger(true));
            let menu = ui.child(
                "menu",
                RuntimeAnchoredActionMenu::new(24.0, 36.0)
                    .menu_size(200.0, 0.0)
                    .open(true),
            );
            ui.nest(menu, |ui| {
                ui.adopt(rename);
                ui.adopt(delete);
            });
            menu.stable_id()
        })?,
        Component::ContextMenu => document.context_mut().build(document_id, |ui| {
            let rename = ui.leaf(RuntimeActionMenuItem::new("Rename"));
            let delete = ui.leaf(RuntimeActionMenuItem::new("Delete").danger(true));
            let menu = ui.child(
                "menu",
                RuntimeContextMenu::new(24.0, 36.0)
                    .items([
                        RuntimeContextMenuItem::new("rename", "Rename"),
                        RuntimeContextMenuItem::new("delete", "Delete").danger(true),
                    ])
                    .open(true),
            );
            ui.nest(menu, |ui| {
                ui.adopt(rename);
                ui.adopt(delete);
            });
            menu.stable_id()
        })?,
        Component::SidebarFrame => mount_runtime_sidebar_frame(&mut document, fixture)?,
        Component::SidebarSection => mount_runtime_sidebar_section(
            &mut document,
            fixture.state != "collapsed",
            &["外观", "工作区"],
            true,
        )?,
        Component::SidebarFooter => document.context_mut().build(document_id, |ui| {
            let settings =
                ui.leaf(RuntimeSidebarFooterButton::new("设置", Icon::Settings).selected(true));
            let search = ui.leaf(RuntimeSidebarFooterButton::new("搜索", Icon::Search));
            let footer = ui.child("footer", RuntimeSidebarFooter::new());
            ui.nest(footer, |ui| {
                ui.adopt(settings);
                ui.adopt(search);
            });
            footer.stable_id()
        })?,
        Component::AppearanceSection => {
            let mut appearance = AppearanceSettings::default();
            if fixture.state != "solid" {
                let _ = appearance.set_window_material(WindowMaterialMode::Translucent);
            }
            let section = document.context_mut().create_component(
                document_id,
                RuntimeAppearanceSection::new(theme, appearance),
            )?;
            document
                .context_mut()
                .assemble_appearance_section(section)?;
            section.stable_id()
        }
        Component::AboutSection => {
            let section = document.context_mut().create_component(
                document_id,
                RuntimeAboutSection::new(
                    RuntimeAboutMetadata::new("NanaUI Gallery", "0.1.0")
                        .description("Injected product metadata for the about card."),
                ),
            )?;
            document.context_mut().assemble_about_section(section)?;
            section.stable_id()
        }
        Component::SettingsCollapsibleCard => {
            let card = document.context_mut().build(document_id, |ui| {
                let summary = ui.leaf(RuntimeText::new("高级选项"));
                let details = ui.leaf(RuntimeText::new("折叠后应隐藏这段说明。"));
                ui.child(
                    "card",
                    RuntimeSettingsCollapsibleCard::new(fixture.state != "collapsed")
                        .summary(summary.stable_id())
                        .details(details.stable_id()),
                )
            })?;
            document
                .context_mut()
                .assemble_settings_collapsible_card(card)?;
            card.stable_id()
        }
        Component::CommandPalette => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCommandPalette::new(
                    "命令面板",
                    [
                        CommandPaletteItem::new(ActionId::new("rename"), "重命名"),
                        CommandPaletteItem::new(ActionId::new("delete"), "删除"),
                    ],
                ),
            )?
            .stable_id(),
        Component::OverlayHost => document.context_mut().build(document_id, |ui| {
            let base = ui.leaf(RuntimeText::new("Base surface"));
            let host = ui.child("host", RuntimeOverlayHost::new());
            ui.nest(host, |ui| ui.adopt(base));
            host.stable_id()
        })?,
        Component::Dropdown => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeDropdown::single(Some("code"))
                    .placeholder("Choose view")
                    .options([
                        RuntimeDropdownOption::new("code", "Code"),
                        RuntimeDropdownOption::new("split", "Split").disabled(true),
                        RuntimeDropdownOption::new("preview", "Preview"),
                    ]),
            )?
            .stable_id(),
        Component::SearchDropdown => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeSearchDropdown::new(Some("code"))
                    .placeholder("Search views")
                    .options([
                        RuntimeSearchDropdownOption::new("code", "Code"),
                        RuntimeSearchDropdownOption::new("preview", "Preview"),
                    ]),
            )?
            .stable_id(),
        Component::TreeView => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTreeView::new([
                    TreeNode::branch(
                        std::sync::Arc::<str>::from("src"),
                        "src",
                        true,
                        [
                            TreeNode::leaf(std::sync::Arc::<str>::from("main"), "main.rs")
                                .selected(true),
                        ],
                    ),
                    TreeNode::leaf(std::sync::Arc::<str>::from("readme"), "README.md"),
                ]),
            )?
            .stable_id(),
        Component::SidebarRow => document.context_mut().build(document_id, |ui| {
            let leading = ui.leaf(nana_ui::runtime::SidebarRowIcon::new(Icon::Workspace));
            let row = ui.child(
                "row",
                RuntimeSidebarRow::new("工作区")
                    .state(nana_ui::runtime::SidebarRowState::Active)
                    .slots(nana_ui::runtime::ListItemSlots {
                        leading: Some(leading.stable_id()),
                        content: None,
                        trailing: None,
                    }),
            );
            ui.nest(row, |ui| ui.adopt(leading));
            row.stable_id()
        })?,
        Component::Settings => {
            let control = document
                .context_mut()
                .build_detached(document_id, |ui| ui.leaf(RuntimeText::new("暗色")))?;
            let row = document.context_mut().mount_settings_leaf_row(
                document_id,
                "主题",
                Some("选择应用配色，立即生效"),
                control.stable_id(),
            )?;
            document.context_mut().build(document_id, |ui| {
                let card = ui.child("card", RuntimeSettingsCard::new("外观"));
                ui.nest(card, |ui| ui.adopt(row));
                card.stable_id()
            })?
        }
        Component::SettingsSidebar => mount_runtime_settings_sidebar(&mut document)?,
        Component::SettingsPage => mount_runtime_settings_page(&mut document, theme, fixture)?,
        Component::Workspace => mount_runtime_workspace(&mut document)?,
        Component::Dock => mount_runtime_dock(&mut document)?,
        Component::DockPanel => document.context_mut().build(document_id, |ui| {
            let title = ui.leaf(RuntimeText::new("Inspector"));
            let hint = ui.leaf(RuntimeText::new("Selection").style({
                let mut style = NodeStyle {
                    foreground: Some(SemanticColorRole::Muted),
                    ..NodeStyle::default()
                };
                Arc::make_mut(&mut style.layout).font_size = Some(10.0);
                style
            }));
            let mut body_style = NodeStyle::default();
            {
                let layout = Arc::make_mut(&mut body_style.layout);
                layout.direction = Some(nana_ui_core::FlexDirection::Column);
                layout.gap = Some(LengthSpec::Px(4.0));
            }
            let body = ui.leaf(RuntimeList::new().style(body_style));
            ui.nest(body, |ui| {
                ui.adopt(title);
                ui.adopt(hint);
            });
            let panel = ui.child(
                "panel",
                RuntimeDockPanel::new()
                    .padding(10.0)
                    .content(body.stable_id()),
            );
            ui.nest(panel, |ui| ui.adopt(body));
            panel.stable_id()
        })?,
        Component::SplitPane => mount_runtime_split_pane(&mut document)?,
        Component::PaneChrome => mount_runtime_pane_chrome(&mut document)?,
        Component::PaneTree => mount_runtime_pane_tree(&mut document)?,
        Component::AppShell => mount_runtime_app_shell(&mut document)?,
        Component::DesktopShell => mount_runtime_desktop_shell(&mut document, theme)?,
        Component::AppTitleBar => document
            .context_mut()
            .create_component(document_id, RuntimeAppTitleBar::new("NanaUI"))?
            .stable_id(),
        Component::GpuTextureView => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeGpuTextureView::new(gpu::SNAPSHOT_GPU_SLOT),
            )?
            .stable_id(),
        Component::Thumbnail => {
            let thumb = match fixture.state {
                "ready" => RuntimeThumbnail::new(gpu::SNAPSHOT_GPU_SLOT),
                "wide" => RuntimeThumbnail::empty().aspect(16.0 / 9.0),
                _ => RuntimeThumbnail::empty(),
            };
            document
                .context_mut()
                .create_component(document_id, thumb)?
                .stable_id()
        }
        Component::GpuView => {
            let colors = theme.colors();
            document
                .context_mut()
                .create_component(
                    document_id,
                    RuntimeGpuView::new(1).palette(RuntimeGpuViewPalette {
                        background: [
                            colors.background.r,
                            colors.background.g,
                            colors.background.b,
                            colors.background.a,
                        ],
                        accent: [
                            colors.accent_strong.r,
                            colors.accent_strong.g,
                            colors.accent_strong.b,
                            colors.accent_strong.a,
                        ],
                    }),
                )?
                .stable_id()
        }
    };
    let mut hierarchy = MutationQueue::new();
    hierarchy.insert(root.stable_id(), target, None);
    document.context_mut().commit_mutations(hierarchy)?;

    let viewport = LayoutViewport::new(size.width as f32, size.height as f32);
    let mut shaper = NanaTextShaper::default();
    let first = document.flush(viewport, &mut shaper)?;
    let (action_applied, feedback_contract_ok, segmented_contract_ok) = if let Some(action) =
        feedback_action
    {
        let contract_ok = exercise_feedback_action_lifecycle(
            &mut document,
            viewport,
            &mut shaper,
            fixture,
            target,
            action,
        )?;
        (contract_ok, contract_ok, true)
    } else if let Some(segmented) = segmented_fixture.as_ref() {
        let contract_ok =
            exercise_segmented_contract(&mut document, viewport, &mut shaper, fixture, segmented)?;
        (contract_ok, true, contract_ok)
    } else {
        (
            apply_runtime_state(&mut document, fixture, target)?,
            true,
            true,
        )
    };
    let final_update = document.flush(viewport, &mut shaper)?;
    let idle = document.flush(viewport, &mut shaper)?.is_idle();
    let next_deadline = document.context().next_animation_deadline();
    let segmented_options = segmented_fixture
        .as_ref()
        .map(|segmented| {
            segmented
                .options
                .iter()
                .map(|option| option.stable_id())
                .collect()
        })
        .unwrap_or_default();
    let segmented_requests = segmented_fixture
        .as_ref()
        .map(|segmented| segmented.requests.lock().expect("segmented requests").len())
        .unwrap_or_default();
    Ok(RuntimeEvidence {
        document,
        target,
        first_passes: first.passes,
        first_accessibility_updates: first.accessibility.updated.len(),
        final_passes: final_update.passes,
        final_accessibility_updates: final_update.accessibility.updated.len(),
        idle,
        action_applied,
        feedback_contract_ok,
        segmented_contract_ok,
        segmented_options,
        segmented_requests,
        next_deadline,
    })
}

fn slot_label_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    let inset = LengthSpec::Px(SLOT_INSET);
    layout.padding_left = Some(inset);
    layout.padding_right = Some(inset);
    layout.padding_top = Some(inset);
    layout.padding_bottom = Some(inset);
    style
}

fn apply_runtime_state(
    document: &mut RuntimeDocument,
    fixture: Fixture,
    target: StableNodeId,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let context = document.context_mut();
    let mut adapter = RuntimeInputAdapter::default();
    let bounds = context.world().layout_box(target).expect("target layout");
    let center_x = bounds.x + bounds.width / 2.0;
    let center_y = bounds.y + bounds.height / 2.0;
    let (drag_x, drag_width, drag_y) = match context.world().component_geometry(target) {
        Some(nana_ui::runtime::ComponentGeometry::Range { track, .. }) => {
            (track.x, track.width, track.y + track.height / 2.0)
        }
        _ => (bounds.x, bounds.width, center_y),
    };
    match fixture.state {
        "hover" | "selected-hover" => Ok(adapter
            .dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
            )?
            .prevent_default),
        "open" if !matches!(fixture.component, Component::Tooltip) => Ok(true),
        "tooltip-delay" | "tooltip-edge" | "open" | "edge" => {
            adapter.dispatch_at(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
                Duration::ZERO,
            )?;
            let deadline = context.next_animation_deadline();
            if let Some(deadline) = deadline {
                context.advance_animations(deadline);
            }
            Ok(if matches!(fixture.state, "open" | "edge") {
                context
                    .world()
                    .overlay_host(target)
                    .is_some_and(|host| host.active.is_some())
            } else {
                deadline.is_some()
            })
        }
        "delay" if fixture.component == Component::Tooltip => {
            adapter.dispatch_at(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
                Duration::ZERO,
            )?;
            Ok(context.next_animation_deadline().is_some()
                && context
                    .world()
                    .overlay_host(target)
                    .is_some_and(|host| host.active.is_none()))
        }
        "pressed" | "selected-pressed" => Ok(adapter
            .dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, center_x, center_y),
            )?
            .prevent_default),
        "focused" => {
            let target = if fixture.component == Component::Tabs {
                context
                    .world()
                    .node(target)
                    .and_then(|node| node.children.first().copied())
                    .unwrap_or(target)
            } else {
                target
            };
            Ok(context.focus_node(document_id, target)?)
        }
        "invalid" if fixture.component == Component::TextInput => {
            Ok(context.focus_node(document_id, target)?)
        }
        "invalid-focused" if fixture.component == Component::Textarea => {
            Ok(context.focus_node(document_id, target)?)
        }
        "selection" => {
            context.focus_node(document_id, target)?;
            Ok(context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection {
                        anchor: 0,
                        focus: "release".len(),
                    }),
                },
            )?)
        }
        "multiline-selection" if fixture.component == Component::Textarea => {
            context.focus_node(document_id, target)?;
            Ok(context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection {
                        anchor: "First ".len(),
                        focus: "First line\nSecond line\nThird".len(),
                    }),
                },
            )?)
        }
        "scroll" if fixture.component == Component::Textarea => {
            let end = context
                .world()
                .text_input(target)
                .expect("textarea input state")
                .value
                .len();
            let focused = context.focus_node(document_id, target)?;
            let selected = context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection::caret(end)),
                },
            )?;
            Ok(focused || selected)
        }
        "keyboard-activation" | "space-toggle" => {
            context.focus_node(document_id, target)?;
            Ok(adapter
                .dispatch(context, document_id, &keyboard("Space"))?
                .prevent_default)
        }
        "pointer-activation" | "pointer-toggle" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, center_x, center_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Up, center_x, center_y),
                )?
                .prevent_default)
        }
        "accessibility-toggle" => Ok(context.apply_accessibility_action(
            document_id,
            AccessibilityActionRequest {
                target,
                action: AccessibilityAction::Click,
            },
        )?),
        "keyboard-edit" => {
            context.focus_node(document_id, target)?;
            Ok(adapter
                .dispatch(context, document_id, &keyboard_text("X"))?
                .prevent_default)
        }
        "ime-preedit" => {
            context.focus_node(document_id, target)?;
            Ok(context.set_ime_preedit(document_id, "你".into(), None)?)
        }
        "ime-commit" => {
            context.focus_node(document_id, target)?;
            context.set_ime_preedit(document_id, "你".into(), None)?;
            Ok(context.commit_ime(document_id, "你")?)
        }
        "drag" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, drag_x + drag_width * 0.25, drag_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Move, drag_x + drag_width * 0.8, drag_y),
                )?
                .prevent_default)
        }
        "drag-cancel" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, drag_x + drag_width * 0.25, drag_y),
            )?;
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Move, drag_x + drag_width * 0.8, drag_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Cancel, drag_x + drag_width * 0.8, drag_y),
                )?
                .prevent_default)
        }
        "arrow-decrement" => dispatch_range_key(context, document_id, target, adapter, "ArrowLeft"),
        "arrow-increment" => {
            dispatch_range_key(context, document_id, target, adapter, "ArrowRight")
        }
        "page-decrement" => dispatch_range_key(context, document_id, target, adapter, "PageDown"),
        "page-increment" => dispatch_range_key(context, document_id, target, adapter, "PageUp"),
        "home" => dispatch_range_key(context, document_id, target, adapter, "Home"),
        "end" => dispatch_range_key(context, document_id, target, adapter, "End"),
        "accessibility-set-value" => Ok(context.apply_accessibility_action(
            document_id,
            AccessibilityActionRequest {
                target,
                action: AccessibilityAction::SetValue(
                    if fixture.component == Component::TextInput {
                        "updated"
                    } else {
                        "0.73"
                    }
                    .into(),
                ),
            },
        )?),
        _ => Ok(false),
    }
}

fn dispatch_range_key(
    context: &mut nana_ui::runtime::AppContext,
    document: DocumentId,
    target: StableNodeId,
    mut adapter: RuntimeInputAdapter,
    key: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    context.focus_node(document, target)?;
    Ok(adapter
        .dispatch(context, document, &keyboard(key))?
        .prevent_default)
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase == PointerPhase::Down),
        pressure: 0.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        activation_click: false,
        modifiers: InputModifiers::default(),
    }
}

fn keyboard(key: &str) -> InputEvent {
    keyboard_with_repeat(key, false)
}

fn keyboard_with_repeat(key: &str, repeat: bool) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: key.into(),
        text: None,
        code: key.into(),
        repeat,
        modifiers: InputModifiers::default(),
    }
}

fn keyboard_text(text: &str) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: text.into(),
        text: Some(text.into()),
        code: "KeyX".into(),
        repeat: false,
        modifiers: InputModifiers::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_fixture_registry;

    #[test]
    fn fixture_registry_covers_compiled_runtime_qualified_components() {
        validate_fixture_registry().expect("snapshot fixture registry must match the catalog");
    }
}
