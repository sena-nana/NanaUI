mod app_shell;
mod desktop_shell;
mod title_bar;

pub use app_shell::AppShell;
pub use desktop_shell::DesktopShell;
pub use title_bar::{AppTitleBar, AppTitleBarControls, WindowChromeAction};

use std::collections::HashSet;
use std::sync::Arc;

use crate::{
    AppContext, ComponentView, DocumentId, Entity, FrameworkError, MutationQueue, NodeKind,
    StableNodeId, UiWorld,
};

pub(super) const SLOT_PADDING: f32 = nana_ui_core::space::SM;
pub(super) const OVERLAY_Z_INDEX: i32 = 1;
pub(super) const STATUS_OVERLAY_Z_INDEX: i32 = 2;

pub(super) fn document_of(
    context: &AppContext,
    id: StableNodeId,
) -> Result<DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

pub(super) fn view_is<C: ComponentView>(context: &AppContext, id: StableNodeId) -> bool {
    context
        .read(Entity::<C>::from_stable_id(id), |_| ())
        .is_ok()
}

pub(super) fn is_scroll_node(context: &AppContext, id: StableNodeId) -> bool {
    matches!(
        context.world().node(id).map(|node| node.kind),
        Some(NodeKind::Element { tag }) if tag == "scroll"
    )
}

pub(super) fn reconcile_ids(
    context: &mut AppContext,
    parent: StableNodeId,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    let ordered = ordered
        .iter()
        .copied()
        .filter(|id| *id != parent && context.world().contains(*id))
        .collect::<Vec<_>>();
    let current = context
        .world()
        .node(parent)
        .ok_or(FrameworkError::MissingView(parent))?
        .children
        .clone();
    if current.as_slice() == ordered.as_slice() {
        return Ok(false);
    }
    let keep = ordered.iter().copied().collect::<HashSet<_>>();
    let mut mutations = MutationQueue::new();
    for child in &current {
        if !keep.contains(child) {
            mutations.park_subtree(*child);
        }
    }
    for child in ordered {
        mutations.insert(parent, child, None);
    }
    context.commit_mutations(mutations)?;
    Ok(true)
}

pub(super) fn patch_layout(
    world: &UiWorld,
    mutations: &mut MutationQueue,
    id: StableNodeId,
    patch: impl FnOnce(&mut nana_ui_core::LayoutStyle),
) {
    if world.node(id).is_none() {
        return;
    }
    let mut style = world.node_style(id).cloned().unwrap_or_default();
    patch(Arc::make_mut(&mut style.layout));
    if world.node_style(id) != Some(&style) {
        mutations.set_style(id, style);
    }
}

pub(super) fn finite_positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}
