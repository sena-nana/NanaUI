//! Snapshot catalog; no product state is stored here.
use super::*;

// Snapshot expectations are validation metadata, independent of build availability.
pub(super) const FIXTURE_REGISTRY: &[Fixture] = &[
    f(
        Component::GraphMinimap,
        "normal",
        "graph nodes and viewport project into the minimap",
    ),
    f(
        Component::Text,
        "normal",
        "body text uses the shared 13px baseline",
    ),
    f(
        Component::Text,
        "wrap",
        "long text wraps inside its authored content box",
    ),
    f(
        Component::Text,
        "ellipsis",
        "single-line overflow clips with an ellipsis",
    ),
    f(
        Component::Text,
        "centered",
        "horizontal and vertical anchors use the content box",
    ),
    f(
        Component::Text,
        "muted",
        "muted text keeps readable semantic contrast",
    ),
    f(
        Component::Button,
        "ghost",
        "ghost kind is transparent until interaction",
    ),
    f(
        Component::Button,
        "subtle",
        "subtle kind has neutral surface and border",
    ),
    f(
        Component::Button,
        "selected",
        "selected kind keeps persistent selected semantics",
    ),
    f(
        Component::Button,
        "primary",
        "primary kind uses accent-soft semantics",
    ),
    f(
        Component::Button,
        "warning",
        "warning kind uses warning semantics",
    ),
    f(
        Component::Button,
        "danger",
        "danger kind uses danger semantics",
    ),
    f(
        Component::Button,
        "text-kind",
        "text kind uses accent text without a surface",
    ),
    f(
        Component::Button,
        "small",
        "small size follows compact control metrics",
    ),
    f(
        Component::Button,
        "medium",
        "medium size follows standard control metrics",
    ),
    f(
        Component::Button,
        "large",
        "large size follows large control metrics",
    ),
    f(
        Component::Button,
        "hover",
        "hover feedback covers the complete hit target",
    ),
    f(
        Component::Button,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::Button,
        "focused",
        "focused keeps idle chrome; hover and pressed use background",
    ),
    f(
        Component::Button,
        "disabled",
        "disabled button cannot activate or focus",
    ),
    f(
        Component::Button,
        "loading",
        "loading is visible and prevents duplicate activation",
    ),
    f(
        Component::Button,
        "pointer-activation",
        "pointer activation emits once",
    ),
    f(
        Component::Button,
        "keyboard-activation",
        "Space activation emits once",
    ),
    f(
        Component::TextInput,
        "value",
        "committed value uses field padding and baseline",
    ),
    f(
        Component::TextInput,
        "placeholder",
        "empty input paints faint placeholder text",
    ),
    f(
        Component::TextInput,
        "hover",
        "hover strengthens the neutral border",
    ),
    f(
        Component::TextInput,
        "focused",
        "focus paints border and caret",
    ),
    f(
        Component::TextInput,
        "selection",
        "selected text has shaped highlight geometry",
    ),
    f(
        Component::TextInput,
        "disabled",
        "disabled input is inert and visibly disabled",
    ),
    f(
        Component::TextInput,
        "invalid",
        "invalid input keeps a danger border while focused",
    ),
    f(
        Component::TextInput,
        "secure",
        "secure input masks committed text",
    ),
    f(
        Component::TextInput,
        "small",
        "small size follows compact field metrics",
    ),
    f(
        Component::TextInput,
        "large",
        "large size follows large field metrics",
    ),
    f(
        Component::TextInput,
        "read-only",
        "read-only input remains focusable but rejects edits",
    ),
    f(
        Component::TextInput,
        "loading",
        "loading input is busy and rejects input",
    ),
    f(
        Component::TextInput,
        "keyboard-edit",
        "keyboard input commits through typed state",
    ),
    f(
        Component::TextInput,
        "ime-preedit",
        "IME preedit is visibly distinct from committed text",
    ),
    f(
        Component::TextInput,
        "ime-commit",
        "IME commit updates value and clears preedit",
    ),
    f(
        Component::TextInput,
        "accessibility-set-value",
        "accessibility SetValue updates the field",
    ),
    f(
        Component::Textarea,
        "placeholder",
        "empty multiline input paints faint placeholder text",
    ),
    f(
        Component::Textarea,
        "multiline",
        "committed lines retain top alignment and line spacing",
    ),
    f(
        Component::Textarea,
        "focused",
        "focus paints the multiline border and caret",
    ),
    f(
        Component::Textarea,
        "selection",
        "single-line selection uses shaped highlight geometry",
    ),
    f(
        Component::Textarea,
        "multiline-selection",
        "selection spanning line breaks produces one highlight per line",
    ),
    f(
        Component::Textarea,
        "invalid-focused",
        "focused invalid textarea uses a 2px danger field border without a second ring",
    ),
    f(
        Component::Textarea,
        "disabled",
        "disabled textarea is visibly inert and cannot focus",
    ),
    f(
        Component::Textarea,
        "clipped",
        "overflowing lines are clipped to the authored content box",
    ),
    f(
        Component::Textarea,
        "scroll",
        "the scrolled caret and text remain inside the content box",
    ),
    f(
        Component::HostedTextarea,
        "rust",
        "committed rust text is colored by the Runtime highlight presenter",
    ),
    f(
        Component::HostedTextarea,
        "placeholder",
        "empty highlighted editor paints faint placeholder text",
    ),
    f(
        Component::HostedTextarea,
        "disabled",
        "disabled highlighted editor is visibly inert",
    ),
    f(
        Component::CalendarHeatmap,
        "weeks",
        "week columns and level fills use theme accent, not a second canvas",
    ),
    f(
        Component::TimeSeriesChart,
        "series",
        "grid, area and line stay inside the 148px chart box",
    ),
    f(
        Component::ReorderList,
        "rows",
        "selected row uses the selected surface; labels stay left aligned",
    ),
    f(
        Component::NativeMarkdown,
        "blocks",
        "heading and paragraph project as wrapped body text",
    ),
    f(
        Component::SelectableRichText,
        "plain",
        "concatenated spans paint as selectable body text",
    ),
    f(
        Component::ImageViewer,
        "open",
        "scrim, surface and close chrome are present without embedded pixels",
    ),
    f(
        Component::GraphCanvas,
        "nodes",
        "two connected nodes paint as Scene quads inside the canvas",
    ),
    f(
        Component::KeyCaptureLayer,
        "recording",
        "recording badge is visible while capture is armed",
    ),
    f(
        Component::KeymapLayer,
        "idle",
        "keymap chrome is a non-focusable 28px badge",
    ),
    f(
        Component::GpuTextureView,
        "slot",
        "host texture slot snapshot-gpu is sampled in the layout region",
    ),
    f(
        Component::GpuView,
        "inline",
        "gpu-view custom renderer paints the inline slot",
    ),
    f(
        Component::Thumbnail,
        "empty",
        "empty thumbnail keeps the 1:1 control box without a host-texture node",
    ),
    f(
        Component::Thumbnail,
        "ready",
        "ready thumbnail samples nana.host-texture with contain",
    ),
    f(
        Component::Thumbnail,
        "wide",
        "host-declared 16:9 aspect widens the shared box",
    ),
    // Platform IME events cannot be injected into the compatibility widget by
    // this headless harness. Preedit remains a real Hosted acceptance gate.
    f(
        Component::Checkbox,
        "off",
        "off state exposes false in paint and accessibility",
    ),
    f(
        Component::Checkbox,
        "on",
        "on state exposes true in paint and accessibility",
    ),
    f(
        Component::Checkbox,
        "hover",
        "hover feedback reaches the indicator",
    ),
    f(
        Component::Checkbox,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::Checkbox,
        "focused",
        "keyboard focus is visible around the indicator",
    ),
    f(
        Component::Checkbox,
        "disabled",
        "disabled checkbox cannot toggle or focus",
    ),
    f(
        Component::Checkbox,
        "invalid",
        "invalid state keeps semantic danger treatment",
    ),
    f(
        Component::Checkbox,
        "pointer-toggle",
        "pointer activation toggles once",
    ),
    f(
        Component::Checkbox,
        "space-toggle",
        "Space activation toggles once",
    ),
    f(
        Component::Checkbox,
        "accessibility-toggle",
        "accessibility click toggles once",
    ),
    f(
        Component::IconButton,
        "normal",
        "labelled icon action has a complete square hit target",
    ),
    f(
        Component::IconButton,
        "hover",
        "hover feedback preserves icon contrast",
    ),
    f(
        Component::IconButton,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::IconButton,
        "focused",
        "focused keeps idle chrome; hover and pressed use background",
    ),
    f(
        Component::IconButton,
        "selected",
        "selected state is persistent and distinguishable",
    ),
    f(
        Component::IconButton,
        "disabled",
        "disabled action cannot receive pointer or focus",
    ),
    f(
        Component::IconButton,
        "keyboard-activation",
        "Space or Enter invokes the typed default action once",
    ),
    f(
        Component::IconButton,
        "tooltip-delay",
        "hover delay opens a real labelled tooltip",
    ),
    f(
        Component::IconButton,
        "tooltip-edge",
        "tooltip remains inside the viewport near an edge",
    ),
    f(
        Component::Switch,
        "off",
        "off state exposes false through paint and accessibility",
    ),
    f(
        Component::Switch,
        "on",
        "on state exposes true through paint and accessibility",
    ),
    f(
        Component::Switch,
        "hover",
        "hover feedback covers the complete row",
    ),
    f(
        Component::Switch,
        "pressed",
        "pressed feedback is distinguishable",
    ),
    f(Component::Switch, "focused", "keyboard focus is visible"),
    f(
        Component::Switch,
        "disabled",
        "disabled switch cannot toggle",
    ),
    f(
        Component::Switch,
        "invalid",
        "invalid state uses semantic danger treatment",
    ),
    f(
        Component::Switch,
        "label-hint",
        "label and hint retain hierarchy without clipping",
    ),
    f(
        Component::Switch,
        "control-start",
        "control may be placed before label content",
    ),
    f(
        Component::Switch,
        "control-end",
        "control defaults after label content",
    ),
    f(
        Component::Switch,
        "pointer-toggle",
        "pointer activation toggles once",
    ),
    f(
        Component::Switch,
        "space-toggle",
        "Space activation toggles once",
    ),
    f(
        Component::Switch,
        "accessibility-toggle",
        "accessibility click toggles once",
    ),
    f(
        Component::Card,
        "surface",
        "surface card contains title and arbitrary body",
    ),
    f(
        Component::Card,
        "outlined",
        "outlined kind has a semantic border",
    ),
    f(
        Component::Card,
        "raised",
        "raised kind remains legible above its background",
    ),
    f(
        Component::Card,
        "flat",
        "flat kind removes unnecessary chrome",
    ),
    f(
        Component::Card,
        "selected",
        "selected kind is visibly selected",
    ),
    f(
        Component::Card,
        "padding",
        "custom padding changes the content box, not semantics",
    ),
    f(
        Component::Card,
        "fixed-height",
        "fixed height constrains the outer card",
    ),
    f(
        Component::Card,
        "loading",
        "loading is busy and schedules only active animation",
    ),
    f(
        Component::Card,
        "long-content",
        "long content is clipped by the card content box",
    ),
    f(
        Component::ListItem,
        "three-slots",
        "leading content and trailing slots retain order and gap",
    ),
    f(
        Component::ListItem,
        "normal",
        "unselected item has a complete row hit target",
    ),
    f(
        Component::ListItem,
        "hover",
        "hover feedback covers the complete row",
    ),
    f(
        Component::ListItem,
        "pressed",
        "pressed feedback is distinguishable",
    ),
    f(Component::ListItem, "focused", "keyboard focus is visible"),
    f(
        Component::ListItem,
        "selected",
        "selected state is persistent",
    ),
    f(
        Component::ListItem,
        "selected-hover",
        "selected hover remains selected while adding hover feedback",
    ),
    f(
        Component::ListItem,
        "selected-pressed",
        "selected pressed remains selected while adding pressed feedback",
    ),
    f(
        Component::ListItem,
        "disabled",
        "disabled item cannot activate",
    ),
    f(
        Component::ListItem,
        "small",
        "small density remains readable",
    ),
    f(
        Component::ListItem,
        "medium",
        "medium density follows control metrics",
    ),
    f(
        Component::ListItem,
        "large",
        "large density follows control metrics",
    ),
    f(
        Component::ListItem,
        "auto-height",
        "multi-line content determines height without clipping",
    ),
    f(
        Component::ListItem,
        "pointer-activation",
        "pointer activation emits once",
    ),
    f(
        Component::ListItem,
        "keyboard-activation",
        "keyboard activation emits once",
    ),
    f(
        Component::RangeField,
        "minimum",
        "minimum maps to the start of the complete track",
    ),
    f(
        Component::RangeField,
        "middle",
        "middle value maps proportionally",
    ),
    f(
        Component::RangeField,
        "maximum",
        "maximum maps to the end of the complete track",
    ),
    f(
        Component::RangeField,
        "decimal-step",
        "decimal values are quantized to step",
    ),
    f(
        Component::RangeField,
        "drag",
        "drag updates through pointer capture",
    ),
    f(
        Component::RangeField,
        "drag-cancel",
        "cancel restores the drag origin and releases capture",
    ),
    f(
        Component::RangeField,
        "disabled",
        "disabled range cannot change or focus",
    ),
    f(
        Component::RangeField,
        "invalid",
        "invalid state uses semantic danger treatment",
    ),
    f(
        Component::RangeField,
        "arrow-decrement",
        "Arrow decreases by one step",
    ),
    f(
        Component::RangeField,
        "arrow-increment",
        "Arrow increases by one step",
    ),
    f(
        Component::RangeField,
        "page-decrement",
        "PageDown decreases by page step",
    ),
    f(
        Component::RangeField,
        "page-increment",
        "PageUp increases by page step",
    ),
    f(Component::RangeField, "home", "Home moves to minimum"),
    f(Component::RangeField, "end", "End moves to maximum"),
    f(
        Component::RangeField,
        "accessibility-set-value",
        "SetValue quantizes and updates once",
    ),
    f(
        Component::SegmentedControl,
        "small",
        "small density preserves the concentric segmented inset",
    ),
    f(
        Component::SegmentedControl,
        "medium-icon",
        "medium options align an authored semantic icon with the label",
    ),
    f(
        Component::SegmentedControl,
        "large",
        "large density preserves control and option height contracts",
    ),
    f(
        Component::SegmentedControl,
        "hover",
        "hover is visible without changing controlled selection",
    ),
    f(
        Component::SegmentedControl,
        "pressed",
        "pressed feedback is distinct from neutral hover",
    ),
    f(
        Component::SegmentedControl,
        "selected-hover",
        "selected hover retains checked semantics with its own surface",
    ),
    f(
        Component::SegmentedControl,
        "selected-pressed",
        "selected press retains checked semantics with its own surface",
    ),
    f(
        Component::SegmentedControl,
        "focused",
        "focus uses a two pixel outline with a two pixel external offset",
    ),
    f(
        Component::SegmentedControl,
        "disabled-selected",
        "a disabled selected option remains checked but is not a tab stop",
    ),
    f(
        Component::SegmentedControl,
        "empty",
        "an empty group remains inert and exposes no synthetic option",
    ),
    f(
        Component::SegmentedControl,
        "all-disabled",
        "an all-disabled group has no focus target or interactive option",
    ),
    f(
        Component::SegmentedControl,
        "pointer-request",
        "pointer activation requests selection once without committing it",
    ),
    f(
        Component::SegmentedControl,
        "pointer-cancel",
        "pointer cancel consumes its lease without requesting selection",
    ),
    f(
        Component::SegmentedControl,
        "selected-repeat-request",
        "activating the selected option still emits one controlled request",
    ),
    f(
        Component::SegmentedControl,
        "arrow-skip-wrap",
        "horizontal arrows skip disabled options and wrap",
    ),
    f(
        Component::SegmentedControl,
        "home-end",
        "Home and End request the first and last enabled options",
    ),
    f(
        Component::SegmentedControl,
        "space-enter-repeat",
        "Space and Enter reject repeats while normal activation requests selection",
    ),
    f(
        Component::SegmentedControl,
        "no-selection",
        "without selection the first enabled option is the sole sequential tab stop",
    ),
    f(
        Component::SegmentedControl,
        "dynamic-disable",
        "disabling the focused selected option repairs focus but preserves checked state",
    ),
    f(
        Component::SegmentedControl,
        "controlled-commit",
        "a request leaves selection unchanged until the application setter commits once",
    ),
    f(
        Component::SegmentedControl,
        "a11y-radio",
        "SegmentedControl exposes RadioGroup and Radio roles for one controlled checked option",
    ),
    f(
        Component::SegmentedControl,
        "atomic-reconcile",
        "invalid reconciliation is atomic and removed options park without ghosts",
    ),
    f(
        Component::StatusBadge,
        "neutral",
        "neutral status remains descriptive and visually quiet",
    ),
    f(
        Component::StatusBadge,
        "info",
        "informational status uses the semantic accent tone",
    ),
    f(
        Component::StatusBadge,
        "success",
        "successful status uses the semantic success tone",
    ),
    f(
        Component::StatusBadge,
        "warning",
        "warning status uses the semantic warning tone",
    ),
    f(
        Component::StatusBadge,
        "danger",
        "danger status uses the semantic danger tone",
    ),
    f(
        Component::ValidationMessage,
        "warning",
        "warning validation retains its outlined marker and regular text weight",
    ),
    f(
        Component::ValidationMessage,
        "danger",
        "danger validation retains its outlined marker and regular text weight",
    ),
    f(
        Component::EmptyState,
        "complete-action",
        "normal empty state owns icon, title and message while a real child owns action",
    ),
    f(
        Component::EmptyState,
        "compact",
        "compact empty state is start aligned with tighter intrinsic spacing",
    ),
    f(
        Component::EmptyState,
        "title-only",
        "title-only empty state has no synthetic icon, message or action",
    ),
    f(
        Component::EmptyState,
        "narrow-cjk",
        "long CJK and emoji content wraps and remains ordered at narrow width",
    ),
    f(
        Component::EmptyState,
        "extreme-clip",
        "extreme width clips intrinsic content and descendants to the authored box",
    ),
    f(
        Component::LabeledValue,
        "normal",
        "normal value uses medium emphasis without making the parent interactive",
    ),
    f(
        Component::LabeledValue,
        "strong",
        "strong value uses semibold emphasis without changing its semantic value",
    ),
    f(
        Component::LabeledValue,
        "action",
        "a real application-owned child action remains separate from the inert summary",
    ),
    f(
        Component::Progress,
        "labeled",
        "determinate progress keeps a Subtle track, Accent fill and optional label",
    ),
    f(
        Component::Progress,
        "empty",
        "zero progress leaves the Accent fill collapsed on the track",
    ),
    f(
        Component::Spinner,
        "loading",
        "spinner reuses the host-sampled Scene primitive beside a muted label",
    ),
    f(
        Component::Tabs,
        "selected",
        "tabs share segmented selection without the bordered pill chrome",
    ),
    f(
        Component::Tabs,
        "focused",
        "tab focus uses the same 2px external ring as segmented options",
    ),
    f(
        Component::Skeleton,
        "block",
        "skeleton is a Subtle rounded placeholder with authored width and height",
    ),
    f(
        Component::LevelMeter,
        "success",
        "level meter fills a compact tone-colored track without a progress label",
    ),
    f(
        Component::FormField,
        "error",
        "form field shows the label and danger support while the control stays a child",
    ),
    f(
        Component::InteractiveCard,
        "selected",
        "interactive card uses selected surface, border and activation semantics",
    ),
    f(
        Component::Tooltip,
        "open",
        "zero-delay hover opens a compact label-only tooltip bound to the pointer",
    ),
    f(
        Component::Tooltip,
        "delay",
        "hover before the delay leaves the tooltip closed",
    ),
    f(
        Component::Tooltip,
        "edge",
        "tooltip stays inside the viewport near an edge",
    ),
    f(
        Component::Dialog,
        "titled",
        "dialog paints a scrim, titled surface and application-owned body",
    ),
    f(
        Component::ConfirmDialog,
        "danger",
        "confirm dialog keeps cancel and danger confirm actions",
    ),
    f(
        Component::ConfirmDialog,
        "busy",
        "busy confirm dialog disables dismiss and shows a loading confirm",
    ),
    f(
        Component::Drawer,
        "right",
        "right drawer docks to the viewport edge over a scrim",
    ),
    f(
        Component::Drawer,
        "left",
        "left drawer docks to the start edge over a scrim",
    ),
    f(
        Component::Toast,
        "info",
        "info toast is an outlined tone card with a title",
    ),
    f(
        Component::Toast,
        "dismissible",
        "dismissible toast keeps a real dismiss affordance",
    ),
    f(
        Component::XYPad,
        "rest",
        "xy pad shows the two-axis value inside a field border",
    ),
    f(
        Component::XYPad,
        "invalid",
        "invalid xy pad uses a danger field border",
    ),
    f(
        Component::QrCode,
        "encoded",
        "qr code paints a white quiet zone and black modules",
    ),
    f(
        Component::Select,
        "closed",
        "select shows the selected label inside a field with a handle",
    ),
    f(
        Component::Select,
        "opened",
        "opened select keeps the field and paints a surface menu of options",
    ),
    f(
        Component::Select,
        "invalid",
        "invalid select uses a danger field border",
    ),
    f(
        Component::Popover,
        "open",
        "popover paints an anchored surface without a scrim",
    ),
    f(
        Component::ActionMenu,
        "open",
        "action menu uses start alignment and compact padding",
    ),
    f(
        Component::ActionMenuItem,
        "danger",
        "danger action menu item uses danger text and an optional hint",
    ),
    f(
        Component::AnchoredActionMenu,
        "open",
        "anchored action menu pins a menu surface to a logical point",
    ),
    f(
        Component::ContextMenu,
        "open",
        "context menu opens at the pointer anchor",
    ),
    f(
        Component::SidebarFrame,
        "chrome",
        "fixed top and footer stay outside the independently scrolling body",
    ),
    f(
        Component::SidebarSection,
        "expanded",
        "section header uses uppercase faint title, count and an expanded body",
    ),
    f(
        Component::SidebarSection,
        "collapsed",
        "collapsed section clips the body while keeping the header",
    ),
    f(
        Component::SidebarFooter,
        "actions",
        "footer hugs small icon actions without growing",
    ),
    f(
        Component::AppearanceSection,
        "solid",
        "appearance rows keep host-owned theme, material and radius controls",
    ),
    f(
        Component::AboutSection,
        "metadata",
        "about section shows injected name, version and description",
    ),
    f(
        Component::SettingsCollapsibleCard,
        "expanded",
        "collapsible card shows summary, divider and details when expanded",
    ),
    f(
        Component::SettingsCollapsibleCard,
        "collapsed",
        "collapsed card keeps summary and hides details",
    ),
    f(
        Component::CommandPalette,
        "open",
        "command palette shows search field and windowed rows",
    ),
    f(
        Component::OverlayHost,
        "stacked",
        "overlay host keeps exclusive stacking order",
    ),
    f(
        Component::Dropdown,
        "closed",
        "dropdown field shows the selected value and keeps disabled options",
    ),
    f(
        Component::SearchDropdown,
        "closed",
        "search dropdown field shows the committed query surface",
    ),
    f(
        Component::TreeView,
        "expanded",
        "tree view flattens visible disclosure rows",
    ),
    f(
        Component::SidebarRow,
        "active",
        "sidebar row keeps a 14px leading icon without a selected plate",
    ),
    f(
        Component::Settings,
        "row-card",
        "settings card groups a labeled control row",
    ),
    f(
        Component::SettingsSidebar,
        "settings-sidebar",
        "settings sidebar keeps back and tab rows in the frame",
    ),
    f(
        Component::SettingsPage,
        "settings-page",
        "settings page shows the tab title above host content",
    ),
    f(
        Component::SettingsPage,
        "settings-page-full",
        "full-page settings tab fills with content only",
    ),
    f(
        Component::Workspace,
        "default-regions",
        "workspace lays out start, primary, end and bottom regions from the model",
    ),
    f(
        Component::Dock,
        "split-tabs",
        "dock paints a split with a tabbed leaf and an item leaf",
    ),
    f(
        Component::DockPanel,
        "bordered",
        "dock panel is a radius-0 surface with a soft border",
    ),
    f(
        Component::SplitPane,
        "horizontal",
        "split pane sizes the first child and keeps an 8px handle",
    ),
    f(
        Component::PaneChrome,
        "active",
        "pane chrome keeps a 34px header over the body",
    ),
    f(
        Component::PaneTree,
        "nested",
        "pane tree preserves leaf order across a nested split",
    ),
    f(
        Component::AppShell,
        "stacked",
        "app shell stacks a 36px title bar over a fill body",
    ),
    f(
        Component::DesktopShell,
        "desktop-settings",
        "desktop shell stacks title, resources sidebar and primary settings page",
    ),
    f(
        Component::AppTitleBar,
        "titled",
        "title bar is 36px with a centered title",
    ),
];

const fn f(component: Component, state: &'static str, expected: &'static str) -> Fixture {
    Fixture {
        id: component.id(),
        component,
        state,
        expected,
        reference_contract: "reference rendered; interaction semantics may be incomplete",
        runtime_contract: "canonical frame must settle with layout, hit-test, accessibility and scene",
        divergence: "none unless recorded by the observed verdict",
    }
}
