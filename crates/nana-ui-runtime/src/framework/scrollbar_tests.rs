//! Scroll view bars: where they sit, when they show and fade, and that they
//! never move layout.

use std::time::Duration;

use super::*;
use crate::{LayoutBox, NodeStyle, ScrollbarBar, Text};
use nana_ui_core::{LengthSpec, ScrollbarAxis, ScrollbarVisibility};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn sized(width: f32, height: f32) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Px(width));
    layout.height = Some(LengthSpec::Px(height));
    style
}

fn row(context: &mut AppContext, parent: Entity<ScrollView>, height: f32) -> StableNodeId {
    let mut style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(height));
    }
    let row = context
        .create_detached_component(document(), Text::new("row").style(style))
        .unwrap();
    context.append_child(parent, row).unwrap();
    row.stable_id()
}

/// A 200×120 scroll view holding 200px of rows, laid out.
fn scroll_view(
    context: &mut AppContext,
    visibility: Option<ScrollbarVisibility>,
    edit: impl FnOnce(&mut nana_ui_core::LayoutStyle),
) -> Entity<ScrollView> {
    let mut view = ScrollView::new(ScrollAxes::Vertical).style(sized(200.0, 120.0));
    view.scrollbars = visibility;
    let view = view.with_layout(edit);
    let scroll = context.create_component(document(), view).unwrap();
    for _ in 0..5 {
        row(context, scroll, 40.0);
    }
    lay_out(context);
    scroll
}

fn lay_out(context: &mut AppContext) {
    context
        .layout_document(document(), crate::LayoutViewport::new(400.0, 300.0))
        .unwrap();
    context.rebuild_hit_test(document());
}

fn bar(context: &AppContext, scroll: Entity<ScrollView>) -> Option<ScrollbarBar> {
    context
        .world()
        .scrollbar_bars(scroll.stable_id())
        .and_then(|bars| bars.vertical)
}

fn held(context: &AppContext, scroll: Entity<ScrollView>) -> bool {
    context.read(scroll, |view| view.held).unwrap()
}

fn delay() -> Duration {
    nana_ui_core::ScrollbarMotion::DEFAULT.hide_delay()
}

/// The lane runs `track_inset` inside the border, over the padding, so a
/// view that leaves end padding keeps its bar off its content.
#[test]
fn the_lane_runs_inside_the_border_over_the_padding() {
    let mut context = AppContext::new();
    let scroll = scroll_view(&mut context, Some(ScrollbarVisibility::Always), |layout| {
        layout.border_width = Some(3.0);
        layout.padding_left = Some(LengthSpec::Px(20.0));
        layout.padding_right = Some(LengthSpec::Px(20.0));
    });
    let bar = bar(&context, scroll).expect("a resident bar");
    let metrics = nana_ui_core::SCROLLBAR_METRICS;
    assert_eq!(
        bar.lane.x + bar.lane.width,
        200.0 - 3.0 - metrics.track_inset
    );
    assert_eq!(bar.lane.width, metrics.thickness);
    assert_eq!(bar.lane.y, 3.0 + metrics.track_inset);
    let content = context
        .world()
        .component_content_box(scroll.stable_id())
        .unwrap();
    assert!(
        bar.lane.x >= content.x + content.width,
        "20px of end padding holds the whole lane: {bar:?} beside {content:?}"
    );
}

/// Bars overlay: whether they hide, stay or never show, and whether they
/// are shown, every box lays out the same.
#[test]
fn bars_never_move_layout() {
    let boxes = |visibility: Option<ScrollbarVisibility>, hover: bool| {
        let mut context = AppContext::new();
        let scroll = scroll_view(&mut context, visibility, |_| {});
        if hover {
            context
                .set_pointer_hover_at(document(), 1, Some(scroll.stable_id()), Duration::ZERO)
                .unwrap();
            context.advance_animations(Duration::from_millis(500));
            lay_out(&mut context);
        }
        let world = context.world();
        let mut boxes: Vec<LayoutBox> = vec![world.layout_box(scroll.stable_id()).unwrap()];
        boxes.extend(
            world
                .node(scroll.stable_id())
                .unwrap()
                .children
                .iter()
                .map(|child| world.layout_box(*child).unwrap()),
        );
        boxes
    };
    let reference = boxes(Some(ScrollbarVisibility::Hidden), false);
    for (visibility, hover) in [
        (None, false),
        (None, true),
        (Some(ScrollbarVisibility::Always), false),
        (Some(ScrollbarVisibility::Always), true),
    ] {
        assert_eq!(
            boxes(visibility, hover),
            reference,
            "{visibility:?} {hover}"
        );
    }
}

/// A pointer inside an inner scroll view is inside the outer one too: both
/// show their bars, and leaving holds both.
#[test]
fn every_enclosing_scroll_view_shows_its_bars() {
    let mut context = AppContext::new();
    let outer = context
        .create_component(
            document(),
            ScrollView::new(ScrollAxes::Vertical).style(sized(200.0, 120.0)),
        )
        .unwrap();
    let inner = context
        .create_detached_component(
            document(),
            ScrollView::new(ScrollAxes::Vertical).style(sized(180.0, 60.0)),
        )
        .unwrap();
    context.append_child(outer, inner).unwrap();
    row(&mut context, outer, 200.0);
    row(&mut context, inner, 200.0);
    lay_out(&mut context);
    let hovered = |context: &AppContext, view: Entity<ScrollView>| {
        context.read(view, |view| view.hovered).unwrap()
    };

    context
        .set_pointer_hover_at(document(), 1, Some(inner.stable_id()), Duration::ZERO)
        .unwrap();
    assert!(hovered(&context, inner) && hovered(&context, outer));
    context.advance_animations(Duration::from_millis(300));
    assert!(bar(&context, inner).is_some() && bar(&context, outer).is_some());

    context
        .set_pointer_hover_at(document(), 1, None, Duration::from_millis(300))
        .unwrap();
    assert!(!hovered(&context, inner) && !hovered(&context, outer));
    assert!(held(&context, inner) && held(&context, outer));
}

/// A scroll the user makes shows the bars without the pointer — touch,
/// keyboard, a wheel over a resting pointer — and they fade once the hide
/// delay passes. Nothing polls in between.
#[test]
fn a_user_scroll_reveals_the_bars_until_the_hide_delay() {
    let mut context = AppContext::new();
    let scroll = scroll_view(&mut context, None, |_| {});
    let start = Duration::from_millis(1000);
    context.advance_animations(start);
    assert_eq!(context.next_animation_deadline(), None);

    let scrolled = context
        .scroll_at(document(), 100.0, 60.0, ScrollOffset { x: 0.0, y: 30.0 })
        .unwrap();
    assert_eq!(scrolled, Some(scroll.stable_id()));
    assert!(held(&context, scroll));
    context.advance_animations(start + Duration::from_millis(200));
    assert!(bar(&context, scroll).is_some(), "the scroll shows the bar");
    assert_eq!(context.next_animation_deadline(), Some(start + delay()));

    context.advance_animations(start + delay());
    assert!(!held(&context, scroll));
    context.advance_animations(start + delay() + Duration::from_millis(500));
    assert!(
        bar(&context, scroll).is_none(),
        "and fades once the delay ran out"
    );
    assert_eq!(context.next_animation_deadline(), None);
}

/// Content the application scrolls — `scroll_to`, following the end — is not
/// the user scrolling: the bars stay where they were.
#[test]
fn a_programmatic_scroll_does_not_reveal_the_bars() {
    let mut context = AppContext::new();
    let scroll = scroll_view(&mut context, None, |_| {});
    assert!(
        context
            .scroll_to(scroll, ScrollOffset { x: 0.0, y: 40.0 })
            .unwrap()
    );
    assert!(!held(&context, scroll));
    assert!(bar(&context, scroll).is_none());
    assert_eq!(context.next_animation_deadline(), None);
}

/// Bars at rest keep nothing alive: a resident bar, a hidden one and an
/// auto-hiding one nobody points at.
#[test]
fn bars_at_rest_leave_nothing_to_wake_for() {
    for visibility in [
        None,
        Some(ScrollbarVisibility::Always),
        Some(ScrollbarVisibility::Hidden),
    ] {
        let mut context = AppContext::new();
        let scroll = scroll_view(&mut context, visibility, |_| {});
        assert_eq!(context.next_animation_deadline(), None, "{visibility:?}");
        assert_eq!(
            bar(&context, scroll).is_some(),
            visibility == Some(ScrollbarVisibility::Always)
        );
    }
}

/// Under reduced motion the bars show, grow and go at once.
#[test]
fn reduced_motion_lands_the_bars_at_once() {
    let mut context = AppContext::new();
    context.set_reduced_motion(true);
    let scroll = scroll_view(&mut context, None, |_| {});
    context
        .set_pointer_hover_at(document(), 1, Some(scroll.stable_id()), Duration::ZERO)
        .unwrap();
    let shown = bar(&context, scroll).expect("shown at once");
    assert_eq!(
        shown.thumb.width,
        nana_ui_core::SCROLLBAR_METRICS.thumb_thickness
    );
    context
        .update_component(scroll, |view, _| view.hot = Some(ScrollbarAxis::Vertical))
        .unwrap();
    assert_eq!(
        bar(&context, scroll).unwrap().thumb.width,
        nana_ui_core::SCROLLBAR_METRICS.thumb_active_thickness,
        "grown at once"
    );
    context
        .update_component(scroll, |view, _| view.hot = None)
        .unwrap();
    context
        .set_pointer_hover_at(document(), 1, None, Duration::from_millis(100))
        .unwrap();
    context.advance_animations(Duration::from_millis(100) + delay());
    assert!(bar(&context, scroll).is_none(), "gone at once");
    assert_eq!(context.next_animation_deadline(), None);
}

/// Turning back mid-fade turns from where the bar is, and leaves no stray
/// deadline once it lands.
#[test]
fn a_fade_reverses_from_where_it_is() {
    let mut context = AppContext::new();
    let scroll = scroll_view(&mut context, None, |_| {});
    let presence = |context: &AppContext| {
        context
            .world()
            .widget_visual_as::<crate::ScrollbarVisual>(scroll.stable_id())
            .expect("a scrollbar")
            .presence
    };
    context
        .set_pointer_hover_at(document(), 1, Some(scroll.stable_id()), Duration::ZERO)
        .unwrap();
    context.advance_animations(Duration::from_millis(40));
    let partway = presence(&context);
    assert!(partway > 0.0 && partway < 1.0);
    context
        .update_component(scroll, |view, _| view.hovered = false)
        .unwrap();
    assert_eq!(presence(&context), partway, "the turn starts where it was");
    context.advance_animations(Duration::from_millis(600));
    assert_eq!(presence(&context), 0.0);
    assert_eq!(context.next_animation_deadline(), None);
}

/// The theme's policy reaches every view that follows it, an installed
/// theme included; a view that chose its own keeps it.
#[test]
fn the_theme_policy_reaches_views_that_follow_it() {
    let mut context = AppContext::new();
    let follows = scroll_view(&mut context, None, |_| {});
    let chose = scroll_view(
        &mut context,
        Some(ScrollbarVisibility::AutoHide),
        |layout| {
            layout.margin_top = Some(LengthSpec::Px(150.0));
        },
    );
    assert!(bar(&context, follows).is_none() && bar(&context, chose).is_none());
    assert!(
        context
            .set_theme_definition(
                &nana_ui_core::ThemeDefinition::NANA_DARK
                    .with_scrollbar_visibility(ScrollbarVisibility::Always),
            )
            .unwrap()
    );
    assert!(
        bar(&context, follows).is_some(),
        "the policy made it resident"
    );
    assert!(bar(&context, chose).is_none(), "its own choice stands");
}

/// The recipe decides what the bar paints with.
#[test]
fn a_recipe_colour_reaches_the_thumb() {
    let mut context = AppContext::new();
    let scroll = scroll_view(&mut context, Some(ScrollbarVisibility::Always), |_| {});
    let recipe = nana_ui_core::ScrollbarRecipe {
        thumb: nana_ui_core::SemanticColorMix::role(nana_ui_core::SemanticColorRole::Danger),
        ..nana_ui_core::ScrollbarRecipe::DEFAULT
    };
    context
        .set_theme_definition(&nana_ui_core::ThemeDefinition::NANA_DARK.with_scrollbar(recipe))
        .unwrap();
    let danger = context
        .world()
        .theme()
        .style_model()
        .color(nana_ui_core::SemanticColorRole::Danger)
        .as_rgba_array();
    assert_eq!(bar(&context, scroll).unwrap().thumb_background, danger);
}
