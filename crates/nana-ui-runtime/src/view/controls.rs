//! Element functions of the built-in controls, and their typed bindable
//! fields, `model` and event methods expanded from the control table in
//! `nana-ui-view-schema`.

use std::any::Any;
use std::sync::Arc;

use super::node::IntoView;
use super::node::{El, node_ref, widget};
use super::prop::{FieldWrite, IntoProp, PropSource, Writable};
pub use super::prop::{InteractiveIdle, NonEmpty, NonEmptyString, WhileIdle};
use super::reactive::{Signal, on_mount, signal};
use super::style::InlineOverride;
use crate::{
    ActionMenu, ActionMenuItem, ColorChanged, ColorField, ColorSubmitted, HoverCard,
    IconButtonTooltip, InlineContent, ListItemRole, MenuItemMark, SegmentedControl, SizeField,
    SizeFieldChanged, SizeFieldSubmitted, SizePreset, TabsEvent, TextSelection, ThumbnailState,
    TreeNode, TreeSelection, TreeSelectionRequested, TreeView,
};
use crate::{
    Activate, AppContext, Avatar, Button, Card, Checkbox, Chip, CurveChanged, CurveField,
    CurveInput, Divider, EmptyState, Entity, GpuTextureView, IconButton, ListItem, NodeStyle,
    NumberChanged, NumberInput, NumberSubmitted, Progress, RangeChanged, RangeField, RangeInput,
    RangeSpanChanged, RangeSpanEntered, RangeSpanField, RangeSpanInput, RangeSpanOrientation,
    RangeSpanThumb, ScrollView, Select, SelectChanged, SelectOption, SelectToggled, Spinner,
    StableNodeId, Stack, StatusBadge, Switch, Text, TextArea, TextChanged, TextInput,
    TextSubmitted, TextureSlotName, Thumbnail, ThumbnailPick, ToggleChanged,
};
use crate::{
    AppTitleBar, Breadcrumb, Dialog, Drawer, FormField, GpuView, IconGlyph, InteractiveCard,
    LabeledValue, LevelMeter, List, MediaTransportBar, OverlayHost, Panel, PathText, SettingsCard,
    SettingsGroup, SidebarRow, Skeleton, StatusBar, Tabs, ToolGroup, Toolbar, Tooltip,
    ValidationMessage, Video,
};
use nana_ui_core::{
    ButtonKind, ElevationRole, FlexDirection, Icon, LengthSpec, PaintTransform, RadiusTier,
    SemanticColorRole, StatusTone,
};

/// Components whose [`NodeStyle`] the view layer may write (visibility).
pub trait StyledComponent {
    #[doc(hidden)]
    fn node_style(&self) -> &NodeStyle;
    #[doc(hidden)]
    fn node_style_mut(&mut self) -> &mut NodeStyle;
}

impl StyledComponent for Stack {
    fn node_style(&self) -> &NodeStyle {
        self.style_ref()
    }

    fn node_style_mut(&mut self) -> &mut NodeStyle {
        self.style_mut()
    }
}

/// Whether the node takes part in layout, paint and hit testing: what
/// [`El::visible`] binds.
pub struct Visible;

impl<C: StyledComponent> FieldWrite<C, bool> for Visible {
    const FIELD: &'static str = "style.layout.hidden";

    fn write(target: &mut C, visible: bool) {
        let style = target.node_style_mut();
        if style.layout.hidden == visible {
            Arc::make_mut(&mut style.layout).hidden = !visible;
        }
    }

    fn differs(target: &C, visible: &bool) -> bool {
        target.node_style().layout.hidden == *visible
    }
}

/// Semantic roles of [`NodeStyle`], resolved against the installed theme
/// when painted, so they follow a theme switch; a stylesheet's colours are
/// fixed when it is compiled.
macro_rules! style_roles {
    ($($writer:ident: $field:ident: $ty:ty, $doc:literal;)*) => {$(
        #[doc(hidden)]
        pub struct $writer;

        impl<C: StyledComponent> FieldWrite<C, $ty> for $writer {
            const FIELD: &'static str = concat!("style.", stringify!($field));

            fn write(target: &mut C, value: $ty) {
                target.node_style_mut().$field = value;
            }

            fn differs(target: &C, value: &$ty) -> bool {
                target.node_style().$field != *value
            }
        }

        impl<C: StyledComponent + crate::ComponentView, K> El<C, K> {
            #[doc = $doc]
            #[track_caller]
            pub fn $field(self, value: impl IntoProp<$ty>) -> Self {
                self.prop::<$ty, $writer>(value)
            }
        }
    )*};
}

style_roles! {
    Foreground: foreground: Option<SemanticColorRole>, "Text and glyph colour by theme role.";
    Background: background: Option<SemanticColorRole>, "Fill by theme role.";
    Border: border: Option<SemanticColorRole>, "Border colour by theme role; the width is the layout's.";
    Radius: radius: Option<RadiusTier>, "Corner radius by theme step.";
}

/// Theme elevation shadow role on [`nana_ui_core::PaintStyle::elevation`].
pub struct Elevation;

impl<C: StyledComponent> FieldWrite<C, Option<ElevationRole>> for Elevation {
    const FIELD: &'static str = "style.layout.paint.elevation";

    fn write(target: &mut C, value: Option<ElevationRole>) {
        target.node_style_mut().set_elevation(value);
    }

    fn differs(target: &C, value: &Option<ElevationRole>) -> bool {
        target.node_style().elevation_role() != *value
    }
}

/// Node opacity (`style.layout.opacity`).
pub struct Opacity;

impl<C: StyledComponent> FieldWrite<C, f32> for Opacity {
    const FIELD: &'static str = "style.layout.opacity";

    fn write(target: &mut C, value: f32) {
        let style = target.node_style_mut();
        if style.layout.opacity != Some(value) {
            Arc::make_mut(&mut style.layout).opacity = Some(value);
        }
    }

    fn differs(target: &C, value: &f32) -> bool {
        target.node_style().layout.opacity != Some(*value)
    }
}

/// Node paint transform (`style.layout.transforms.transform`).
pub struct Transform;

impl<C: StyledComponent> FieldWrite<C, Option<PaintTransform>> for Transform {
    const FIELD: &'static str = "style.layout.transforms.transform";

    fn write(target: &mut C, value: Option<PaintTransform>) {
        let style = target.node_style_mut();
        if style.layout.transforms.transform != value {
            Arc::make_mut(&mut style.layout).transforms.transform = value;
        }
    }

    fn differs(target: &C, value: &Option<PaintTransform>) -> bool {
        target.node_style().layout.transforms.transform != *value
    }
}

/// Direct paint fill color (`style.layout.paint_colors.background`).
pub struct Fill;

impl<C: StyledComponent> FieldWrite<C, Option<nana_ui_core::style_model::PaintColor>> for Fill {
    const FIELD: &'static str = "style.layout.paint_colors.background";

    fn write(target: &mut C, value: Option<nana_ui_core::style_model::PaintColor>) {
        let style = target.node_style_mut();
        if style.layout.paint_colors.background != value {
            Arc::make_mut(&mut style.layout).paint_colors.background = value;
        }
    }

    fn differs(target: &C, value: &Option<nana_ui_core::style_model::PaintColor>) -> bool {
        target.node_style().layout.paint_colors.background != *value
    }
}

/// Direct paint text/glyph ink color (`style.layout.paint_colors.color`).
pub struct Ink;

impl<C: StyledComponent> FieldWrite<C, Option<nana_ui_core::style_model::PaintColor>> for Ink {
    const FIELD: &'static str = "style.layout.paint_colors.color";

    fn write(target: &mut C, value: Option<nana_ui_core::style_model::PaintColor>) {
        let style = target.node_style_mut();
        if style.layout.paint_colors.color != value {
            Arc::make_mut(&mut style.layout).paint_colors.color = value;
        }
    }

    fn differs(target: &C, value: &Option<nana_ui_core::style_model::PaintColor>) -> bool {
        target.node_style().layout.paint_colors.color != *value
    }
}

/// Direct paint border color (`style.layout.paint_colors.border`).
pub struct Edge;

impl<C: StyledComponent> FieldWrite<C, Option<nana_ui_core::style_model::PaintColor>> for Edge {
    const FIELD: &'static str = "style.layout.paint_colors.border";

    fn write(target: &mut C, value: Option<nana_ui_core::style_model::PaintColor>) {
        let style = target.node_style_mut();
        if style.layout.paint_colors.border != value {
            Arc::make_mut(&mut style.layout).paint_colors.border = value;
        }
    }

    fn differs(target: &C, value: &Option<nana_ui_core::style_model::PaintColor>) -> bool {
        target.node_style().layout.paint_colors.border != *value
    }
}

macro_rules! layout_field_writers {
    ($($writer:ident: $field:ident: $ty:ty;)*) => {$(
        pub struct $writer;

        impl<C: StyledComponent> FieldWrite<C, $ty> for $writer {
            const FIELD: &'static str = concat!("style.layout.", stringify!($field));

            fn write(target: &mut C, value: $ty) {
                let style = target.node_style_mut();
                if style.layout.$field != value {
                    Arc::make_mut(&mut style.layout).$field = value;
                }
            }

            fn differs(target: &C, value: &$ty) -> bool {
                target.node_style().layout.$field != *value
            }
        }
    )*};
}

layout_field_writers! {
    LayoutDirection: direction: Option<FlexDirection>;
    LayoutWidth: width: Option<LengthSpec>;
    LayoutHeight: height: Option<LengthSpec>;
    LayoutMinWidth: min_width: Option<LengthSpec>;
    LayoutMaxWidth: max_width: Option<LengthSpec>;
    LayoutMinHeight: min_height: Option<LengthSpec>;
    LayoutMaxHeight: max_height: Option<LengthSpec>;
    LayoutFlexGrow: flex_grow: Option<f32>;
    LayoutFlexShrink: flex_shrink: Option<f32>;
}

/// Uniform box padding (`style.layout.padding`), clearing per-side overrides
/// when set so Layer 3 reactive padding wins over Layer 1 per-side rules.
pub struct LayoutPadding;

impl<C: StyledComponent> FieldWrite<C, Option<LengthSpec>> for LayoutPadding {
    const FIELD: &'static str = "style.layout.padding";

    fn write(target: &mut C, value: Option<LengthSpec>) {
        let style = target.node_style_mut();
        let needs_clear = value.is_some()
            && (style.layout.padding_top.is_some()
                || style.layout.padding_right.is_some()
                || style.layout.padding_bottom.is_some()
                || style.layout.padding_left.is_some()
                || !style.layout.logical_edges.is_shared_default());
        if style.layout.padding != value || needs_clear {
            let layout = Arc::make_mut(&mut style.layout);
            layout.padding = value;
            if value.is_some() {
                layout.padding_top = None;
                layout.padding_right = None;
                layout.padding_bottom = None;
                layout.padding_left = None;
                if !layout.logical_edges.is_shared_default() {
                    layout.logical_edges.padding = Default::default();
                }
            }
        }
    }

    fn differs(target: &C, value: &Option<LengthSpec>) -> bool {
        let layout = &target.node_style().layout;
        layout.padding != *value
            || (value.is_some()
                && (layout.padding_top.is_some()
                    || layout.padding_right.is_some()
                    || layout.padding_bottom.is_some()
                    || layout.padding_left.is_some()))
    }
}

/// Uniform box margin (`style.layout.margin`), clearing per-side overrides
/// when set so Layer 3 reactive margin wins over Layer 1 per-side rules.
pub struct LayoutMargin;

impl<C: StyledComponent> FieldWrite<C, Option<LengthSpec>> for LayoutMargin {
    const FIELD: &'static str = "style.layout.margin";

    fn write(target: &mut C, value: Option<LengthSpec>) {
        let style = target.node_style_mut();
        let needs_clear = value.is_some()
            && (style.layout.margin_top.is_some()
                || style.layout.margin_right.is_some()
                || style.layout.margin_bottom.is_some()
                || style.layout.margin_left.is_some()
                || !style.layout.logical_edges.is_shared_default());
        if style.layout.margin != value || needs_clear {
            let layout = Arc::make_mut(&mut style.layout);
            layout.margin = value;
            if value.is_some() {
                layout.margin_top = None;
                layout.margin_right = None;
                layout.margin_bottom = None;
                layout.margin_left = None;
                if !layout.logical_edges.is_shared_default() {
                    layout.logical_edges.margin = Default::default();
                }
            }
        }
    }

    fn differs(target: &C, value: &Option<LengthSpec>) -> bool {
        let layout = &target.node_style().layout;
        layout.margin != *value
            || (value.is_some()
                && (layout.margin_top.is_some()
                    || layout.margin_right.is_some()
                    || layout.margin_bottom.is_some()
                    || layout.margin_left.is_some()))
    }
}

impl<C: StyledComponent + crate::ComponentView, K> El<C, K> {
    #[track_caller]
    fn layout_prop<T: Clone + Send + 'static, W: FieldWrite<C, T> + 'static>(
        mut self,
        value: impl IntoProp<T>,
        inline_override: impl FnOnce(T) -> InlineOverride,
    ) -> Self {
        let site = std::panic::Location::caller();
        match value.into_source() {
            PropSource::Const(v) => {
                self.push_inline_override(inline_override(v));
            }
            source => {
                let (component, bindings) = self.parts_mut();
                source.bind_field::<C, W>(component, bindings, site);
            }
        }
        self
    }

    /// Theme elevation shadow role (`ElevationRole::Surface` / `Overlay`).
    #[track_caller]
    pub fn elevation(self, value: impl IntoProp<Option<ElevationRole>>) -> Self {
        self.layout_prop::<Option<ElevationRole>, Elevation>(value, InlineOverride::Elevation)
    }

    /// Static or reactive node opacity (`style.layout.opacity`).
    #[track_caller]
    pub fn paint_opacity(self, value: impl IntoProp<f32>) -> Self {
        self.layout_prop::<f32, Opacity>(value, InlineOverride::Opacity)
    }

    /// Static or reactive paint transform (`style.layout.transforms.transform`).
    #[track_caller]
    pub fn transform(self, value: impl IntoProp<Option<PaintTransform>>) -> Self {
        self.layout_prop::<Option<PaintTransform>, Transform>(value, InlineOverride::Transform)
    }

    /// Static or reactive background paint color (`style.layout.paint_colors.background`).
    #[track_caller]
    pub fn paint_fill(
        self,
        value: impl IntoProp<Option<nana_ui_core::style_model::PaintColor>>,
    ) -> Self {
        self.layout_prop::<Option<nana_ui_core::style_model::PaintColor>, Fill>(
            value,
            InlineOverride::Fill,
        )
    }

    /// Static or reactive foreground/ink paint color (`style.layout.paint_colors.color`).
    #[track_caller]
    pub fn paint_ink(
        self,
        value: impl IntoProp<Option<nana_ui_core::style_model::PaintColor>>,
    ) -> Self {
        self.layout_prop::<Option<nana_ui_core::style_model::PaintColor>, Ink>(
            value,
            InlineOverride::Ink,
        )
    }

    /// Static or reactive border paint color (`style.layout.paint_colors.border`).
    #[track_caller]
    pub fn paint_edge(
        self,
        value: impl IntoProp<Option<nana_ui_core::style_model::PaintColor>>,
    ) -> Self {
        self.layout_prop::<Option<nana_ui_core::style_model::PaintColor>, Edge>(
            value,
            InlineOverride::Edge,
        )
    }

    /// Static or reactive uniform padding (`style.layout.padding`).
    #[track_caller]
    pub fn padding(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutPadding>(value, InlineOverride::Padding)
    }

    /// Static or reactive uniform margin (`style.layout.margin`).
    #[track_caller]
    pub fn margin(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutMargin>(value, InlineOverride::Margin)
    }

    /// Static or reactive width (`style.layout.width`).
    #[track_caller]
    pub fn width(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutWidth>(value, InlineOverride::Width)
    }

    /// Static or reactive height (`style.layout.height`).
    #[track_caller]
    pub fn height(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutHeight>(value, InlineOverride::Height)
    }

    /// Static or reactive minimum width (`style.layout.min_width`).
    #[track_caller]
    pub fn min_width(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutMinWidth>(value, InlineOverride::MinWidth)
    }

    /// Static or reactive maximum width (`style.layout.max_width`).
    #[track_caller]
    pub fn max_width(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutMaxWidth>(value, InlineOverride::MaxWidth)
    }

    /// Static or reactive minimum height (`style.layout.min_height`).
    #[track_caller]
    pub fn min_height(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutMinHeight>(value, InlineOverride::MinHeight)
    }

    /// Static or reactive maximum height (`style.layout.max_height`).
    #[track_caller]
    pub fn max_height(self, value: impl IntoProp<Option<LengthSpec>>) -> Self {
        self.layout_prop::<Option<LengthSpec>, LayoutMaxHeight>(value, InlineOverride::MaxHeight)
    }

    /// Static or reactive `flex-grow` (`style.layout.flex_grow`).
    #[track_caller]
    pub fn flex_grow(self, value: impl IntoProp<Option<f32>>) -> Self {
        self.layout_prop::<Option<f32>, LayoutFlexGrow>(value, InlineOverride::FlexGrow)
    }

    /// Static or reactive `flex-shrink` (`style.layout.flex_shrink`).
    #[track_caller]
    pub fn flex_shrink(self, value: impl IntoProp<Option<f32>>) -> Self {
        self.layout_prop::<Option<f32>, LayoutFlexShrink>(value, InlineOverride::FlexShrink)
    }

    /// Static or reactive flex direction (`style.layout.direction`).
    #[track_caller]
    pub fn direction(self, value: impl IntoProp<Option<FlexDirection>>) -> Self {
        self.layout_prop::<Option<FlexDirection>, LayoutDirection>(value, InlineOverride::Direction)
    }
}

/// Reactive snapshot of the world's active localization catalog, formatter,
/// fallback chain, and default locale. Updated automatically whenever
/// [`AppContext::set_message_catalog`], [`AppContext::set_default_locale`],
/// [`AppContext::set_document_locale`], or related i18n setters run.
pub fn locale_resolver() -> Signal<Arc<crate::i18n::LocalizedResolver>> {
    let value = signal(Arc::new(crate::i18n::LocalizedResolver::default()));
    on_mount(move |cx| {
        value.set_if_changed(cx.world().localized_resolver());
        cx.i18n_signals.push(value);
    });
    value
}

/// Turn a reactive [`crate::LocalizedText`] producer into a `String` prop
/// closure that re-evaluates whenever either its arguments or the world's
/// locale/catalog change.
pub fn localized_string(
    f: impl Fn() -> crate::LocalizedText + Send + 'static,
) -> impl Fn() -> String + Send + 'static {
    let resolver = locale_resolver();
    move || resolver.get().resolve(&f())
}

/// [`localized_string`] producing `Arc<str>`.
pub fn localized_arc(
    f: impl Fn() -> crate::LocalizedText + Send + 'static,
) -> impl Fn() -> Arc<str> + Send + 'static {
    let resolver = locale_resolver();
    move || Arc::<str>::from(resolver.get().resolve(&f()))
}

/// [`localized_string`] producing `Option<Arc<str>>` (`None` when empty).
pub fn localized_opt(
    f: impl Fn() -> crate::LocalizedText + Send + 'static,
) -> impl Fn() -> Option<Arc<str>> + Send + 'static {
    let resolver = locale_resolver();
    move || {
        let text = resolver.get().resolve(&f());
        (!text.is_empty()).then(|| Arc::from(text))
    }
}

/// [`NodeStyle::color_overrides`]: what [`El::theme_colors`] binds. An empty
/// set clears it.
#[doc(hidden)]
pub struct ThemeColors;

impl<C: StyledComponent> FieldWrite<C, nana_ui_core::ColorOverrides> for ThemeColors {
    const FIELD: &'static str = "style.color_overrides";

    fn write(target: &mut C, overrides: nana_ui_core::ColorOverrides) {
        target.node_style_mut().color_overrides =
            (!overrides.is_empty()).then(|| Arc::new(overrides));
    }

    fn differs(target: &C, overrides: &nana_ui_core::ColorOverrides) -> bool {
        match target.node_style().color_overrides.as_deref() {
            Some(current) => current != overrides,
            None => !overrides.is_empty(),
        }
    }
}

impl<C: StyledComponent + crate::ComponentView, K> El<C, K> {
    /// Give this node and its whole subtree its own values for some theme
    /// colour roles ([`nana_ui_core::ColorOverrides`]): a switch, badge or
    /// button inside paints its accent, text or fills from these. Bind a
    /// closure to follow an appearance; an empty set gives the theme back.
    #[track_caller]
    pub fn theme_colors(self, overrides: impl IntoProp<nana_ui_core::ColorOverrides>) -> Self {
        self.prop::<nana_ui_core::ColorOverrides, ThemeColors>(overrides)
    }
}

/// [`NodeStyle::theme_patch`]: what [`El::theme_scope`] binds. An empty patch
/// clears the node's own scope.
#[doc(hidden)]
pub struct ThemeScopeProp;

impl<C: StyledComponent> FieldWrite<C, nana_ui_core::ThemePatch> for ThemeScopeProp {
    const FIELD: &'static str = "style.theme_patch";

    fn write(target: &mut C, patch: nana_ui_core::ThemePatch) {
        target.node_style_mut().theme_patch = (!patch.is_empty()).then(|| Arc::new(patch));
    }

    fn differs(target: &C, patch: &nana_ui_core::ThemePatch) -> bool {
        match target.node_style().theme_patch.as_deref() {
            Some(current) => current != patch,
            None => !patch.is_empty(),
        }
    }
}

impl<C: StyledComponent + crate::ComponentView, K> El<C, K> {
    /// Scope this node and its subtree to a sparse [`nana_ui_core::ThemePatch`]
    /// overlay (Issue #104).
    ///
    /// Every token category, metric, recipe, chart palette/typography, and
    /// descendant `ThemeScope` under this node composes on top of its resolved
    /// scope theme. Bind a signal/closure to update reactively; an empty patch
    /// restores the parent scope.
    #[track_caller]
    pub fn theme_scope(self, patch: impl IntoProp<nana_ui_core::ThemePatch>) -> Self {
        self.prop::<nana_ui_core::ThemePatch, ThemeScopeProp>(patch)
    }
}

/// Convenience container wrapping a [`ThemeScope`](El::theme_scope) over a
/// vertical stack.
#[track_caller]
pub fn theme_scope(patch: impl IntoProp<nana_ui_core::ThemePatch>) -> El<Stack> {
    column().theme_scope(patch)
}

impl<C: StyledComponent + crate::ComponentView, K> El<C, K> {
    /// Keep the node but take it out of layout, paint and hit testing
    /// (`v-show`). Use [`super::when`] to drop the subtree instead (`v-if`).
    #[track_caller]
    pub fn visible(self, visible: impl IntoProp<bool>) -> Self {
        self.layout_prop::<bool, Visible>(visible, |v| InlineOverride::Hidden(!v))
    }

    /// [`Self::visible`] declared at `at`: a structural view's container.
    pub(crate) fn visible_at(
        mut self,
        visible: super::prop::PropSource<bool>,
        at: &'static std::panic::Location<'static>,
    ) -> Self {
        match visible {
            PropSource::Const(v) => {
                self.push_inline_override(InlineOverride::Hidden(!v));
            }
            source => {
                let (component, bindings) = self.parts_mut();
                source.bind_field::<C, Visible>(component, bindings, at);
            }
        }
        self
    }
}

/// Components outside the control table whose style views may still write.
macro_rules! styled {
    ($($component:ident),*) => {$(
        impl StyledComponent for $component {
            fn node_style(&self) -> &NodeStyle {
                &self.style
            }

            fn node_style_mut(&mut self) -> &mut NodeStyle {
                &mut self.style
            }
        }
    )*};
}

styled!(
    Card,
    ScrollView,
    ValidationMessage,
    Skeleton,
    Tooltip,
    Panel,
    List,
    Toolbar,
    ToolGroup,
    InteractiveCard,
    SettingsCard,
    SettingsGroup,
    FormField,
    Breadcrumb,
    StatusBar,
    GpuView,
    Video,
    SidebarRow,
    MediaTransportBar,
    Dialog,
    Drawer,
    LevelMeter,
    AppTitleBar,
    OverlayHost
);

/// An inline icon glyph (`<Icon icon={...}/>`).
pub fn icon(icon: Icon) -> El<IconGlyph> {
    widget(IconGlyph::new(icon))
}

/// Expands the control table of `nana-ui-view-schema`: per control, one
/// [`FieldWrite`] and `El` setter per field, `model` and event methods, and
/// [`StyledComponent`] for `.visible`.
macro_rules! controls {
    ($(
        $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
            $($field:ident: $ty:ty = $write:ident),* $(,)?
        }
        $(on { $($on:ident: $on_event:ident),* $(,)? })?
        $(with { $($with:ident: $with_event:ident),* $(,)? })?
        $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
        ;
    )*) => {
        /// One [`FieldWrite`] per bindable field of each control in the
        /// table, by element function and field: `generated::text::value`.
        /// Applications name them through [`fields`].
        pub mod generated {
            $(
                #[allow(non_camel_case_types)]
                pub mod $function {
                    $(
                        #[doc = concat!("`", stringify!($component), "` field `", stringify!($field), "`.")]
                        pub struct $field;
                    )*
                }
            )*
        }
        $(
        $(
            impl FieldWrite<$component, $ty> for generated::$function::$field {
                const FIELD: &'static str = concat!(stringify!($component), ".", stringify!($field));

                fn write(target: &mut $component, value: $ty) {
                    controls!(@write $write target, value, $field)
                }

                fn differs(target: &$component, value: &$ty) -> bool {
                    controls!(@differs $write target, value, $field)
                }
            }
        )*

        impl<K> El<$component, K> {
            $(
                #[track_caller]
                pub fn $field(self, value: impl IntoProp<$ty>) -> Self {
                    self.prop::<$ty, generated::$function::$field>(value)
                }
            )*

            $(
                /// `v-model`: shows the signal or store path and writes each
                /// change back.
                #[track_caller]
                pub fn model(self, value: impl Writable<$model_ty>) -> Self {
                    self.$model(value)
                        .on(move |$event: &$model_event| value.write_back($from_event))
                }
            )?

            $($(
                /// `@` event of the template.
                pub fn $on(self, mut handler: impl FnMut() + Send + 'static) -> Self {
                    self.on(move |_: &$on_event| handler())
                }
            )*)?

            $($(
                /// `@` event of the template, with its value.
                pub fn $with(self, handler: impl FnMut(&$with_event) + Send + 'static) -> Self {
                    self.on(handler)
                }
            )*)?
        }

        impl StyledComponent for $component {
            fn node_style(&self) -> &NodeStyle {
                &self.style
            }

            fn node_style_mut(&mut self) -> &mut NodeStyle {
                &mut self.style
            }
        }
    )*};
    (@write inline $target:ident, $value:ident, $field:ident) => {
        *$target=$target.clone().content($value)
    };
    (@differs inline $target:ident, $value:ident, $field:ident) => {
        *$target != $target.clone().content($value.clone())
    };
    (@write set $target:ident, $value:ident, $field:ident) => {
        $target.$field = $value
    };
    (@differs set $target:ident, $value:ident, $field:ident) => {
        $target.$field != *$value
    };
    // Only a different value replaces the text, so echoing an edit back
    // through its signal leaves the caret and selection alone.
    (@write text_state $target:ident, $value:ident, $field:ident) => {
        if $target.state.value != $value {
            $target.state.replace_value($value);
        }
    };
    (@differs text_state $target:ident, $value:ident, $field:ident) => {
        $target.state.value != *$value
    };
    (@write assign $target:ident, $value:ident, $field:ident) => {{
        if $target.state.value == $target.formatted_value() && $target.assign($value) {
            $target.state.selection = TextSelection::caret($target.state.value.len());
        }
    }};
    (@differs assign $target:ident, $value:ident, $field:ident) => {
        $target.state.value == $target.formatted_value() && $target.$field() != *$value
    };
    (@write clamp $target:ident, $value:ident, $field:ident) => {{
        if $target.dragging.is_none() && !$target.editing {
            $target.$field = $target.quantize(clamp_to_range($target.minimum, $target.maximum, $value));
        }
    }};
    (@differs clamp $target:ident, $value:ident, $field:ident) => {
        $target.dragging.is_none()
            && !$target.editing
            && $target.$field != $target.quantize(clamp_to_range($target.minimum, $target.maximum, *$value))
    };
    (@write span_low $target:ident, $value:ident, $field:ident) => {
        $target.assign(RangeSpanThumb::Low, $value)
    };
    (@differs span_low $target:ident, $value:ident, $field:ident) => {
        $target.low != $target.assigned(RangeSpanThumb::Low, *$value)
    };
    (@write span_high $target:ident, $value:ident, $field:ident) => {
        $target.assign(RangeSpanThumb::High, $value)
    };
    (@differs span_high $target:ident, $value:ident, $field:ident) => {
        $target.high != $target.assigned(RangeSpanThumb::High, *$value)
    };
    (@write span $target:ident, $value:ident, $field:ident) => {
        $target.set_span($value.0, $value.1)
    };
    (@differs span $target:ident, $value:ident, $field:ident) => {{
        let mut next = $target.clone();
        next.set_span($value.0, $value.1);
        ($target.low, $target.high) != (next.low, next.high)
    }};
    (@write curve $target:ident, $value:ident, $field:ident) => {{
        if $target.dragging.is_none() {
            $target.set_values($value);
        }
    }};
    (@differs curve $target:ident, $value:ident, $field:ident) => {
        $target.dragging.is_none() && $target.values != $target.quantized($value)
    };
    (@write color_value $target:ident, $value:ident, $field:ident) => {{
        if !$target.opened {
            *$target = $target.clone().$field($value);
        }
    }};
    (@differs color_value $target:ident, $value:ident, $field:ident) => {
        !$target.opened && $target.$field != *$value
    };
    (@write tooltip_text $target:ident, $value:ident, $field:ident) => {{
        let next = $value.filter(|text| !text.is_empty());
        match next {
            Some(label) => {
                if let Some(existing) = &mut $target.$field {
                    existing.label = label;
                } else {
                    $target.$field = Some(IconButtonTooltip {
                        label,
                        config: Default::default(),
                    });
                }
            }
            None => {
                $target.$field = None;
            }
        }
    }};
    (@differs tooltip_text $target:ident, $value:ident, $field:ident) => {{
        let next = $value.as_deref().filter(|text| !text.is_empty());
        $target.$field.as_ref().map(|t| t.label.as_ref()) != next
    }};
    (@write builder $target:ident, $value:ident, $field:ident) => {
        *$target = $target.clone().$field($value)
    };
    (@differs builder $target:ident, $value:ident, $field:ident) => {
        $target.$field != *$value
    };
    (@write tab $target:ident, $value:ident, $field:ident) => {{
        $target.selected = Some(Arc::clone(&$value));
        $target.focus = Some($value);
    }};
    (@differs tab $target:ident, $value:ident, $field:ident) => {
        $target.selected.as_ref() != Some($value) || $target.focus.as_ref() != Some($value)
    };
    (@write resource $target:ident, $value:ident, $field:ident) => {{
        $target.state = if $value.is_empty() {
            ThumbnailState::Empty
        } else {
            ThumbnailState::Ready
        };
        $target.resource = $value;
    }};
    (@differs resource $target:ident, $value:ident, $field:ident) => {
        $target.resource != *$value
    };
}

/// `value` inside `minimum..=maximum`; a non-finite value is the minimum.
fn clamp_to_range(minimum: f64, maximum: f64, value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(minimum, maximum.max(minimum))
    } else {
        minimum
    }
}

nana_ui_view_schema::for_each_control!(controls);

/// Vertical stack: `column().gap(8).with(|c| { c.add(…); })`, or
/// `.children((a, b))` for a fixed few.
#[track_caller]
pub fn column() -> El<Stack> {
    widget(Stack::column(0.0))
}

/// Horizontal stack; see [`column`].
#[track_caller]
pub fn row() -> El<Stack> {
    widget(Stack::row(0.0))
}

/// A length in logical pixels, whole or not: `gap(8)`, `gap(7.5)`.
pub trait Px {
    fn px(self) -> f32;
}

macro_rules! px {
    ($($ty:ty),*) => {$(
        impl Px for $ty {
            fn px(self) -> f32 {
                self as f32
            }
        }
    )*};
}

px!(f32, f64, i32, u32, i64, u64, usize, u16, u8);

impl<K> El<Stack, K> {
    /// Space between the children, in logical pixels (Layer 2 inline override).
    pub fn gap(mut self, gap: impl Px) -> Self {
        let gap = crate::LengthSpec::Px(gap.px().max(0.0));
        self.push_inline_override(InlineOverride::Gap(gap));
        self
    }

    /// Make this container participate in hit testing (`Stack::hittable`).
    pub fn hittable(self) -> Self {
        self.map_component(Stack::hittable)
    }

    /// Make this container focusable (`Stack::focusable`).
    pub fn focusable(self) -> Self {
        self.map_component(Stack::focusable)
    }
}

/// Text. For interpolation see [`crate::text!`].
#[track_caller]
pub fn text(value: impl IntoProp<String>) -> El<Text> {
    widget(Text::new("")).value(value)
}

/// Localized text: `text` resolved in the locale of the scope it is in.
#[track_caller]
pub fn localized(text: crate::LocalizedText) -> El<Text> {
    widget(Text::localized(text))
}

/// Localized text that follows its message (`<T id="files" :count="n" />`
/// in a template): a [`LocalizedText`](crate::LocalizedText), a signal or
/// computed of one, or a closure building one,
/// `t(move || LocalizedText::new("files").arg("count", n.get()))`. The text
/// resolves in the locale of the scope it is in. A message equal to the one
/// shown, the same key with the same arguments, changes nothing and is not
/// sent again; a locale switch resolves the text in the world and runs no
/// binding.
#[track_caller]
pub fn t(text: impl IntoProp<crate::LocalizedText>) -> El<Text> {
    widget(Text::new("")).prop::<crate::LocalizedText, Localized>(text)
}

/// A `<T>` argument written as a path or a field, cloned so the closure
/// building the message can run again. A function rather than a written
/// `Clone::clone`, which reads as a needless clone of a `Copy` argument at the
/// template's own span.
#[doc(hidden)]
pub fn __arg<T: Clone>(value: &T) -> T {
    value.clone()
}

/// The message a [`Text`] says, resolved in its scope's locale: what [`t`]
/// binds. Writing one makes the text localized.
pub struct Localized;

impl FieldWrite<Text, crate::LocalizedText> for Localized {
    const FIELD: &'static str = "Text.localized";

    fn write(target: &mut Text, text: crate::LocalizedText) {
        target.localized = Some(text);
    }

    fn differs(target: &Text, text: &crate::LocalizedText) -> bool {
        target.localized.as_ref() != Some(text)
    }
}

#[track_caller]
pub fn button(label: impl IntoProp<String>) -> El<Button> {
    widget(Button::new("")).label(label)
}

/// A range slider between `minimum` and `maximum`.
#[track_caller]
pub fn slider(minimum: f64, maximum: f64, step: f64) -> El<RangeField> {
    widget(RangeField::new(minimum, minimum, maximum, step))
}

/// A two-thumb range between `minimum` and `maximum`, starting at the whole
/// range; bind `.low(…)` / `.high(…)` or `.model(…)` with `(low, high)`.
#[track_caller]
pub fn range_span(minimum: f64, maximum: f64, step: f64) -> El<RangeSpanField> {
    widget(RangeSpanField::new(
        minimum, maximum, minimum, maximum, step,
    ))
}

/// A curve over the unit square whose points move up and down, on a grid
/// of `step`; bind `.values(…)` or `.model(…)`, one height per point.
#[track_caller]
pub fn curve(step: f64) -> El<CurveField> {
    widget(CurveField::new(Vec::new(), step))
}

#[track_caller]
pub fn text_input() -> El<TextInput> {
    widget(TextInput::new(""))
}

#[track_caller]
pub fn checkbox(label: impl IntoProp<String>) -> El<Checkbox> {
    widget(Checkbox::new("", false)).label(label)
}

#[track_caller]
pub fn switch(label: impl IntoProp<String>) -> El<Switch> {
    widget(Switch::new("", false)).label(label)
}

/// A multi-line text field.
#[track_caller]
pub fn text_area() -> El<TextArea> {
    widget(TextArea::new(""))
}

#[track_caller]
pub fn number_input() -> El<NumberInput> {
    widget(NumberInput::new(0.0))
}

/// A single-value field; give it `.options(…)`.
#[track_caller]
pub fn select() -> El<Select> {
    widget(Select::new(None::<Arc<str>>))
}

#[track_caller]
pub fn list_item(label: impl IntoProp<String>) -> El<ListItem> {
    widget(ListItem::new("")).label(label)
}

/// Progress towards `max`; bind `.value(…)`.
#[track_caller]
pub fn progress(max: f64) -> El<Progress> {
    widget(Progress::new(0.0, max))
}

#[track_caller]
pub fn spinner() -> El<Spinner> {
    widget(Spinner::new(""))
}

/// A horizontal rule.
#[track_caller]
pub fn divider() -> El<Divider> {
    widget(Divider::horizontal())
}

/// A host-texture image at its aspect ratio; bind `.resource(..)` and
/// re-register the slot when the host has new pixels. `.pickable(true)` with
/// `.on_pick(..)` reports where on the picture the pointer is.
#[track_caller]
pub fn thumbnail() -> El<Thumbnail> {
    widget(Thumbnail::new(""))
}

/// A round host-texture image `size` pixels across.
#[track_caller]
pub fn avatar(size: f32) -> El<Avatar> {
    widget(Avatar::new("")).size(size)
}

/// A host texture filling its box: video frames, previews.
#[track_caller]
pub fn texture() -> El<GpuTextureView> {
    widget(GpuTextureView::new(""))
}

/// A glyph button; `label` is its accessible name and tooltip.
#[track_caller]
pub fn icon_button(icon: Icon, label: impl IntoProp<Arc<str>>) -> El<IconButton> {
    widget(IconButton::new(icon, "")).label(label)
}

#[track_caller]
pub fn chip(label: impl IntoProp<Arc<str>>) -> El<Chip> {
    widget(Chip::new("")).label(label)
}

/// Short status text in its tone; bind `.tone(..)`.
#[track_caller]
pub fn status_badge(label: impl IntoProp<Arc<str>>) -> El<StatusBadge> {
    widget(StatusBadge::new("", StatusTone::Neutral)).label(label)
}

/// A title with an optional message, icon and [`El::action`].
#[track_caller]
pub fn empty_state(title: impl IntoProp<Arc<str>>) -> El<EmptyState> {
    widget(EmptyState::new("")).title(title)
}

/// A label and its value on one line.
#[track_caller]
pub fn labeled_value(label: impl IntoProp<Arc<str>>) -> El<LabeledValue> {
    widget(LabeledValue::new("", "")).label(label)
}

/// A tab strip; give it its options and bind `.selected(..)` by key.
#[track_caller]
pub fn tabs() -> El<Tabs> {
    widget(Tabs::new(""))
}

/// A tree of rows; bind `.nodes(..)`.
#[track_caller]
pub fn tree_view() -> El<TreeView> {
    widget(TreeView::new(Vec::new()))
}

/// A width × height field with a preset menu; bind `.value(..)`.
#[track_caller]
pub fn size_field() -> El<SizeField> {
    widget(SizeField::new([0.0, 0.0]))
}

/// A colour well with its picker; bind `.value(..)`.
#[track_caller]
pub fn color_field() -> El<ColorField> {
    widget(ColorField::new([0.0, 0.0, 0.0, 1.0]))
}

/// A menu behind a text trigger. Its items are its children, fixed ones
/// and keyed lists (`each`) alike; an empty label hides the trigger. Not a
/// template tag (a control there is a leaf): templates write
/// `<Widget of={ActionMenu::new()}>`.
#[track_caller]
pub fn action_menu(label: impl IntoProp<Arc<str>>) -> El<ActionMenu> {
    widget(ActionMenu::new()).label(label)
}

macro_rules! menu_fields {
    ($($field:ident: $ty:ty, $doc:literal, |$target:ident, $value:ident| $write:expr, |$read:ident| $current:expr;)*) => {
        $(
            impl FieldWrite<ActionMenu, $ty> for fields::action_menu::$field {
                const FIELD: &'static str = concat!("ActionMenu.", stringify!($field));

                fn write($target: &mut ActionMenu, $value: $ty) {
                    $write
                }

                fn differs($read: &ActionMenu, value: &$ty) -> bool {
                    $current != *value
                }
            }
        )*

        impl<K> El<ActionMenu, K> {
            $(
                #[doc = $doc]
                #[track_caller]
                pub fn $field(self, value: impl IntoProp<$ty>) -> Self {
                    self.prop::<$ty, fields::action_menu::$field>(value)
                }
            )*
        }
    };
}

menu_fields! {
    label: Arc<str>,
        "The trigger's text. A menu without one has nothing to hang from: it closes and shows nothing.",
        |target, value| {
            target.popover.open &= !value.is_empty();
            target.popover.trigger = value;
        },
        |menu| menu.popover.trigger;
    accessible_name: Arc<str>,
        "What a screen reader says for the trigger when its text is ambiguous.",
        |target, value| target.popover.accessible_name = value,
        |menu| menu.popover.accessible_name;
    open: bool,
        "Whether the menu is open; it also opens and closes itself.",
        |target, value| target.popover.open = value,
        |menu| menu.popover.open;
    overlay_mount: nana_ui_core::OverlayMount,
        "Where the floating menu surface mounts visually in the scene tree.",
        |target, value| target.popover.overlay_mount = value,
        |menu| menu.popover.overlay_mount;
}

/// One command of an [`action_menu`]; `.on_activate(..)` runs it.
#[track_caller]
pub fn action_menu_item(label: impl IntoProp<Arc<str>>) -> El<ActionMenuItem> {
    widget(ActionMenuItem::new("")).label(label)
}

impl<K> El<EmptyState, K> {
    /// The one action under the message, such as a retry button.
    pub fn action(self, view: impl IntoView) -> Self {
        self.child_slot(view, |empty, id| empty.action_child(id))
    }
}

/// Field writers to bind with [`El::prop`], for the fields the generated
/// setters do not cover or with an added rule:
///
/// - `fields::<element function>::<field>`: the writer behind each setter of
///   the control table (`fields::text::value` is what `.value(..)` binds on a
///   `text`), so a writer can be named and wrapped;
/// - [`fields::Visible`]: whether a node takes part in layout, paint and hit
///   testing (what `.visible(..)` binds);
/// - [`fields::Localized`]: the message a text says (what [`t`] binds);
/// - [`fields::HiddenWhenEmpty`]: any of those fields, with the node hidden
///   while the value is empty (`.prop::<_, HiddenWhenEmpty<fields::text::value>>(title)`);
/// - [`fields::FromString`] / [`fields::FromOptionalString`]: an `Arc<str>`
///   field bound from the `String` a store holds.
pub mod fields {
    pub use super::generated::*;
    pub use super::{
        Background, Blank, Border, Edge, Elevation, Fill, Foreground, FromOptionalString,
        FromString, GuardedText, GuardedTextArea, HiddenWhenEmpty, Ink, InteractiveIdle, Localized,
        NonEmpty, NonEmptyString, Opacity, Radius, Transform, Visible, WhileIdle,
    };

    /// Responsive layout [`FieldWrite`](crate::view::FieldWrite)s for any
    /// [`StyledComponent`](super::StyledComponent).
    pub mod layout {
        pub use super::super::{
            LayoutDirection as direction, LayoutFlexGrow as flex_grow,
            LayoutFlexShrink as flex_shrink, LayoutHeight as height, LayoutMargin as margin,
            LayoutMaxHeight as max_height, LayoutMaxWidth as max_width,
            LayoutMinHeight as min_height, LayoutMinWidth as min_width, LayoutPadding as padding,
            LayoutWidth as width,
        };
    }

    /// [`crate::IconButton`] extra writers beyond the control table.
    pub struct IconLabelAndTooltip;

    /// [`crate::SegmentedControl`] fields.
    #[allow(non_camel_case_types)]
    pub mod segmented_control {
        /// Accessible label for a segmented control.
        pub struct label;
    }

    /// [`crate::Toolbar`] fields.
    #[allow(non_camel_case_types)]
    pub mod toolbar {
        /// Accessible label for a toolbar.
        pub struct label;
    }

    /// [`crate::ScrollView`] fields.
    #[allow(non_camel_case_types)]
    pub mod scroll_view {
        /// Accessible label for a scroll view.
        pub struct label;
    }

    /// [`crate::HoverCard`] fields.
    #[allow(non_camel_case_types)]
    pub mod hover_card {
        /// Trigger label / spoken name for a hover card.
        pub struct trigger;
    }

    /// [`crate::ActionMenu`] fields (not a template tag: its items are its
    /// children).
    #[allow(non_camel_case_types)]
    pub mod action_menu {
        /// The trigger's text; an empty one closes the menu.
        pub struct label;
        /// The trigger's spoken name.
        pub struct accessible_name;
        /// Whether the menu is open.
        pub struct open;
        /// Where the floating menu surface mounts visually.
        pub struct overlay_mount;
    }

    /// [`crate::ReorderList`] fields (the `controls` feature).
    #[cfg(feature = "controls")]
    #[allow(non_camel_case_types)]
    pub mod reorder_list {
        /// `ReorderList` field `label`.
        pub struct label;
        /// `ReorderList` field `items`.
        pub struct items;
        /// `ReorderList` field `drop_zone`.
        pub struct drop_zone;
        /// `ReorderList` field `transfer_group`.
        pub struct transfer_group;
        /// `ReorderList` field `accepts_root`.
        pub struct accepts_root;
        /// `ReorderList` field `reorder`.
        pub struct reorder;
    }
}

impl InteractiveIdle for RangeField {
    fn is_idle(&self) -> bool {
        self.dragging.is_none() && !self.editing
    }
}

impl InteractiveIdle for RangeSpanField {
    fn is_idle(&self) -> bool {
        self.dragging.is_none()
    }
}

impl InteractiveIdle for CurveField {
    fn is_idle(&self) -> bool {
        self.dragging.is_none()
    }
}

impl InteractiveIdle for ColorField {
    fn is_idle(&self) -> bool {
        !self.opened
    }
}

impl InteractiveIdle for NumberInput {
    fn is_idle(&self) -> bool {
        self.state.value == self.formatted_value()
    }
}

/// Dirty-guarded value writer for [`TextInput`]: takes `(value, dirty)` and
/// skips external writes while `dirty` is `true` on an interactive field.
pub struct GuardedText;

impl FieldWrite<TextInput, (String, bool)> for GuardedText {
    const FIELD: &'static str = "TextInput.value";

    fn write(target: &mut TextInput, (value, dirty): (String, bool)) {
        if dirty && !target.disabled && !target.read_only {
            return;
        }
        if target.state.value != value {
            target.state = crate::TextInputState::new(value);
        }
    }

    fn differs(target: &TextInput, (value, dirty): &(String, bool)) -> bool {
        if *dirty && !target.disabled && !target.read_only {
            return false;
        }
        target.state.value.as_str() != value.as_str()
    }
}

/// Dirty-guarded value writer for [`TextArea`]: takes `(value, dirty)` and
/// skips external writes while `dirty` is `true` on an interactive field.
pub struct GuardedTextArea;

impl FieldWrite<TextArea, (String, bool)> for GuardedTextArea {
    const FIELD: &'static str = "TextArea.value";

    fn write(target: &mut TextArea, (value, dirty): (String, bool)) {
        if dirty && !target.disabled && !target.read_only {
            return;
        }
        if target.state.value != value {
            target.state = crate::TextInputState::new(value);
        }
    }

    fn differs(target: &TextArea, (value, dirty): &(String, bool)) -> bool {
        if *dirty && !target.disabled && !target.read_only {
            return false;
        }
        target.state.value.as_str() != value.as_str()
    }
}

impl FieldWrite<IconButton, Arc<str>> for fields::IconLabelAndTooltip {
    const FIELD: &'static str = "IconButton.label_and_tooltip";

    fn write(target: &mut IconButton, value: Arc<str>) {
        if let Some(tooltip) = &mut target.tooltip {
            tooltip.label = Arc::clone(&value);
        }
        target.label = value;
    }

    fn differs(target: &IconButton, value: &Arc<str>) -> bool {
        target.label != *value
            || target
                .tooltip
                .as_ref()
                .is_some_and(|tooltip| tooltip.label != *value)
    }
}

impl<K> El<IconButton, K> {
    /// Bind both [`IconButton::label`] and, when present, its tooltip text to
    /// the same source (updating both together on locale switches).
    #[track_caller]
    pub fn label_and_tooltip(self, value: impl IntoProp<Arc<str>>) -> Self {
        self.prop::<Arc<str>, fields::IconLabelAndTooltip>(value)
    }
}

impl<K> El<TextInput, K> {
    /// Focus this text input in place and select all text once mounted into a
    /// visible subtree (shorthand for setting `focus_and_select_on_mount(true)`
    /// and wiring `on_mount`).
    #[track_caller]
    pub fn auto_focus_select(self) -> Self {
        let input_ref = node_ref();
        on_mount(move |cx| {
            if let Some(id) = input_ref.get_untracked() {
                let _ = cx.focus_and_select_text_input(Entity::from_stable_id(id));
            }
        });
        self.focus_and_select_on_mount(true).node_ref(input_ref)
    }
}

impl FieldWrite<SegmentedControl, Option<Arc<str>>> for fields::segmented_control::label {
    const FIELD: &'static str = "SegmentedControl.label";

    fn write(target: &mut SegmentedControl, value: Option<Arc<str>>) {
        target.label = value;
    }

    fn differs(target: &SegmentedControl, value: &Option<Arc<str>>) -> bool {
        target.label != *value
    }
}

impl<K> El<SegmentedControl, K> {
    #[track_caller]
    pub fn label(self, value: impl IntoProp<Option<Arc<str>>>) -> Self {
        self.prop::<Option<Arc<str>>, fields::segmented_control::label>(value)
    }
}

impl FieldWrite<Toolbar, Option<Arc<str>>> for fields::toolbar::label {
    const FIELD: &'static str = "Toolbar.label";

    fn write(target: &mut Toolbar, value: Option<Arc<str>>) {
        target.label = value;
    }

    fn differs(target: &Toolbar, value: &Option<Arc<str>>) -> bool {
        target.label != *value
    }
}

impl<K> El<Toolbar, K> {
    #[track_caller]
    pub fn label(self, value: impl IntoProp<Option<Arc<str>>>) -> Self {
        self.prop::<Option<Arc<str>>, fields::toolbar::label>(value)
    }
}

impl FieldWrite<ScrollView, Option<Arc<str>>> for fields::scroll_view::label {
    const FIELD: &'static str = "ScrollView.label";

    fn write(target: &mut ScrollView, value: Option<Arc<str>>) {
        target.label = value;
    }

    fn differs(target: &ScrollView, value: &Option<Arc<str>>) -> bool {
        target.label != *value
    }
}

impl<K> El<ScrollView, K> {
    #[track_caller]
    pub fn label(self, value: impl IntoProp<Option<Arc<str>>>) -> Self {
        self.prop::<Option<Arc<str>>, fields::scroll_view::label>(value)
    }
}

impl FieldWrite<HoverCard, Arc<str>> for fields::hover_card::trigger {
    const FIELD: &'static str = "HoverCard.trigger";

    fn write(target: &mut HoverCard, value: Arc<str>) {
        target.trigger = value;
    }

    fn differs(target: &HoverCard, value: &Arc<str>) -> bool {
        target.trigger != *value
    }
}

impl<K> El<HoverCard, K> {
    #[track_caller]
    pub fn trigger(self, value: impl IntoProp<Arc<str>>) -> Self {
        self.prop::<Arc<str>, fields::hover_card::trigger>(value)
    }
}

#[cfg(feature = "controls")]
impl FieldWrite<crate::ReorderList, Option<Arc<str>>> for fields::reorder_list::label {
    const FIELD: &'static str = "ReorderList.label";

    fn write(target: &mut crate::ReorderList, label: Option<Arc<str>>) {
        if target.label != label {
            target.label = label;
        }
    }

    fn differs(target: &crate::ReorderList, label: &Option<Arc<str>>) -> bool {
        target.label != *label
    }
}

#[cfg(feature = "controls")]
impl<K> El<crate::ReorderList, K> {
    #[track_caller]
    pub fn label(self, value: impl IntoProp<Option<Arc<str>>>) -> Self {
        self.prop::<Option<Arc<str>>, fields::reorder_list::label>(value)
    }
}

#[cfg(feature = "controls")]
impl FieldWrite<crate::ReorderList, Vec<crate::ReorderItem>> for fields::reorder_list::items {
    const FIELD: &'static str = "ReorderList.items";

    fn write(target: &mut crate::ReorderList, items: Vec<crate::ReorderItem>) {
        if target.items != items {
            target.items = items;
        }
    }

    fn differs(target: &crate::ReorderList, items: &Vec<crate::ReorderItem>) -> bool {
        target.items != *items
    }
}

#[cfg(feature = "controls")]
impl FieldWrite<crate::ReorderList, Option<StableNodeId>> for fields::reorder_list::drop_zone {
    const FIELD: &'static str = "ReorderList.drop_zone";

    fn write(target: &mut crate::ReorderList, zone: Option<StableNodeId>) {
        if target.drop_zone != zone {
            target.drop_zone = zone;
        }
    }

    fn differs(target: &crate::ReorderList, zone: &Option<StableNodeId>) -> bool {
        target.drop_zone != *zone
    }
}

#[cfg(feature = "controls")]
impl FieldWrite<crate::ReorderList, Option<Arc<str>>> for fields::reorder_list::transfer_group {
    const FIELD: &'static str = "ReorderList.transfer_group";

    fn write(target: &mut crate::ReorderList, group: Option<Arc<str>>) {
        if target.transfer_group != group {
            target.transfer_group = group;
        }
    }

    fn differs(target: &crate::ReorderList, group: &Option<Arc<str>>) -> bool {
        target.transfer_group != *group
    }
}

#[cfg(feature = "controls")]
impl FieldWrite<crate::ReorderList, crate::ReorderAccepts> for fields::reorder_list::accepts_root {
    const FIELD: &'static str = "ReorderList.accepts_root";

    fn write(target: &mut crate::ReorderList, accepts: crate::ReorderAccepts) {
        if target.accepts_root != accepts {
            target.accepts_root = accepts;
        }
    }

    fn differs(target: &crate::ReorderList, accepts: &crate::ReorderAccepts) -> bool {
        target.accepts_root != *accepts
    }
}

#[cfg(feature = "controls")]
impl FieldWrite<crate::ReorderList, bool> for fields::reorder_list::reorder {
    const FIELD: &'static str = "ReorderList.reorder";

    fn write(target: &mut crate::ReorderList, reorder: bool) {
        if target.reorder != reorder {
            target.reorder = reorder;
        }
    }

    fn differs(target: &crate::ReorderList, reorder: &bool) -> bool {
        target.reorder != *reorder
    }
}

#[cfg(feature = "controls")]
impl StyledComponent for crate::ReorderList {
    fn node_style(&self) -> &NodeStyle {
        &self.style
    }

    fn node_style_mut(&mut self) -> &mut NodeStyle {
        &mut self.style
    }
}

/// An empty message is no message: `EmptyState.message` from text, `None`
/// while the text is empty.
impl FieldWrite<EmptyState, String> for generated::empty_state::message {
    const FIELD: &'static str = "EmptyState.message";

    fn write(target: &mut EmptyState, message: String) {
        let message = (!message.is_empty()).then(|| Arc::from(message));
        if target.message != message {
            target.message = message;
        }
    }

    fn differs(target: &EmptyState, message: &String) -> bool {
        target.message.as_deref().unwrap_or("") != message.as_str()
    }
}

/// The `Arc<str>` field `W` writes, bound from a `String`. It compares before
/// converting, so an unchanged value costs no allocation.
pub struct FromString<W>(std::marker::PhantomData<W>);

impl<C, W: FieldWrite<C, Arc<str>>> FieldWrite<C, String> for FromString<W> {
    const FIELD: &'static str = W::FIELD;

    fn write(target: &mut C, value: String) {
        W::write(target, value.into());
    }

    fn differs(target: &C, value: &String) -> bool {
        W::differs(target, &Arc::from(value.as_str()))
    }
}

/// [`FromString`] for an optional text field.
pub struct FromOptionalString<W>(std::marker::PhantomData<W>);

impl<C, W: FieldWrite<C, Option<Arc<str>>>> FieldWrite<C, Option<String>>
    for FromOptionalString<W>
{
    const FIELD: &'static str = W::FIELD;

    fn write(target: &mut C, value: Option<String>) {
        W::write(target, value.map(Arc::from));
    }

    fn differs(target: &C, value: &Option<String>) -> bool {
        W::differs(target, &value.as_deref().map(Arc::from))
    }
}

/// A value that can be empty, for [`HiddenWhenEmpty`].
pub trait Blank {
    fn is_blank(&self) -> bool;
}

impl Blank for String {
    fn is_blank(&self) -> bool {
        self.is_empty()
    }
}

impl Blank for Arc<str> {
    fn is_blank(&self) -> bool {
        self.is_empty()
    }
}

/// `None` is empty; `Some("")` is a value.
impl<T> Blank for Option<T> {
    fn is_blank(&self) -> bool {
        self.is_none()
    }
}

impl<T> Blank for Vec<T> {
    fn is_blank(&self) -> bool {
        self.is_empty()
    }
}

/// The field `W` writes, with the node out of layout, paint and hit testing
/// while the value is empty ([`Blank`]): a line that shows only while it says
/// something, a list only while it has rows. Both change in the same write,
/// so the node never shows an empty value.
pub struct HiddenWhenEmpty<W>(std::marker::PhantomData<W>);

impl<C, T, W> FieldWrite<C, T> for HiddenWhenEmpty<W>
where
    C: StyledComponent,
    T: Blank,
    W: FieldWrite<C, T>,
{
    const FIELD: &'static str = W::FIELD;

    fn write(target: &mut C, value: T) {
        let hidden = value.is_blank();
        if W::differs(target, &value) {
            W::write(target, value);
        }
        let style = target.node_style_mut();
        if style.layout.hidden != hidden {
            Arc::make_mut(&mut style.layout).hidden = hidden;
        }
    }

    fn differs(target: &C, value: &T) -> bool {
        W::differs(target, value) || target.node_style().layout.hidden != value.is_blank()
    }
}

/// A field value written as text by devtools and parsed back.
trait FieldText: Sized {
    fn parse(text: &str) -> Result<Self, String>;
}

impl FieldText for InlineContent {
    fn parse(text: &str) -> Result<Self, String> {
        Ok(text.into())
    }
}

impl FieldText for String {
    fn parse(text: &str) -> Result<Self, String> {
        Ok(text.to_owned())
    }
}

impl FieldText for Arc<str> {
    fn parse(text: &str) -> Result<Self, String> {
        Ok(Arc::from(text))
    }
}

impl FieldText for TextureSlotName {
    fn parse(text: &str) -> Result<Self, String> {
        Ok(Self::from(text))
    }
}

impl FieldText for Option<TextureSlotName> {
    /// Empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        Ok((!text.is_empty()).then(|| TextureSlotName::from(text)))
    }
}

impl FieldText for Option<Arc<str>> {
    /// Empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        Ok((!text.is_empty()).then(|| Arc::from(text)))
    }
}

impl FieldText for bool {
    fn parse(text: &str) -> Result<Self, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("`{text}` is not `true` or `false`"))
    }
}

impl FieldText for f64 {
    fn parse(text: &str) -> Result<Self, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("`{text}` is not a number"))
    }
}

impl FieldText for f32 {
    fn parse(text: &str) -> Result<Self, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("`{text}` is not a number"))
    }
}

impl FieldText for Option<usize> {
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        text.parse()
            .map(Some)
            .map_err(|_| format!("`{text}` is not a non-negative integer"))
    }
}

impl FieldText for Option<f64> {
    /// Empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        f64::parse(text).map(Some)
    }
}

impl FieldText for Option<f32> {
    /// Empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        f32::parse(text).map(Some)
    }
}

impl FieldText for Option<(u32, u32)> {
    /// A pixel size as `width×height` (or `x`, `,`); empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let mut sides = text
            .split(['×', 'x', ','])
            .map(|side| side.trim().parse::<u32>());
        match (sides.next(), sides.next(), sides.next()) {
            (Some(Ok(width)), Some(Ok(height)), None) => Ok(Some((width, height))),
            _ => Err(format!("`{text}` is not a size such as 512×512")),
        }
    }
}

impl FieldText for Option<SemanticColorRole> {
    /// A palette role by its token name (`accent`, `border-soft`); empty
    /// text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        SemanticColorRole::from_css_token_name(text)
            .map(Some)
            .ok_or_else(|| format!("`{text}` is not a palette role"))
    }
}

impl FieldText for (f64, f64) {
    /// Two numbers separated by a comma: `0.2, 0.8`.
    fn parse(text: &str) -> Result<Self, String> {
        let (low, high) = text
            .split_once(',')
            .ok_or_else(|| format!("`{text}` is not two numbers separated by a comma"))?;
        Ok((f64::parse(low)?, f64::parse(high)?))
    }
}

impl FieldText for Option<(f64, f64)> {
    /// Two numbers separated by a comma; empty text is `None`.
    fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        <(f64, f64)>::parse(text).map(Some)
    }
}

impl FieldText for Vec<f64> {
    /// Numbers separated by commas: `0.1, 0.4, 0.9`; empty text is none.
    fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        text.split(',').map(f64::parse).collect()
    }
}

impl FieldText for Vec<Arc<str>> {
    /// Names separated by commas; empty text is none.
    fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        Ok(text.split(',').map(|name| Arc::from(name.trim())).collect())
    }
}

impl FieldText for u64 {
    fn parse(text: &str) -> Result<Self, String> {
        text.trim()
            .parse()
            .map_err(|_| format!("`{text}` is not a whole number"))
    }
}

macro_rules! edited_in_code {
    ($($ty:ty),*) => {$(
        impl FieldText for $ty {
            fn parse(_: &str) -> Result<Self, String> {
                Err(concat!("`", stringify!($ty), "` is edited in code, not as text").into())
            }
        }
    )*};
}

edited_in_code!(
    Icon,
    Option<Icon>,
    StatusTone,
    ButtonKind,
    ListItemRole,
    MenuItemMark,
    RangeSpanOrientation,
    [f32; 4],
    [f64; 2],
    Vec<SizePreset>,
    Vec<TreeNode<Arc<str>>>,
    TreeSelection
);

impl FieldText for Vec<SelectOption> {
    fn parse(_: &str) -> Result<Self, String> {
        Err("options are edited in code, not as text".into())
    }
}

/// Every bindable field of a built-in control, as text: the control table's
/// tag and `(field, value)` pairs, `None` for anything else.
pub(crate) fn inspect_control(
    view: &dyn Any,
) -> Option<(&'static str, Vec<(&'static str, String)>)> {
    macro_rules! inspectors {
        ($(
            $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
                $($field:ident: $ty:ty = $write:ident),* $(,)?
            }
            $(on { $($on:ident: $on_event:ident),* $(,)? })?
            $(with { $($with:ident: $with_event:ident),* $(,)? })?
            $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
            ;
        )*) => {$(
            if let Some(control) = view.downcast_ref::<$component>() {
                let _ = control;
                return Some((
                    stringify!($tag),
                    vec![$((
                        stringify!($field),
                        format!("{:?}", inspectors!(@read $write control, $field)),
                    )),*],
                ));
            }
        )*};
        (@read inline $control:ident, $field:ident) => { &$control.content };
        (@read set $control:ident, $field:ident) => { &$control.$field };
        (@read text_state $control:ident, $field:ident) => { &$control.state.value };
        (@read assign $control:ident, $field:ident) => { $control.$field() };
        (@read clamp $control:ident, $field:ident) => { &$control.$field };
        (@read span_low $control:ident, $field:ident) => { &$control.low };
        (@read span_high $control:ident, $field:ident) => { &$control.high };
        (@read span $control:ident, $field:ident) => { &($control.low, $control.high) };
        (@read curve $control:ident, $field:ident) => { &$control.values };
        (@read color_value $control:ident, $field:ident) => { &$control.$field };
        (@read tooltip_text $control:ident, $field:ident) => {
            $control.$field.as_ref().map(|t| &t.label)
        };
        (@read builder $control:ident, $field:ident) => { &$control.$field };
        (@read tab $control:ident, $field:ident) => { &$control.selected };
        (@read resource $control:ident, $field:ident) => { &$control.resource };
    }
    nana_ui_view_schema::for_each_control!(inspectors);
    None
}

/// Write `field` of the built-in control at `node` from `text`, as its
/// binding would. `None` when the node is not a built-in control.
pub(crate) fn edit_control(
    cx: &mut AppContext,
    node: StableNodeId,
    field: &str,
    text: &str,
) -> Option<Result<(), String>> {
    macro_rules! editors {
        ($(
            $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
                $($field:ident: $ty:ty = $write:ident),* $(,)?
            }
            $(on { $($on:ident: $on_event:ident),* $(,)? })?
            $(with { $($with:ident: $with_event:ident),* $(,)? })?
            $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
            ;
        )*) => {$(
            if cx.view_is::<$component>(node) {
                return Some(match field {
                    $(stringify!($field) => <$ty as FieldText>::parse(text).and_then(|value| {
                        cx.update_component(Entity::<$component>::from_stable_id(node), |control, _| {
                            <generated::$function::$field as FieldWrite<$component, $ty>>::write(control, value)
                        })
                        .map_err(|error| error.to_string())
                    }),)*
                    other => Err(format!(
                        "`<{}>` has no field `{other}`",
                        stringify!($tag)
                    )),
                });
            }
        )*};
    }
    nana_ui_view_schema::for_each_control!(editors);
    None
}

macro_rules! item_slots {
    ($($component:ty),*) => {$(
        /// A list row's slots, as its own children: give them in the order
        /// leading, content, trailing.
        impl<K> El<$component, K> {
            /// Before the label (an icon, a check).
            pub fn leading(self, view: impl super::IntoView) -> Self {
                self.child_slot(view, |mut item: $component, id| {
                    item.slots.leading = Some(id);
                    item
                })
            }

            /// In place of the label.
            pub fn content(self, view: impl super::IntoView) -> Self {
                self.child_slot(view, |mut item: $component, id| {
                    item.slots.content = Some(id);
                    item
                })
            }

            /// After the label (a count, a shortcut, a control).
            pub fn trailing(self, view: impl super::IntoView) -> Self {
                self.child_slot(view, |mut item: $component, id| {
                    item.slots.trailing = Some(id);
                    item
                })
            }
        }
    )*};
}

item_slots!(crate::SidebarRow);

impl<K> El<ListItem, K> {
    /// Before the label (an icon, a check). On a first-line row it sits in
    /// the first line's band, which the row keeps in step with its
    /// alignment; see [`crate::TrailingAlign`].
    pub fn leading(self, view: impl super::IntoView) -> Self {
        self.child_slot(view, |mut item: ListItem, id| {
            item.slots.leading = Some(id);
            item
        })
    }

    /// In place of the label.
    pub fn content(self, view: impl super::IntoView) -> Self {
        self.child_slot(view, |mut item: ListItem, id| {
            item.slots.content = Some(id);
            item
        })
    }

    /// Immediately after the label (and `note`), before `detail` and
    /// `trailing`: a category seal, a badge or an inline control. The label
    /// elides first; `detail` and `trailing` keep their places at the end.
    pub fn after_label(self, view: impl super::IntoView) -> Self {
        self.slot(view, ListItem::after_label)
    }

    /// After the label (a count, a shortcut, a control). On a first-line row
    /// it sits in the first line's band, which the row keeps in step with
    /// its alignment; see [`crate::TrailingAlign`].
    pub fn trailing(self, view: impl super::IntoView) -> Self {
        self.child_slot(view, |mut item: ListItem, id| {
            item.slots.trailing = Some(id);
            item
        })
    }
}

impl<K> El<ListItem, K> {
    /// Row tools in the trailing slot: shown while the pointer is on the row
    /// or keyboard focus is inside it, so Tab reaches them. Give it in place
    /// of `.trailing`; [`ListItem::tools_rest`] says how they sit while put
    /// away, and `keep_tools` keeps them shown at rest (a pin that is on).
    pub fn tools(self, view: impl super::IntoView) -> Self {
        self.child_slot(view, |mut item: ListItem, id| {
            item.slots.trailing = Some(id);
            item.trailing_tools = true;
            item
        })
    }
}

impl<K> El<crate::SidebarRow, K> {
    /// Row tools after the label, shown in place of the trailing slot while
    /// the pointer is on the row or keyboard focus is inside it.
    pub fn tools(self, view: impl super::IntoView) -> Self {
        self.child_slot(view, crate::SidebarRow::tools)
    }
}

impl<K> El<TextArea, K> {
    /// A real action inside the field's bottom trailing corner.
    pub fn trailing_action(self, view: impl IntoView) -> Self {
        self.child_slot(view, TextArea::trailing_action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{FluentCatalog, Locale, LocalizedText};
    use crate::view::node_ref;
    use crate::{DocumentId, LayoutViewport, RangeDragState, TextSubmitted};
    use nana_ui_core::{ElevationRole, PaintTransform};

    #[test]
    fn elevation_and_styled_component_writers_update_node_style() {
        let mut stack = Stack::column(0.0).elevation(ElevationRole::Overlay);
        assert_eq!(
            stack.node_style().elevation_role(),
            Some(ElevationRole::Overlay)
        );

        Elevation::write(&mut stack, Some(ElevationRole::Surface));
        assert_eq!(
            stack.node_style().elevation_role(),
            Some(ElevationRole::Surface)
        );

        Opacity::write(&mut stack, 0.5);
        assert_eq!(stack.node_style().layout.opacity, Some(0.5));

        let tx = PaintTransform {
            e: 4.0,
            f: -2.0,
            ..PaintTransform::IDENTITY
        };
        Transform::write(&mut stack, Some(tx));
        assert_eq!(stack.node_style().layout.transforms.transform, Some(tx));

        let color = nana_ui_core::style_model::PaintColor::Srgb {
            rgba: [0.2, 0.4, 0.6, 1.0],
        };
        Fill::write(&mut stack, Some(color));
        assert_eq!(
            stack.node_style().layout.paint_colors.background,
            Some(color)
        );

        fields::layout::direction::write(&mut stack, Some(FlexDirection::Row));
        assert_eq!(
            stack.node_style().layout.direction,
            Some(FlexDirection::Row)
        );
        fields::layout::max_width::write(&mut stack, Some(LengthSpec::Px(320.0)));
        assert_eq!(
            stack.node_style().layout.max_width,
            Some(LengthSpec::Px(320.0))
        );
    }

    #[test]
    fn interactive_value_bindings_skip_writes_during_live_interaction_and_quantize() {
        // RangeField: quantize on idle, skip while dragging or inline-editing.
        let mut range = RangeField::new(0.0, 0.0, 1.0, 0.25);
        fields::slider::value::write(&mut range, 0.26);
        assert_eq!(range.value, 0.25);
        range.dragging = Some(RangeDragState::new(1, 0.25));
        assert!(!range.is_idle());
        assert!(!fields::slider::value::differs(&range, &0.75));
        fields::slider::value::write(&mut range, 0.75);
        assert_eq!(range.value, 0.25);

        // ColorField: skip while HSV picker popover is open.
        let mut color = ColorField::new([0.1, 0.2, 0.3, 1.0]);
        color.opened = true;
        assert!(!color.is_idle());
        assert!(!fields::color_field::value::differs(
            &color,
            &[0.9, 0.8, 0.7, 1.0]
        ));
        fields::color_field::value::write(&mut color, [0.9, 0.8, 0.7, 1.0]);
        assert_eq!(color.value, [0.1, 0.2, 0.3, 1.0]);

        // NumberInput: skip while user holds an uncommitted draft.
        let mut number = NumberInput::new(10.0);
        number.state.replace_value("12.".to_owned());
        assert!(!number.is_idle());
        assert!(!fields::number_input::value::differs(&number, &50.0));
        fields::number_input::value::write(&mut number, 50.0);
        assert_eq!(number.state.value, "12.");

        // CurveField: skip while dragging.
        let mut curve = CurveField::new(vec![0.0, 0.5, 1.0], 0.05);
        curve.dragging = Some(crate::CurveDragState {
            pointer_id: 1,
            point: 1,
            grab_offset: 0.0,
            initial: vec![0.0, 0.5, 1.0],
        });
        assert!(!curve.is_idle());
        fields::curve::values::write(&mut curve, vec![0.0, 0.9, 1.0]);
        assert_eq!(curve.values, vec![0.0, 0.5, 1.0]);

        // Generic WhileIdle<W> and NonEmpty<W>.
        let mut idle_range = RangeField::new(0.0, 0.0, 1.0, 0.1);
        NonEmpty::<fields::slider::unit>::write(&mut idle_range, Arc::from(""));
        assert_eq!(idle_range.unit, None);
        NonEmpty::<fields::slider::unit>::write(&mut idle_range, Arc::from("%"));
        assert_eq!(idle_range.unit.as_deref(), Some("%"));
        idle_range.editing = true;
        WhileIdle::<fields::slider::disabled>::write(&mut idle_range, true);
        assert!(!idle_range.disabled);
    }

    #[test]
    fn control_level_localized_text_updates_on_locale_switch() {
        let catalog = FluentCatalog::new()
            .with_ftl("en-US", "save = Save\nsearch = Search items")
            .unwrap()
            .with_ftl("zh-CN", "save = 保存\nsearch = 搜索项目")
            .unwrap();
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        cx.set_message_catalog(Some(Arc::new(catalog)));
        cx.set_default_locale(Locale::parse("en-US"));

        let btn_ref = node_ref();
        let input_ref = node_ref();
        cx.mount_view_root(doc, move || {
            column().children((
                button(LocalizedText::new("save")).node_ref(btn_ref),
                text_input()
                    .placeholder(LocalizedText::new("search"))
                    .node_ref(input_ref),
            ))
        })
        .unwrap();
        cx.flush_reactive().unwrap();

        let btn_id = btn_ref.get_untracked().unwrap();
        let input_id = input_ref.get_untracked().unwrap();
        assert_eq!(
            cx.read(Entity::<Button>::from_stable_id(btn_id), |b| b
                .label
                .clone())
                .unwrap(),
            "Save"
        );
        assert_eq!(
            cx.read(Entity::<TextInput>::from_stable_id(input_id), |t| t
                .placeholder
                .to_string())
                .unwrap(),
            "Search items"
        );

        cx.set_default_locale(Locale::parse("zh-CN"));
        cx.flush_reactive().unwrap();
        assert_eq!(
            cx.read(Entity::<Button>::from_stable_id(btn_id), |b| b
                .label
                .clone())
                .unwrap(),
            "保存"
        );
        assert_eq!(
            cx.read(Entity::<TextInput>::from_stable_id(input_id), |t| t
                .placeholder
                .to_string())
                .unwrap(),
            "搜索项目"
        );
    }

    #[test]
    fn view_context_blur_and_text_input_blur_on_submit_and_focus_on_mount() {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let input_ref = node_ref();
        let seen_doc = Arc::new(std::sync::Mutex::new(None));
        let seen_doc_cb = Arc::clone(&seen_doc);
        cx.mount_view_root(doc, move || {
            text_input()
                .value("hello")
                .blur_on_submit(true)
                .auto_focus_select()
                .node_ref(input_ref)
                .on_cx(move |_: &mut TextInput, _: &TextSubmitted, vcx| {
                    *seen_doc_cb.lock().unwrap() = Some(vcx.document());
                })
        })
        .unwrap();
        cx.layout_document(doc, LayoutViewport::new(320.0, 200.0))
            .unwrap();
        let input_id = input_ref.get_untracked().unwrap();
        assert_eq!(cx.world().focused(doc), Some(input_id));
        let sel = cx
            .read(Entity::<TextInput>::from_stable_id(input_id), |t| {
                t.state.selection
            })
            .unwrap();
        assert_eq!(sel.range(), 0..5);

        assert!(cx.submit_focused_text_input(doc).unwrap());
        assert_eq!(*seen_doc.lock().unwrap(), Some(doc));
        assert_eq!(cx.world().focused(doc), None);
    }
}
