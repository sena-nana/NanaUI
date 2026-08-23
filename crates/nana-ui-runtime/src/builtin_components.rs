//! Built-in components install through the same [`UiExtension`] ABI as plugins.

use std::sync::Arc;

use nana_ui_core::{
    ButtonKind, CardKind, Icon, LengthSpec, SplitAxis, SplitPaneModel, StatusTone,
    SwitchControlPosition, ValidationIntent,
};

use crate::component_registry::{RegisterableComponent, SemanticSpec};
use crate::{
    ActionMenu, ActionMenuItem, AppShell, Button, CalendarHeatmap, Card, Checkbox, CommandPalette,
    ConfirmDialog, ContextMenu, Dialog, Dock, DockNode, Drawer, Dropdown, EmptyState,
    ExtensionRegistrar, FormField, FrameworkError, GraphCanvas, GraphModel, IconButton,
    ImageViewer, ImageViewerContent, InteractiveCard, LabeledValue, LevelMeter, ListItem,
    NativeMarkdown, Popover, Progress, QrCode, RangeField, SearchDropdown, SegmentedControl,
    Select, SidebarFrame, SidebarRow, SidebarRowState, Skeleton, Spinner, SplitPane, StatusBadge,
    Switch, Tabs, Text, TextArea, TextInput, TextInputState, Thumbnail, ThumbnailState, Toast,
    ToastTone, Tooltip, TreeView, UiExtension, ValidationMessage, Workspace, XYPad, XYPadValue,
};

pub struct NanaBuiltinComponents;

impl UiExtension for NanaBuiltinComponents {
    fn name(&self) -> &'static str {
        "nana.builtin"
    }

    fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
        registrar.register_tags(
            "nana.column",
            &[
                "column", "col", "vstack", "div", "section", "article", "main", "nav", "header",
                "footer", "ul", "ol",
            ],
        )?;
        registrar.register_tags("nana.row", &["row", "hstack"])?;
        registrar.register_tags("nana.box", &["box", "container", "layout"])?;
        registrar.register_component::<Text>()?;
        registrar.register_component::<Button>()?;
        registrar.register_component::<IconButton>()?;
        registrar.register_component::<Checkbox>()?;
        registrar.register_component::<Switch>()?;
        registrar.register_component::<Card>()?;
        registrar.register_component::<ListItem>()?;
        registrar.register_component::<Thumbnail>()?;
        registrar.register_component::<TextInput>()?;
        registrar.register_component::<TextArea>()?;
        registrar.register_component::<RangeField>()?;
        registrar.register_component::<Progress>()?;
        registrar.register_component::<Spinner>()?;
        registrar.register_component::<StatusBadge>()?;
        registrar.register_component::<ValidationMessage>()?;
        registrar.register_component::<EmptyState>()?;
        registrar.register_component::<LabeledValue>()?;
        registrar.register_component::<Dialog>()?;
        registrar.register_component::<ConfirmDialog>()?;
        registrar.register_component::<Select>()?;
        registrar.register_component::<Tabs>()?;
        registrar.register_component::<SegmentedControl>()?;
        registrar.register_component::<Dropdown>()?;
        registrar.register_component::<SearchDropdown>()?;
        registrar.register_component::<Drawer>()?;
        registrar.register_component::<Popover>()?;
        registrar.register_component::<ContextMenu>()?;
        registrar.register_component::<Toast>()?;
        registrar.register_component::<ActionMenu>()?;
        registrar.register_component::<ActionMenuItem>()?;
        registrar.register_component::<Tooltip>()?;
        registrar.register_component::<XYPad>()?;
        registrar.register_component::<QrCode>()?;
        registrar.register_component::<FormField>()?;
        registrar.register_component::<InteractiveCard>()?;
        registrar.register_component::<Skeleton>()?;
        registrar.register_component::<LevelMeter>()?;
        registrar.register_component::<CommandPalette>()?;
        registrar.register_component::<TreeView>()?;
        registrar.register_component::<CalendarHeatmap<()>>()?;
        registrar.register_component::<ImageViewer>()?;
        registrar.register_component::<NativeMarkdown>()?;
        registrar.register_component::<GraphCanvas>()?;
        registrar.register_component::<Workspace>()?;
        registrar.register_component::<Dock>()?;
        registrar.register_component::<SplitPane>()?;
        registrar.register_component::<AppShell>()?;
        registrar.register_component::<SidebarFrame>()?;
        registrar.register_component::<SidebarRow>()?;
        registrar.register_tags("nana.icon", &["icon", "i"])?;
        Ok(())
    }
}

impl RegisterableComponent for Text {
    const TYPE_ID: &'static str = "nana.text";
    const TAGS: &'static [&'static str] = &["text", "label", "p", "span", "h1", "h2", "h3"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Text::new(spec.display_label()).style(crate::NodeStyle {
            layout: Arc::clone(spec.layout),
            ..crate::NodeStyle::default()
        })
    }
}

impl RegisterableComponent for Button {
    const TYPE_ID: &'static str = "nana.button";
    const TAGS: &'static [&'static str] = &["button", "btn", "chip"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Button::new(spec.display_label())
            .layout(Arc::clone(spec.layout))
            .kind(spec.button_kind)
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for IconButton {
    const TYPE_ID: &'static str = "nana.icon-button";
    const TAGS: &'static [&'static str] = &["icon-button", "iconbutton"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let icon = spec.icon.unwrap_or(Icon::Add);
        let mut component = IconButton::new(icon, Arc::<str>::from(spec.display_label()))
            .kind(spec.button_kind)
            .size(spec.size)
            .selected(spec.active)
            .disabled(spec.disabled);
        if !spec.hint.is_empty() {
            component = component.tooltip(
                Arc::<str>::from(spec.hint),
                nana_ui_core::TooltipConfig::default(),
            );
        }
        component
    }
}

impl RegisterableComponent for Checkbox {
    const TYPE_ID: &'static str = "nana.checkbox";
    const TAGS: &'static [&'static str] = &["checkbox", "check"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Checkbox::new(spec.display_label(), spec.toggled)
            .disabled(spec.disabled)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for Switch {
    const TYPE_ID: &'static str = "nana.switch";
    const TAGS: &'static [&'static str] = &["switch", "toggle"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut component = Switch::new(spec.display_label(), spec.toggled)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
            .size(spec.size)
            .control_position(parse_control_position(spec.attr("control-position")));
        if !spec.hint.is_empty() {
            component = component.hint(Arc::<str>::from(spec.hint));
        }
        component
    }
}

impl RegisterableComponent for Card {
    const TYPE_ID: &'static str = "nana.card";
    const TAGS: &'static [&'static str] = &["card"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut card = Card::new()
            .kind(parse_card_kind(spec.attr("card-kind")))
            .loading(spec.loading);
        if !spec.label.is_empty() {
            card = card.title(Arc::<str>::from(spec.label));
        }
        if let Some(LengthSpec::Px(padding)) = spec.layout.padding_left {
            card = card.padding(padding);
        }
        if let Some(LengthSpec::Px(height)) = spec.layout.height {
            card = card.height(height);
        }
        card
    }
}

impl RegisterableComponent for ListItem {
    const TYPE_ID: &'static str = "nana.list-item";
    const TAGS: &'static [&'static str] = &["list-item", "listitem", "li"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        ListItem::new(spec.display_label())
            .selected(spec.active)
            .disabled(spec.disabled)
            .size(spec.size)
    }
}

impl RegisterableComponent for Thumbnail {
    const TYPE_ID: &'static str = "nana.thumbnail";
    const TAGS: &'static [&'static str] = &["thumbnail"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut thumbnail = Thumbnail::new(spec.value).size(spec.size);
        if !spec.display_label().is_empty() {
            thumbnail = thumbnail.label(Arc::<str>::from(spec.display_label()));
        }
        if let Some(aspect) = spec.attr("aspect").and_then(|value| value.parse().ok()) {
            thumbnail = thumbnail.aspect(aspect);
        }
        if spec.loading {
            thumbnail = thumbnail.state(ThumbnailState::Loading);
        } else if spec.invalid {
            thumbnail = thumbnail.state(ThumbnailState::Unavailable);
        }
        thumbnail
    }
}

impl RegisterableComponent for TextInput {
    const TYPE_ID: &'static str = "nana.text-input";
    const TAGS: &'static [&'static str] = &["input", "text-field", "textfield"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let placeholder = if spec.placeholder.is_empty() {
            spec.hint
        } else {
            spec.placeholder
        };
        let mut component = TextInput::new("")
            .placeholder(Arc::<str>::from(placeholder))
            .layout(Arc::clone(spec.layout))
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .read_only(spec.read_only)
            .secure(spec.secure)
            .invalid(spec.invalid);
        component.state = TextInputState::new(spec.value);
        if !spec.label.is_empty() {
            component = component.label(Arc::<str>::from(spec.label));
        }
        component
    }
}

impl RegisterableComponent for TextArea {
    const TYPE_ID: &'static str = "nana.textarea";
    const TAGS: &'static [&'static str] = &["textarea"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let placeholder = if spec.placeholder.is_empty() {
            spec.hint
        } else {
            spec.placeholder
        };
        let mut component = TextArea::new("")
            .placeholder(Arc::<str>::from(placeholder))
            .disabled(spec.disabled)
            .invalid(spec.invalid);
        if let Some(nana_ui_core::LengthSpec::Px(height)) = spec.layout.height {
            component = component.height(height);
        }
        if !spec.label.is_empty() {
            component = component.label(Arc::<str>::from(spec.label));
        }
        component.state = TextInputState::new(spec.value);
        component
    }
}

impl RegisterableComponent for RangeField {
    const TYPE_ID: &'static str = "nana.range-field";
    const TAGS: &'static [&'static str] = &["range", "range-field", "slider"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let min = spec.min as f64;
        let max = if spec.max > spec.min {
            spec.max as f64
        } else {
            min + 1.0
        };
        let step = if spec.step > 0.0 {
            spec.step as f64
        } else {
            0.1
        };
        let value = (spec.number as f64).clamp(min, max);
        RangeField::new(value, min, max, step)
            .unwrap_or_else(|_| RangeField::new(0.0, 0.0, 1.0, 0.1).expect("default range"))
            .disabled(spec.disabled)
            .invalid(spec.invalid)
            .size(spec.size)
    }
}

impl RegisterableComponent for Progress {
    const TYPE_ID: &'static str = "nana.progress";
    const TAGS: &'static [&'static str] = &["progress"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut component = Progress::new(spec.number as f64, spec.max.max(1.0) as f64);
        if !spec.display_label().is_empty() {
            component = component.label(Arc::<str>::from(spec.display_label()));
        }
        component.cancellable(spec.attr("cancellable").is_some_and(truthy_attr))
    }
}

impl RegisterableComponent for Spinner {
    const TYPE_ID: &'static str = "nana.spinner";
    const TAGS: &'static [&'static str] = &["spinner", "loading"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Spinner::new(spec.display_label())
    }
}

impl RegisterableComponent for StatusBadge {
    const TYPE_ID: &'static str = "nana.status-badge";
    const TAGS: &'static [&'static str] = &["status", "status-badge", "statusbadge"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        StatusBadge::new(spec.display_label(), parse_status_tone(spec.attr("tone")))
            .compact(spec.attr("compact").is_some_and(truthy_attr))
    }
}

impl RegisterableComponent for ValidationMessage {
    const TYPE_ID: &'static str = "nana.validation-message";
    const TAGS: &'static [&'static str] =
        &["validation", "validation-message", "validationmessage"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let message = if spec.hint.is_empty() {
            spec.display_label()
        } else {
            spec.hint
        };
        ValidationMessage::new(message, validation_intent_from_spec(spec))
    }
}

impl RegisterableComponent for EmptyState {
    const TYPE_ID: &'static str = "nana.empty-state";
    const TAGS: &'static [&'static str] = &["empty", "empty-state", "emptystate"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        EmptyState::new(spec.display_label())
    }
}

impl RegisterableComponent for LabeledValue {
    const TYPE_ID: &'static str = "nana.labeled-value";
    const TAGS: &'static [&'static str] = &["labeled-value", "labeledvalue"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        LabeledValue::new(spec.label, spec.value)
    }
}

impl RegisterableComponent for Dialog {
    const TYPE_ID: &'static str = "nana.dialog";
    const TAGS: &'static [&'static str] = &["dialog", "modal"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Dialog::new(spec.display_label())
    }
}

impl RegisterableComponent for ConfirmDialog {
    const TYPE_ID: &'static str = "nana.confirm-dialog";
    const TAGS: &'static [&'static str] = &["confirm-dialog", "confirm", "alertdialog"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let message = if spec.hint.is_empty() {
            spec.value
        } else {
            spec.hint
        };
        let mut component = ConfirmDialog::new(spec.display_label(), message);
        component.busy = spec.loading;
        component.danger = spec.invalid;
        component
    }
}

impl RegisterableComponent for Select {
    const TYPE_ID: &'static str = "nana.select";
    const TAGS: &'static [&'static str] = &["select"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Select::new((!spec.value.is_empty()).then_some(spec.value))
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for Tabs {
    const TYPE_ID: &'static str = "nana.tabs";
    const TAGS: &'static [&'static str] = &["tabs", "tablist"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Tabs::new(if spec.value.is_empty() {
            spec.display_label()
        } else {
            spec.value
        })
        .size(spec.size)
    }
}

impl RegisterableComponent for SegmentedControl {
    const TYPE_ID: &'static str = "nana.segmented";
    const TAGS: &'static [&'static str] = &["segmented", "segmented-control"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        SegmentedControl::new().size(spec.size)
    }
}

impl RegisterableComponent for Dropdown {
    const TYPE_ID: &'static str = "nana.dropdown";
    const TAGS: &'static [&'static str] = &["dropdown"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Dropdown::single((!spec.value.is_empty()).then_some(spec.value))
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for SearchDropdown {
    const TYPE_ID: &'static str = "nana.search";
    const TAGS: &'static [&'static str] = &["search-dropdown"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        SearchDropdown::new((!spec.value.is_empty()).then_some(spec.value))
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid)
    }
}

impl RegisterableComponent for Drawer {
    const TYPE_ID: &'static str = "nana.drawer";
    const TAGS: &'static [&'static str] = &["drawer", "sheet"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Drawer::new(spec.display_label())
    }
}

impl RegisterableComponent for Popover {
    const TYPE_ID: &'static str = "nana.popover";
    const TAGS: &'static [&'static str] = &["popover"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Popover::new()
            .trigger(spec.display_label())
            .open(spec.active || spec.toggled)
    }
}

impl RegisterableComponent for ContextMenu {
    const TYPE_ID: &'static str = "nana.context-menu";
    const TAGS: &'static [&'static str] = &["context-menu", "contextmenu"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        ContextMenu::new(0.0, 0.0).open(spec.active || spec.toggled)
    }
}

impl RegisterableComponent for Toast {
    const TYPE_ID: &'static str = "nana.toast";
    const TAGS: &'static [&'static str] = &["toast"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut component = Toast::new(spec.display_label(), parse_toast_tone(spec.attr("tone")));
        if !spec.hint.is_empty() {
            component = component.description(Arc::<str>::from(spec.hint));
        }
        component.dismissible(spec.attr("dismissible").is_some_and(truthy_attr))
    }
}

impl RegisterableComponent for ActionMenu {
    const TYPE_ID: &'static str = "nana.action-menu";
    const TAGS: &'static [&'static str] = &["action-menu"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        ActionMenu::new()
            .trigger(spec.display_label())
            .open(spec.active || spec.toggled)
    }
}

impl RegisterableComponent for ActionMenuItem {
    const TYPE_ID: &'static str = "nana.action-menu-item";
    const TAGS: &'static [&'static str] = &["action-menu-item"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut component = ActionMenuItem::new(spec.display_label())
            .active(spec.active)
            .danger(action_menu_item_danger(spec))
            .disabled(spec.disabled)
            .size(spec.size);
        if !spec.hint.is_empty() {
            component = component.hint(Arc::<str>::from(spec.hint));
        }
        component
    }
}

impl RegisterableComponent for Tooltip {
    const TYPE_ID: &'static str = "nana.tooltip";
    const TAGS: &'static [&'static str] = &["tooltip"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Tooltip::new(if spec.display_label().is_empty() {
            spec.hint
        } else {
            spec.display_label()
        })
    }
}

impl RegisterableComponent for XYPad {
    const TYPE_ID: &'static str = "nana.xy-pad";
    const TAGS: &'static [&'static str] = &["xy-pad", "xypad"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let x = attr_f32(spec, &["x", "data-x"]).unwrap_or(spec.number);
        let y = attr_f32(spec, &["y", "data-y"]).unwrap_or(0.0);
        let x_min = attr_f32(spec, &["x-min", "xmin", "data-x-min"]).unwrap_or(spec.min);
        let x_max = attr_f32(spec, &["x-max", "xmax", "data-x-max"]).unwrap_or(spec.max);
        let y_min = attr_f32(spec, &["y-min", "ymin", "data-y-min"]).unwrap_or(spec.min);
        let y_max = attr_f32(spec, &["y-max", "ymax", "data-y-max"]).unwrap_or(spec.max);
        let mut component = XYPad::new(XYPadValue::new(x, y))
            .x_range(x_min, x_max)
            .y_range(y_min, y_max)
            .size(spec.size)
            .disabled(spec.disabled)
            .loading(spec.loading)
            .invalid(spec.invalid);
        if spec.step.is_finite() && spec.step > 0.0 {
            component = component.step(spec.step);
        }
        if !spec.display_label().is_empty() {
            component = component.label(Arc::<str>::from(spec.display_label()));
        }
        component
    }
}

impl RegisterableComponent for QrCode {
    const TYPE_ID: &'static str = "nana.qr-code";
    const TAGS: &'static [&'static str] = &["qr-code", "qr"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let size = match spec.layout.width {
            Some(LengthSpec::Px(px)) if px.is_finite() && px > 0.0 => px,
            _ => QrCode::DEFAULT_SIZE,
        };
        let label = if spec.display_label().is_empty() {
            "QR code"
        } else {
            spec.display_label()
        };
        if let Some((modules, width)) = qr_modules_from_spec(spec)
            && let Ok(component) = QrCode::from_modules(modules, width, size)
        {
            return component.label(Arc::<str>::from(label));
        }
        let payload = spec
            .attr("payload")
            .or_else(|| spec.attr("data-payload"))
            .filter(|value| !value.is_empty())
            .unwrap_or(spec.value);
        if !payload.is_empty()
            && let Ok(component) = QrCode::encode(payload, size)
        {
            return component.label(Arc::<str>::from(label));
        }
        qr_placeholder().label(Arc::<str>::from(label))
    }
}

impl RegisterableComponent for FormField {
    const TYPE_ID: &'static str = "nana.form-field";
    const TAGS: &'static [&'static str] = &["form-field"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        FormField::new(spec.display_label()).size(spec.size)
    }
}

impl RegisterableComponent for InteractiveCard {
    const TYPE_ID: &'static str = "nana.interactive-card";
    const TAGS: &'static [&'static str] = &["interactive-card"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        InteractiveCard::new()
            .selected(spec.active)
            .disabled(spec.disabled)
    }
}

impl RegisterableComponent for Skeleton {
    const TYPE_ID: &'static str = "nana.skeleton";
    const TAGS: &'static [&'static str] = &["skeleton"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        Skeleton::new(
            spec.layout.width.unwrap_or(LengthSpec::Fill),
            match spec.layout.height {
                Some(LengthSpec::Px(h)) if h.is_finite() && h > 0.0 => h,
                _ => 16.0,
            },
        )
    }
}

impl RegisterableComponent for LevelMeter {
    const TYPE_ID: &'static str = "nana.level-meter";
    const TAGS: &'static [&'static str] = &["level-meter", "level"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let mut component = LevelMeter::new(spec.number).tone(parse_status_tone(spec.attr("tone")));
        if let Some(LengthSpec::Px(height)) = spec.layout.height
            && height.is_finite()
            && height > 0.0
        {
            component = component.height(height);
        }
        component
    }
}

impl RegisterableComponent for CommandPalette {
    const TYPE_ID: &'static str = "nana.command-palette";
    const TAGS: &'static [&'static str] = &["command-palette"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        CommandPalette::new(spec.display_label(), Vec::new())
    }
}

impl RegisterableComponent for TreeView {
    const TYPE_ID: &'static str = "nana.tree-view";
    const TAGS: &'static [&'static str] = &["tree-view"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        TreeView::new(Vec::new()).size(spec.size)
    }
}

impl RegisterableComponent for CalendarHeatmap<()> {
    const TYPE_ID: &'static str = "nana.calendar-heatmap";
    const TAGS: &'static [&'static str] = &["calendar", "calendar-heatmap"];
    fn from_semantic(_spec: &SemanticSpec<'_>) -> Self {
        CalendarHeatmap::new(Vec::new())
    }
}

impl RegisterableComponent for ImageViewer {
    const TYPE_ID: &'static str = "nana.image-viewer";
    const TAGS: &'static [&'static str] = &["image-viewer"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let slot = spec
            .attr("src")
            .or_else(|| spec.attr("data-src"))
            .filter(|value| !value.is_empty())
            .unwrap_or(spec.value);
        let content = if slot.is_empty() {
            ImageViewerContent::None
        } else {
            ImageViewerContent::host_texture(slot)
        };
        let mut component = ImageViewer::new(content);
        if !spec.display_label().is_empty() {
            component = component.name(Arc::<str>::from(spec.display_label()));
        }
        if !spec.hint.is_empty() {
            component = component.metadata(Arc::<str>::from(spec.hint));
        }
        component
    }
}

impl RegisterableComponent for NativeMarkdown {
    const TYPE_ID: &'static str = "nana.native-markdown";
    const TAGS: &'static [&'static str] = &["markdown", "native-markdown"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        NativeMarkdown::from_source(spec.value)
    }
}

impl RegisterableComponent for GraphCanvas {
    const TYPE_ID: &'static str = "nana.graph-canvas";
    const TAGS: &'static [&'static str] = &["graph-canvas", "graphcanvas"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        GraphCanvas::new("main", GraphModel::empty()).disabled(spec.disabled)
    }
}

impl RegisterableComponent for Workspace {
    const TYPE_ID: &'static str = "nana.workspace";
    const TAGS: &'static [&'static str] = &["workspace"];
    fn from_semantic(_spec: &SemanticSpec<'_>) -> Self {
        Workspace::new()
    }
}

impl RegisterableComponent for Dock {
    const TYPE_ID: &'static str = "nana.dock";
    const TAGS: &'static [&'static str] = &["dock"];
    fn from_semantic(_spec: &SemanticSpec<'_>) -> Self {
        Dock::new(DockNode::item("dock", None))
    }
}

impl RegisterableComponent for SplitPane {
    const TYPE_ID: &'static str = "nana.split-pane";
    const TAGS: &'static [&'static str] = &["split-pane"];
    fn from_semantic(_spec: &SemanticSpec<'_>) -> Self {
        SplitPane {
            first: None,
            second: None,
            handle: None,
            model: SplitPaneModel::new(SplitAxis::Horizontal, 240.0, 120.0, 800.0),
        }
    }
}

impl RegisterableComponent for AppShell {
    const TYPE_ID: &'static str = "nana.app-shell";
    const TAGS: &'static [&'static str] = &["app-shell"];
    fn from_semantic(_spec: &SemanticSpec<'_>) -> Self {
        AppShell::new()
    }
}

impl RegisterableComponent for SidebarFrame {
    const TYPE_ID: &'static str = "nana.sidebar-frame";
    const TAGS: &'static [&'static str] = &["sidebar-frame"];
    fn from_semantic(_spec: &SemanticSpec<'_>) -> Self {
        SidebarFrame::new()
    }
}

impl RegisterableComponent for SidebarRow {
    const TYPE_ID: &'static str = "nana.sidebar-row";
    const TAGS: &'static [&'static str] = &["sidebar-row"];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        let state = if spec.disabled {
            SidebarRowState::Disabled
        } else if spec.active {
            SidebarRowState::Active
        } else {
            SidebarRowState::Idle
        };
        SidebarRow::new(spec.display_label())
            .size(spec.size)
            .state(state)
    }
}

fn truthy_attr(value: &str) -> bool {
    let value = value.trim();
    !(value.eq_ignore_ascii_case("false") || value == "0")
}

fn parse_control_position(raw: Option<&str>) -> SwitchControlPosition {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "start" | "left" => SwitchControlPosition::Start,
        _ => SwitchControlPosition::End,
    }
}

fn parse_card_kind(raw: Option<&str>) -> CardKind {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "outlined" | "outline" => CardKind::Outlined,
        "raised" | "elevated" => CardKind::Raised,
        "flat" => CardKind::Flat,
        "selected" => CardKind::Selected,
        _ => CardKind::Surface,
    }
}

fn parse_status_tone(raw: Option<&str>) -> StatusTone {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "info" => StatusTone::Info,
        "success" => StatusTone::Success,
        "warning" | "warn" => StatusTone::Warning,
        "danger" | "error" => StatusTone::Danger,
        _ => StatusTone::Neutral,
    }
}

fn parse_toast_tone(raw: Option<&str>) -> ToastTone {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "success" => ToastTone::Success,
        "warning" | "warn" => ToastTone::Warning,
        "danger" | "error" => ToastTone::Danger,
        _ => ToastTone::Info,
    }
}

fn parse_validation_intent(raw: Option<&str>) -> ValidationIntent {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "danger" | "error" => ValidationIntent::Danger,
        _ => ValidationIntent::Warning,
    }
}

fn validation_intent_from_spec(spec: &SemanticSpec<'_>) -> ValidationIntent {
    if spec.invalid {
        ValidationIntent::Danger
    } else {
        parse_validation_intent(spec.attr("intent").or_else(|| spec.attr("data-intent")))
    }
}

fn action_menu_item_danger(spec: &SemanticSpec<'_>) -> bool {
    spec.invalid
        || spec.button_kind == ButtonKind::Danger
        || spec.attr("danger").is_some_and(truthy_attr)
        || spec
            .attr("intent")
            .or_else(|| spec.attr("data-intent"))
            .or_else(|| spec.attr("data-variant"))
            .is_some_and(|value| value.eq_ignore_ascii_case("danger"))
}

fn attr_f32(spec: &SemanticSpec<'_>, names: &[&str]) -> Option<f32> {
    names
        .iter()
        .find_map(|name| spec.attr(name))
        .and_then(|raw| raw.trim().parse().ok())
}

fn qr_modules_from_spec(spec: &SemanticSpec<'_>) -> Option<(Arc<[bool]>, usize)> {
    let raw = spec.attr("modules").or_else(|| spec.attr("data-modules"))?;
    let width_hint = spec
        .attr("module-width")
        .or_else(|| spec.attr("modules-width"))
        .or_else(|| spec.attr("data-module-width"))
        .and_then(|raw| raw.trim().parse().ok());
    parse_qr_module_matrix(raw, width_hint)
}

fn parse_qr_module_matrix(raw: &str, width_hint: Option<usize>) -> Option<(Arc<[bool]>, usize)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cells: Vec<bool> = if trimmed.chars().all(|c| c == '0' || c == '1') {
        trimmed.chars().map(|c| c == '1').collect()
    } else {
        trimmed
            .split(['[', ']', ',', ';', ' ', '\n', '\t', '\r'])
            .filter(|part| !part.is_empty())
            .map(|part| match part {
                "1" | "true" | "TRUE" => Some(true),
                "0" | "false" | "FALSE" => Some(false),
                _ => None,
            })
            .collect::<Option<_>>()?
    };
    if cells.is_empty() {
        return None;
    }
    let width = width_hint.or_else(|| {
        let root = (cells.len() as f64).sqrt() as usize;
        (root > 0 && root.saturating_mul(root) == cells.len()).then_some(root)
    })?;
    if width == 0 || width.checked_mul(width) != Some(cells.len()) {
        return None;
    }
    Some((Arc::<[bool]>::from(cells), width))
}

fn qr_placeholder() -> QrCode {
    QrCode::from_modules(vec![true], 1, QrCode::DEFAULT_SIZE).unwrap_or_else(|_| QrCode {
        modules: Arc::from([true].as_slice()),
        width: 1,
        size: QrCode::DEFAULT_SIZE,
        label: Arc::from("QR code"),
    })
}
