use super::{SLOT_PADDING, document_of, finite_positive, patch_layout, reconcile_ids, view_is};
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LengthSpec, OverflowSpec,
    SemanticColorMix, SemanticColorRole, TITLE_BAR_HEIGHT, WINDOW_CONTROL_GAP,
    WINDOW_CONTROL_PADDING, WINDOW_CONTROL_WIDTH, WindowChrome,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, IconButton, InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    Text, TextContent, TextHorizontalAlignment, TextVerticalAlignment, UiWorld,
};

const CENTER_PADDING_X: f32 = nana_ui_core::space::XXL;
const DEFAULT_CENTER_WIDTH: f32 = 168.0;
/// Extra gap after the native traffic-light exclusion so leading chrome
/// (sidebar toggle) cannot sit on the caption buttons.
const NATIVE_LEADING_CLEARANCE: f32 = nana_ui_core::space::MD;
/// Width of the native window-button placeholder: the macOS traffic-light
/// cluster centered with equal margins.
pub(super) const NATIVE_WINDOW_CONTROLS_WIDTH: f32 = 78.0;
/// A window-control capsule's inset around its buttons, sideways and up and
/// down, inside its one-pixel hairline.
const CONTROLS_CAPSULE_INSET_X: f32 = nana_ui_core::space::XXS;
const CONTROLS_CAPSULE_INSET_Y: f32 = nana_ui_core::HAIRLINE;
/// A capsule's height: a window button, its inset and its hairline.
const CONTROLS_CAPSULE_HEIGHT: f32 = WINDOW_CONTROL_WIDTH + 2.0 * (CONTROLS_CAPSULE_INSET_Y + 1.0);

pub(super) const LEADING_COLUMN_TAG: &str = "app-title-bar-leading";
pub(super) const CENTER_COLUMN_TAG: &str = "app-title-bar-center";
pub(super) const TRAILING_COLUMN_TAG: &str = "app-title-bar-trailing";

/// Typed window command. Runtime never touches a platform window handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChromeAction {
    Minimize,
    ToggleMaximize,
    Close,
}

impl WindowChromeAction {
    pub const ALL: [Self; 3] = [Self::Minimize, Self::ToggleMaximize, Self::Close];

    pub fn icon(self, maximized: bool) -> Icon {
        match self {
            Self::Minimize => Icon::Minimize,
            Self::ToggleMaximize if maximized => Icon::Restore,
            Self::ToggleMaximize => Icon::Maximize,
            Self::Close => Icon::Close,
        }
    }

    /// What a screen reader calls the button, from the framework strings.
    pub fn label(self, strings: &nana_ui_core::FrameworkStrings, maximized: bool) -> Arc<str> {
        Arc::clone(match self {
            Self::Minimize => &strings.window_minimize,
            Self::ToggleMaximize if maximized => &strings.window_restore,
            Self::ToggleMaximize => &strings.window_maximize,
            Self::Close => &strings.window_close,
        })
    }
}

/// What an [`AppTitleBar`] draws behind itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TitleBarBackdrop {
    /// The theme's title-bar background.
    #[default]
    Themed,
    /// Nothing; see [`AppTitleBar::transparent`].
    Transparent,
    /// The media scrim, with on-media title and window buttons; see
    /// [`AppTitleBar::over_media`].
    OverMedia,
}

/// 36px application title bar. Leading / center / trailing / controls are host-mounted.
#[derive(Debug, Clone, PartialEq)]
pub struct AppTitleBar {
    pub title: Arc<str>,
    pub(crate) backdrop: TitleBarBackdrop,
    pub drag_enabled: bool,
    pub leading: Option<StableNodeId>,
    pub center: Option<StableNodeId>,
    pub trailing: Option<StableNodeId>,
    pub controls: Option<StableNodeId>,
    pub center_width: f32,
    /// Window buttons are the platform's native ones (macOS traffic lights):
    /// the controls slot is an empty leading placeholder the host moves them
    /// onto. Otherwise it holds the custom trailing buttons.
    pub native_controls: bool,
    pub show_window_controls: bool,
    pub maximized: bool,
    pub style: NodeStyle,
    /// How far the slots stand in from the bar's outer edges.
    pub(crate) edge_inset: nana_ui_core::SpacingStep,
    /// The window buttons sit in a capsule: see [`Self::controls_capsule`].
    pub(crate) controls_capsule: bool,
}

impl AppTitleBar {
    pub fn new(title: impl Into<Arc<str>>) -> Self {
        Self {
            title: title.into(),
            backdrop: TitleBarBackdrop::Themed,
            drag_enabled: true,
            leading: None,
            center: None,
            trailing: None,
            controls: None,
            center_width: DEFAULT_CENTER_WIDTH,
            native_controls: !WindowChrome::platform_default().uses_custom_controls(),
            show_window_controls: true,
            maximized: false,
            style: NodeStyle::default(),
            edge_inset: nana_ui_core::SpacingStep::Sm,
            controls_capsule: false,
        }
    }

    /// Sets the window buttons on a capsule of their own: a `Surface` plate
    /// with a `BorderSoft` hairline and round ends, standing in from the
    /// window edge by the bar's edge inset, its buttons round. For a bar laid
    /// over a scene, where bare buttons would sit on whatever is behind and
    /// the application's own controls beside them are capsules too. The
    /// native traffic lights get the same plate under them, the placeholder's
    /// width. Off by default.
    pub fn controls_capsule(mut self, capsule: bool) -> Self {
        self.controls_capsule = capsule;
        self
    }

    /// How far the leading slot's content stands in from the bar's left
    /// edge, and the trailing slot's from its right edge: `Sm` by default.
    /// A bar heading a card gives the card's own inset, so its title and
    /// tools stand on the card's content line without the application
    /// taking the bar's `Sm` back off its own padding. The window controls
    /// keep their place: the native placeholder still sits flush at the
    /// leading edge and custom controls still hug the trailing edge.
    pub fn edge_inset(mut self, step: nana_ui_core::SpacingStep) -> Self {
        self.edge_inset = step;
        self
    }

    /// Removes only the title-bar background, preserving foreground and slot
    /// content. A bar [`Self::over_media`] stays so.
    pub fn transparent(mut self, transparent: bool) -> Self {
        if self.backdrop != TitleBarBackdrop::OverMedia {
            self.backdrop = if transparent {
                TitleBarBackdrop::Transparent
            } else {
                TitleBarBackdrop::Themed
            };
        }
        self
    }

    /// Lays the bar over media, the way an image viewer shows an image: the
    /// theme's media scrim fades from the top edge to nothing behind the bar,
    /// and the title and window buttons take the theme's on-media foreground
    /// ([`SemanticColorRole::OnMedia`]). The media is dark in the light theme
    /// too, so both themes look the same. Implies [`Self::transparent`];
    /// turning it off gives the themed bar back.
    ///
    /// It covers the title and the window buttons only: content the
    /// application puts in the bar's slots keeps its own roles.
    pub fn over_media(mut self, over_media: bool) -> Self {
        if over_media {
            self.backdrop = TitleBarBackdrop::OverMedia;
        } else if self.backdrop == TitleBarBackdrop::OverMedia {
            self.backdrop = TitleBarBackdrop::Themed;
        }
        self
    }

    fn is_over_media(&self) -> bool {
        self.backdrop == TitleBarBackdrop::OverMedia
    }

    /// Enables native window dragging from the bar's non-interactive content.
    pub fn drag_enabled(mut self, enabled: bool) -> Self {
        self.drag_enabled = enabled;
        self
    }

    pub fn leading(mut self, leading: StableNodeId) -> Self {
        self.leading = Some(leading);
        self
    }

    pub fn center(mut self, center: StableNodeId) -> Self {
        self.center = Some(center);
        self
    }

    pub fn trailing(mut self, trailing: StableNodeId) -> Self {
        self.trailing = Some(trailing);
        self
    }

    pub fn controls(mut self, controls: StableNodeId) -> Self {
        self.controls = Some(controls);
        self
    }

    pub fn center_width(mut self, width: f32) -> Self {
        self.center_width = finite_positive(width, DEFAULT_CENTER_WIDTH).max(1.0);
        self
    }

    pub fn native_controls(mut self, native: bool) -> Self {
        self.native_controls = native;
        self
    }

    pub fn show_window_controls(mut self, show: bool) -> Self {
        self.show_window_controls = show;
        self
    }

    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// [`Self::edge_inset`] in logical px at the default metrics, beside the
    /// theme intent that follows an installed theme.
    fn edge_px(&self) -> f32 {
        nana_ui_core::SpacingTokens::DEFAULT.resolve(self.edge_inset)
    }

    fn resolved_center_width(&self) -> f32 {
        finite_positive(self.center_width, DEFAULT_CENTER_WIDTH).max(1.0)
    }

    /// Whether the native placeholder leads the bar.
    fn leads_with_native_controls(&self) -> bool {
        self.native_controls && self.show_window_controls
    }

    /// An unassembled bar has no placeholder, so it keeps the native band
    /// clear with padding instead.
    fn chrome_padding_left(&self) -> f32 {
        if self.leads_with_native_controls() && self.controls.is_none() {
            NATIVE_WINDOW_CONTROLS_WIDTH + NATIVE_LEADING_CLEARANCE
        } else {
            0.0
        }
    }

    fn effective_style(&self, world: &UiWorld, id: StableNodeId) -> NodeStyle {
        let columns = title_bar_has_columns(world, id);
        let mut style = self.style.clone();
        style.foreground = Some(if self.is_over_media() {
            SemanticColorRole::OnMedia
        } else {
            SemanticColorRole::Text
        });
        style.background =
            (self.backdrop == TitleBarBackdrop::Themed).then_some(SemanticColorRole::Titlebar);
        if self.is_over_media() {
            style.painter = Some(crate::NodePainter::new(MediaScrimPainter));
        }
        style.text_horizontal_alignment = TextHorizontalAlignment::Center;
        style.text_vertical_alignment = TextVerticalAlignment::Center;
        let layout = Arc::make_mut(&mut style.layout);
        layout.background = None;
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
        layout.min_height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
        layout.max_height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        crate::theme_intent::font(layout, nana_ui_core::TypeRole::Body);
        crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Semibold);
        if columns {
            layout.padding_left = Some(LengthSpec::Px(0.0));
            layout.padding_right = Some(LengthSpec::Px(0.0));
        } else {
            layout.padding_left = Some(LengthSpec::Px(self.chrome_padding_left()));
            layout.padding_right = Some(LengthSpec::Px(0.0));
        }
        layout.overflow_x = OverflowSpec::Hidden;
        style
    }

    fn project_slots(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let children = world.node(id).map(|node| node.children).unwrap_or_default();
        let center_has_content = children.iter().any(|&child| {
            node_tag(world, child).as_deref() == Some(CENTER_COLUMN_TAG)
                && world.node(child).is_some_and(|slot| {
                    slot.children.iter().any(|&content| {
                        world.node(content).is_some_and(|node| {
                            !matches!(node.kind, NodeKind::Text)
                                || world.text(content).is_some_and(|text| !text.is_empty())
                        })
                    })
                })
        });
        let content_only = self.title.is_empty() && self.center.is_none() && !center_has_content;
        let mut saw_columns = false;
        for child in children {
            match node_tag(world, child).as_deref() {
                Some(LEADING_COLUMN_TAG) => {
                    saw_columns = true;
                    patch_layout(world, mutations, child, |layout| {
                        apply_fill_column(layout, JustifySpec::Start, false);
                        layout.overflow_x = OverflowSpec::Hidden;
                        // The native placeholder sits flush with the bar edge
                        // and carries its own clearance; a capsule stands in
                        // by the edge inset.
                        layout.padding_left = Some(LengthSpec::Px(
                            if self.leads_with_native_controls() && self.controls.is_some() {
                                if self.controls_capsule {
                                    self.edge_px()
                                } else {
                                    0.0
                                }
                            } else {
                                self.edge_px() + self.chrome_padding_left()
                            },
                        ));
                        layout.padding_right = Some(LengthSpec::Px(SLOT_PADDING));
                        layout.theme.padding_right =
                            Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                        layout.padding_top = Some(LengthSpec::Px(0.0));
                        layout.padding_bottom = Some(LengthSpec::Px(0.0));
                    });
                }
                Some(CENTER_COLUMN_TAG) => {
                    saw_columns = true;
                    patch_layout(world, mutations, child, |layout| {
                        apply_center_column(layout, self.resolved_center_width());
                        layout.hidden = content_only;
                    });
                }
                Some(TRAILING_COLUMN_TAG) => {
                    saw_columns = true;
                    patch_layout(world, mutations, child, |layout| {
                        if content_only {
                            apply_hug_slot(layout, AlignSpec::Center, JustifySpec::End);
                            layout.hidden = false;
                        } else {
                            apply_fill_column(layout, JustifySpec::End, true);
                        }
                        layout.padding_left = Some(LengthSpec::Px(SLOT_PADDING));
                        layout.theme.padding_left =
                            Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                        // Custom controls hug the window edge; native or
                        // absent controls, and a capsule, keep the bar's edge
                        // inset.
                        layout.padding_right = Some(LengthSpec::Px(
                            if self.show_window_controls
                                && !self.native_controls
                                && !self.controls_capsule
                            {
                                0.0
                            } else {
                                self.edge_px()
                            },
                        ));
                        layout.padding_top = Some(LengthSpec::Px(0.0));
                        layout.padding_bottom = Some(LengthSpec::Px(0.0));
                    });
                }
                _ => {}
            }
        }
        if !saw_columns {
            // No-slot bars keep the legacy direct-child slots. The patches
            // must only touch nodes that are currently children of this bar:
            // before `assemble` runs, a mounted center (e.g. a `Breadcrumb`
            // component) is still detached, and stamping the column style on
            // it would clobber the component's own layout for good.
            let is_bar_child = |node: StableNodeId| world.parent_id(node) == Some(id);
            if let Some(leading) = self.leading.filter(|&n| is_bar_child(n)) {
                patch_layout(world, mutations, leading, |layout| {
                    apply_hug_slot(layout, AlignSpec::Center, JustifySpec::Start);
                    layout.padding_left = Some(LengthSpec::Px(self.edge_px()));
                    layout.theme.padding_left = Some(crate::theme_intent::space(self.edge_inset));
                    layout.padding_right = Some(LengthSpec::Px(SLOT_PADDING));
                    layout.theme.padding_right =
                        Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                    layout.padding_top = Some(LengthSpec::Px(0.0));
                    layout.padding_bottom = Some(LengthSpec::Px(0.0));
                });
            }
            if let Some(center) = self.center.filter(|&n| is_bar_child(n)) {
                patch_layout(world, mutations, center, |layout| {
                    apply_center_column(layout, self.resolved_center_width());
                });
            }
            if let Some(trailing) = self.trailing.filter(|&n| is_bar_child(n)) {
                patch_layout(world, mutations, trailing, |layout| {
                    apply_hug_slot(layout, AlignSpec::Center, JustifySpec::End);
                    layout.padding_left = Some(LengthSpec::Px(SLOT_PADDING));
                    layout.theme.padding_left =
                        Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Sm));
                    layout.padding_right = Some(LengthSpec::Px(self.edge_px()));
                    layout.theme.padding_right = Some(crate::theme_intent::space(self.edge_inset));
                    layout.padding_top = Some(LengthSpec::Px(0.0));
                    layout.padding_bottom = Some(LengthSpec::Px(0.0));
                });
            }
        }
        self.project_controls(world, mutations);
    }

    fn project_controls(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        let Some(controls) = self.controls else {
            return;
        };
        if !self.show_window_controls {
            patch_layout(world, mutations, controls, |layout| {
                layout.hidden = true;
            });
            return;
        }
        AppTitleBarControls::new(self.maximized)
            .native(self.native_controls)
            .over_media(self.is_over_media())
            .capsule(self.controls_capsule)
            .project(controls, world, mutations);
    }
}

impl ComponentView for AppTitleBar {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::APP_TITLE_BAR.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        hooks: Some(|hooks| {
            crate::framework::routes::title_bar(hooks);
        }),
        slot_assembler: Some(AppContext::assemble_app_title_bar),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Lays out the leading, center and trailing slot nodes the application owns.
    const ALWAYS_REPROJECT: bool = true;

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-title-bar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let root_text = if self.center.is_some() || title_bar_has_columns(world, id) {
            ""
        } else {
            self.title.as_ref()
        };
        if world.text(id) != Some(root_text) {
            mutations.set_text(
                id,
                TextContent {
                    value: root_text.to_owned().into(),
                },
            );
        }
        if world.widget_visual_ref(id).is_some() {
            mutations.set_widget_visual_handle(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(world, id),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&self.title)),
                ..AccessibilityState::default()
            },
        );
        self.project_slots(id, world, mutations);
    }
}

/// [`AppTitleBar::over_media`]'s scrim: the theme's media scrim at the top
/// edge, fading to nothing at the bottom. Painted rather than written into
/// the layout so a theme install re-records it.
struct MediaScrimPainter;

impl crate::Painter for MediaScrimPainter {
    fn paint(&self, cx: &mut crate::PaintContext<'_>) {
        let scrim = cx.effects().media_scrim;
        let clear = nana_ui_core::SemanticColor { a: 0.0, ..scrim };
        let mut rect = crate::PaintPath::new();
        rect.rect(cx.bounds());
        cx.fill_path(
            &rect,
            crate::Gradient::linear([0.0, 0.0], [0.0, cx.size()[1]])
                .stop(0.0, scrim.as_rgba_array())
                .stop(1.0, clear.as_rgba_array()),
        );
        // After the scrim: the bar's own title goes on top of it.
        cx.draw_default();
    }

    fn paint_key(&self) -> u64 {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleBarColumn {
    Leading,
    Center,
    Trailing,
}

impl TitleBarColumn {
    fn tag(self) -> &'static str {
        match self {
            Self::Leading => LEADING_COLUMN_TAG,
            Self::Center => CENTER_COLUMN_TAG,
            Self::Trailing => TRAILING_COLUMN_TAG,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct AppTitleBarSlot {
    column: TitleBarColumn,
}

impl ComponentView for AppTitleBarSlot {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: self.column.tag().into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        if world.widget_visual_ref(id).is_some() {
            mutations.set_widget_visual_handle(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &NodeStyle::default(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// The window's system buttons. Custom controls are host-mounted Minimize /
/// Maximize-or-Restore / Close icons; native controls are an empty placeholder
/// the size of the platform's own buttons, which the window host moves onto
/// the placeholder's layout box.
///
/// A title bar's controls slot is the usual home for the placeholder, and
/// `assemble_app_title_bar` puts it there. It is an ordinary component
/// though: mounted anywhere in a window's document, it still says where that
/// window's native buttons belong.
#[derive(Debug, Clone, PartialEq)]
pub struct AppTitleBarControls {
    pub maximized: bool,
    pub native: bool,
    /// The custom buttons sit over media: see [`AppTitleBar::over_media`].
    pub over_media: bool,
    /// The buttons sit on a capsule: see [`AppTitleBar::controls_capsule`].
    pub capsule: bool,
    pub minimize: Option<StableNodeId>,
    pub maximize: Option<StableNodeId>,
    pub close: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl AppTitleBarControls {
    pub fn new(maximized: bool) -> Self {
        Self {
            maximized,
            native: !WindowChrome::platform_default().uses_custom_controls(),
            over_media: false,
            capsule: false,
            minimize: None,
            maximize: None,
            close: None,
            style: NodeStyle::default(),
        }
    }

    pub fn native(mut self, native: bool) -> Self {
        self.native = native;
        self
    }

    pub fn over_media(mut self, over_media: bool) -> Self {
        self.over_media = over_media;
        self
    }

    pub fn capsule(mut self, capsule: bool) -> Self {
        self.capsule = capsule;
        self
    }

    pub fn minimize(mut self, minimize: StableNodeId) -> Self {
        self.minimize = Some(minimize);
        self
    }

    pub fn maximize(mut self, maximize: StableNodeId) -> Self {
        self.maximize = Some(maximize);
        self
    }

    pub fn close(mut self, close: StableNodeId) -> Self {
        self.close = Some(close);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        if self.capsule {
            style.background = Some(SemanticColorRole::Surface);
            style.border = Some(SemanticColorRole::BorderSoft);
            style.radius = Some(nana_ui_core::RadiusTier::Full);
        }
        let layout = Arc::make_mut(&mut style.layout);
        if self.capsule {
            layout.border_width = Some(nana_ui_core::HAIRLINE);
            layout.theme.border_width =
                Some(nana_ui_core::ThemeToken::Border(nana_ui_core::BorderWidth::Hairline).into());
        }
        if self.native {
            let width = NATIVE_WINDOW_CONTROLS_WIDTH;
            layout.width = Some(LengthSpec::Px(width));
            layout.min_width = Some(LengthSpec::Px(width));
            layout.max_width = Some(LengthSpec::Px(width));
            layout.height = Some(if self.capsule {
                LengthSpec::Px(CONTROLS_CAPSULE_HEIGHT)
            } else {
                LengthSpec::Fill
            });
            layout.padding_left = Some(LengthSpec::Px(0.0));
            layout.padding_right = Some(LengthSpec::Px(0.0));
            layout.margin_right = Some(LengthSpec::Px(SLOT_PADDING + NATIVE_LEADING_CLEARANCE));
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
            layout.hidden = false;
            return style;
        }
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::End;
        layout.gap = Some(layout.gap.unwrap_or(LengthSpec::Px(WINDOW_CONTROL_GAP)));
        if self.capsule {
            layout.padding_left = Some(LengthSpec::Px(CONTROLS_CAPSULE_INSET_X));
            layout.padding_right = Some(LengthSpec::Px(CONTROLS_CAPSULE_INSET_X));
            layout.padding_top = Some(LengthSpec::Px(CONTROLS_CAPSULE_INSET_Y));
            layout.padding_bottom = Some(LengthSpec::Px(CONTROLS_CAPSULE_INSET_Y));
            layout.margin_left = Some(LengthSpec::Px(nana_ui_core::space::XS));
            layout.height = Some(LengthSpec::Px(CONTROLS_CAPSULE_HEIGHT));
            layout.flex_grow = Some(0.0);
            layout.flex_shrink = Some(0.0);
            layout.hidden = false;
            return style;
        }
        layout.padding_left = Some(LengthSpec::Px(WINDOW_CONTROL_PADDING));
        layout.padding_right = Some(LengthSpec::Px(WINDOW_CONTROL_PADDING));
        layout.height = Some(LengthSpec::Fill);
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.hidden = false;
        style
    }

    fn control_ids(&self, world: &UiWorld, id: StableNodeId) -> [Option<StableNodeId>; 3] {
        let explicit = [self.minimize, self.maximize, self.close];
        if explicit.iter().any(Option::is_some) {
            return explicit;
        }
        let children = world.node(id).map(|node| node.children).unwrap_or_default();
        [
            children.first().copied(),
            children.get(1).copied(),
            children.get(2).copied(),
        ]
    }
}

impl Default for AppTitleBarControls {
    fn default() -> Self {
        Self::new(false)
    }
}

impl ComponentView for AppTitleBarControls {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-title-bar-controls".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        if world.widget_visual_ref(id).is_some() {
            mutations.set_widget_visual_handle(id, None);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            // The native placeholder takes the pointer so the title bar never
            // starts a drag over the buttons it stands for.
            InteractionState {
                pointer_events: self.native,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
        if self.native {
            return;
        }
        for (action, child) in WindowChromeAction::ALL
            .into_iter()
            .zip(self.control_ids(world, id))
        {
            let Some(child) = child else {
                continue;
            };
            project_window_control(
                child,
                action,
                self.maximized,
                // On its plate a button reads like any other on a surface.
                self.over_media && !self.capsule,
                self.capsule,
                world,
                mutations,
            );
        }
    }
}

impl AppContext {
    /// Restore the three-column title bar and mount window controls.
    ///
    /// Leading and trailing columns fill leftover width; the center column is a
    /// fixed title slot. Custom Minimize / Maximize / Close buttons live in the
    /// trailing column; the native window-button placeholder leads the leading
    /// column. Only declared slots are mounted; unrelated children are never
    /// reclassified or reparented across components.
    pub fn assemble_app_title_bar(
        &mut self,
        bar: Entity<AppTitleBar>,
    ) -> Result<bool, FrameworkError> {
        let parent = bar.stable_id();
        let document = document_of(self, parent)?;
        let snapshot = self.read(bar, Clone::clone)?;
        let leading_slot = ensure_title_bar_slot(self, document, parent, TitleBarColumn::Leading)?;
        let center_slot = ensure_title_bar_slot(self, document, parent, TitleBarColumn::Center)?;
        let trailing_slot =
            ensure_title_bar_slot(self, document, parent, TitleBarColumn::Trailing)?;

        let mut controls = snapshot
            .controls
            .filter(|id| self.world().contains(*id))
            .or_else(|| find_title_bar_controls_child(self, parent));
        let mut changed = false;
        // A native placeholder is mounted even while hidden, so showing the
        // controls later still has a box for the platform buttons to follow.
        if snapshot.show_window_controls || snapshot.native_controls {
            let mounted = ensure_window_controls(
                self,
                document,
                parent,
                controls,
                snapshot.maximized,
                snapshot.native_controls,
                snapshot.is_over_media(),
            )?;
            changed |= controls != Some(mounted);
            controls = Some(mounted);
            if changed {
                self.update_component(bar, |bar, _| {
                    bar.controls = Some(mounted);
                })?;
            }
        }

        let controls = controls.filter(|id| self.world().contains(*id));
        let mut leading_children = Vec::new();
        if let Some(controls) = controls.filter(|_| snapshot.native_controls) {
            leading_children.push(controls);
        }
        if let Some(leading) = snapshot.leading.filter(|id| self.world().contains(*id)) {
            leading_children.push(leading);
        }
        let mut center_children = Vec::new();
        if let Some(center) = snapshot.center.filter(|id| self.world().contains(*id)) {
            center_children.push(center);
        }
        let mut trailing_children = Vec::new();
        if let Some(trailing) = snapshot.trailing.filter(|id| self.world().contains(*id)) {
            trailing_children.push(trailing);
        }
        if let Some(controls) = controls.filter(|_| !snapshot.native_controls) {
            trailing_children.push(controls);
        }
        if snapshot.center.is_none() && center_children.is_empty() {
            center_children.push(ensure_title_label(
                self,
                document,
                center_slot,
                snapshot.title.as_ref(),
            )?);
        }

        changed |= reconcile_ids(self, leading_slot, &leading_children)?;
        changed |= reconcile_ids(self, center_slot, &center_children)?;
        changed |= reconcile_ids(self, trailing_slot, &trailing_children)?;
        changed |= reconcile_ids(self, parent, &[leading_slot, center_slot, trailing_slot])?;
        self.reproject_component(bar)?;
        Ok(changed)
    }
}

fn find_title_bar_controls_child(
    context: &AppContext,
    parent: StableNodeId,
) -> Option<StableNodeId> {
    let mut stack = context
        .world()
        .node(parent)
        .map(|node| node.children)
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        if view_is::<AppTitleBarControls>(context, id)
            || matches!(
                context.world().node(id).map(|node| node.kind),
                Some(NodeKind::Element { tag }) if tag == "app-title-bar-controls"
            )
        {
            return Some(id);
        }
        if let Some(children) = context.world().node(id).map(|node| node.children) {
            stack.extend(children);
        }
    }
    None
}

fn title_bar_has_columns(world: &UiWorld, id: StableNodeId) -> bool {
    world.node(id).is_some_and(|node| {
        node.children
            .iter()
            .any(|child| is_title_bar_column_tag(node_tag(world, *child).as_deref()))
    })
}

pub(super) fn is_title_bar_column_tag(tag: Option<&str>) -> bool {
    matches!(
        tag,
        Some(LEADING_COLUMN_TAG | CENTER_COLUMN_TAG | TRAILING_COLUMN_TAG)
    )
}

pub(super) fn node_tag(world: &UiWorld, id: StableNodeId) -> Option<String> {
    match world.node(id).map(|node| node.kind) {
        Some(NodeKind::Element { tag }) => Some(tag),
        _ => None,
    }
}

fn ensure_title_bar_slot(
    context: &mut AppContext,
    document: DocumentId,
    parent: StableNodeId,
    column: TitleBarColumn,
) -> Result<StableNodeId, FrameworkError> {
    let tag = column.tag();
    let existing = context
        .world()
        .node(parent)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| node_tag(context.world(), id).as_deref() == Some(tag))
        .or_else(|| {
            context
                .world()
                .node(parent)
                .into_iter()
                .flat_map(|node| node.children)
                .flat_map(|id| {
                    context
                        .world()
                        .node(id)
                        .map(|node| node.children)
                        .unwrap_or_default()
                })
                .find(|&id| node_tag(context.world(), id).as_deref() == Some(tag))
        });
    if let Some(id) = existing.filter(|id| context.world().contains(*id)) {
        return Ok(id);
    }
    Ok(context
        .create_detached_component(document, AppTitleBarSlot { column })?
        .stable_id())
}

fn ensure_title_label(
    context: &mut AppContext,
    document: DocumentId,
    center_slot: StableNodeId,
    title: &str,
) -> Result<StableNodeId, FrameworkError> {
    if let Some(existing) = context
        .world()
        .node(center_slot)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| view_is::<Text>(context, id) || is_title_label_node(context, id))
    {
        if view_is::<Text>(context, existing) {
            context.update_component(Entity::<Text>::from_stable_id(existing), |text, _| {
                if text.value != title {
                    text.value = title.to_owned();
                }
            })?;
        } else if context.world().text(existing) != Some(title) {
            let mut mutations = MutationQueue::new();
            mutations.set_text(
                existing,
                TextContent {
                    value: title.to_owned().into(),
                },
            );
            context.commit_mutations(mutations)?;
        }
        return Ok(existing);
    }
    Ok(context
        .create_detached_component(document, Text::new(title).style(title_label_style()))?
        .stable_id())
}

/// No foreground of its own: the title takes the bar's, `Text` or, over
/// media, `OnMedia`.
fn title_label_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.text_horizontal_alignment = TextHorizontalAlignment::Center;
    style.text_vertical_alignment = TextVerticalAlignment::Center;
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    crate::theme_intent::font(layout, nana_ui_core::TypeRole::Body);
    crate::theme_intent::weight(layout, nana_ui_core::TextWeight::Semibold);
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = true;
    style
}

fn is_title_label_node(context: &AppContext, id: StableNodeId) -> bool {
    view_is::<Text>(context, id)
        || matches!(
            context.world().node(id).map(|node| node.kind),
            Some(NodeKind::Text)
        )
}

fn window_control_buttons(
    context: &mut AppContext,
    document: DocumentId,
    maximized: bool,
) -> Result<[StableNodeId; 3], FrameworkError> {
    let mut button = |action: WindowChromeAction| {
        let label = action.label(context.world().framework_strings(), maximized);
        context
            .create_detached_component(
                document,
                IconButton::new(action.icon(maximized), label).size(ControlSize::Small),
            )
            .map(|entity| entity.stable_id())
    };
    Ok([
        button(WindowChromeAction::Minimize)?,
        button(WindowChromeAction::ToggleMaximize)?,
        button(WindowChromeAction::Close)?,
    ])
}

fn ensure_window_controls(
    context: &mut AppContext,
    document: DocumentId,
    parent: StableNodeId,
    existing: Option<StableNodeId>,
    maximized: bool,
    native: bool,
    over_media: bool,
) -> Result<StableNodeId, FrameworkError> {
    let controls = existing
        .filter(|id| context.world().contains(*id))
        .or_else(|| find_title_bar_controls_child(context, parent));
    if native {
        // The platform draws the buttons; the placeholder has no children.
        if let Some(controls) = controls {
            if view_is::<AppTitleBarControls>(context, controls) {
                context.update_component(
                    Entity::<AppTitleBarControls>::from_stable_id(controls),
                    |controls, _| {
                        controls.maximized = maximized;
                        controls.native = true;
                    },
                )?;
            }
            // Custom buttons left from an earlier mode have no place here.
            if context
                .world()
                .node(controls)
                .is_some_and(|node| !node.children.is_empty())
            {
                reconcile_ids(context, controls, &[])?;
            }
            return Ok(controls);
        }
        let controls = context.create_detached_component(
            document,
            AppTitleBarControls::new(maximized).native(true),
        )?;
        return Ok(controls.stable_id());
    }
    if let Some(controls) = controls {
        let count = context
            .world()
            .node(controls)
            .map(|node| node.children.len())
            .unwrap_or(0);
        if count < 3 {
            let [minimize, maximize, close] = window_control_buttons(context, document, maximized)?;
            if view_is::<AppTitleBarControls>(context, controls) {
                context.update_component(
                    Entity::<AppTitleBarControls>::from_stable_id(controls),
                    |controls, _| {
                        controls.maximized = maximized;
                        controls.native = false;
                        controls.over_media = over_media;
                        controls.minimize = Some(minimize);
                        controls.maximize = Some(maximize);
                        controls.close = Some(close);
                    },
                )?;
            }
            reconcile_ids(context, controls, &[minimize, maximize, close])?;
        } else if view_is::<AppTitleBarControls>(context, controls) {
            context.update_component(
                Entity::<AppTitleBarControls>::from_stable_id(controls),
                |controls, _| {
                    controls.maximized = maximized;
                    controls.native = false;
                    controls.over_media = over_media;
                },
            )?;
            // The buttons keep the words they were made with; say the
            // framework's current ones (a consumer may have replaced them).
            let children = context
                .world()
                .node(controls)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            for (action, child) in WindowChromeAction::ALL.into_iter().zip(children.iter()) {
                if !view_is::<IconButton>(context, *child) {
                    continue;
                }
                let label = action.label(context.world().framework_strings(), maximized);
                let button = Entity::<IconButton>::from_stable_id(*child);
                if context.read(button, |button| button.label != label)? {
                    context.update_component(button, |button, _| button.label = label)?;
                }
            }
        }
        return Ok(controls);
    }
    let [minimize, maximize, close] = window_control_buttons(context, document, maximized)?;
    let controls = context.create_detached_component(
        document,
        AppTitleBarControls::new(maximized)
            .native(false)
            .over_media(over_media)
            .minimize(minimize)
            .maximize(maximize)
            .close(close),
    )?;
    reconcile_ids(context, controls.stable_id(), &[minimize, maximize, close])?;
    Ok(controls.stable_id())
}

fn project_window_control(
    id: StableNodeId,
    action: WindowChromeAction,
    maximized: bool,
    over_media: bool,
    round: bool,
    world: &UiWorld,
    mutations: &mut MutationQueue,
) {
    if world.node(id).is_none() {
        return;
    }
    if world.text(id) != Some("") {
        mutations.set_text(
            id,
            TextContent {
                value: String::new().into(),
            },
        );
    }
    let visual = crate::IconVisual {
        icon: action.icon(maximized),
        size: ControlSize::Small.icon_size(),
        tooltip: None,
    };
    if world.widget_visual_as::<crate::IconVisual>(id) != Some(&visual) {
        mutations.set_widget_visual(id, Some(visual));
    }
    project_common(
        id,
        world,
        mutations,
        &{
            let mut style = window_control_style(
                action == WindowChromeAction::Close,
                over_media,
                world.node_theme(id).effects(),
            );
            // Round in a capsule, so a hover fill follows its ends.
            if round {
                style.radius = Some(nana_ui_core::RadiusTier::Full);
            }
            style
        },
        InteractionState {
            pointer_events: true,
            focusable: true,
        },
        AccessibilityState {
            role: AccessibilityRole::Button,
            label: Some(action.label(world.framework_strings(), maximized)),
            ..AccessibilityState::default()
        },
    );
}

/// Over media, a window button's hover and press are the on-media foreground
/// at the theme's media state alphas; close keeps its danger fill and
/// darkens it when pressed.
fn window_control_style(
    danger: bool,
    over_media: bool,
    effects: nana_ui_core::EffectTokens,
) -> NodeStyle {
    let mut style = IconButton::new(Icon::Close, "")
        .size(ControlSize::Small)
        .style;
    if over_media {
        let on_media = |background_mix| crate::SemanticPaint {
            foreground: Some(SemanticColorRole::OnMedia),
            background_mix: Some(background_mix),
            ..crate::SemanticPaint::default()
        };
        style.foreground = Some(SemanticColorRole::OnMedia);
        let (hovered, pressed) = if danger {
            (
                SemanticColorMix::alpha(SemanticColorRole::Danger, 1.0),
                SemanticColorMix::alpha(
                    SemanticColorRole::Danger,
                    effects.media_danger_pressed_alpha,
                ),
            )
        } else {
            (
                SemanticColorMix::alpha(SemanticColorRole::OnMedia, effects.media_hover_alpha),
                SemanticColorMix::alpha(SemanticColorRole::OnMedia, effects.media_pressed_alpha),
            )
        };
        style.interaction.hovered = on_media(hovered);
        style.interaction.pressed = on_media(pressed);
    } else if danger {
        style.interaction.hovered.foreground = Some(SemanticColorRole::Danger);
        style.interaction.hovered.background = Some(SemanticColorRole::DangerSoftHover);
        style.interaction.pressed.foreground = Some(SemanticColorRole::Danger);
        style.interaction.pressed.background = Some(SemanticColorRole::DangerSoftPressed);
    }
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Px(WINDOW_CONTROL_WIDTH));
    layout.height = Some(LengthSpec::Px(WINDOW_CONTROL_WIDTH));
    layout.min_width = Some(LengthSpec::Px(WINDOW_CONTROL_WIDTH));
    layout.min_height = Some(LengthSpec::Px(WINDOW_CONTROL_WIDTH));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.padding_left = Some(LengthSpec::Px(0.0));
    layout.padding_right = Some(LengthSpec::Px(0.0));
    layout.padding_top = Some(LengthSpec::Px(0.0));
    layout.padding_bottom = Some(LengthSpec::Px(0.0));
    layout.border_width = Some(0.0);
    style
}

fn apply_hug_slot(layout: &mut nana_ui_core::LayoutStyle, align: AlignSpec, justify: JustifySpec) {
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = align;
    layout.justify_content = justify;
    layout.width = Some(LengthSpec::Shrink);
    layout.height = Some(LengthSpec::Fill);
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
}

/// A side column shares the space the centre leaves, evenly so the centre
/// stays centred. The trailing one, which holds the window controls, never
/// goes below what its content measures: the centre gives way first
/// ([`apply_center_column`]), so the controls are never cut. The leading one
/// may be squeezed.
fn apply_fill_column(
    layout: &mut nana_ui_core::LayoutStyle,
    justify: JustifySpec,
    keeps_content: bool,
) {
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = AlignSpec::Center;
    layout.justify_content = justify;
    if keeps_content {
        // Sized by its content (what the minimum keeps), then grown into the
        // space the centre leaves.
        layout.width = None;
        layout.min_width = Some(LengthSpec::MaxContent);
    } else {
        layout.width = Some(LengthSpec::Fill);
        layout.min_width = Some(LengthSpec::Px(0.0));
    }
    layout.height = Some(LengthSpec::Fill);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.hidden = false;
}

fn apply_center_column(layout: &mut nana_ui_core::LayoutStyle, width: f32) {
    layout.direction = Some(FlexDirection::Row);
    layout.align_items = AlignSpec::Center;
    layout.justify_content = JustifySpec::Center;
    layout.width = Some(LengthSpec::Px(width));
    layout.max_width = Some(LengthSpec::Px(width));
    layout.height = Some(LengthSpec::Fill);
    layout.flex_grow = Some(0.0);
    // Its width is what it asks for when there is room; in a narrow window
    // it shrinks, and its content ellipsizes, before the side columns do.
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.padding_left = Some(LengthSpec::Px(CENTER_PADDING_X));
    layout.theme.padding_left = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxl));
    layout.padding_right = Some(LengthSpec::Px(CENTER_PADDING_X));
    layout.theme.padding_right = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxl));
    layout.padding_top = Some(LengthSpec::Px(0.0));
    layout.padding_bottom = Some(LengthSpec::Px(0.0));
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    layout.white_space = nana_ui_core::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = true;
    layout.hidden = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::{AppContext, DocumentId, Entity, IconButton, LayoutViewport, Text};
    use nana_ui_core::PositionSpec;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn icon_of(visual: Option<&crate::IconVisual>) -> Option<Icon> {
        visual.map(|visual| visual.icon)
    }

    #[test]
    fn empty_title_bar_keeps_implicitly_mounted_center_text() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                Text::new("Mounted title").style(title_label_style()),
            )
            .unwrap();
        let bar = context
            .create_component(document(), AppTitleBar::new("").center(text.stable_id()))
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(640.0, 400.0))
            .unwrap();
        let center = context
            .world()
            .node(text.stable_id())
            .unwrap()
            .parent
            .unwrap();
        assert!(!context.world().node_style(center).unwrap().layout.hidden);
        let bounds = context
            .world()
            .canonical_layout_box(text.stable_id())
            .unwrap();
        assert!(bounds.width > 0.0 && bounds.height > 0.0);
        assert_eq!(
            context.world().text(text.stable_id()),
            Some("Mounted title")
        );
        assert_eq!(
            context.world().canonical_layout_box(center).unwrap().width,
            DEFAULT_CENTER_WIDTH
        );
    }

    #[test]
    fn empty_title_bar_reserves_trailing_content_at_narrow_width() {
        let mut context = AppContext::new();
        let leading = context
            .create_component(
                document(),
                Text::new("A long stage name").style(NodeStyle {
                    layout: Arc::new(nana_ui_core::LayoutStyle {
                        width: Some(LengthSpec::Fill),
                        min_width: Some(LengthSpec::Px(0.0)),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        let trailing = context
            .create_component(
                document(),
                Text::new("Status").style(NodeStyle {
                    layout: Arc::new(nana_ui_core::LayoutStyle {
                        width: Some(LengthSpec::Px(280.0)),
                        flex_shrink: Some(0.0),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("")
                    .leading(leading.stable_id())
                    .trailing(trailing.stable_id())
                    .show_window_controls(true)
                    .native_controls(false),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(640.0, 400.0))
            .unwrap();
        let controls = context.read(bar, |bar| bar.controls.unwrap()).unwrap();
        let a = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        let b = context
            .world()
            .canonical_layout_box(trailing.stable_id())
            .unwrap();
        let c = context.world().canonical_layout_box(controls).unwrap();
        assert!((b.width - 280.0).abs() < 0.1);
        assert!(a.x + a.width <= b.x + 0.1);
        assert!(b.x + b.width <= c.x + 0.1);
        assert!((c.x + c.width - 640.0).abs() < 0.1);
        assert!(a.width > 0.0);
    }

    #[test]
    fn transparent_title_bar_preserves_slot_content_and_geometry() {
        let mut context = AppContext::new();
        let text = context
            .create_component(document(), Text::new("Status"))
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").trailing(text.stable_id()),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let before = context
            .world()
            .canonical_layout_box(text.stable_id())
            .unwrap();
        let parent = context.world().node(text.stable_id()).unwrap().parent;
        for transparent in [true, false] {
            context
                .update_component(bar, |bar, _| *bar = bar.clone().transparent(transparent))
                .unwrap();
            context
                .layout_document(document(), LayoutViewport::new(800.0, 400.0))
                .unwrap();
            let style = context.world().node_style(bar.stable_id()).unwrap();
            assert_eq!(
                style.background,
                (!transparent).then_some(SemanticColorRole::Titlebar)
            );
            assert!(style.layout.background.is_none());
            assert_eq!(style.foreground, Some(SemanticColorRole::Text));
            assert_eq!(
                context
                    .world()
                    .canonical_layout_box(text.stable_id())
                    .unwrap(),
                before
            );
            assert_eq!(
                context.world().node(text.stable_id()).unwrap().parent,
                parent
            );
            assert_eq!(context.world().text(text.stable_id()), Some("Status"));
        }
    }

    /// Over media the light theme looks like the dark one: the media scrim
    /// fades behind the bar, and the title and window buttons are light at
    /// rest, hovered and pressed. Turning it off gives the themed bar back.
    #[test]
    fn a_title_bar_over_media_is_light_on_the_media_scrim_in_the_light_theme() {
        let mut context = AppContext::new();
        let palette = nana_ui_core::SemanticPalette::light();
        context
            .set_style_tokens(
                nana_ui_core::ThemeAppearance::Light,
                nana_ui_core::UI_METRICS,
                palette,
                palette.surface,
            )
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .native_controls(false)
                    .over_media(true),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let effects = context.world().theme().effects();
        let light = effects.media_foreground.as_rgba_array();
        assert!(light[..3].iter().all(|channel| *channel > 0.9), "{light:?}");
        let center = context
            .world()
            .node(bar.stable_id())
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|&id| node_tag(context.world(), id).as_deref() == Some(CENTER_COLUMN_TAG))
            .unwrap();
        let title = context.world().node(center).unwrap().children[0];
        assert_eq!(context.world().text(title), Some("Nana"));
        let controls = find_title_bar_controls_child(&context, bar.stable_id()).unwrap();
        let buttons = context.world().node(controls).unwrap().children.clone();
        assert_eq!(buttons.len(), 3);
        let color =
            |context: &AppContext, id| context.world().extract_nodes(&[id])[0].style.color.unwrap();

        let node = &context.world().extract_nodes(&[bar.stable_id()])[0];
        assert_eq!(node.style.background, None, "no themed bar fill");
        let scrim = node.custom_paint.as_ref().expect("the scrim is painted");
        let [
            crate::PaintOp::FillPath {
                paint: crate::ResolvedPaint::Gradient(gradient),
                ..
            },
            crate::PaintOp::DrawDefault,
        ] = scrim.behind_children.as_slice()
        else {
            panic!("a gradient under the bar's own paint: {scrim:?}");
        };
        assert_eq!(
            gradient.shape,
            crate::GradientShape::Linear {
                start: [0.0, 0.0],
                end: [0.0, TITLE_BAR_HEIGHT],
            },
            "top to bottom"
        );
        assert_eq!(
            gradient.stops.first(),
            Some(&(0.0, effects.media_scrim.as_rgba_array()))
        );
        assert_eq!(
            gradient
                .stops
                .last()
                .map(|(offset, color)| (*offset, color[3])),
            Some((1.0, 0.0)),
            "fades to nothing"
        );
        assert_eq!(color(&context, title), light);

        let model = context.world().style_model();
        for (index, &button) in buttons.iter().enumerate() {
            assert_eq!(color(&context, button), light, "button {index} at rest");
            let style = context.world().node_style(button).unwrap();
            for state in [style.interaction.hovered, style.interaction.pressed] {
                assert_eq!(state.foreground, Some(SemanticColorRole::OnMedia));
                let fill = state.background_mix.unwrap().resolve(model);
                if index == 2 {
                    assert!(fill.r > 0.6 && fill.g < 0.4, "close stays red: {fill:?}");
                } else {
                    assert!(
                        fill.r.min(fill.g).min(fill.b) > 0.9 && fill.a > 0.0 && fill.a < 0.5,
                        "a light wash: {fill:?}"
                    );
                }
            }
        }

        context
            .update_component(bar, |bar, _| *bar = bar.clone().over_media(false))
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let node = &context.world().extract_nodes(&[bar.stable_id()])[0];
        assert!(node.custom_paint.is_none());
        assert_eq!(node.style.background, Some(palette.surface.as_rgba_array()));
        assert_eq!(color(&context, title), palette.text.as_rgba_array());
        for &button in &buttons {
            assert_eq!(color(&context, button), palette.muted.as_rgba_array());
        }
    }

    #[test]
    fn title_bar_is_36px_and_projects_title_without_center() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let id = bar.stable_id();
        assert_eq!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element {
                tag: "app-title-bar".into(),
            }
        );
        assert_eq!(context.world().text(id), Some("Nana"));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(
            style.layout.font_size,
            Some(nana_ui_core::TypographyTokens::DEFAULT.body)
        );
        assert_eq!(
            style.layout.font_weight,
            Some(nana_ui_core::TypographyTokens::DEFAULT.semibold)
        );
        assert_eq!(style.foreground, Some(SemanticColorRole::Text));
        assert_eq!(style.background, Some(SemanticColorRole::Titlebar));
        assert!(style.layout.background.is_none());
        assert_eq!(style.layout.direction, Some(FlexDirection::Row));
        let chrome = WindowChrome::platform_default();
        // Unassembled, a native bar has no placeholder and pads the band clear.
        let expected_leading_pad = if chrome.uses_custom_controls() {
            0.0
        } else {
            NATIVE_WINDOW_CONTROLS_WIDTH + NATIVE_LEADING_CLEARANCE
        };
        assert_eq!(
            style.layout.padding_left,
            Some(LengthSpec::Px(expected_leading_pad))
        );

        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let bounds = context.world().canonical_layout_box(id).unwrap();
        assert_eq!(bounds.height, TITLE_BAR_HEIGHT);
        assert_eq!(bounds.width, 800.0);
        let bar_view = context.read(bar, |bar| bar.clone()).unwrap();
        assert!(bar_view.show_window_controls);
        assert_eq!(bar_view.native_controls, !chrome.uses_custom_controls());
    }

    #[test]
    fn assemble_title_bar_mounts_custom_window_controls_when_enabled() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("Nana").native_controls(false))
            .unwrap();
        let mounted = context.assemble_app_title_bar(bar).unwrap();
        let snapshot = context.read(bar, Clone::clone).unwrap();
        {
            assert!(mounted);
            let controls = snapshot.controls.expect("custom chrome mounts controls");
            assert_eq!(
                context.world().node(controls).unwrap().children.len(),
                3,
                "custom title bar must own minimize, maximize, and close"
            );
            let columns = context.world().node(bar.stable_id()).unwrap().children;
            assert_eq!(columns.len(), 3, "title bar is leading | center | trailing");
            assert_eq!(
                node_tag(context.world(), columns[0]).as_deref(),
                Some(LEADING_COLUMN_TAG)
            );
            assert_eq!(
                node_tag(context.world(), columns[1]).as_deref(),
                Some(CENTER_COLUMN_TAG)
            );
            assert_eq!(
                node_tag(context.world(), columns[2]).as_deref(),
                Some(TRAILING_COLUMN_TAG)
            );
            assert_eq!(
                context
                    .world()
                    .node(columns[2])
                    .unwrap()
                    .children
                    .last()
                    .copied(),
                Some(controls)
            );
            assert!(
                context
                    .world()
                    .node(controls)
                    .unwrap()
                    .children
                    .iter()
                    .all(|id| {
                        context
                            .world()
                            .accessibility(*id)
                            .is_some_and(|state| state.role == AccessibilityRole::Button)
                    })
            );
            assert!(!context.assemble_app_title_bar(bar).unwrap());
        }
    }

    #[test]
    fn caption_buttons_project_chinese_accessible_names() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("Nana").native_controls(false))
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        let controls = context.read(bar, |bar| bar.controls).unwrap().unwrap();
        let buttons = context.world().node(controls).unwrap().children.clone();
        assert_eq!(buttons.len(), 3);

        let spoken = |context: &AppContext, id| {
            context
                .world()
                .accessibility(id)
                .and_then(|state| state.label.as_deref().map(str::to_owned))
        };
        assert_eq!(spoken(&context, buttons[0]).as_deref(), Some("最小化"));
        assert_eq!(spoken(&context, buttons[1]).as_deref(), Some("最大化"));
        assert_eq!(spoken(&context, buttons[2]).as_deref(), Some("关闭"));
        for id in &buttons {
            assert_eq!(context.world().text(*id).unwrap_or(""), "");
        }

        context
            .update_component(bar, |bar, _| {
                bar.maximized = true;
            })
            .unwrap();
        assert_eq!(spoken(&context, buttons[0]).as_deref(), Some("最小化"));
        assert_eq!(spoken(&context, buttons[1]).as_deref(), Some("还原"));
        assert_eq!(spoken(&context, buttons[2]).as_deref(), Some("关闭"));
        for id in &buttons {
            assert_eq!(context.world().text(*id).unwrap_or(""), "");
        }
    }

    /// A consumer's strings reach caption buttons already built: replacing
    /// the table re-says them, in either window state.
    #[test]
    fn caption_buttons_say_the_strings_a_consumer_installs() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("Nana").native_controls(false))
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        let controls = context.read(bar, |bar| bar.controls).unwrap().unwrap();
        let buttons = context.world().node(controls).unwrap().children.clone();
        let mut english = nana_ui_core::FrameworkStrings::default();
        for (key, value) in [
            ("window.minimize", "Minimize"),
            ("window.maximize", "Maximize"),
            ("window.restore", "Restore"),
            ("window.close", "Close"),
        ] {
            assert!(english.set(key, value));
        }
        context.set_framework_strings(english).unwrap();
        let spoken = |context: &AppContext, id| {
            context
                .world()
                .accessibility(id)
                .and_then(|state| state.label.as_deref().map(str::to_owned))
        };
        assert_eq!(spoken(&context, buttons[0]).as_deref(), Some("Minimize"));
        assert_eq!(spoken(&context, buttons[1]).as_deref(), Some("Maximize"));
        assert_eq!(spoken(&context, buttons[2]).as_deref(), Some("Close"));
        context
            .update_component(bar, |bar, _| bar.maximized = true)
            .unwrap();
        assert_eq!(spoken(&context, buttons[1]).as_deref(), Some("Restore"));
    }

    fn native_title_bar(
        context: &mut AppContext,
        style: NodeStyle,
    ) -> (Entity<AppTitleBar>, Entity<IconButton>) {
        let leading = context
            .create_component(
                document(),
                IconButton::new(Icon::Sidebar, "sidebar").size(ControlSize::Small),
            )
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("")
                    .native_controls(true)
                    .leading(leading.stable_id())
                    .style(style),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        (bar, leading)
    }

    #[test]
    fn explicit_custom_controls_are_never_treated_as_native() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("Nana").native_controls(false))
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        let controls = context.read(bar, |bar| bar.controls).unwrap().unwrap();
        let stored = context
            .read(
                Entity::<AppTitleBarControls>::from_stable_id(controls),
                |controls| controls.native,
            )
            .unwrap();
        assert!(!stored);
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        assert_eq!(context.world().node(controls).unwrap().children.len(), 3);
    }

    #[test]
    fn native_window_controls_are_an_empty_leading_placeholder() {
        let mut context = AppContext::new();
        let (bar, leading) = native_title_bar(&mut context, NodeStyle::default());
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let controls = context
            .read(bar, |bar| bar.controls)
            .unwrap()
            .expect("native chrome mounts a placeholder");
        assert!(context.world().node(controls).unwrap().children.is_empty());
        let columns = context.world().node(bar.stable_id()).unwrap().children;
        assert_eq!(
            context.world().node(columns[0]).unwrap().children.first(),
            Some(&controls)
        );
        let placeholder = context.world().canonical_layout_box(controls).unwrap();
        assert_eq!(placeholder.x, 0.0);
        assert_eq!(placeholder.width, NATIVE_WINDOW_CONTROLS_WIDTH);
        assert_eq!(placeholder.height, TITLE_BAR_HEIGHT);
        let leading = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        assert!(
            leading.x + 0.5 >= placeholder.x + placeholder.width + NATIVE_LEADING_CLEARANCE,
            "leading chrome overlapped the native buttons"
        );
        assert!(!context.assemble_app_title_bar(bar).unwrap());
    }

    #[test]
    fn native_window_controls_follow_an_inset_title_bar() {
        let mut context = AppContext::new();
        let root = context
            .create_component(document(), crate::Stack::fill_column(0.0))
            .unwrap();
        let mut style = NodeStyle::default();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.position = PositionSpec::Absolute;
            layout.offset_top = Some(LengthSpec::Px(12.0));
            layout.offset_left = Some(LengthSpec::Px(12.0));
            layout.offset_right = Some(LengthSpec::Px(12.0));
        }
        let (bar, leading) = native_title_bar(&mut context, style);
        context.append_child(root, bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let controls = context.read(bar, |bar| bar.controls).unwrap().unwrap();
        let placeholder = context.world().canonical_layout_box(controls).unwrap();
        assert_eq!((placeholder.x, placeholder.y), (12.0, 12.0));
        let leading = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        assert!(leading.x + 0.5 >= placeholder.x + placeholder.width);
    }

    #[test]
    fn hidden_native_controls_release_the_leading_band() {
        let mut context = AppContext::new();
        let (bar, leading) = native_title_bar(&mut context, NodeStyle::default());
        context
            .update_component(bar, |bar, _| bar.show_window_controls = false)
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let leading = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        assert!(leading.x < NATIVE_WINDOW_CONTROLS_WIDTH);
    }

    /// A bar heading a card stands its slots on the card's content line:
    /// the edge inset replaces the slots' `Sm` at the outer edges only.
    #[test]
    fn edge_inset_moves_the_slots_off_the_outer_edges() {
        let width = 400.0;
        let inset = nana_ui_core::space::XXXL;
        for (step, expected) in [
            (nana_ui_core::SpacingStep::Sm, nana_ui_core::space::SM),
            (nana_ui_core::SpacingStep::Xxxl, inset),
        ] {
            let mut context = AppContext::new();
            let leading = context
                .create_component(
                    document(),
                    IconButton::new(Icon::Sidebar, "sidebar").size(ControlSize::Small),
                )
                .unwrap();
            let trailing = context
                .create_component(
                    document(),
                    IconButton::new(Icon::Close, "close").size(ControlSize::Small),
                )
                .unwrap();
            let bar = context
                .create_component(
                    document(),
                    AppTitleBar::new("")
                        .show_window_controls(false)
                        .leading(leading.stable_id())
                        .trailing(trailing.stable_id())
                        .edge_inset(step),
                )
                .unwrap();
            context.assemble_app_title_bar(bar).unwrap();
            context
                .layout_document(document(), LayoutViewport::new(width, 400.0))
                .unwrap();
            let world = context.world();
            let lead = world.canonical_layout_box(leading.stable_id()).unwrap();
            let trail = world.canonical_layout_box(trailing.stable_id()).unwrap();
            assert!(
                (lead.x - expected).abs() < 0.5,
                "{step:?}: leading at {}",
                lead.x
            );
            assert!(
                (width - (trail.x + trail.width) - expected).abs() < 0.5,
                "{step:?}: trailing ends {} from the edge",
                width - (trail.x + trail.width)
            );
        }
    }

    #[test]
    fn custom_window_controls_are_inset_square_buttons() {
        let window_width = 800.0;
        let mut context = AppContext::new();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .native_controls(false)
                    .show_window_controls(true),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(window_width, 400.0))
            .unwrap();
        let world = context.world();
        let controls = find_title_bar_controls_child(&context, bar.stable_id())
            .expect("custom chrome mounts controls");
        let buttons = world.node(controls).unwrap().children.clone();
        assert_eq!(buttons.len(), 3);
        let container = world.layout_box(controls).unwrap();
        assert_eq!(container.height, TITLE_BAR_HEIGHT);
        assert_eq!(container.y, 0.0);
        assert_eq!(
            container.x + container.width,
            window_width,
            "controls must sit flush against the window edge"
        );
        for (index, button) in buttons.iter().enumerate() {
            let bounds = world.layout_box(*button).unwrap();
            assert_eq!(bounds.width, WINDOW_CONTROL_WIDTH);
            assert_eq!(bounds.height, WINDOW_CONTROL_WIDTH);
            assert_eq!(bounds.y, (TITLE_BAR_HEIGHT - WINDOW_CONTROL_WIDTH) / 2.0);
            assert_eq!(
                bounds.x,
                container.x
                    + WINDOW_CONTROL_PADDING
                    + index as f32 * (WINDOW_CONTROL_WIDTH + WINDOW_CONTROL_GAP),
                "buttons keep the shared control gap"
            );
        }
        let close = world.layout_box(buttons[2]).unwrap();
        assert_eq!(close.x + close.width, window_width - WINDOW_CONTROL_PADDING);
        assert!(close.y > container.y);
    }

    /// In a capsule the custom buttons sit on a `Surface` plate with a
    /// hairline and round ends, centred in the bar and standing in from the
    /// window edge by the bar's edge inset; the buttons go round too.
    #[test]
    fn capsule_window_controls_stand_in_on_a_plate() {
        let window_width = 800.0;
        let mut context = AppContext::new();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .native_controls(false)
                    .show_window_controls(true)
                    .over_media(true)
                    .controls_capsule(true),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(window_width, 400.0))
            .unwrap();
        let world = context.world();
        let controls = find_title_bar_controls_child(&context, bar.stable_id()).unwrap();
        let plate = world.layout_box(controls).unwrap();
        assert_eq!(plate.height, CONTROLS_CAPSULE_HEIGHT);
        assert_eq!(plate.y, (TITLE_BAR_HEIGHT - CONTROLS_CAPSULE_HEIGHT) / 2.0);
        assert_eq!(
            plate.x + plate.width,
            window_width - nana_ui_core::space::SM,
            "the capsule keeps the bar's edge inset"
        );
        let style = world.node_style(controls).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.border, Some(SemanticColorRole::BorderSoft));
        assert_eq!(style.radius, Some(nana_ui_core::RadiusTier::Full));
        let buttons = world.node(controls).unwrap().children.clone();
        assert_eq!(buttons.len(), 3);
        for (index, button) in buttons.iter().enumerate() {
            let bounds = world.layout_box(*button).unwrap();
            assert_eq!(bounds.width, WINDOW_CONTROL_WIDTH);
            assert_eq!(
                bounds.x,
                plate.x
                    + 1.0
                    + CONTROLS_CAPSULE_INSET_X
                    + index as f32 * (WINDOW_CONTROL_WIDTH + WINDOW_CONTROL_GAP)
            );
            let style = world.node_style(*button).unwrap();
            assert_eq!(style.radius, Some(nana_ui_core::RadiusTier::Full));
            // On the plate the buttons drop the on-media foreground.
            assert_ne!(style.foreground, Some(SemanticColorRole::OnMedia));
        }
    }

    /// The native traffic lights get the same plate under them: the
    /// placeholder is a capsule the placeholder's width, standing in from
    /// the leading edge by the bar's edge inset.
    #[test]
    fn capsule_native_controls_are_a_plate_under_the_traffic_lights() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("")
                    .native_controls(true)
                    .controls_capsule(true),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let controls = context.read(bar, |bar| bar.controls).unwrap().unwrap();
        let world = context.world();
        assert!(world.node(controls).unwrap().children.is_empty());
        let plate = world.canonical_layout_box(controls).unwrap();
        assert_eq!(plate.x, nana_ui_core::space::SM);
        assert_eq!(plate.width, NATIVE_WINDOW_CONTROLS_WIDTH);
        assert_eq!(plate.height, CONTROLS_CAPSULE_HEIGHT);
        assert_eq!(plate.y, (TITLE_BAR_HEIGHT - CONTROLS_CAPSULE_HEIGHT) / 2.0);
        let style = world.node_style(controls).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Surface));
        assert_eq!(style.radius, Some(nana_ui_core::RadiusTier::Full));
    }

    /// A wide centre column gives way before the trailing cluster and the
    /// window controls do: at 800 px a 420 px centre moves over so the
    /// account button and all three controls stay inside the window, at
    /// 600 px it shrinks, and with room to spare it keeps its width, centred.
    #[test]
    fn a_narrow_window_shrinks_the_centre_before_the_trailing_cluster() {
        let mut context = AppContext::new();
        let sized = |context: &mut AppContext, width: f32| {
            context
                .create_detached_component(
                    document(),
                    crate::Stack::row(0.0).with_layout(move |layout| {
                        layout.width = Some(LengthSpec::Px(width));
                        layout.height = Some(LengthSpec::Px(24.0));
                        layout.flex_shrink = Some(0.0);
                    }),
                )
                .unwrap()
        };
        let leading = sized(&mut context, 60.0);
        let center = sized(&mut context, 300.0);
        let trailing = sized(&mut context, 110.0);
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .native_controls(false)
                    .center_width(420.0)
                    .leading(leading.stable_id())
                    .center(center.stable_id())
                    .trailing(trailing.stable_id()),
            )
            .unwrap();
        context.assemble_app_title_bar(bar).unwrap();
        let controls = find_title_bar_controls_child(&context, bar.stable_id()).unwrap();
        let close = *context
            .world()
            .node(controls)
            .unwrap()
            .children
            .last()
            .unwrap();
        let column = |context: &AppContext, tag: &str| {
            context
                .world()
                .node(bar.stable_id())
                .unwrap()
                .children
                .into_iter()
                .find(|child| node_tag(context.world(), *child).as_deref() == Some(tag))
                .unwrap()
        };
        let center_column = column(&context, CENTER_COLUMN_TAG);
        for width in [800.0, 600.0] {
            context
                .layout_document(document(), LayoutViewport::new(width, 400.0))
                .unwrap();
            let world = context.world();
            let close_box = world.layout_box(close).unwrap();
            assert!(
                close_box.x + close_box.width <= width,
                "{width}: close {close_box:?} is inside the window"
            );
            let trailing_box = world.layout_box(trailing.stable_id()).unwrap();
            let center_box = world.layout_box(center_column).unwrap();
            assert!(
                trailing_box.x >= center_box.x + center_box.width,
                "{width}: the trailing cluster {trailing_box:?} is not under the centre {center_box:?}"
            );
            if width < 700.0 {
                assert!(center_box.width < 420.0, "{width}: the centre gave way");
            }
        }
        context
            .layout_document(document(), LayoutViewport::new(1400.0, 400.0))
            .unwrap();
        let center_box = context.world().canonical_layout_box(center_column).unwrap();
        assert_eq!(center_box.width, 420.0);
        assert!((center_box.x + center_box.width / 2.0 - 700.0).abs() < 0.5);
    }

    #[test]
    fn blank_title_bar_is_hittable_and_child_button_wins() {
        let mut context = AppContext::new();
        let button = context
            .create_component(
                document(),
                IconButton::new(Icon::Sidebar, "toggle").size(ControlSize::Small),
            )
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").leading(button.stable_id()),
            )
            .unwrap();
        context.append_child(bar, button).unwrap();
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        context.rebuild_hit_test(document());

        let bounds = context
            .world()
            .canonical_layout_box(bar.stable_id())
            .unwrap();
        let blank_x = bounds.x + bounds.width - 24.0;
        let blank_y = bounds.y + bounds.height / 2.0;
        assert_eq!(
            context.pointer_target(document(), blank_x, blank_y),
            Some(bar.stable_id())
        );

        let button_box = context
            .world()
            .canonical_layout_box(button.stable_id())
            .unwrap();
        let button_x = button_box.x + button_box.width / 2.0;
        let button_y = button_box.y + button_box.height / 2.0;
        assert_eq!(
            context.pointer_target(document(), button_x, button_y),
            Some(button.stable_id())
        );
    }

    #[test]
    fn center_slot_suppresses_default_title_text() {
        let mut context = AppContext::new();
        let center = context
            .create_component(document(), Text::new("Workspace / File"))
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").center(center.stable_id()),
            )
            .unwrap();
        assert_eq!(context.world().text(bar.stable_id()), Some(""));
        assert_eq!(
            context.world().text(center.stable_id()),
            Some("Workspace / File")
        );

        context
            .update_component(bar, |bar, _| {
                bar.center = None;
            })
            .unwrap();
        assert_eq!(context.world().text(bar.stable_id()), Some("Nana"));
    }

    #[test]
    fn assembled_title_bar_uses_three_columns() {
        let mut context = AppContext::new();
        let leading = context
            .create_component(
                document(),
                IconButton::new(Icon::Sidebar, "sidebar").size(ControlSize::Small),
            )
            .unwrap();
        let center = context
            .create_component(document(), Text::new("NanaShader"))
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .leading(leading.stable_id())
                    .center(center.stable_id()),
            )
            .unwrap();
        context.append_child(bar, leading).unwrap();
        context.append_child(bar, center).unwrap();
        assert!(context.assemble_app_title_bar(bar).unwrap());
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();

        let bar_box = context
            .world()
            .canonical_layout_box(bar.stable_id())
            .unwrap();
        let columns = context.world().node(bar.stable_id()).unwrap().children;
        assert_eq!(columns.len(), 3);
        let leading_col = context.world().canonical_layout_box(columns[0]).unwrap();
        let center_col = context.world().canonical_layout_box(columns[1]).unwrap();
        let trailing_col = context.world().canonical_layout_box(columns[2]).unwrap();
        let leading_box = context
            .world()
            .canonical_layout_box(leading.stable_id())
            .unwrap();
        let center_layout = &context.world().node_style(columns[1]).unwrap().layout;
        assert_eq!(
            center_layout.width,
            Some(LengthSpec::Px(DEFAULT_CENTER_WIDTH))
        );
        assert!(
            leading_box.width < 80.0,
            "leading chrome must hug, got width {}",
            leading_box.width
        );
        if !WindowChrome::platform_default().uses_custom_controls() {
            assert!(
                leading_box.x + 0.5 >= bar_box.x + NATIVE_WINDOW_CONTROLS_WIDTH,
                "leading must start after the traffic lights"
            );
        }
        let title_mid = center_col.x + center_col.width / 2.0;
        assert!(
            (title_mid - (bar_box.x + bar_box.width / 2.0)).abs() < 48.0,
            "title column mid {title_mid} should stay near the window center"
        );
        assert!(
            leading_col.x + leading_col.width <= center_col.x + 0.5,
            "title must sit after leading chrome"
        );
        assert!(
            trailing_col.width > bar_box.width * 0.2,
            "trailing column should fill leftover, got {}",
            trailing_col.width
        );

        if WindowChrome::platform_default().uses_custom_controls() {
            let controls = context
                .read(bar, |bar| bar.controls)
                .unwrap()
                .expect("custom chrome mounts controls");
            let controls_box = context.world().canonical_layout_box(controls).unwrap();
            assert!(
                controls_box.x + controls_box.width > bar_box.x + bar_box.width * 0.75,
                "window controls must sit on the trailing edge, got x={}",
                controls_box.x
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn default_title_bar_places_window_controls_on_the_right() {
        let mut context = AppContext::new();
        let bar = context
            .create_component(document(), AppTitleBar::new("NanaLive"))
            .unwrap();
        assert!(context.assemble_app_title_bar(bar).unwrap());
        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        context.rebuild_hit_test(document());

        let bar_box = context
            .world()
            .canonical_layout_box(bar.stable_id())
            .unwrap();
        let controls = context
            .read(bar, |bar| bar.controls)
            .unwrap()
            .expect("custom chrome mounts controls");
        let controls_box = context.world().canonical_layout_box(controls).unwrap();
        assert!(
            controls_box.x > bar_box.x + bar_box.width * 0.7,
            "controls were on the left: x={} width={}",
            controls_box.x,
            bar_box.width
        );
        let close = context.world().node(controls).unwrap().children[2];
        let close_box = context.world().canonical_layout_box(close).unwrap();
        assert!(
            close_box.x + close_box.width > bar_box.x + bar_box.width - 48.0,
            "close button must sit near the trailing edge"
        );
    }

    #[test]
    fn controls_helper_projects_three_icons_and_restore() {
        let mut context = AppContext::new();
        let minimize = context
            .create_component(
                document(),
                IconButton::new(Icon::Search, "x").size(ControlSize::Small),
            )
            .unwrap();
        let maximize = context
            .create_component(
                document(),
                IconButton::new(Icon::Search, "x").size(ControlSize::Small),
            )
            .unwrap();
        let close = context
            .create_component(
                document(),
                IconButton::new(Icon::Search, "x").size(ControlSize::Small),
            )
            .unwrap();
        let controls = context
            .create_component(
                document(),
                AppTitleBarControls::new(false)
                    .native(false)
                    .minimize(minimize.stable_id())
                    .maximize(maximize.stable_id())
                    .close(close.stable_id()),
            )
            .unwrap();

        assert_eq!(
            context.world().node(controls.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "app-title-bar-controls".into(),
            }
        );
        assert_eq!(
            icon_of(
                context
                    .world()
                    .widget_visual_as::<crate::IconVisual>(minimize.stable_id())
            ),
            Some(Icon::Minimize)
        );
        assert_eq!(
            icon_of(
                context
                    .world()
                    .widget_visual_as::<crate::IconVisual>(maximize.stable_id())
            ),
            Some(Icon::Maximize)
        );
        assert_eq!(
            icon_of(
                context
                    .world()
                    .widget_visual_as::<crate::IconVisual>(close.stable_id())
            ),
            Some(Icon::Close)
        );
        assert_eq!(
            context
                .world()
                .node_style(minimize.stable_id())
                .unwrap()
                .layout
                .width,
            Some(LengthSpec::Px(WINDOW_CONTROL_WIDTH))
        );
        assert_eq!(
            context
                .world()
                .accessibility(maximize.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("最大化")
        );

        context
            .update_component(controls, |controls, _| {
                controls.maximized = true;
            })
            .unwrap();
        assert_eq!(
            icon_of(
                context
                    .world()
                    .widget_visual_as::<crate::IconVisual>(maximize.stable_id())
            ),
            Some(Icon::Restore)
        );
        assert_eq!(
            context
                .world()
                .accessibility(maximize.stable_id())
                .unwrap()
                .label
                .as_deref(),
            Some("还原")
        );
        assert_eq!(
            icon_of(
                context
                    .world()
                    .widget_visual_as::<crate::IconVisual>(minimize.stable_id())
            ),
            Some(Icon::Minimize)
        );
        assert_eq!(
            context
                .world()
                .node_style(close.stable_id())
                .unwrap()
                .interaction
                .focused
                .border,
            None
        );
    }

    #[test]
    fn window_controls_are_omitted_when_disabled() {
        let mut context = AppContext::new();
        let minimize = context
            .create_component(
                document(),
                IconButton::new(Icon::Minimize, "Minimize").size(ControlSize::Small),
            )
            .unwrap();
        let maximize = context
            .create_component(
                document(),
                IconButton::new(Icon::Maximize, "Maximize").size(ControlSize::Small),
            )
            .unwrap();
        let close = context
            .create_component(
                document(),
                IconButton::new(Icon::Close, "Close").size(ControlSize::Small),
            )
            .unwrap();
        let controls = context
            .create_component(
                document(),
                AppTitleBarControls::new(false)
                    .native(false)
                    .minimize(minimize.stable_id())
                    .maximize(maximize.stable_id())
                    .close(close.stable_id()),
            )
            .unwrap();
        context.append_child(controls, minimize).unwrap();
        context.append_child(controls, maximize).unwrap();
        context.append_child(controls, close).unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana")
                    .native_controls(false)
                    .controls(controls.stable_id())
                    .show_window_controls(false),
            )
            .unwrap();
        assert!(
            context
                .world()
                .node_style(controls.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            context
                .world()
                .node(bar.stable_id())
                .unwrap()
                .children
                .is_empty()
        );

        context
            .update_component(bar, |bar, _| {
                bar.show_window_controls = true;
                bar.maximized = true;
            })
            .unwrap();
        assert!(
            !context
                .world()
                .node_style(controls.stable_id())
                .unwrap()
                .layout
                .hidden
        );
        assert_eq!(
            icon_of(
                context
                    .world()
                    .widget_visual_as::<crate::IconVisual>(maximize.stable_id())
            ),
            Some(Icon::Restore)
        );
    }
}
