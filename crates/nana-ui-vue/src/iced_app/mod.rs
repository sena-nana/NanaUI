//! Semantic snapshot → real NanaUI (Iced) widgets (`iced_app`).
//!
//! Every mapped kind draws through `nana_ui::*` foundations. Vue custom UI is
//! expressed as layout composition + control variants — not a paint bypass.
//! CSS 子集经 [`crate::css_map::LayoutStyle`] 落到 iced Length / padding / gap /
//! justify spacer / scrollable / 主轴 flex。
//!
//! ## Known temporary L1 paint exceptions
//! - [`crate::svg_icon`]: Vue-emitted SVG / chart geometry → iced `svg`
//! - [`l1_charts`]: deferred canvas path-d fallback when SVG chart is absent
//! Prefer sinking both to L3 (`CalendarHeatmap` / generic SvgChart) — do not add
//! new iced-primitive branches here. See `docs/css-layout-engine-boundary.md`.
//!
//! Each visible widget is wrapped in [`LayoutProbe`], which records iced
//! `layout.bounds()` into [`crate::LayoutBoxStore`] on draw so JS
//! `getBoundingClientRect` / `layoutBox` match painted geometry.
//!
//! ## L2 adapter boundary
//! Semantic snapshot → `nana_ui` widgets. Subfiles via `include!` (same module):
//! `layout_flow`, `button`, `settings`, `layout_convert`, `l1_charts`, `surface`,
//! `overlay`, `selection`. Do not grow a second paint core here.

use std::cell::RefCell;
use std::sync::Arc;

use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer as adv_renderer};
use iced::font;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path as CanvasPath};
use iced::widget::svg::Handle as SvgHandle;
use iced::widget::text::{self as text_widget, Ellipsis, LineHeight};
use iced::widget::{Space, column, container, row, scrollable, space, stack, text};
use iced::{Alignment, Background, Border, Color, Element, Event, Length, Padding, Shadow, Size};
use iced::{Point, Rectangle, Renderer, Theme};
use nana_ui::compatibility::{
    Button, Card, Checkbox, IconButton, Input, ListItem, RangeField, Switch,
};
use nana_ui::{
    ActionMenuItem, AnchoredMenuPosition, ButtonKind, ButtonPaintOverride, ConfirmDialog,
    ControlSize, Dialog, Drawer, DrawerSide, EmptyState, HostTextureBinding, HostTextureRegistry,
    Icon, Popover, Progress, SegmentedControl, Select, SelectionOption, SettingsCard, SettingsRow,
    SidebarRow, SidebarRowState, SidebarRowTone, Spinner, Tabs, Textarea, ThemeTokens, Tooltip,
    TooltipConfig, TooltipPlacement, icon, ui_font,
};
use nana_ui::{
    AnchoredActionMenu, AnchoredMenuPlacement, ContextMenuEvent, ContextMenuHost, OverlayHost,
};
use nana_ui_scene::UiScene;
use nana_ui_web_api::{CanvasBitmap, SharedCanvasRuntime};

use crate::bridge::{
    BridgeEvent, MessageBridge, SemanticSnapshot, SemanticWidget, WidgetId, WidgetKind, WidgetProps,
};
use crate::css_map::{
    AlignSpec, BoxSizing, DisplaySpec, FlexDirection, FlexWrap, GridTrack, JustifySpec, LengthSpec,
    OverflowSpec, ParentBox, resolve_grid_column_widths, resolve_grid_track_sizes,
};
use crate::editor_store::EditorStore;
use crate::menu_store::MenuStore;
use crate::native_component::NativeComponentRegistry;
use crate::tree::{LayoutBoxStore, NodeHandle, shared_layout_box_store};

pub(crate) fn hosted_text_widget_id(id: WidgetId) -> String {
    format!("nana-vue-text-{id}")
}

thread_local! {
    /// Build-time texture lookup only. Every produced GPU widget owns a cloned
    /// `HostTexture`, so no thread-local state leaks into rendering.
    static ACTIVE_HOST_TEXTURES: RefCell<Option<HostTextureRegistry>> = const { RefCell::new(None) };
    static ACTIVE_CANVAS_RUNTIME: RefCell<Option<SharedCanvasRuntime>> = const { RefCell::new(None) };
    static ACTIVE_NATIVE_COMPONENTS: RefCell<Option<NativeComponentRegistry>> = const { RefCell::new(None) };
    static ACTIVE_PAINT_AFFINE: RefCell<[f32; 6]> = const { RefCell::new([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) };
    static ACTIVE_SCENE_HOST_TEXTURES: RefCell<Option<nana_ui::HostTextureSceneResolver>> = const { RefCell::new(None) };
    static ACTIVE_SCENE: RefCell<Option<Arc<UiScene>>> = const { RefCell::new(None) };
    static ACTIVE_LAYOUT_BOXES: RefCell<Option<Arc<LayoutBoxStore>>> = const { RefCell::new(None) };
}

fn with_active_layout_boxes<T>(store: Option<Arc<LayoutBoxStore>>, build: impl FnOnce() -> T) -> T {
    struct Reset(Option<Arc<LayoutBoxStore>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE_LAYOUT_BOXES.with(|active| active.replace(self.0.take()));
        }
    }
    ACTIVE_LAYOUT_BOXES.with(|active| {
        let previous = active.replace(store);
        let _reset = Reset(previous);
        build()
    })
}

fn active_layout_box_store() -> Arc<LayoutBoxStore> {
    ACTIVE_LAYOUT_BOXES
        .with(|active| active.borrow().clone())
        .unwrap_or_else(shared_layout_box_store)
}

fn with_active_scene<T>(scene: Option<&UiScene>, build: impl FnOnce() -> T) -> T {
    struct Reset {
        resolver: Option<nana_ui::HostTextureSceneResolver>,
        scene: Option<Arc<UiScene>>,
    }
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE_SCENE_HOST_TEXTURES.with(|active| {
                active.replace(self.resolver.take());
            });
            ACTIVE_SCENE.with(|active| active.replace(self.scene.take()));
        }
    }
    let scene = scene.map(|scene| Arc::new(scene.clone()));
    let resolver = scene.as_ref().map(|scene| {
        ACTIVE_HOST_TEXTURES.with(|textures| {
            let textures = textures.borrow().clone().unwrap_or_default();
            nana_ui::HostTextureSceneResolver::new(&scene, &textures)
                .unwrap_or_else(|error| panic!("Vue scene cannot be presented: {error}"))
        })
    });
    ACTIVE_SCENE_HOST_TEXTURES.with(|active| {
        let previous = active.replace(resolver);
        ACTIVE_SCENE.with(|active_scene| {
            let previous_scene = active_scene.replace(scene);
            let _reset = Reset {
                resolver: previous,
                scene: previous_scene,
            };
            build()
        })
    })
}

fn active_scene_host_texture(id: WidgetId) -> Option<HostTextureBinding> {
    ACTIVE_SCENE_HOST_TEXTURES.with(|active| active.borrow().as_ref()?.binding(id))
}

fn with_active_native_components<T>(
    registry: Option<&NativeComponentRegistry>,
    build: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<NativeComponentRegistry>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE_NATIVE_COMPONENTS.with(|active| {
                active.replace(self.0.take());
            });
        }
    }
    ACTIVE_NATIVE_COMPONENTS.with(|active| {
        let previous = active.replace(registry.cloned());
        let _reset = Reset(previous);
        build()
    })
}

fn active_native_components() -> Option<NativeComponentRegistry> {
    ACTIVE_NATIVE_COMPONENTS.with(|active| active.borrow().clone())
}

fn with_active_host_textures<T>(
    registry: Option<&HostTextureRegistry>,
    build: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<HostTextureRegistry>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE_HOST_TEXTURES.with(|active| {
                active.replace(self.0.take());
            });
        }
    }

    ACTIVE_HOST_TEXTURES.with(|active| {
        let previous = active.replace(registry.cloned());
        let _reset = Reset(previous);
        build()
    })
}

fn active_host_texture(slot: &str) -> Option<HostTextureBinding> {
    ACTIVE_HOST_TEXTURES.with(|active| active.borrow().as_ref()?.get(slot))
}

fn with_active_canvas<T>(runtime: Option<&SharedCanvasRuntime>, build: impl FnOnce() -> T) -> T {
    struct Reset(Option<SharedCanvasRuntime>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE_CANVAS_RUNTIME.with(|active| {
                active.replace(self.0.take());
            });
        }
    }
    ACTIVE_CANVAS_RUNTIME.with(|active| {
        let previous = active.replace(runtime.cloned());
        let _reset = Reset(previous);
        build()
    })
}

fn active_canvas_bitmap(id: u64) -> Option<CanvasBitmap> {
    ACTIVE_CANVAS_RUNTIME.with(|active| {
        active
            .borrow()
            .as_ref()?
            .lock()
            .ok()?
            .bitmap(nana_ui_web_api::CanvasId(id))
            .ok()
    })
}

/// Iced / host 布局回写：把 viewport 与 Fill 父链写入 bridge `containing_block_*`，
/// 供后续 `style` 的 margin/padding/gap `%` 使用。`VueHost::semantic_snapshot` 也会调用。
pub fn writeback_containing_blocks(bridge: &mut MessageBridge, viewport: ParentBox) {
    bridge.sync_layout_containing_blocks(viewport);
}

/// Apply iced paint boxes from [`shared_layout_box_store`] into `boxes` consumer.
///
/// Prefer this (or `VueHost::sync_iced_layout_boxes`) after a frame draws so
/// document hit-tests match `layoutBox` / `getBoundingClientRect`.
pub fn writeback_iced_layout_boxes(apply: impl FnOnce(&[(NodeHandle, crate::LayoutBox)])) {
    let snap = shared_layout_box_store().snapshot();
    if !snap.is_empty() {
        apply(&snap);
    }
}

/// Like [`writeback_iced_layout_boxes`], then re-apply host scroll offsets so
/// JS `layoutBox` / `scrollIntoView` stay consistent without iced Task drain.
pub fn writeback_iced_layout_boxes_with_scroll(
    doc: &mut crate::tree::NanaTreeDocument,
    bridge: &crate::bridge::MessageBridge,
    apply: impl FnOnce(&[(NodeHandle, crate::LayoutBox)]),
) {
    writeback_iced_layout_boxes(apply);
    crate::scroll::reapply_scroll_translations(doc, bridge, &shared_layout_box_store());
}

/// Transparent iced wrapper that records absolute paint bounds into a
/// [`LayoutBoxStore`] on every draw. Does not alter layout or visuals.
struct LayoutProbe<'a, Message> {
    id: WidgetId,
    store: Arc<LayoutBoxStore>,
    content: Element<'a, Message>,
    transform: Option<crate::css_map::PaintTransform>,
}

impl<'a, Message> LayoutProbe<'a, Message> {
    fn new(id: WidgetId, store: Arc<LayoutBoxStore>, content: Element<'a, Message>) -> Self {
        Self {
            id,
            store,
            content,
            transform: None,
        }
    }

    fn with_transform(mut self, transform: Option<crate::css_map::PaintTransform>) -> Self {
        self.transform = transform;
        self
    }
}

impl<Message> Widget<Message, Theme, Renderer> for LayoutProbe<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &adv_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let active = ACTIVE_PAINT_AFFINE.with(|value| *value.borrow());
        let affine = self
            .transform
            .map(|transform| {
                concat_affine(
                    active,
                    transform.around_center(bounds.x, bounds.y, bounds.width, bounds.height),
                )
            })
            .unwrap_or(active);
        if is_identity_affine(affine) {
            self.store.record(
                NodeHandle(self.id),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            );
        } else {
            self.store.record_transformed(
                NodeHandle(self.id),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                affine,
            );
        }
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<LayoutProbe<'a, Message>> for Element<'a, Message> {
    fn from(probe: LayoutProbe<'a, Message>) -> Self {
        Element::new(probe)
    }
}

fn probe_layout<'a, Message: 'a>(
    id: WidgetId,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    LayoutProbe::new(id, active_layout_box_store(), content).into()
}

fn probe_transformed_layout<'a, Message: 'a>(
    id: WidgetId,
    content: Element<'a, Message>,
    transform: Option<crate::css_map::PaintTransform>,
) -> Element<'a, Message> {
    LayoutProbe::new(id, active_layout_box_store(), content)
        .with_transform(transform)
        .into()
}

/// Build an Iced element tree from a semantic snapshot.
pub fn view_semantic_tree<'a, Message>(
    snap: &'a SemanticSnapshot,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    view_semantic_tree_with_editors(snap, tokens, None, None, None, map_event)
}

/// Same as [`view_semantic_tree`] but passes viewport for `%` / 定高链（P0-3/P0-4）。
pub fn view_semantic_tree_with_viewport<'a, Message>(
    snap: &'a SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    view_semantic_tree_with_editors(snap, tokens, viewport, None, None, map_event)
}

/// Borrowed tree with optional host-owned [`EditorStore`] for real Textarea widgets.
pub fn view_semantic_tree_with_editors<'a, Message>(
    snap: &'a SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let build = || {
        active_layout_box_store().begin_frame();
        let parent = viewport
            .map(|(w, h)| ParentBox::from_viewport(w, h))
            .unwrap_or_default();
        let viewport_size = viewport
            .map(|(w, h)| Size::new(w, h))
            .unwrap_or(Size::new(1280.0, 800.0));
        let mut roots = column![].spacing(10).width(Length::Fill);
        if snap.roots.is_empty() {
            roots = roots.push(EmptyState::new("暂无内容").view(tokens));
        } else {
            for &root_id in &snap.roots {
                roots = roots.push(view_widget(
                    snap,
                    root_id,
                    tokens,
                    parent,
                    FlexDirection::Column,
                    AlignSpec::Stretch,
                    editors,
                    menus,
                    None,
                    map_event.clone(),
                ));
            }
        }
        let document = scrollable(roots.padding(16))
            .width(Length::Fill)
            .height(Length::Fill);
        let fixed_ids = collect_css_fixed_ids(snap);
        if fixed_ids.is_empty() {
            return document.into();
        }
        // Borrowed path: fixed layers use owned clones via static helpers where needed.
        // Prefer static tree for full fixed paint; here approximate with empty spacer
        // stack so fixed leaves flow and does not occupy scroll content.
        let mut layers = stack![document].width(Length::Fill).height(Length::Fill);
        for fid in fixed_ids {
            let Some(w) = snap.get(fid) else {
                continue;
            };
            let (x, y, fw, fh) =
                resolve_fixed_box(&w.props.layout, viewport_size.width, viewport_size.height);
            let label = if w.props.label.is_empty() {
                w.props.value.clone()
            } else {
                w.props.label.clone()
            };
            let chip = container(text(label).size(12))
                .width(Length::Fixed(fw.max(1.0)))
                .height(Length::Fixed(fh.max(1.0)));
            layers = layers.push(
                container(
                    column![
                        space().height(Length::Fixed(y.max(0.0))),
                        row![space().width(Length::Fixed(x.max(0.0))), chip],
                    ]
                    .width(Length::Fill)
                    .height(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill),
            );
        }
        layers.into()
    };
    if let Some((w, h)) = viewport {
        crate::css_map::with_active_viewport(w, h, build)
    } else {
        build()
    }
}

/// Owned/`'static` helper used by hosted demos that rebuild the tree each frame.
pub fn view_semantic_tree_static<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    view_semantic_tree_static_with_viewport(snap, tokens, None, map_event)
}

/// Static tree with optional viewport for percent / height chain resolution.
pub fn view_semantic_tree_static_with_viewport<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    view_semantic_tree_static_with_editors(snap, tokens, viewport, None, None, map_event)
}

/// Static tree with host-owned editors (Textarea → `text_editor::Content`).
pub fn view_semantic_tree_static_with_editors<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    view_semantic_tree_static_with_resources(
        snap, tokens, viewport, editors, menus, None, None, map_event,
    )
}

/// Static tree with host-owned editor/menu state and real GPU texture slots.
/// The registry only resolves views while building; the returned elements hold
/// stable `HostTexture` clones and render through Iced's existing WGPU pass.
pub fn view_semantic_tree_static_with_resources<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    host_textures: Option<&HostTextureRegistry>,
    canvas_runtime: Option<&SharedCanvasRuntime>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    view_semantic_tree_static_with_native_components(
        snap,
        tokens,
        viewport,
        editors,
        menus,
        host_textures,
        canvas_runtime,
        None,
        map_event,
    )
}

/// Static semantic tree with application-registered Rust/Iced components.
#[allow(clippy::too_many_arguments)]
pub fn view_semantic_tree_static_with_native_components<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    host_textures: Option<&HostTextureRegistry>,
    canvas_runtime: Option<&SharedCanvasRuntime>,
    components: Option<&NativeComponentRegistry>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    view_semantic_tree_static_with_scene(
        snap,
        tokens,
        viewport,
        editors,
        menus,
        host_textures,
        canvas_runtime,
        components,
        None,
        None,
        map_event,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn view_semantic_tree_static_with_scene<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    host_textures: Option<&HostTextureRegistry>,
    canvas_runtime: Option<&SharedCanvasRuntime>,
    components: Option<&NativeComponentRegistry>,
    scene: Option<&UiScene>,
    layout_boxes: Option<Arc<LayoutBoxStore>>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let build = || {
        with_active_layout_boxes(layout_boxes, || {
            with_active_host_textures(host_textures, || {
                with_active_canvas(canvas_runtime, || {
                    with_active_native_components(components, || {
                        with_active_scene(scene, || {
                            view_semantic_tree_static_with_editors_inner(
                                snap, tokens, viewport, editors, menus, map_event,
                            )
                        })
                    })
                })
            })
        })
    };
    if let Some((w, h)) = viewport {
        crate::css_map::with_active_viewport(w, h, build)
    } else {
        build()
    }
}

fn view_semantic_tree_static_with_editors_inner<Message>(
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    viewport: Option<(f32, f32)>,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    active_layout_box_store().begin_frame();
    let parent = viewport
        .map(|(w, h)| ParentBox::from_viewport(w, h))
        .unwrap_or_default();
    let viewport_size = viewport
        .map(|(w, h)| Size::new(w, h))
        .unwrap_or(Size::new(1280.0, 800.0));
    // Prefer Fixed viewport size over Length::Fill at the tree root so iced
    // cannot collapse the shell height chain to a bottom strip.
    let root_w = root_viewport_axis(parent.width);
    let root_h = root_viewport_axis(parent.height);
    let mut roots = column![].spacing(0).width(root_w).height(root_h);
    if snap.roots.is_empty() {
        roots = roots.push(EmptyState::new("暂无内容").view(tokens));
    } else {
        for &root_id in &snap.roots {
            roots = roots.push(view_widget_owned(
                snap,
                root_id,
                tokens,
                parent,
                FlexDirection::Column,
                AlignSpec::Stretch,
                editors,
                menus,
                viewport_size,
                None,
                map_event.clone(),
            ));
        }
    }
    // Shell layouts (sidebar | main) need a definite height — avoid wrapping the
    // whole tree in an outer scrollable that collapses horizontal rows.
    let document = container(roots).width(root_w).height(root_h);
    // CSS `position:fixed` viewport layers + open Nana Overlay (Dialog/Drawer/…)
    // above in-flow content. Open overlays leave flow so parent size cannot clip
    // the scrim (companion fixed/sticky already stripped on overlay kinds).
    let overlay_ids = collect_open_overlay_ids(snap);
    let fixed_ids = collect_css_fixed_ids(snap);
    if overlay_ids.is_empty() && fixed_ids.is_empty() {
        return document.into();
    }
    let mut layers = stack![document].width(root_w).height(root_h);
    for oid in overlay_ids {
        layers = layers.push(view_overlay_layer_owned(
            snap,
            oid,
            tokens,
            editors,
            menus,
            viewport_size,
            map_event.clone(),
        ));
    }
    for fid in fixed_ids {
        layers = layers.push(view_fixed_layer_owned(
            snap,
            fid,
            tokens,
            editors,
            menus,
            viewport_size,
            map_event.clone(),
        ));
    }
    layers.into()
}

fn runtime_component_for_widget(
    snap: &SemanticSnapshot,
    widget: &SemanticWidget,
) -> Option<nana_ui::ComponentId> {
    match widget.kind {
        WidgetKind::Text => Some(nana_ui::component_ids::TEXT),
        WidgetKind::Checkbox => Some(nana_ui::component_ids::CHECKBOX),
        WidgetKind::Switch => Some(nana_ui::component_ids::SWITCH),
        WidgetKind::Card => Some(nana_ui::component_ids::CARD),
        WidgetKind::ListItem => Some(nana_ui::component_ids::LIST_ITEM),
        WidgetKind::Range => Some(nana_ui::component_ids::RANGE_FIELD),
        WidgetKind::Button => {
            let (icon, label) =
                resolve_button_icon_and_label(snap, &widget.props, &widget.children);
            Some(
                if (is_square_icon_button(&widget.props) || (icon.is_some() && label.is_empty()))
                    && matches!(icon, Some(ResolvedButtonIcon::Glyph(_)))
                {
                    nana_ui::component_ids::ICON_BUTTON
                } else {
                    nana_ui::component_ids::BUTTON
                },
            )
        }
        WidgetKind::Input => Some(nana_ui::component_ids::TEXT_INPUT),
        _ => None,
    }
}

fn qualified_runtime_scene_view<'a, Message>(
    snap: &SemanticSnapshot,
    widget: &SemanticWidget,
) -> Option<Element<'a, Message>>
where
    Message: 'a,
{
    let component = runtime_component_for_widget(snap, widget)
        .filter(|id| nana_ui::component_uses_runtime(*id))?;
    // Public view helpers without a Scene are the explicit compatibility
    // adapter. Once a hosted Runtime Scene is active, a qualified component
    // must remain on that route: missing retained state is an invariant
    // violation, never a reason to manufacture an Iced tree.
    ACTIVE_SCENE.with(|active| {
        active.borrow().clone().map(|scene| {
            let id = nana_ui_runtime::StableNodeId::new(widget.id)
                .expect("Vue widget identity must be non-zero");
            let bounds = scene.node_bounds(id).unwrap_or_else(|| {
                panic!(
                    "qualified Runtime component `{}` is missing Scene node {}",
                    component.as_str(),
                    widget.id
                )
            });
            nana_ui::IcedSceneView::from_shared_node(
                scene,
                id,
                None,
                Size::new(bounds.width, bounds.height),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "qualified Runtime component `{}` cannot create its Scene view: {error}",
                    component.as_str()
                )
            })
            .into()
        })
    })
}

fn view_widget<'a, Message>(
    snap: &'a SemanticSnapshot,
    id: WidgetId,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    parent_align_items: AlignSpec,
    editors: Option<&'a EditorStore>,
    menus: Option<&'a MenuStore>,
    // Pre-resolved flex main px (grow+shrink); `None` → length_from_spec path.
    main_override: Option<f32>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'a,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let Some(widget) = snap.get(id) else {
        return text("").into();
    };
    if widget.props.layout.hidden {
        return text("").into();
    }
    // Absolute is measure-only in iced flow. Fixed paints via the root fixed
    // layer (viewport CB). Sticky stays deferred. Product overlays use Nana Overlay.
    if !widget.kind.is_overlay()
        && (widget.props.layout.is_absolute()
            || widget.props.layout.is_fixed()
            || widget.props.layout.position.is_unsupported_positioning())
    {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let is_layout_chrome = matches!(
        widget.kind,
        WidgetKind::Column
            | WidgetKind::Box
            | WidgetKind::SidebarFrame
            | WidgetKind::SettingsCard
            | WidgetKind::Row
            | WidgetKind::Card
    );
    let runtime_view = qualified_runtime_scene_view(snap, widget);
    let content = if let Some(runtime_view) = runtime_view {
        runtime_view
    } else {
        match widget.kind {
            WidgetKind::Box | WidgetKind::Column
                if let Some(chart) = crate::svg_icon::try_svg_chart_element(snap, widget.id) =>
            {
                // Preferred single track: structural <svg> charts via resvg (`svg_icon`).
                chart
            }
            WidgetKind::Box | WidgetKind::Column
                if widget.children.is_empty() && filled_svg_path_d(&widget.props).is_some() =>
            {
                // DEFER: legacy canvas path-d leaf when no SVG chart root is present.
                // Keep for visual parity; do not extend — sink to L3 CalendarHeatmap.
                heatmap_level_canvas(widget)
            }
            WidgetKind::Box | WidgetKind::Column | WidgetKind::SidebarFrame
                if let Some(canvas) = try_composite_filled_svg_paths(snap, widget) =>
            {
                // DEFER: composite path-d canvas fallback (see `l1_charts`).
                canvas
            }
            WidgetKind::SettingsCard => {
                settings_card_view(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::Column | WidgetKind::Box | WidgetKind::SidebarFrame => {
                layout_column(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::Row => {
                layout_row(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::Card => {
                // Borrowed path still builds through layout_column; Card::view now
                // Fill-matches its inner column so nested Fill bodies do not collapse.
                let body = layout_column(
                    snap,
                    widget,
                    tokens,
                    parent_box,
                    editors,
                    menus,
                    map_event.clone(),
                );
                let mut card = Card::new(body);
                if !widget.props.label.is_empty() {
                    card = card.title(widget.props.label.as_str());
                }
                card = card_with_css_height(card, &widget.props.layout, parent_box);
                card = card_with_css_padding(card, &widget.props.layout, parent_box);
                card.view(tokens)
            }
            WidgetKind::Text => {
                // Vue nests `#text` under `h*` / `span`. Leaf Text keeps the fast path;
                // parents with only empty labels must paint children or titles vanish.
                // `display:flex` hosts (e.g. `.card h2`) must keep row axis so
                // `align-items:center` stays vertical — a forced column would center
                // the title horizontally and inflate top blank in the card heading.
                if !widget.children.is_empty() && widget.props.display_label().is_empty() {
                    if text_host_column_axis(&widget.props.layout) {
                        layout_column(snap, widget, tokens, parent_box, editors, menus, map_event)
                    } else {
                        layout_row(snap, widget, tokens, parent_box, editors, menus, map_event)
                    }
                } else {
                    label_text(
                        widget.props.display_label().to_string(),
                        widget.props.size,
                        &widget.props.layout,
                        parent_box.width,
                    )
                }
            }
            WidgetKind::Icon => {
                let size = crate::svg_icon::resolve_icon_size(&widget.props);
                let color = widget
                    .props
                    .layout
                    .color
                    .map(rgba_color)
                    .unwrap_or(tokens.colors.text);
                if let Some(handle) = crate::svg_icon::try_svg_handle(snap, widget.id) {
                    crate::svg_icon::svg_icon_element(handle, size, color)
                } else if let Some(kind) = resolve_icon_from_props(&widget.props) {
                    icon(kind, size, color)
                } else {
                    crate::svg_icon::empty_icon_placeholder(size)
                }
            }
            WidgetKind::Button => {
                let id = widget.id;
                let map = map_event.clone();
                button_view(
                    &widget.props,
                    &widget.children,
                    snap,
                    tokens,
                    parent_box.width,
                    map(BridgeEvent::Press { id }),
                )
            }
            WidgetKind::Chip => {
                let label = widget.props.display_label();
                if label.is_empty() {
                    space().width(Length::Shrink).height(Length::Shrink).into()
                } else {
                    let id = widget.id;
                    let map = map_event.clone();
                    let kind = if widget.props.active {
                        ButtonKind::Selected
                    } else {
                        ButtonKind::Subtle
                    };
                    let btn = Button::label(label)
                        .kind(kind)
                        .size(ControlSize::Small)
                        .disabled(widget.props.disabled)
                        .on_press(map(BridgeEvent::Select { id }));
                    apply_button_layout_chrome(btn, &widget.props, parent_box.width).view(tokens)
                }
            }
            WidgetKind::Switch => {
                let id = widget.id;
                let map = map_event.clone();
                let mut control = Switch::new(widget.props.toggled, widget.props.display_label())
                    .disabled(widget.props.disabled)
                    .on_toggle(move |value| map(BridgeEvent::Toggle { id, value }));
                if !widget.props.hint.is_empty() {
                    control = control.hint(widget.props.hint.as_str());
                }
                control.view(tokens)
            }
            WidgetKind::Checkbox => {
                let id = widget.id;
                let map = map_event.clone();
                Checkbox::new(widget.props.toggled, widget.props.display_label())
                    .disabled(widget.props.disabled)
                    .on_toggle(move |value| map(BridgeEvent::Toggle { id, value }))
                    .view(tokens)
            }
            WidgetKind::Input => {
                let id = widget.id;
                let map = map_event.clone();
                let placeholder = if widget.props.placeholder.is_empty() {
                    widget.props.hint.as_str()
                } else {
                    widget.props.placeholder.as_str()
                };
                let input = Input::new(placeholder, widget.props.value.as_str())
                    .id(hosted_text_widget_id(id))
                    .size(widget.props.size)
                    .disabled(widget.props.disabled || widget.props.loading)
                    .invalid(widget.props.invalid)
                    .secure(widget.props.secure);
                if widget.props.read_only {
                    input.view(tokens)
                } else {
                    input
                        .on_input(move |value| map(BridgeEvent::Input { id, value }))
                        .view(tokens)
                }
            }
            WidgetKind::Textarea => textarea_view(widget, tokens, editors, menus, map_event),
            WidgetKind::Range => {
                let id = widget.id;
                let map = map_event.clone();
                let min = widget.props.min;
                let max = widget.props.max.max(min + f32::EPSILON);
                let value = widget.props.number.clamp(min, max);
                let mut range = RangeField::new(min..=max, value, move |v| {
                    map(BridgeEvent::Change {
                        id,
                        value: f64::from(v),
                    })
                });
                if !widget.props.label.is_empty() {
                    range = range.label(widget.props.label.as_str());
                }
                if !widget.props.unit.is_empty() {
                    range = range.unit(widget.props.unit.as_str());
                }
                range.size(widget.props.size).view(tokens)
            }
            WidgetKind::Tabs => selection_tabs(widget, tokens, map_event),
            WidgetKind::Segmented => selection_segmented(widget, tokens, map_event),
            WidgetKind::Select => selection_select(widget, tokens, map_event),
            WidgetKind::Dialog => {
                overlay_dialog(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::Drawer => {
                overlay_drawer(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::Popover => {
                overlay_popover(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::ContextMenu => {
                overlay_context_menu(widget, tokens, parent_box, menus, map_event)
            }
            WidgetKind::SidebarRow => {
                let id = widget.id;
                let map = map_event.clone();
                let state = if widget.props.disabled {
                    SidebarRowState::Disabled
                } else if widget.props.active {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                };
                let (leading, label) =
                    resolve_row_leading_and_label(snap, &widget.props, &widget.children, tokens);
                let mut row = SidebarRow::new(label)
                    .state(state)
                    .tone(SidebarRowTone::Default)
                    .gap(widget.props.layout.gap_or(6.0))
                    .on_select(map(BridgeEvent::Select { id }));
                if let Some(icon_el) = leading {
                    row = row.leading(icon_el);
                }
                row.view(tokens)
            }
            WidgetKind::ListItem => {
                let id = widget.id;
                let map = map_event.clone();
                let (leading, label) =
                    resolve_row_leading_and_label(snap, &widget.props, &widget.children, tokens);
                let mut item = ListItem::label(label)
                    .selected(widget.props.active)
                    .disabled(widget.props.disabled)
                    .gap(widget.props.layout.gap_or(8.0))
                    .on_select(map(BridgeEvent::Select { id }));
                if let Some(icon_el) = leading {
                    item = item.leading(icon_el);
                }
                item.view(tokens)
            }
            WidgetKind::SettingsRow => {
                settings_row_view(snap, widget, tokens, parent_box, editors, menus, map_event)
            }
            WidgetKind::EmptyState => {
                let mut empty = EmptyState::new(widget.props.display_label());
                if !widget.props.hint.is_empty() {
                    empty = empty.message(widget.props.hint.as_str());
                }
                empty.view(tokens)
            }
            WidgetKind::Progress => {
                let mut progress =
                    Progress::new(widget.props.progress, widget.props.progress_max.max(1.0));
                if !widget.props.display_label().is_empty() {
                    progress = progress.label(widget.props.display_label());
                }
                progress.view(tokens)
            }
            WidgetKind::Spinner => {
                Spinner::new(widget.props.display_label(), 0).view(tokens.colors)
            }
        }
    };
    let sized = if is_layout_chrome {
        // layout_* already applied padding / scroll / size chrome.
        apply_flex_child_sizing(
            content,
            &widget.props.layout,
            parent_box,
            parent_direction,
            parent_align_items,
            main_override,
        )
    } else {
        let consume = button_box_consume(&widget.kind, &widget.props.layout);
        apply_widget_box_model(
            content,
            &widget.props.layout,
            parent_box,
            parent_direction,
            parent_align_items,
            main_override,
            consume,
        )
    };
    let faded = apply_opacity(sized, widget.props.layout.opacity);
    let transformed = apply_paint_transform(faded, widget.props.layout.transform);
    probe_transformed_layout(widget.id, transformed, widget.props.layout.transform)
}

fn view_widget_owned<Message>(
    snap: &SemanticSnapshot,
    id: WidgetId,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    parent_align_items: AlignSpec,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    main_override: Option<f32>,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    view_widget_owned_forced(
        snap,
        id,
        tokens,
        parent_box,
        parent_direction,
        parent_align_items,
        editors,
        menus,
        viewport,
        main_override,
        false,
        map_event,
    )
}

#[allow(clippy::too_many_arguments)]
fn view_widget_owned_bridge(
    snap: &SemanticSnapshot,
    id: WidgetId,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    parent_align_items: AlignSpec,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    main_override: Option<f32>,
) -> Element<'static, BridgeEvent> {
    view_widget_owned(
        snap,
        id,
        tokens,
        parent_box,
        parent_direction,
        parent_align_items,
        editors,
        menus,
        viewport,
        main_override,
        std::convert::identity::<BridgeEvent>,
    )
}

fn view_widget_owned_forced<Message>(
    snap: &SemanticSnapshot,
    id: WidgetId,
    tokens: ThemeTokens,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    parent_align_items: AlignSpec,
    editors: Option<&EditorStore>,
    menus: Option<&MenuStore>,
    viewport: Size,
    main_override: Option<f32>,
    force_fixed: bool,
    map_event: impl Fn(BridgeEvent) -> Message + Clone + 'static,
) -> Element<'static, Message>
where
    Message: Clone + 'static,
{
    let Some(widget) = snap.get(id) else {
        return text("").into();
    };
    if widget.props.layout.hidden {
        return text("").into();
    }
    // Absolute is measure-only in iced flow. Fixed paints via the root fixed
    // layer (`force_fixed`). Sticky stays deferred. Product overlays use Nana Overlay.
    if !widget.kind.is_overlay()
        && (widget.props.layout.is_absolute()
            || (widget.props.layout.is_fixed() && !force_fixed)
            || widget.props.layout.position.is_unsupported_positioning())
    {
        return space().width(Length::Shrink).height(Length::Shrink).into();
    }
    let kind = widget.kind;
    let props = widget.props.clone();

    let children = widget.children.clone();
    let wid = widget.id;
    let is_layout_chrome = props.native_component.is_none()
        && !is_raster_resource_slot(&props)
        && matches!(
            kind,
            WidgetKind::Column
                | WidgetKind::Box
                | WidgetKind::SidebarFrame
                | WidgetKind::SettingsCard
                | WidgetKind::Row
                | WidgetKind::Card
        );

    let finish = |content| {
        let sized = if is_layout_chrome {
            apply_flex_child_sizing(
                content,
                &props.layout,
                parent_box,
                parent_direction,
                parent_align_items,
                main_override,
            )
        } else {
            let consume = button_box_consume(&kind, &props.layout);
            apply_widget_box_model(
                content,
                &props.layout,
                parent_box,
                parent_direction,
                parent_align_items,
                main_override,
                consume,
            )
        };
        let faded = apply_opacity(sized, props.layout.opacity);
        let transformed = apply_paint_transform(faded, props.layout.transform);
        probe_transformed_layout(wid, transformed, props.layout.transform)
    };
    if let Some(runtime_view) = qualified_runtime_scene_view(snap, widget) {
        return finish(runtime_view);
    }
    let content = match kind {
        _ if props.native_component.is_some() => {
            let name = props.native_component.as_deref().unwrap_or_default();
            let Some(registry) = active_native_components() else {
                return space().width(Length::Shrink).height(Length::Shrink).into();
            };
            let child_direction = props.layout.direction.unwrap_or(FlexDirection::Column);
            let child_align = props.layout.align_items;
            let native_children = children
                .iter()
                .copied()
                .filter(|&child| is_in_flow_layout(snap, child))
                .map(|child| {
                    view_widget_owned_bridge(
                        snap,
                        child,
                        tokens,
                        ParentBox {
                            width: props.containing_block_width.or(parent_box.width),
                            height: props.containing_block_height.or(parent_box.height),
                        },
                        child_direction,
                        child_align,
                        editors,
                        menus,
                        viewport,
                        None,
                    )
                })
                .collect();
            match registry.view(name, wid, props.clone(), tokens, native_children) {
                Ok(element) => element.map(map_event.clone()),
                Err(error) => {
                    registry.report_error(name, wid, error);
                    space().width(Length::Shrink).height(Length::Shrink).into()
                }
            }
        }
        WidgetKind::Box | WidgetKind::Column
            if let Some(chart) = crate::svg_icon::try_svg_chart_element(snap, wid) =>
        {
            // Preferred single track: structural <svg> via resvg.
            chart
        }
        WidgetKind::Box | WidgetKind::Column
            if children.is_empty() && filled_svg_path_d(&props).is_some() =>
        {
            // DEFER: legacy canvas path-d leaf (see `l1_charts`).
            heatmap_level_canvas_owned(&props)
        }
        WidgetKind::Box | WidgetKind::Column
            if is_raster_resource_slot(&props) && !is_gpu_preview_slot(&props) =>
        {
            raster_resource_view(&props)
        }
        WidgetKind::Column
            if props
                .class_names
                .iter()
                .any(|c| c == "nana-settings-page" || c == "settings-page") =>
        {
            // Public settings page contract: definite scrollport under Shrink ancestors.
            let page_h = definite_scroll_extent(parent_box.height, viewport.height);
            let page_w = definite_scroll_extent(parent_box.width, viewport.width);
            let section_box = ParentBox {
                width: parent_box
                    .width
                    .filter(|w| *w > 0.0)
                    .or_else(|| (viewport.width > 0.0).then_some(viewport.width)),
                height: None,
            };
            let pad_base = parent_box
                .width
                .filter(|w| *w > 0.0)
                .or_else(|| (viewport.width > 0.0).then_some(viewport.width));
            let resolved = props.layout.resolved_padding_against(pad_base);
            let pad = Padding {
                top: if props.layout.padding_top.is_some() || props.layout.padding.is_some() {
                    resolved.top
                } else {
                    20.0
                },
                right: if props.layout.padding_right.is_some() || props.layout.padding.is_some() {
                    resolved.right
                } else {
                    24.0
                },
                bottom: if props.layout.padding_bottom.is_some() || props.layout.padding.is_some() {
                    resolved.bottom
                } else {
                    24.0
                },
                left: if props.layout.padding_left.is_some() || props.layout.padding.is_some() {
                    resolved.left
                } else {
                    24.0
                },
            };
            let gap = props.layout.gap_or(16.0);
            let mut col = column![].spacing(gap).width(Length::Fill);
            for child in children
                .iter()
                .copied()
                .filter(|&id| is_in_flow_layout(snap, id))
            {
                col = col.push(view_widget_owned(
                    snap,
                    child,
                    tokens,
                    section_box,
                    FlexDirection::Column,
                    AlignSpec::Stretch,
                    editors,
                    menus,
                    viewport,
                    None,
                    map_event.clone(),
                ));
            }
            scrollable(container(col).padding(pad).width(Length::Fill))
                .width(page_w)
                .height(page_h)
                .into()
        }
        // GPU preview: documented nana-gpu / data-nana-gpu / agent host contract.
        WidgetKind::Column | WidgetKind::Box if is_gpu_preview_slot(&props) => {
            gpu_preview_placeholder(widget.id, &props, &children, snap, tokens)
        }
        // SpaceBetween Row with a definite cross-axis height: trailing Fill spacer.
        WidgetKind::Row if is_space_between_fixed_row(&props) => {
            let header_h = match props.layout.height {
                Some(LengthSpec::Px(h)) if h > 1.0 => h,
                _ => 48.0,
            };
            let child_parent = ParentBox {
                width: parent_box
                    .width
                    .filter(|w| *w > 0.0)
                    .or_else(|| (viewport.width > 0.0).then_some(viewport.width)),
                height: Some(header_h),
            };
            let gap = props.layout.gap_or(12.0);
            let mut kids: Vec<Element<'static, Message>> = children
                .iter()
                .copied()
                .filter(|&id| is_in_flow_layout(snap, id))
                .map(|child| {
                    view_widget_owned(
                        snap,
                        child,
                        tokens,
                        child_parent,
                        FlexDirection::Row,
                        AlignSpec::Stretch,
                        editors,
                        menus,
                        viewport,
                        None,
                        map_event.clone(),
                    )
                })
                .collect();
            let mut r = row![]
                .spacing(0.0)
                .width(Length::Fill)
                .height(Length::Fixed(header_h))
                .align_y(Alignment::Center);
            if kids.len() >= 2 {
                let trailing = kids.pop().unwrap();
                let leading = kids;
                for el in leading {
                    r = r.push(el);
                    if gap > 0.0 {
                        r = r.push(space().width(Length::Fixed(gap)));
                    }
                }
                r = r.push(space().width(Length::Fill));
                r = r.push(trailing);
            } else {
                for el in kids {
                    r = r.push(el);
                }
            }
            return probe_layout(wid, r.into());
        }
        // Chrome tray: any row-ish node with border + padding + background.
        WidgetKind::Row | WidgetKind::Column | WidgetKind::Box if is_chrome_tray(&props) => {
            let tray_h = match props.layout.height {
                Some(LengthSpec::Px(h)) if h > 1.0 => h,
                _ => 40.0,
            };
            let pad = props.layout.resolved_padding_against(parent_box.width);
            let gap = props.layout.gap_or(2.0);
            let child_parent = ParentBox {
                width: parent_box.width,
                height: Some((tray_h - pad.top - pad.bottom).max(24.0)),
            };
            let mut r = row![].spacing(gap).align_y(Alignment::Center);
            for child in children
                .iter()
                .copied()
                .filter(|&id| is_in_flow_layout(snap, id))
            {
                r = r.push(view_widget_owned(
                    snap,
                    child,
                    tokens,
                    child_parent,
                    FlexDirection::Row,
                    AlignSpec::Stretch,
                    editors,
                    menus,
                    viewport,
                    None,
                    map_event.clone(),
                ));
            }
            let colors = tokens.colors;
            let radius = props
                .layout
                .border_radius
                .unwrap_or(tokens.metrics.radius_md);
            let bw = props.layout.border_width.unwrap_or(1.0);
            let bg = props
                .layout
                .background
                .map(rgba_color)
                .unwrap_or(colors.surface);
            let border_c = props
                .layout
                .border_color
                .map(rgba_color)
                .unwrap_or(colors.faint);
            let tray = container(r)
                .padding(Padding {
                    top: if pad.top > 0.0 { pad.top } else { 4.0 },
                    right: if pad.right > 0.0 { pad.right } else { 4.0 },
                    bottom: if pad.bottom > 0.0 { pad.bottom } else { 4.0 },
                    left: if pad.left > 0.0 { pad.left } else { 4.0 },
                })
                .height(Length::Fixed(tray_h))
                .width(Length::Shrink)
                .style(move |_theme| {
                    let mut style = iced::widget::container::Style::default();
                    style.background = Some(Background::Color(bg));
                    style.border = Border {
                        color: border_c,
                        width: bw.max(1.0),
                        radius: radius.into(),
                    };
                    style
                });
            return probe_layout(wid, tray.into());
        }
        // Compact nowrap row (section headers / toggles): Fixed height, single line.
        WidgetKind::Row if is_compact_nowrap_row(&props) => {
            let row_h = match props.layout.height {
                Some(LengthSpec::Px(h)) if h > 1.0 => h,
                _ => 24.0,
            };
            let child_parent = ParentBox {
                width: parent_box.width,
                height: Some(row_h),
            };
            let gap = props.layout.gap_or(5.0);
            let mut r = row![]
                .spacing(gap)
                .width(Length::Fill)
                .height(Length::Fixed(row_h))
                .align_y(Alignment::Center);
            for child in children
                .iter()
                .copied()
                .filter(|&id| is_in_flow_layout(snap, id))
            {
                r = r.push(view_widget_owned(
                    snap,
                    child,
                    tokens,
                    child_parent,
                    FlexDirection::Row,
                    AlignSpec::Stretch,
                    editors,
                    menus,
                    viewport,
                    None,
                    map_event.clone(),
                ));
            }
            return probe_layout(wid, r.into());
        }
        // Fill+grow column: seed a definite parent_box so nested shells paint.
        WidgetKind::Column if needs_definite_fill_column(&props) => {
            let region_h = parent_box
                .height
                .filter(|h| *h > 1.0)
                .or_else(|| (viewport.height > 80.0).then_some((viewport.height - 80.0).max(120.0)))
                .unwrap_or(520.0);
            let region_w = parent_box
                .width
                .filter(|w| *w > 0.0)
                .or_else(|| (viewport.width > 0.0).then_some(viewport.width * 0.25));
            let child_parent = ParentBox {
                width: region_w,
                height: Some(region_h),
            };
            let gap = props.layout.gap_or(0.0);
            let mut col = column![]
                .spacing(gap)
                .width(Length::Fill)
                .height(Length::Fixed(region_h));
            for child in children
                .iter()
                .copied()
                .filter(|&id| is_in_flow_layout(snap, id))
            {
                col = col.push(view_widget_owned(
                    snap,
                    child,
                    tokens,
                    child_parent,
                    FlexDirection::Column,
                    AlignSpec::Stretch,
                    editors,
                    menus,
                    viewport,
                    None,
                    map_event.clone(),
                ));
            }
            return probe_layout(wid, col.into());
        }
        WidgetKind::SettingsCard => settings_card_view_owned(
            snap, &props, &children, tokens, parent_box, editors, menus, viewport, map_event,
        ),
        WidgetKind::Column | WidgetKind::Box | WidgetKind::SidebarFrame
            if let Some(canvas) = try_composite_filled_svg_paths_owned(snap, &children, &props) =>
        {
            // DEFER: composite path-d canvas fallback (see `l1_charts`).
            canvas
        }
        WidgetKind::Column | WidgetKind::Box | WidgetKind::SidebarFrame => {
            let mut el = wrap_layout_owned(
                true,
                &props,
                children,
                snap,
                tokens,
                parent_box,
                parent_direction,
                editors,
                menus,
                viewport,
                Some(wid),
                map_event,
            );
            // Nested SidebarFrame shells often lack width/height; without Fill
            // they shrink-wrap to 0 under a padded region and never paint nav text.
            if kind == WidgetKind::SidebarFrame
                && props.layout.width.is_none()
                && props.layout.height.is_none()
            {
                let h = parent_box.height.filter(|h| *h > 1.0).unwrap_or(0.0);
                el = container(el)
                    .width(Length::Fill)
                    .height(if h > 1.0 {
                        Length::Fixed(h)
                    } else {
                        Length::Fill
                    })
                    .into();
            }
            el
        }
        WidgetKind::Row => wrap_layout_owned(
            false,
            &props,
            children,
            snap,
            tokens,
            parent_box,
            parent_direction,
            editors,
            menus,
            viewport,
            Some(wid),
            map_event,
        ),
        WidgetKind::Card => {
            let body_props = card_body_props(&props);
            let body = wrap_layout_owned(
                true,
                &body_props,
                children,
                snap,
                tokens,
                parent_box,
                parent_direction,
                editors,
                menus,
                viewport,
                Some(wid),
                map_event,
            );
            let mut card = Card::new(body);
            if !props.label.is_empty() {
                card = card.title(props.label.clone());
            }
            card = card_with_css_height(card, &props.layout, parent_box);
            card = card_with_css_padding(card, &props.layout, parent_box);
            card.view(tokens)
        }
        WidgetKind::Text => {
            if !children.is_empty() && owned_display(&props).is_empty() {
                wrap_layout_owned(
                    text_host_column_axis(&props.layout),
                    &props,
                    children,
                    snap,
                    tokens,
                    parent_box,
                    parent_direction,
                    editors,
                    menus,
                    viewport,
                    Some(wid),
                    map_event,
                )
            } else {
                label_text(
                    owned_display(&props),
                    props.size,
                    &props.layout,
                    parent_box.width,
                )
            }
        }
        WidgetKind::Icon => {
            let size = crate::svg_icon::resolve_icon_size(&props);
            let color = props
                .layout
                .color
                .map(rgba_color)
                .unwrap_or(tokens.colors.text);
            if let Some(handle) = crate::svg_icon::try_svg_handle(snap, wid) {
                crate::svg_icon::svg_icon_element(handle, size, color)
            } else if let Some(kind) = resolve_icon_from_props(&props) {
                icon(kind, size, color)
            } else {
                crate::svg_icon::empty_icon_placeholder(size)
            }
        }
        WidgetKind::Button => {
            let map = map_event.clone();
            button_view_owned(
                &props,
                &children,
                snap,
                tokens,
                parent_box.width,
                map(BridgeEvent::Press { id: wid }),
            )
        }
        WidgetKind::Chip => {
            let label = owned_display(&props);
            if label.is_empty() {
                // Drop Vue object-stringified chips (`[object Object]`).
                space().width(Length::Shrink).height(Length::Shrink).into()
            } else {
                let map = map_event.clone();
                let kind = if props.active {
                    ButtonKind::Selected
                } else {
                    ButtonKind::Subtle
                };
                let btn = Button::label(label)
                    .kind(kind)
                    .size(ControlSize::Small)
                    .disabled(props.disabled)
                    .on_press(map(BridgeEvent::Select { id: wid }));
                apply_button_layout_chrome(btn, &props, parent_box.width).view(tokens)
            }
        }
        WidgetKind::Switch => {
            let map = map_event.clone();
            let mut control = Switch::new(props.toggled, owned_display(&props))
                .disabled(props.disabled)
                .on_toggle(move |value| map(BridgeEvent::Toggle { id: wid, value }));
            if !props.hint.is_empty() {
                control = control.hint(props.hint.clone());
            }
            control.view(tokens)
        }
        WidgetKind::Checkbox => {
            let map = map_event.clone();
            Checkbox::new(props.toggled, owned_display(&props))
                .disabled(props.disabled)
                .on_toggle(move |value| map(BridgeEvent::Toggle { id: wid, value }))
                .view(tokens)
        }
        WidgetKind::Input => {
            let map = map_event.clone();
            let placeholder = if props.placeholder.is_empty() {
                props.hint.clone()
            } else {
                props.placeholder.clone()
            };
            let input = Input::new(placeholder, props.value.clone())
                .id(hosted_text_widget_id(wid))
                .size(props.size)
                .disabled(props.disabled || props.loading)
                .invalid(props.invalid)
                .secure(props.secure);
            if props.read_only {
                input.view(tokens)
            } else {
                input
                    .on_input(move |value| map(BridgeEvent::Input { id: wid, value }))
                    .view(tokens)
            }
        }
        WidgetKind::Textarea => textarea_view_owned(&props, wid, tokens, editors, menus, map_event),
        WidgetKind::Range => {
            let map = map_event.clone();
            let min = props.min;
            let max = props.max.max(min + f32::EPSILON);
            let value = props.number.clamp(min, max);
            let mut range = RangeField::new(min..=max, value, move |v| {
                map(BridgeEvent::Change {
                    id: wid,
                    value: f64::from(v),
                })
            });
            if !props.label.is_empty() {
                range = range.label(props.label.clone());
            }
            if !props.unit.is_empty() {
                range = range.unit(props.unit.clone());
            }
            range.size(props.size).view(tokens)
        }
        WidgetKind::Tabs => selection_tabs_owned(&props, wid, tokens, map_event),
        WidgetKind::Segmented => selection_segmented_owned(&props, wid, tokens, map_event),
        WidgetKind::Select => selection_select_owned(&props, wid, tokens, map_event),
        WidgetKind::Dialog => overlay_dialog_owned(
            snap, &props, wid, &children, tokens, parent_box, editors, menus, viewport, map_event,
        ),
        WidgetKind::Drawer => overlay_drawer_owned(
            snap, &props, wid, &children, tokens, parent_box, editors, menus, viewport, map_event,
        ),
        WidgetKind::Popover => overlay_popover_owned(
            snap, &props, wid, &children, tokens, parent_box, editors, menus, viewport, map_event,
        ),
        WidgetKind::ContextMenu => {
            overlay_context_menu_owned(&props, wid, tokens, viewport, menus, map_event)
        }
        WidgetKind::SidebarRow => {
            let map = map_event.clone();
            let state = if props.disabled {
                SidebarRowState::Disabled
            } else if props.active {
                SidebarRowState::Active
            } else {
                SidebarRowState::Idle
            };
            let (leading, label) =
                resolve_row_leading_and_label_owned(snap, &props, &children, tokens);
            let mut row = SidebarRow::new(label)
                .state(state)
                .tone(SidebarRowTone::Default)
                .gap(props.layout.gap_or(6.0))
                .on_select(map(BridgeEvent::Select { id: wid }));
            if let Some(icon_el) = leading {
                row = row.leading(icon_el);
            }
            row.view(tokens)
        }
        WidgetKind::ListItem => {
            let map = map_event.clone();
            let (leading, label) =
                resolve_row_leading_and_label_owned(snap, &props, &children, tokens);
            let mut item = ListItem::label(label)
                .selected(props.active)
                .disabled(props.disabled)
                .gap(props.layout.gap_or(8.0))
                .on_select(map(BridgeEvent::Select { id: wid }));
            if let Some(icon_el) = leading {
                item = item.leading(icon_el);
            }
            item.view(tokens)
        }
        WidgetKind::SettingsRow => settings_row_view_owned(
            snap, &props, &children, tokens, parent_box, editors, menus, viewport, map_event,
        ),
        WidgetKind::EmptyState => {
            let mut empty = EmptyState::new(owned_display(&props));
            if !props.hint.is_empty() {
                empty = empty.message(props.hint.clone());
            }
            empty.view(tokens)
        }
        WidgetKind::Progress => {
            let mut progress = Progress::new(props.progress, props.progress_max.max(1.0));
            let label = owned_display(&props);
            if !label.is_empty() {
                progress = progress.label(label);
            }
            progress.view(tokens)
        }
        WidgetKind::Spinner => Spinner::new(owned_display(&props), 0).view(tokens.colors),
    };
    finish(content)
}

include!("transform.rs");
include!("layout_flow.rs");
include!("button.rs");
include!("settings.rs");
include!("layout_convert.rs");
include!("l1_charts.rs");
include!("surface.rs");
include!("overlay.rs");
include!("selection.rs");
include!("tests.rs");
