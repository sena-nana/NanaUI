//! Snapshot feedback; no product state is stored here.
use super::*;

pub(super) fn exercise_feedback_action_lifecycle(
    document: &mut RuntimeDocument,
    viewport: LayoutViewport,
    shaper: &mut NanaTextShaper,
    fixture: Fixture,
    target: StableNodeId,
    action: FeedbackActionFixture,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let parent_inert = document
        .context()
        .world()
        .interaction(target)
        .is_some_and(|interaction| !interaction.pointer_events && !interaction.focusable);
    let action_bounds = document
        .context()
        .world()
        .layout_box(action.action.stable_id())
        .expect("mounted feedback action layout");
    let mut adapter = RuntimeInputAdapter::default();
    let action_x = action_bounds.x + action_bounds.width / 2.0;
    let action_y = action_bounds.y + action_bounds.height / 2.0;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Down, action_x, action_y),
    )?;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Up, action_x, action_y),
    )?;
    let first_click_once = *action
        .activations
        .lock()
        .expect("feedback activation count")
        == 1;

    set_feedback_action(
        document,
        fixture.component,
        target,
        Some(action.replacement.stable_id()),
    )?;
    let replacement_update = document.flush(viewport, shaper)?;
    let old_parked_without_ghost = parked_without_ghost(
        document,
        action.action.stable_id(),
        action_bounds,
        &replacement_update.accessibility.removed,
    );
    let replacement_bounds = document
        .context()
        .world()
        .layout_box(action.replacement.stable_id())
        .expect("replacement feedback action layout");

    set_feedback_action(document, fixture.component, target, None)?;
    let removal_update = document.flush(viewport, shaper)?;
    let replacement_parked_without_ghost = parked_without_ghost(
        document,
        action.replacement.stable_id(),
        replacement_bounds,
        &removal_update.accessibility.removed,
    );

    set_feedback_action(
        document,
        fixture.component,
        target,
        Some(action.action.stable_id()),
    )?;
    document.flush(viewport, shaper)?;
    let remounted_bounds = document
        .context()
        .world()
        .layout_box(action.action.stable_id())
        .expect("remounted feedback action layout");
    let remounted_x = remounted_bounds.x + remounted_bounds.width / 2.0;
    let remounted_y = remounted_bounds.y + remounted_bounds.height / 2.0;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Down, remounted_x, remounted_y),
    )?;
    adapter.dispatch(
        document.context_mut(),
        document_id,
        &pointer(PointerPhase::Up, remounted_x, remounted_y),
    )?;
    let remount_preserved_handler = *action
        .activations
        .lock()
        .expect("feedback activation count")
        == 2;
    set_feedback_action(document, fixture.component, target, None)?;
    let post_click_removal = document.flush(viewport, shaper)?;
    let focused_action_removed_without_ghost = parked_without_ghost(
        document,
        action.action.stable_id(),
        remounted_bounds,
        &post_click_removal.accessibility.removed,
    );
    set_feedback_action(
        document,
        fixture.component,
        target,
        Some(action.action.stable_id()),
    )?;
    document.flush(viewport, shaper)?;
    let final_parent_inert = document
        .context()
        .world()
        .interaction(target)
        .is_some_and(|interaction| !interaction.pointer_events && !interaction.focusable);
    let final_child_order = document
        .context()
        .world()
        .node(target)
        .is_some_and(|node| node.children == [action.action.stable_id()]);

    Ok(parent_inert
        && first_click_once
        && old_parked_without_ghost
        && replacement_parked_without_ghost
        && remount_preserved_handler
        && focused_action_removed_without_ghost
        && document.context().world().focused(document_id) != Some(action.action.stable_id())
        && final_parent_inert
        && final_child_order)
}

pub(super) fn set_feedback_action(
    document: &mut RuntimeDocument,
    component: Component,
    target: StableNodeId,
    action: Option<StableNodeId>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let changed = match component {
        Component::EmptyState => document
            .context_mut()
            .set_empty_state_action(Entity::<RuntimeEmptyState>::from_stable_id(target), action)?,
        Component::LabeledValue => document.context_mut().set_labeled_value_action(
            Entity::<RuntimeLabeledValue>::from_stable_id(target),
            action,
        )?,
        _ => false,
    };
    Ok(changed)
}

pub(super) fn parked_without_ghost(
    document: &RuntimeDocument,
    action: StableNodeId,
    old_bounds: nana_ui::runtime::LayoutBox,
    accessibility_removed: &[StableNodeId],
) -> bool {
    let world = document.context().world();
    world.mount_state(action) == Some(MountState::Parked)
        && !world.document_order(document.document()).contains(&action)
        && !document
            .scene()
            .primitives()
            .any(|primitive| primitive.node == action)
        && world.hit_test(
            document.document(),
            old_bounds.x + old_bounds.width / 2.0,
            old_bounds.y + old_bounds.height / 2.0,
        ) != Some(action)
        && accessibility_removed.contains(&action)
}
