//! Issue #104, first cut: a node's [`crate::NodeStyle::color_overrides`]
//! answer for it and its whole subtree.

use super::*;
use nana_ui_core::{ColorOverrides, SemanticColor, SemanticColorRole, StatusTone};

const PURPLE: SemanticColor = SemanticColor::rgb8(130, 60, 220);
const ORANGE: SemanticColor = SemanticColor::rgb8(230, 120, 20);

fn scoped_stack(overrides: Option<ColorOverrides>) -> crate::Stack {
    let mut stack = crate::Stack::column(0.0);
    stack.style_mut().color_overrides = overrides.map(Arc::new);
    stack
}

fn layout(context: &mut AppContext, document: DocumentId) {
    context
        .layout_document(document, crate::LayoutViewport::new(320.0, 200.0))
        .unwrap();
    let work = context.compat_world_mut().take_system_work();
    context
        .compat_world_mut()
        .resolve_styles(&work.style)
        .unwrap();
}

fn switch_track(context: &AppContext, id: StableNodeId) -> [f32; 4] {
    match context.world().extract_nodes(&[id])[0]
        .component_geometry
        .as_deref()
    {
        Some(crate::ComponentGeometry::Switch {
            track_background, ..
        }) => *track_background,
        other => panic!("not a switch geometry: {other:?}"),
    }
}

fn badge_foreground(context: &AppContext, id: StableNodeId) -> [f32; 4] {
    match context.world().extract_nodes(&[id])[0]
        .component_geometry
        .as_deref()
    {
        Some(crate::ComponentGeometry::StatusBadge { foreground, .. }) => *foreground,
        other => panic!("not a badge geometry: {other:?}"),
    }
}

fn background(context: &AppContext, id: StableNodeId) -> Option<[f32; 4]> {
    context.world().extract_nodes(&[id])[0].style.background
}

fn theme(context: &AppContext, role: SemanticColorRole) -> [f32; 4] {
    context.world().style_model().color(role).as_rgba_array()
}

#[test]
fn an_override_answers_for_the_node_itself() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let mut stack = scoped_stack(Some(
        ColorOverrides::new().with(SemanticColorRole::Subtle, PURPLE),
    ));
    stack.style_mut().background = Some(SemanticColorRole::Subtle);
    let stack = context.create_component(document, stack).unwrap();
    layout(&mut context, document);
    assert_eq!(
        background(&context, stack.stable_id()),
        Some(PURPLE.as_rgba_array())
    );
}

#[test]
fn an_override_reaches_deep_descendants_and_a_switch_track() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let outer = context
        .create_component(
            document,
            scoped_stack(Some(
                ColorOverrides::new().with(SemanticColorRole::Accent, PURPLE),
            )),
        )
        .unwrap();
    let middle = context
        .create_component(document, scoped_stack(None))
        .unwrap();
    let inner = context
        .create_component(document, scoped_stack(None))
        .unwrap();
    let switch = context
        .create_component(document, crate::Switch::new("Modeler", true))
        .unwrap();
    let outside = context
        .create_component(document, crate::Switch::new("Plain", true))
        .unwrap();
    context.append_child(outer, middle).unwrap();
    context.append_child(middle, inner).unwrap();
    context.append_child(inner, switch).unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        PURPLE.as_rgba_array(),
        "a checked track three levels down takes the scope's accent"
    );
    assert_eq!(
        switch_track(&context, outside.stable_id()),
        theme(&context, SemanticColorRole::Accent),
        "a switch outside the scope keeps the theme's"
    );
}

#[test]
fn an_inner_scope_wins_role_by_role() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let outer = context
        .create_component(
            document,
            scoped_stack(Some(
                ColorOverrides::new()
                    .with(SemanticColorRole::Accent, PURPLE)
                    .with(SemanticColorRole::Subtle, PURPLE),
            )),
        )
        .unwrap();
    let mut inner = scoped_stack(Some(
        ColorOverrides::new().with(SemanticColorRole::Accent, ORANGE),
    ));
    inner.style_mut().background = Some(SemanticColorRole::Subtle);
    let inner = context.create_component(document, inner).unwrap();
    let switch = context
        .create_component(document, crate::Switch::new("Nested", true))
        .unwrap();
    context.append_child(outer, inner).unwrap();
    context.append_child(inner, switch).unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        ORANGE.as_rgba_array()
    );
    assert_eq!(
        background(&context, inner.stable_id()),
        Some(PURPLE.as_rgba_array()),
        "a role the inner scope leaves alone is the outer one's"
    );
}

#[test]
fn removing_the_override_gives_the_theme_back() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let outer = context
        .create_component(
            document,
            scoped_stack(Some(
                ColorOverrides::new().with(SemanticColorRole::Accent, PURPLE),
            )),
        )
        .unwrap();
    let middle = context
        .create_component(document, scoped_stack(None))
        .unwrap();
    let switch = context
        .create_component(document, crate::Switch::new("Back", true))
        .unwrap();
    context.append_child(outer, middle).unwrap();
    context.append_child(middle, switch).unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        PURPLE.as_rgba_array()
    );
    context
        .update_component(outer, |stack, _| stack.style_mut().color_overrides = None)
        .unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        theme(&context, SemanticColorRole::Accent)
    );
    context
        .update_component(outer, |stack, _| {
            stack.style_mut().color_overrides = Some(Arc::new(
                ColorOverrides::new().with(SemanticColorRole::Accent, ORANGE),
            ));
        })
        .unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        ORANGE.as_rgba_array(),
        "a changed scope reaches a descendant already on screen"
    );
}

#[test]
fn a_status_badge_paints_its_tone_from_the_scope() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let tone = StatusTone::Info;
    let role = context.world().theme().recipes().status().role(tone);
    let outer = context
        .create_component(
            document,
            scoped_stack(Some(ColorOverrides::new().with(role, PURPLE))),
        )
        .unwrap();
    let badge = context
        .create_component(document, crate::StatusBadge::new("Modeler", tone))
        .unwrap();
    let outside = context
        .create_component(document, crate::StatusBadge::new("Plain", tone))
        .unwrap();
    context.append_child(outer, badge).unwrap();
    layout(&mut context, document);
    assert_eq!(
        badge_foreground(&context, badge.stable_id()),
        PURPLE.as_rgba_array()
    );
    assert_eq!(
        badge_foreground(&context, outside.stable_id()),
        theme(&context, role)
    );
}

#[test]
fn overrides_merge_with_the_inner_set_on_top() {
    let outer = ColorOverrides::new()
        .with(SemanticColorRole::Accent, PURPLE)
        .with(SemanticColorRole::Text, PURPLE);
    let inner = ColorOverrides::new().with(SemanticColorRole::Accent, ORANGE);
    let merged = inner.over(&outer);
    assert_eq!(merged.get(SemanticColorRole::Accent), Some(ORANGE));
    assert_eq!(merged.get(SemanticColorRole::Text), Some(PURPLE));
    assert_eq!(merged.get(SemanticColorRole::Danger), None);
}

/// `El::theme_colors` takes a plain set or a closure, the shapes a view
/// binds it with.
#[allow(dead_code)]
fn theme_colors_binds_a_value_and_a_closure() {
    let set = ColorOverrides::new().with(SemanticColorRole::Accent, PURPLE);
    let _ = crate::view::widget(crate::Stack::column(0.0)).theme_colors(set);
    let _ = crate::view::widget(crate::Stack::column(0.0))
        .theme_colors(|| ColorOverrides::new().with(SemanticColorRole::Accent, ORANGE));
}
