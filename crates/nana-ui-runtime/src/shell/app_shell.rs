use super::title_bar::{AppTitleBar, is_title_bar_column_tag, node_tag};
use super::{OVERLAY_Z_INDEX, document_of, patch_layout, reconcile_ids, view_is};
use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, FlexDirection, JustifySpec, LengthSpec, PositionSpec, SemanticColorRole,
    TITLE_BAR_HEIGHT,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle, OverlayHost,
    StableNodeId, TextContent, UiWorld,
};

/// Title bar + fill body + optional overlay stack sibling.
#[derive(Debug, Clone, PartialEq)]
pub struct AppShell {
    pub title_bar: Option<StableNodeId>,
    pub body: Option<StableNodeId>,
    pub overlay: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl AppShell {
    pub fn new() -> Self {
        Self {
            title_bar: None,
            body: None,
            overlay: None,
            style: NodeStyle::default(),
        }
    }

    pub fn title_bar(mut self, title_bar: StableNodeId) -> Self {
        self.title_bar = Some(title_bar);
        self
    }

    pub fn body(mut self, body: StableNodeId) -> Self {
        self.body = Some(body);
        self
    }

    pub fn overlay(mut self, overlay: StableNodeId) -> Self {
        self.overlay = Some(overlay);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(SemanticColorRole::Text);
        let layout = Arc::make_mut(&mut style.layout);
        layout.paint_colors.background = None;
        layout.direction = Some(FlexDirection::Column);
        layout.align_items = AlignSpec::Stretch;
        layout.justify_content = JustifySpec::Start;
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.position = PositionSpec::Relative;
        style
    }

    fn project_slots(&self, world: &UiWorld, mutations: &mut MutationQueue) {
        if let Some(title_bar) = self.title_bar {
            patch_layout(world, mutations, title_bar, |layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
                layout.min_height = Some(LengthSpec::Px(TITLE_BAR_HEIGHT));
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
            });
        }
        if let Some(body) = self.body {
            patch_layout(world, mutations, body, |layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Fill);
                layout.min_height = Some(LengthSpec::Px(0.0));
                layout.flex_grow = Some(1.0);
                layout.flex_shrink = Some(1.0);
                layout.position = PositionSpec::Static;
            });
        }
        if let Some(overlay) = self.overlay {
            patch_layout(world, mutations, overlay, |layout| {
                layout.position = PositionSpec::Absolute;
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_right = Some(LengthSpec::Px(0.0));
                layout.offset_bottom = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Fill);
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
                layout.z_index = Some(layout.z_index.unwrap_or(OVERLAY_Z_INDEX));
            });
            // An `OverlayHost` decides this from the active overlay's kind, and
            // that activation changes without re-projecting the shell, so the
            // host has to own the field. Any other node the caller supplied as
            // the overlay region takes the pointer while it holds content: a
            // child that is not hidden. Its children change without the shell
            // changing (a `when` placed as the region swapping its branch), so
            // the shell watches them (`assemble_app_shell`).
            if world.overlay_host(overlay).is_none() {
                let has_content = world.node(overlay).is_some_and(|node| {
                    node.children.iter().any(|child| {
                        !world
                            .node_style(*child)
                            .is_some_and(|style| style.layout.omits_box())
                    })
                });
                let interaction = InteractionState {
                    pointer_events: has_content,
                    focusable: false,
                };
                if world.interaction(overlay) != Some(interaction) {
                    mutations.set_interaction(overlay, interaction);
                }
            }
        }
    }
}

impl Default for AppShell {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentView for AppShell {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::APP_SHELL.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_app_shell),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// Patches the title bar, body and overlay nodes, which other components own and project.
    const ALWAYS_REPROJECT: bool = true;

    /// Whether the overlay region takes the pointer follows its children,
    /// which the shell watches.
    fn wants_child_reproject() -> bool {
        true
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "app-shell".into(),
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
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
        self.project_slots(world, mutations);
    }
}

impl AppContext {
    /// Reconcile title bar, body, and optional overlay, then re-project slots.
    ///
    /// Host-mounted slots are kept. A title bar or overlay node is created
    /// only when that slot was already assigned or already present as a child.
    pub fn assemble_app_shell(&mut self, shell: Entity<AppShell>) -> Result<bool, FrameworkError> {
        let parent = shell.stable_id();
        let document = document_of(self, parent)?;
        let snapshot = self.read(shell, Clone::clone)?;
        let title_bar = resolve_app_title_bar(self, document, parent, &snapshot)?;
        let overlay = resolve_app_overlay(self, document, parent, &snapshot)?;
        // The region's content decides whether it takes the pointer
        // (`project_slots`); an `OverlayHost` decides that itself.
        self.watch_child_structure(
            parent,
            overlay.filter(|region| self.world().overlay_host(*region).is_none()),
        );
        let body = snapshot
            .body
            .filter(|id| {
                self.world().contains(*id) && Some(*id) != title_bar && Some(*id) != overlay
            })
            .or_else(|| find_app_shell_body_child(self, parent, title_bar, overlay));
        let fields_changed =
            title_bar != snapshot.title_bar || body != snapshot.body || overlay != snapshot.overlay;
        if fields_changed {
            self.update_component(shell, |shell, _| {
                shell.title_bar = title_bar;
                shell.body = body;
                shell.overlay = overlay;
            })?;
        }
        let children = app_shell_child_ids(title_bar, body, overlay);
        let mut changed = reconcile_ids(self, parent, &children)?;
        self.reproject_component(shell)?;
        if let Some(title_bar) = title_bar {
            changed |= self
                .assemble_app_title_bar(Entity::<AppTitleBar>::from_stable_id(title_bar))
                .unwrap_or(false);
        }
        Ok(changed || fields_changed)
    }
}

fn resolve_app_title_bar(
    context: &mut AppContext,
    document: DocumentId,
    parent: StableNodeId,
    snapshot: &AppShell,
) -> Result<Option<StableNodeId>, FrameworkError> {
    if let Some(id) = snapshot
        .title_bar
        .filter(|id| context.world().contains(*id))
    {
        return Ok(Some(id));
    }
    if let Some(id) = find_title_bar_child(context, parent) {
        return Ok(Some(id));
    }
    if snapshot.title_bar.is_none() {
        return Ok(None);
    }
    let title = recovered_title(context, parent).unwrap_or_else(|| Arc::from(""));
    Ok(Some(
        context
            .create_detached_component(document, AppTitleBar::new(title))?
            .stable_id(),
    ))
}

fn resolve_app_overlay(
    context: &mut AppContext,
    document: DocumentId,
    parent: StableNodeId,
    snapshot: &AppShell,
) -> Result<Option<StableNodeId>, FrameworkError> {
    if let Some(id) = snapshot.overlay.filter(|id| context.world().contains(*id)) {
        return Ok(Some(id));
    }
    if let Some(id) = find_overlay_child(context, parent) {
        return Ok(Some(id));
    }
    if snapshot.overlay.is_none() {
        return Ok(None);
    }
    Ok(Some(
        context
            .create_detached_component(document, OverlayHost::new())?
            .stable_id(),
    ))
}

fn app_shell_child_ids(
    title_bar: Option<StableNodeId>,
    body: Option<StableNodeId>,
    overlay: Option<StableNodeId>,
) -> Vec<StableNodeId> {
    let mut children = Vec::new();
    if let Some(title_bar) = title_bar {
        children.push(title_bar);
    }
    if let Some(body) = body {
        children.push(body);
    }
    if let Some(overlay) = overlay {
        children.push(overlay);
    }
    children
}

fn find_app_shell_body_child(
    context: &AppContext,
    parent: StableNodeId,
    title_bar: Option<StableNodeId>,
    overlay: Option<StableNodeId>,
) -> Option<StableNodeId> {
    context
        .world()
        .node(parent)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| {
            Some(id) != title_bar
                && Some(id) != overlay
                && !is_title_bar_column_tag(node_tag(context.world(), id).as_deref())
        })
}

fn find_title_bar_child(context: &AppContext, parent: StableNodeId) -> Option<StableNodeId> {
    context
        .world()
        .node(parent)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| {
            view_is::<AppTitleBar>(context, id)
                || matches!(
                    context.world().node(id).map(|node| node.kind),
                    Some(NodeKind::Element { tag })
                        if tag.contains("title-bar") || tag.contains("titlebar")
                )
        })
}

fn find_overlay_child(context: &AppContext, parent: StableNodeId) -> Option<StableNodeId> {
    context
        .world()
        .node(parent)
        .into_iter()
        .flat_map(|node| node.children)
        .find(|&id| {
            view_is::<OverlayHost>(context, id)
                || matches!(
                    context.world().node(id).map(|node| node.kind),
                    Some(NodeKind::Element { tag }) if tag.contains("overlay")
                )
        })
}

fn recovered_title(context: &AppContext, parent: StableNodeId) -> Option<Arc<str>> {
    let id = find_title_bar_child(context, parent)?;
    if let Ok(title) = context.read(Entity::<AppTitleBar>::from_stable_id(id), |bar| {
        Arc::clone(&bar.title)
    }) && !title.is_empty()
    {
        return Some(title);
    }
    if let Some(text) = context.world().text(id).filter(|text| !text.is_empty()) {
        return Some(Arc::from(text));
    }
    context
        .world()
        .accessibility(id)
        .and_then(|state| state.label.clone())
        .filter(|label| !label.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use super::super::title_bar::{
        CENTER_COLUMN_TAG, LEADING_COLUMN_TAG, TRAILING_COLUMN_TAG, node_tag,
    };
    use crate::{
        AppContext, Card, DocumentId, LayoutViewport, NodeKind, StableNodeId, Text, UiWorld,
    };

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn is_descendant(world: &UiWorld, ancestor: StableNodeId, node: StableNodeId) -> bool {
        let mut current = world.parent_id(node);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = world.parent_id(parent);
        }
        false
    }

    #[test]
    fn app_shell_stacks_title_then_fill_body_with_overlay_out_of_flow() {
        let mut context = AppContext::new();
        let title = context
            .create_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_component(document(), Text::new("workspace"))
            .unwrap();
        let overlay = context
            .create_component(document(), Text::new("overlay"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id())
                    .overlay(overlay.stable_id()),
            )
            .unwrap();
        context.append_child(shell, title).unwrap();
        context.append_child(shell, body).unwrap();
        context.append_child(shell, overlay).unwrap();

        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().kind,
            NodeKind::Element {
                tag: "app-shell".into(),
            }
        );
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id(), overlay.stable_id()]
        );
        let title_layout = &context
            .world()
            .node_style(title.stable_id())
            .unwrap()
            .layout;
        assert_eq!(title_layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(title_layout.flex_grow, Some(0.0));
        let shell_layout = &context
            .world()
            .node_style(shell.stable_id())
            .unwrap()
            .layout;
        assert!(
            shell_layout.padding.is_none()
                && shell_layout.padding_left.is_none()
                && shell_layout.padding_right.is_none()
                && shell_layout.padding_top.is_none()
                && shell_layout.padding_bottom.is_none()
        );
        let body_layout = &context.world().node_style(body.stable_id()).unwrap().layout;
        assert_eq!(body_layout.flex_grow, Some(1.0));
        assert_eq!(body_layout.height, Some(LengthSpec::Fill));
        assert_eq!(body_layout.min_height, Some(LengthSpec::Px(0.0)));
        assert_eq!(body_layout.position, PositionSpec::Static);
        let overlay_layout = &context
            .world()
            .node_style(overlay.stable_id())
            .unwrap()
            .layout;
        assert_eq!(overlay_layout.position, PositionSpec::Absolute);
        assert_eq!(overlay_layout.width, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.height, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.flex_grow, Some(0.0));

        context
            .layout_document(document(), LayoutViewport::new(800.0, 400.0))
            .unwrap();
        let title_box = context
            .world()
            .canonical_layout_box(title.stable_id())
            .unwrap();
        let body_box = context
            .world()
            .canonical_layout_box(body.stable_id())
            .unwrap();
        let overlay_box = context
            .world()
            .canonical_layout_box(overlay.stable_id())
            .unwrap();
        let shell_box = context
            .world()
            .canonical_layout_box(shell.stable_id())
            .unwrap();
        assert_eq!(title_box.height, TITLE_BAR_HEIGHT);
        assert_eq!(title_box.y, shell_box.y);
        assert_eq!(body_box.y, title_box.y + title_box.height);
        assert_eq!(body_box.height, 400.0 - TITLE_BAR_HEIGHT);
        assert_eq!(overlay_box.x, shell_box.x);
        assert_eq!(overlay_box.y, shell_box.y);
        assert_eq!(overlay_box.width, shell_box.width);
        assert_eq!(overlay_box.height, shell_box.height);
        assert_eq!(shell_box.height, 400.0);
    }

    #[test]
    fn title_bar_drops_css_layout_background() {
        let mut layout = nana_ui_core::LayoutStyle::default();
        layout.paint_colors.background = Some(nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0]));
        let mut context = AppContext::new();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").style(NodeStyle {
                    layout: Arc::new(layout),
                    ..NodeStyle::default()
                }),
            )
            .unwrap();
        let style = context.world().node_style(bar.stable_id()).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Titlebar));
        assert!(
            style.layout.paint_colors.background.is_none(),
            "CSS fill must not cover Titlebar token alpha"
        );
    }

    #[test]
    fn app_shell_default_background_is_none_and_title_bar_keeps_surface() {
        let mut context = AppContext::new();
        let title = context
            .create_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let shell = context
            .create_component(document(), AppShell::new().title_bar(title.stable_id()))
            .unwrap();
        context.append_child(shell, title).unwrap();

        assert_eq!(
            context
                .world()
                .node_style(shell.stable_id())
                .unwrap()
                .background,
            None
        );
        assert_eq!(
            context
                .world()
                .node_style(title.stable_id())
                .unwrap()
                .background,
            Some(SemanticColorRole::Titlebar)
        );
    }

    #[test]
    fn app_shell_preserves_caller_set_background() {
        let mut context = AppContext::new();
        let shell = context
            .create_component(
                document(),
                AppShell::new().style(NodeStyle {
                    background: Some(SemanticColorRole::Background),
                    ..NodeStyle::default()
                }),
            )
            .unwrap();

        assert_eq!(
            context
                .world()
                .node_style(shell.stable_id())
                .unwrap()
                .background,
            Some(SemanticColorRole::Background)
        );
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let center = context
            .create_component(document(), Text::new("crumbs"))
            .unwrap();
        let body = context
            .create_component(document(), Text::new("workspace"))
            .unwrap();
        let bar = context
            .create_component(
                document(),
                AppTitleBar::new("Nana").center(center.stable_id()),
            )
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(bar.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();
        let _ = context.take_system_work();
        context.reproject_component(bar).unwrap();
        context.reproject_component(shell).unwrap();
        assert!(context.take_system_work().is_empty());
    }

    #[test]
    fn assemble_app_shell_reconciles_title_bar_and_body() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();

        assert!(context.assemble_app_shell(shell).unwrap());
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id()]
        );
        let title_layout = &context
            .world()
            .node_style(title.stable_id())
            .unwrap()
            .layout;
        assert_eq!(title_layout.height, Some(LengthSpec::Px(TITLE_BAR_HEIGHT)));
        assert_eq!(
            title_layout.min_height,
            Some(LengthSpec::Px(TITLE_BAR_HEIGHT))
        );
        assert_eq!(title_layout.flex_grow, Some(0.0));
        let body_layout = &context.world().node_style(body.stable_id()).unwrap().layout;
        assert_eq!(body_layout.flex_grow, Some(1.0));
        assert_eq!(body_layout.height, Some(LengthSpec::Fill));
        assert_eq!(body_layout.min_height, Some(LengthSpec::Px(0.0)));
        assert!(
            context
                .read(shell, |shell| shell.overlay)
                .unwrap()
                .is_none()
        );
        assert_eq!(context.world().text(title.stable_id()), Some(""));
        assert_eq!(
            context
                .world()
                .accessibility(title.stable_id())
                .and_then(|state| state.label.clone())
                .as_deref(),
            Some("Nana")
        );
        let columns = context.world().node(title.stable_id()).unwrap().children;
        assert_eq!(
            node_tag(context.world(), columns[1]).as_deref(),
            Some(CENTER_COLUMN_TAG)
        );
        let title_label = context
            .world()
            .node(columns[1])
            .unwrap()
            .children
            .first()
            .copied()
            .expect("center column title");
        assert_eq!(context.world().text(title_label), Some("Nana"));
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        assert!(
            !is_descendant(context.world(), title.stable_id(), body.stable_id()),
            "body must stay a sibling of the title bar, not a descendant"
        );
        let title_box = context
            .world()
            .canonical_layout_box(title.stable_id())
            .unwrap();
        let body_box = context
            .world()
            .canonical_layout_box(body.stable_id())
            .unwrap();
        assert!(
            body_box.y + 0.5 >= title_box.y + TITLE_BAR_HEIGHT,
            "body.y={} must sit below title bar y={} height={}",
            body_box.y,
            title_box.y,
            title_box.height
        );
    }

    #[test]
    fn assemble_app_shell_keeps_nested_body_out_of_title_bar() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Card::new())
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();
        context.append_child(shell, title).unwrap();
        context.append_child(title, body).unwrap();

        assert!(context.assemble_app_shell(shell).unwrap());
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id()]
        );
        assert_eq!(
            context.read(shell, |shell| shell.body).unwrap(),
            Some(body.stable_id())
        );
        assert!(
            !is_descendant(context.world(), title.stable_id(), body.stable_id()),
            "nested AppShell body must be lifted out of the title bar"
        );
        context
            .layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        let title_box = context
            .world()
            .canonical_layout_box(title.stable_id())
            .unwrap();
        let body_box = context
            .world()
            .canonical_layout_box(body.stable_id())
            .unwrap();
        assert!(
            body_box.y + 0.5 >= title_box.y + TITLE_BAR_HEIGHT,
            "body.y={} must sit below title bar y={} height={}",
            body_box.y,
            title_box.y,
            title_box.height
        );
        let leading = context
            .world()
            .node(title.stable_id())
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|&id| node_tag(context.world(), id).as_deref() == Some(LEADING_COLUMN_TAG))
            .expect("leading column");
        assert!(
            !context
                .world()
                .node(leading)
                .unwrap()
                .children
                .contains(&body.stable_id()),
            "body must not land in the title-bar leading column"
        );
    }

    #[test]
    fn assemble_app_shell_empty_title_bar_still_has_columns() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new(""))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();

        assert!(context.assemble_app_shell(shell).unwrap());
        assert_eq!(context.world().text(title.stable_id()), Some(""));
        let columns = context.world().node(title.stable_id()).unwrap().children;
        assert_eq!(columns.len(), 3, "empty title still uses three columns");
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
        let title_label = context
            .world()
            .node(columns[1])
            .unwrap()
            .children
            .first()
            .copied()
            .expect("center column title");
        assert_eq!(context.world().text(title_label), Some(""));
    }

    #[test]
    fn assemble_app_shell_is_idempotent() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )
            .unwrap();
        context.assemble_app_shell(shell).unwrap();
        let children = context.world().node(shell.stable_id()).unwrap().children;

        assert!(!context.assemble_app_shell(shell).unwrap());
        assert_eq!(
            context
                .read(shell, |shell| (shell.title_bar, shell.body, shell.overlay))
                .unwrap(),
            (Some(title.stable_id()), Some(body.stable_id()), None)
        );
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            children
        );
    }

    #[test]
    fn assemble_app_shell_keeps_overlay_absolute_fill() {
        let mut context = AppContext::new();
        let title = context
            .create_detached_component(document(), AppTitleBar::new("Nana"))
            .unwrap();
        let body = context
            .create_detached_component(document(), Text::new("workspace"))
            .unwrap();
        let overlay = context
            .create_detached_component(document(), Text::new("overlay"))
            .unwrap();
        let shell = context
            .create_component(
                document(),
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id())
                    .overlay(overlay.stable_id()),
            )
            .unwrap();

        context.assemble_app_shell(shell).unwrap();
        assert_eq!(
            context.world().node(shell.stable_id()).unwrap().children,
            vec![title.stable_id(), body.stable_id(), overlay.stable_id()]
        );
        let overlay_layout = &context
            .world()
            .node_style(overlay.stable_id())
            .unwrap()
            .layout;
        assert_eq!(overlay_layout.position, PositionSpec::Absolute);
        assert_eq!(overlay_layout.offset_top, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.offset_right, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.offset_bottom, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.offset_left, Some(LengthSpec::Px(0.0)));
        assert_eq!(overlay_layout.width, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.height, Some(LengthSpec::Fill));
        assert_eq!(overlay_layout.flex_grow, Some(0.0));
        assert_eq!(overlay_layout.z_index, Some(OVERLAY_Z_INDEX));
        assert_eq!(
            context.read(shell, |shell| shell.overlay).unwrap(),
            Some(overlay.stable_id())
        );
    }
}
