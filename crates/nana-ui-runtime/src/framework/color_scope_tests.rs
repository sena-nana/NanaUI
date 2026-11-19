//! Issue #104, first cut: a node's [`crate::NodeStyle::color_overrides`]
//! answer for it and its whole subtree.

use super::*;
use crate::NodeStyle;
use nana_ui_core::{ColorOverrides, SemanticColor, SemanticColorRole, StatusTone};

const PURPLE: SemanticColor = SemanticColor::rgb8(130, 60, 220);
const ORANGE: SemanticColor = SemanticColor::rgb8(230, 120, 20);

fn scoped_stack(overrides: Option<ColorOverrides>) -> crate::Stack {
    let mut stack = crate::Stack::column(0.0);
    stack.style_mut().color_overrides = overrides.map(Arc::new);
    stack
}

fn layout(context: &mut AppContext, document: DocumentId) {
    let viewport = crate::LayoutViewport::new(320.0, 200.0);
    context.layout_document(document, viewport).unwrap();
    let work = context.compat_world_mut().take_system_work();
    context
        .compat_world_mut()
        .resolve_styles(&work.style)
        .unwrap();
    if !work.text.is_empty() {
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut crate::MeasureTextShaper)
            .unwrap();
        context.layout_document(document, viewport).unwrap();
        let follow_up = context.compat_world_mut().take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&follow_up.style)
            .unwrap();
    }
}

fn switch_track(context: &AppContext, id: StableNodeId) -> [f32; 4] {
    let paint = context.world().widget_paint(id).unwrap();
    match paint
        .ops
        .iter()
        .find(|tagged| tagged.slot == 4)
        .map(|tagged| &tagged.op)
    {
        Some(crate::WidgetPaintOp::Quad(quad)) => quad.fill.expect("switch track fill"),
        other => panic!("not a switch track quad: {other:?}"),
    }
}

fn badge_foreground(context: &AppContext, id: StableNodeId) -> [f32; 4] {
    let paint = context.world().widget_paint(id).unwrap();
    match paint
        .ops
        .iter()
        .find(|tagged| tagged.slot == 3)
        .map(|tagged| &tagged.op)
    {
        Some(crate::WidgetPaintOp::Quad(quad)) => quad.fill.expect("badge dot fill"),
        other => panic!("not a badge dot quad: {other:?}"),
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

fn patched_stack(patch: nana_ui_core::ThemePatch) -> crate::Stack {
    let mut stack = crate::Stack::column(0.0);
    stack.style_mut().theme_patch = Some(Arc::new(patch));
    stack
}

fn button_fill(context: &AppContext, id: StableNodeId) -> [f32; 4] {
    context.world().extract_nodes(&[id])[0]
        .style
        .background
        .expect("button background fill")
}

#[test]
fn theme_scope_nests_patches_and_overrides_button_recipe_and_metrics() {
    use nana_ui_core::{ButtonKind, ButtonVariantRecipe, ThemeMetricsPatch, ThemePatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let outer = context
        .create_component(
            document,
            patched_stack(
                ThemePatch::default()
                    .with_metrics_patch(ThemeMetricsPatch {
                        control_height: Some(44.0),
                        ..Default::default()
                    })
                    .with_color(SemanticColorRole::Accent, PURPLE),
            ),
        )
        .unwrap();
    let inner = context
        .create_component(
            document,
            patched_stack(
                ThemePatch::default()
                    .with_button_variant(
                        ButtonKind::Primary,
                        ButtonVariantRecipe {
                            foreground: SemanticColorRole::Text,
                            background: Some(SemanticColorRole::Danger),
                            border: None,
                            hovered_background: SemanticColorRole::Danger,
                            pressed_background: SemanticColorRole::Danger,
                        },
                    )
                    .with_color(SemanticColorRole::Danger, ORANGE),
            ),
        )
        .unwrap();

    let outer_switch = context
        .create_component(document, crate::Switch::new("Outer", true))
        .unwrap();
    let scoped_button = context
        .create_component(
            document,
            crate::Button::new("Scoped").kind(ButtonKind::Primary),
        )
        .unwrap();
    let plain_button = context
        .create_component(
            document,
            crate::Button::new("Plain").kind(ButtonKind::Primary),
        )
        .unwrap();

    context.append_child(outer, outer_switch).unwrap();
    context.append_child(outer, inner).unwrap();
    context.append_child(inner, scoped_button).unwrap();

    layout(&mut context, document);

    assert_eq!(
        switch_track(&context, outer_switch.stable_id()),
        PURPLE.as_rgba_array(),
        "outer ThemeScope accent reaches Switch track"
    );
    assert_eq!(
        button_fill(&context, scoped_button.stable_id()),
        ORANGE.as_rgba_array(),
        "inner ThemeScope overrides primary ButtonVariantRecipe + Danger color"
    );
    let scoped_height = context
        .world()
        .layout_result(scoped_button.stable_id())
        .unwrap()
        .bounds
        .height;
    assert!(
        (scoped_height - 44.0).abs() < 0.01,
        "outer metrics control_height inherits into nested ThemeScope: {scoped_height}"
    );

    let plain_height = context
        .world()
        .layout_result(plain_button.stable_id())
        .unwrap()
        .bounds
        .height;
    assert_ne!(
        plain_height, 44.0,
        "root button outside ThemeScope keeps default metrics height"
    );
    assert_ne!(
        button_fill(&context, plain_button.stable_id()),
        ORANGE.as_rgba_array(),
        "root button outside ThemeScope keeps default primary background"
    );

    // Also exercise the public `El::theme_scope` / `view::theme_scope` view builder.
    let _ = crate::view::theme_scope(
        ThemePatch::default().with_color(SemanticColorRole::Accent, PURPLE),
    );
    let _ = crate::view::widget(crate::Stack::column(0.0))
        .theme_scope(|| ThemePatch::default().with_color(SemanticColorRole::Accent, ORANGE));
}

#[test]
fn theme_scope_resolution_cache_records_hits_and_misses() {
    use nana_ui_core::ThemePatch;

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let patch = ThemePatch::default().with_color(SemanticColorRole::Accent, PURPLE);

    let scope_a = context
        .create_component(document, patched_stack(patch.clone()))
        .unwrap();
    let scope_b = context
        .create_component(document, patched_stack(patch))
        .unwrap();
    let switch_a = context
        .create_component(document, crate::Switch::new("A", true))
        .unwrap();
    let switch_b = context
        .create_component(document, crate::Switch::new("B", true))
        .unwrap();
    context.append_child(scope_a, switch_a).unwrap();
    context.append_child(scope_b, switch_b).unwrap();

    layout(&mut context, document);

    assert_eq!(
        switch_track(&context, switch_a.stable_id()),
        PURPLE.as_rgba_array()
    );
    assert_eq!(
        switch_track(&context, switch_b.stable_id()),
        PURPLE.as_rgba_array()
    );

    let work = context.world().last_theme_work_counters();
    assert!(
        work.scope_cache_misses >= 1,
        "first distinct ThemePatch must record a cache miss: {work:?}"
    );
    assert!(
        work.scope_cache_hits >= 1,
        "equal sibling ThemePatch must hit the O(1) cache: {work:?}"
    );
}

#[test]
fn invalid_theme_patch_fails_closed_to_parent_and_records_event() {
    use nana_ui_core::{SpacingPatch, ThemeMetricsPatch, ThemePatch};

    const CHILD: &str = "NANA_RUNTIME_TEST_INVALID_SCOPE_PATCH_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let exe = std::env::current_exe().expect("test binary");
        let mut child = std::process::Command::new(exe)
            .args([
                "--exact",
                "framework::color_scope_tests::invalid_theme_patch_fails_closed_to_parent_and_records_event",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .spawn()
            .expect("spawn child");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait().expect("wait child") {
                assert!(status.success(), "child test failed: {status}");
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child test timed out after 15s");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nana-invalid-scope-patch-{}-{seq}",
        std::process::id()
    ));
    let guard = nana_diagnostics::install(
        nana_diagnostics::DiagnosticsConfig {
            min_severity: nana_diagnostics::Severity::Debug,
            persist: nana_diagnostics::PersistMode::All,
            poll_interval: std::time::Duration::from_millis(5),
            panic_hook: false,
            ..nana_diagnostics::DiagnosticsConfig::default()
        },
        nana_diagnostics::SessionMetadata::new("dev.nana.test", "invalid-scope-patch", "0"),
        nana_diagnostics::DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
    )
    .expect("isolated child owns global diagnostics");
    nana_diagnostics::register_thread();

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let valid_outer = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_color(SemanticColorRole::Accent, PURPLE)),
        )
        .unwrap();
    let bad_spacing = context
        .create_component(
            document,
            patched_stack(
                ThemePatch::default()
                    .with_spacing_patch(SpacingPatch {
                        md: Some(-8.0),
                        ..Default::default()
                    })
                    .with_color(SemanticColorRole::Accent, ORANGE),
            ),
        )
        .unwrap();
    let bad_nan = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_metrics_patch(ThemeMetricsPatch {
                radius_md: Some(f32::NAN),
                ..Default::default()
            })),
        )
        .unwrap();
    let switch_spacing = context
        .create_component(document, crate::Switch::new("BadSpacing", true))
        .unwrap();
    let switch_nan = context
        .create_component(document, crate::Switch::new("BadNaN", true))
        .unwrap();

    context.append_child(valid_outer, bad_spacing).unwrap();
    context.append_child(bad_spacing, switch_spacing).unwrap();
    context.append_child(valid_outer, bad_nan).unwrap();
    context.append_child(bad_nan, switch_nan).unwrap();

    layout(&mut context, document);

    assert_eq!(
        switch_track(&context, switch_spacing.stable_id()),
        PURPLE.as_rgba_array(),
        "invalid SpacingPatch must fail closed to parent scope's valid PURPLE theme"
    );
    assert_eq!(
        switch_track(&context, switch_nan.stable_id()),
        PURPLE.as_rgba_array(),
        "invalid NaN ThemeMetricsPatch must fail closed to parent scope"
    );
    assert!(
        context
            .world()
            .node_theme(switch_spacing.stable_id())
            .spacing()
            .md
            > 0.0
    );

    assert!(
        guard
            .diagnostics()
            .flush(false, std::time::Duration::from_secs(5))
    );
    let log = guard
        .diagnostics()
        .current_log()
        .expect("session log opened");
    drop(guard);

    let file = nana_diagnostics::nlog::read_file(&log).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let events: Vec<_> = file
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry,
                nana_diagnostics::nlog::Entry::Event { key, .. }
                    if file.event_name(*key) == Some("runtime.theme.scope_invalid_patch")
            )
        })
        .collect();
    assert_eq!(
        events.len(),
        4,
        "expected invalid patch events on initial apply and reparent validation"
    );
}

#[test]
fn reparenting_subtree_between_theme_scopes_updates_effective_theme() {
    use nana_ui_core::{ThemeMetricsPatch, ThemePatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let default_height = context.world().theme().metrics().control_height;

    let scope_a = context
        .create_component(
            document,
            patched_stack(
                ThemePatch::default()
                    .with_metrics_patch(ThemeMetricsPatch {
                        control_height: Some(40.0),
                        ..Default::default()
                    })
                    .with_color(SemanticColorRole::Accent, PURPLE),
            ),
        )
        .unwrap();
    let scope_b = context
        .create_component(
            document,
            patched_stack(
                ThemePatch::default()
                    .with_metrics_patch(ThemeMetricsPatch {
                        control_height: Some(48.0),
                        ..Default::default()
                    })
                    .with_color(SemanticColorRole::Accent, ORANGE),
            ),
        )
        .unwrap();

    let subtree = context
        .create_component(document, crate::Stack::column(4.0))
        .unwrap();
    let button = context
        .create_component(
            document,
            crate::Button::new("Move").kind(nana_ui_core::ButtonKind::Primary),
        )
        .unwrap();
    let switch = context
        .create_component(document, crate::Switch::new("Move", true))
        .unwrap();
    context.append_child(subtree, button).unwrap();
    context.append_child(subtree, switch).unwrap();

    // Move into scope A.
    context.append_child(scope_a, subtree).unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        PURPLE.as_rgba_array()
    );
    assert!(
        (context
            .world()
            .layout_result(button.stable_id())
            .unwrap()
            .bounds
            .height
            - 40.0)
            .abs()
            < 0.01
    );

    // Reparent from scope A into scope B.
    context.append_child(scope_b, subtree).unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        ORANGE.as_rgba_array()
    );
    assert!(
        (context
            .world()
            .layout_result(button.stable_id())
            .unwrap()
            .bounds
            .height
            - 48.0)
            .abs()
            < 0.01
    );

    // Move back into a root container outside every ThemeScope.
    let plain_root = context
        .create_component(document, crate::Stack::column(4.0))
        .unwrap();
    context.append_child(plain_root, subtree).unwrap();
    layout(&mut context, document);
    assert_eq!(
        switch_track(&context, switch.stable_id()),
        theme(&context, SemanticColorRole::Accent)
    );
    assert!(
        (context
            .world()
            .layout_result(button.stable_id())
            .unwrap()
            .bounds
            .height
            - default_height)
            .abs()
            < 0.01
    );

    // Also verify detaching directly drops parent scope back to `ThemeScopeId::ROOT`.
    let mut detach = MutationQueue::new();
    detach.detach(subtree.stable_id());
    context.commit_batch(detach, &[]).unwrap();
    assert_eq!(
        context
            .world()
            .node_theme(switch.stable_id())
            .palette()
            .accent
            .as_rgba_array(),
        theme(&context, SemanticColorRole::Accent)
    );
    assert!(
        (context
            .world()
            .node_theme(button.stable_id())
            .metrics()
            .control_height
            - default_height)
            .abs()
            < 0.01
    );
}

#[test]
fn document_themes_isolate_roots_and_survive_root_theme_changes() {
    use nana_ui_core::{ThemeAppearance, ThemePatch};

    let mut context = AppContext::new();
    let doc_1 = DocumentId::new(1).unwrap();
    let doc_2 = DocumentId::new(2).unwrap();

    let switch_1 = context
        .create_component(doc_1, crate::Switch::new("Doc1", true))
        .unwrap();
    let switch_2 = context
        .create_component(doc_2, crate::Switch::new("Doc2", true))
        .unwrap();

    let custom_theme = Arc::new(
        ThemePatch::default()
            .with_color(SemanticColorRole::Accent, PURPLE)
            .apply_to_compiled(context.world().theme())
            .unwrap(),
    );
    context
        .set_document_theme(doc_1, Some(custom_theme))
        .unwrap();

    layout(&mut context, doc_1);
    layout(&mut context, doc_2);

    assert_eq!(
        switch_track(&context, switch_1.stable_id()),
        PURPLE.as_rgba_array(),
        "doc_1 uses its custom per-document theme"
    );
    assert_eq!(
        switch_track(&context, switch_2.stable_id()),
        theme(&context, SemanticColorRole::Accent),
        "doc_2 stays isolated on the root theme"
    );

    context.set_preset_theme(ThemeAppearance::Light).unwrap();
    layout(&mut context, doc_1);
    layout(&mut context, doc_2);

    assert_eq!(
        switch_track(&context, switch_1.stable_id()),
        PURPLE.as_rgba_array(),
        "root preset change does not overwrite doc_1's explicit theme"
    );
    assert_eq!(
        switch_track(&context, switch_2.stable_id()),
        theme(&context, SemanticColorRole::Accent),
        "doc_2 updates with the new root preset theme"
    );

    context.set_document_theme(doc_1, None).unwrap();
    layout(&mut context, doc_1);
    assert_eq!(
        switch_track(&context, switch_1.stable_id()),
        theme(&context, SemanticColorRole::Accent),
        "clearing doc_1 override returns it to the root theme"
    );
}

#[cfg(feature = "charts")]
#[test]
fn scoped_chart_and_chart_tooltip_reflect_theme_scope_tokens() {
    use nana_ui_charts::hit::{TooltipContent, TooltipRow};
    use nana_ui_charts::{Axis, ChartOption, LineSeries};
    use nana_ui_core::{ThemePatch, TypographyPatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let mut chart_style = crate::NodeStyle::default();
    let layout_mut = Arc::make_mut(&mut chart_style.layout);
    layout_mut.width = Some(LengthSpec::Px(260.0));
    layout_mut.height = Some(LengthSpec::Px(160.0));

    let option = ChartOption::new()
        .x_axis(Axis::category(["Mon", "Tue"]))
        .y_axis(Axis::value())
        .series(LineSeries::new("v", [1.0, 3.0]));

    let scope = context
        .create_component(
            document,
            patched_stack(
                ThemePatch::default()
                    .with_color(SemanticColorRole::Text, ORANGE)
                    .with_color(SemanticColorRole::Muted, ORANGE)
                    .with_typography_patch(TypographyPatch {
                        regular: Some(500),
                        medium: Some(650),
                        ..Default::default()
                    }),
            ),
        )
        .unwrap();

    let scoped_chart = context
        .create_component(
            document,
            crate::Chart::new(option.clone()).style(chart_style.clone()),
        )
        .unwrap();
    let root_chart = context
        .create_component(document, crate::Chart::new(option).style(chart_style))
        .unwrap();

    let tip_content = Arc::new(TooltipContent {
        title: "Sample".into(),
        rows: vec![TooltipRow {
            color: Some([0.2, 0.6, 0.9, 1.0]),
            name: "Series".into(),
            value: "3.0".into(),
        }],
    });
    let scoped_tip = context
        .create_component(document, crate::ChartTooltip::new(tip_content.clone()))
        .unwrap();
    let root_tip = context
        .create_component(document, crate::ChartTooltip::new(tip_content))
        .unwrap();

    context.append_child(scope, scoped_chart).unwrap();
    context.append_child(scope, scoped_tip).unwrap();

    layout(&mut context, document);

    let first_text_color_and_weight = |id: StableNodeId| {
        let extracted = context.world().extract_nodes(&[id]);
        let paint = extracted[0].widget_paint.as_deref().unwrap();
        paint
            .ops
            .iter()
            .find_map(|tagged| match &tagged.op {
                crate::WidgetPaintOp::Text(t) => Some((t.color, t.font_weight)),
                _ => None,
            })
            .expect("expected text paint op")
    };

    let (scoped_chart_color, _) = first_text_color_and_weight(scoped_chart.stable_id());
    let (root_chart_color, _) = first_text_color_and_weight(root_chart.stable_id());
    assert_eq!(
        scoped_chart_color,
        Some(ORANGE.as_rgba_array()),
        "scoped chart axis text reads ThemeScope's overridden Text role"
    );
    assert_ne!(scoped_chart_color, root_chart_color);

    let (scoped_tip_color, scoped_tip_weight) = first_text_color_and_weight(scoped_tip.stable_id());
    let (root_tip_color, root_tip_weight) = first_text_color_and_weight(root_tip.stable_id());
    assert_eq!(scoped_tip_color, Some(ORANGE.as_rgba_array()));
    assert_eq!(scoped_tip_weight, Some(650));
    assert_ne!(scoped_tip_color, root_tip_color);
    assert_ne!(scoped_tip_weight, root_tip_weight);
}

#[test]
fn input_recipe_overrides_reach_text_input_and_text_area() {
    use nana_ui_core::{InputRecipe, ThemePatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let custom_input = InputRecipe {
        background: SemanticColorRole::Surface,
        border: SemanticColorRole::Accent,
        placeholder: SemanticColorRole::Warning,
        selection_background: SemanticColorRole::Danger,
        preedit_underline: SemanticColorRole::Success,
        invalid_focus_border_width: 4.0,
        multiline_height_factor: 4.5,
        ..InputRecipe::DEFAULT
    };

    let scope = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_input_recipe(custom_input)),
        )
        .unwrap();
    let scoped_input = context
        .create_component(
            document,
            crate::TextInput::new("").placeholder("Hint").invalid(true),
        )
        .unwrap();
    let scoped_area = context
        .create_component(document, crate::TextArea::new("Line 1"))
        .unwrap();
    let root_input = context
        .create_component(document, crate::TextInput::new("").placeholder("Hint"))
        .unwrap();
    let root_area = context
        .create_component(document, crate::TextArea::new("Line 1"))
        .unwrap();

    context.append_child(scope, scoped_input).unwrap();
    context.append_child(scope, scoped_area).unwrap();
    context
        .focus_node(document, scoped_input.stable_id())
        .unwrap();
    layout(&mut context, document);

    let scoped_geom = context
        .world()
        .text_input_geometry(scoped_input.stable_id())
        .unwrap();
    let root_geom = context
        .world()
        .text_input_geometry(root_input.stable_id())
        .unwrap();

    assert_eq!(
        scoped_geom.border_width, 4.0,
        "focused invalid width comes from InputRecipe"
    );
    assert_eq!(
        scoped_geom.text.color,
        Some(theme(&context, SemanticColorRole::Warning))
    );
    assert_eq!(
        scoped_geom.selection_color,
        theme(&context, SemanticColorRole::Danger)
    );
    assert_eq!(
        scoped_geom.preedit_color,
        theme(&context, SemanticColorRole::Success)
    );
    assert_ne!(scoped_geom.border_width, root_geom.border_width);
    assert_ne!(scoped_geom.selection_color, root_geom.selection_color);
    assert_eq!(
        background(&context, scoped_area.stable_id()),
        Some(theme(&context, SemanticColorRole::Surface))
    );
    assert!(
        context
            .world()
            .layout_result(scoped_area.stable_id())
            .unwrap()
            .bounds
            .height
            > context
                .world()
                .layout_result(root_area.stable_id())
                .unwrap()
                .bounds
                .height,
        "multiline_height_factor expands scoped TextArea min height"
    );
}

#[test]
fn select_recipe_overrides_reach_select_dropdown_and_search_dropdown() {
    use nana_ui_core::{ElevationRole, SelectRecipe, ThemePatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let custom_select = SelectRecipe {
        background: SemanticColorRole::Surface,
        handle: SemanticColorRole::Warning,
        menu_selected_background: SemanticColorRole::AccentSoft,
        menu_elevation: ElevationRole::Overlay,
        checkmark_width: 24.0,
        checkmark_weight: 600,
        invalid_open_border_width: 3.5,
        ..SelectRecipe::DEFAULT
    };

    let scope = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_select_recipe(custom_select)),
        )
        .unwrap();
    let scoped_select = context
        .create_component(
            document,
            crate::Select::new(Some("a"))
                .options([
                    crate::SelectOption::new("a", "Alpha"),
                    crate::SelectOption::new("b", "Beta"),
                ])
                .opened(true)
                .invalid(true),
        )
        .unwrap();
    let mut scoped_dropdown = crate::Dropdown::multiple(["a"])
        .options([crate::DropdownOption::new("a", "Alpha")])
        .opened(true);
    scoped_dropdown.highlighted = Some(0);
    let scoped_dropdown = context.create_component(document, scoped_dropdown).unwrap();
    let scoped_search = context
        .create_component(
            document,
            crate::SearchDropdown::new(Some("a"))
                .options([crate::SearchDropdownOption::new("a", "Alpha")]),
        )
        .unwrap();
    let root_select = context
        .create_component(
            document,
            crate::Select::new(Some("a"))
                .options([crate::SelectOption::new("a", "Alpha")])
                .opened(true),
        )
        .unwrap();

    context.append_child(scope, scoped_select).unwrap();
    context.append_child(scope, scoped_dropdown).unwrap();
    context.append_child(scope, scoped_search).unwrap();
    layout(&mut context, document);

    assert_eq!(
        background(&context, scoped_select.stable_id()),
        Some(theme(&context, SemanticColorRole::Surface))
    );
    assert_eq!(
        context
            .world()
            .layout_style(scoped_select.stable_id())
            .unwrap()
            .border_width,
        Some(3.5)
    );
    assert_eq!(
        background(&context, scoped_search.stable_id()),
        Some(theme(&context, SemanticColorRole::Surface))
    );

    let scoped_geom = context
        .world()
        .select_geometry(scoped_select.stable_id())
        .unwrap();
    let root_geom = context
        .world()
        .select_geometry(root_select.stable_id())
        .unwrap();
    assert_eq!(
        scoped_geom.handle_color,
        theme(&context, SemanticColorRole::Warning)
    );
    assert_eq!(
        scoped_geom.menu.as_ref().unwrap().elevation,
        crate::ComponentElevation::from_shadow(
            context
                .world()
                .theme()
                .effects()
                .shadow(ElevationRole::Overlay)
        )
    );
    assert_ne!(scoped_geom.handle_color, root_geom.handle_color);

    let scoped_dd_menu = context
        .world()
        .select_geometry(scoped_dropdown.stable_id())
        .unwrap()
        .menu
        .unwrap();
    assert_eq!(
        scoped_dd_menu.options[0].background,
        Some(theme(&context, SemanticColorRole::AccentSoft))
    );
    let extracted = context
        .world()
        .extract_nodes(&[scoped_dropdown.stable_id()]);
    let check_mark = extracted[0]
        .widget_paint
        .as_deref()
        .unwrap()
        .ops
        .iter()
        .find_map(|tagged| match &tagged.op {
            crate::WidgetPaintOp::Text(t) if t.content.as_str() == "✓" => Some(t),
            _ => None,
        })
        .expect("multi-select dropdown checkmark");
    assert_eq!(check_mark.bounds.width, 24.0);
    assert_eq!(check_mark.font_weight, Some(600));
}

#[test]
fn tabs_recipe_overrides_reach_tabs_and_segmented_options() {
    use nana_ui_core::{SpacingStep, TabsRecipe, TextWeight, ThemePatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let custom_tabs = TabsRecipe {
        strip_gap: SpacingStep::Lg,
        option_padding_extra: SpacingStep::Lg,
        icon_gap: SpacingStep::Lg,
        font_weight: TextWeight::Bold,
        foreground: SemanticColorRole::Warning,
        selected_background: SemanticColorRole::AccentSoft,
        ..TabsRecipe::DEFAULT
    };

    let scope = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_tabs_recipe(custom_tabs)),
        )
        .unwrap();
    let scoped_tabs = context
        .create_component(
            document,
            crate::Tabs::new("a").options([
                crate::TabOption::new("a", "One").icon(nana_ui_core::Icon::File),
                crate::TabOption::new("b", "Two"),
            ]),
        )
        .unwrap();
    let root_tabs = context
        .create_component(
            document,
            crate::Tabs::new("a").options([
                crate::TabOption::new("a", "One").icon(nana_ui_core::Icon::File),
                crate::TabOption::new("b", "Two"),
            ]),
        )
        .unwrap();

    context.append_child(scope, scoped_tabs).unwrap();
    layout(&mut context, document);

    let scoped_children = context
        .world()
        .node(scoped_tabs.stable_id())
        .unwrap()
        .children
        .clone();
    let root_children = context
        .world()
        .node(root_tabs.stable_id())
        .unwrap()
        .children
        .clone();

    let scoped_b0 = context.world().layout_box(scoped_children[0]).unwrap();
    let scoped_b1 = context.world().layout_box(scoped_children[1]).unwrap();
    let root_b0 = context.world().layout_box(root_children[0]).unwrap();
    let root_b1 = context.world().layout_box(root_children[1]).unwrap();

    let scoped_strip_gap = scoped_b1.x - (scoped_b0.x + scoped_b0.width);
    let root_strip_gap = root_b1.x - (root_b0.x + root_b0.width);
    assert!(
        (scoped_strip_gap - context.world().theme().spacing().lg).abs() < 0.01,
        "scoped={scoped_strip_gap} lg={}",
        context.world().theme().spacing().lg
    );
    assert!(scoped_strip_gap > root_strip_gap);

    let scoped_tab_style = &context.world().extract_nodes(&[scoped_children[0]])[0].style;
    assert_eq!(
        scoped_tab_style.background,
        Some(theme(&context, SemanticColorRole::AccentSoft))
    );
    assert_eq!(
        scoped_tab_style.font_weight,
        Some(context.world().theme().typography().bold)
    );
}

#[test]
fn tree_view_recipe_overrides_reach_rows_toggles_and_drop_mark() {
    use nana_ui_core::{ThemePatch, TreeViewRecipe};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let custom_tree = TreeViewRecipe {
        selected_background: SemanticColorRole::AccentSoft,
        foreground: SemanticColorRole::Warning,
        icon: SemanticColorRole::Success,
        toggle_on: SemanticColorRole::Danger,
        selected_weight: 700,
        drop_indicator: SemanticColorRole::Warning,
        drop_line_width: 5.0,
        drop_outline_width: 3.0,
        ..TreeViewRecipe::DEFAULT
    };

    let make_tree = || {
        let mut tree = crate::TreeView::new([
            nana_ui_core::TreeNode::leaf(Arc::from("n1"), "Selected")
                .selected(true)
                .icon(nana_ui_core::Icon::File)
                .toggles([true]),
            nana_ui_core::TreeNode::leaf(Arc::from("n2"), "Target"),
        ])
        .columns([crate::TreeToggleColumn::new(
            nana_ui_core::Icon::Eye,
            "Mark",
        )]);
        tree.drop = Some(crate::TreeDropIntent {
            target: Arc::from("n2"),
            position: crate::TreeDropPosition::Inside,
        });
        tree
    };

    let scope = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_tree_view_recipe(custom_tree)),
        )
        .unwrap();
    let scoped_tree = context.create_component(document, make_tree()).unwrap();
    let root_tree = context.create_component(document, make_tree()).unwrap();
    context.append_child(scope, scoped_tree).unwrap();
    layout(&mut context, document);

    let scoped_layout = context
        .world()
        .tree_view_layout(scoped_tree.stable_id())
        .unwrap();
    let root_layout = context
        .world()
        .tree_view_layout(root_tree.stable_id())
        .unwrap();

    assert_eq!(
        scoped_layout.rows[0].background,
        Some(theme(&context, SemanticColorRole::AccentSoft))
    );
    assert_eq!(
        scoped_layout.rows[0].label.color,
        Some(theme(&context, SemanticColorRole::Warning))
    );
    assert_eq!(scoped_layout.rows[0].label.font_weight, Some(700));
    assert_eq!(
        scoped_layout.rows[0].icon.unwrap().2,
        theme(&context, SemanticColorRole::Success)
    );
    assert_eq!(
        scoped_layout.rows[0].toggles[0].2,
        theme(&context, SemanticColorRole::Danger)
    );
    assert_ne!(
        scoped_layout.rows[0].background,
        root_layout.rows[0].background
    );

    let extracted = context.world().extract_nodes(&[scoped_tree.stable_id()]);
    let drop_quad = extracted[0]
        .widget_paint
        .as_deref()
        .unwrap()
        .ops
        .iter()
        .find_map(|tagged| match &tagged.op {
            crate::WidgetPaintOp::Quad(q) if q.border.is_some() => Some(q),
            _ => None,
        })
        .expect("drop outline quad");
    assert_eq!(
        drop_quad.border,
        Some((theme(&context, SemanticColorRole::Warning), 3.0))
    );
}

#[test]
fn menu_recipe_overrides_reach_menus_and_menu_bar() {
    use nana_ui_core::{ElevationRole, MenuRecipe, SpacingStep, TextWeight, ThemePatch};

    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();

    let custom_menu = MenuRecipe {
        surface_background: SemanticColorRole::Subtle,
        surface_border: SemanticColorRole::Accent,
        surface_padding: SpacingStep::Lg,
        item_gap: SpacingStep::Md,
        surface_elevation: ElevationRole::Overlay,
        bar_elevation: ElevationRole::Surface,
        item_foreground: SemanticColorRole::Warning,
        item_label_weight: TextWeight::Bold,
        mark_width: 24.0,
        bar_title_padding_x: SpacingStep::Xl,
        ..MenuRecipe::DEFAULT
    };

    let scope = context
        .create_component(
            document,
            patched_stack(ThemePatch::default().with_menu_recipe(custom_menu)),
        )
        .unwrap();

    let action_menu = context
        .create_component(document, crate::ActionMenu::new().open(true))
        .unwrap();
    let item_1 = context
        .create_component(document, crate::ActionMenuItem::new("First"))
        .unwrap();
    let item_2 = context
        .create_component(document, crate::ActionMenuItem::new("Second"))
        .unwrap();
    context.append_child(scope, action_menu).unwrap();
    context.append_child(action_menu, item_1).unwrap();
    context.append_child(action_menu, item_2).unwrap();

    let ctx_menu = context
        .create_component(
            document,
            crate::ContextMenu::new(16.0, 16.0)
                .items([crate::ContextMenuItem::new("open", "Open")])
                .open(true),
        )
        .unwrap();
    context.append_child(scope, ctx_menu).unwrap();

    let bar = crate::AppMenuBar::new(nana_ui_core::MenuBar::new([nana_ui_core::Menu::new(
        "File",
        [nana_ui_core::MenuEntry::item(1, "New").checked(true)],
    )]));
    let menu_bar = context.create_component(document, bar).unwrap();
    context.append_child(scope, menu_bar).unwrap();

    layout(&mut context, document);
    assert!(context.toggle_menu_bar_keyboard(menu_bar).unwrap());
    assert!(context.menu_bar_key(menu_bar, "ArrowDown").unwrap());
    layout(&mut context, document);

    let spacing = context.world().theme().spacing();
    let item1_box = context.world().layout_box(item_1.stable_id()).unwrap();
    let item2_box = context.world().layout_box(item_2.stable_id()).unwrap();
    let actual_item_gap = item2_box.y - (item1_box.y + item1_box.height);
    assert!(
        (actual_item_gap - spacing.resolve(SpacingStep::Md)).abs() < 0.01,
        "item gap should reflect MenuRecipe::item_gap: got {actual_item_gap}"
    );

    let item1_geom = context
        .world()
        .action_menu_item_geometry(item_1.stable_id())
        .unwrap();
    assert_eq!(
        item1_geom.label.color,
        Some(theme(&context, SemanticColorRole::Warning))
    );
    assert_eq!(
        item1_geom.label.font_weight,
        Some(context.world().theme().typography().bold)
    );

    let ctx_geom = context
        .world()
        .menu_surface_geometry(ctx_menu.stable_id())
        .unwrap();
    assert_eq!(
        ctx_geom.background,
        theme(&context, SemanticColorRole::Subtle)
    );
    assert_eq!(ctx_geom.border, theme(&context, SemanticColorRole::Accent));
    assert_eq!(
        ctx_geom.elevation,
        crate::ComponentElevation::from_shadow(
            context
                .world()
                .theme()
                .effects()
                .shadow(ElevationRole::Overlay)
        )
    );
    assert!((ctx_geom.options[0].bounds.x - ctx_geom.surface.x - spacing.lg).abs() < 0.01);

    let parts = context
        .read(menu_bar, |bar| bar.parts.clone())
        .unwrap()
        .unwrap();
    let title_style = context.world().layout_style(parts.titles[0].0).unwrap();
    assert_eq!(
        title_style.padding_left,
        Some(LengthSpec::Px(spacing.resolve(SpacingStep::Xl)))
    );
    let panel_style = context.world().layout_style(parts.panels[0].panel).unwrap();
    let bar_shadow = context
        .world()
        .theme()
        .effects()
        .shadow(ElevationRole::Surface);
    assert_eq!(panel_style.paint.box_shadows.len(), 1);
    assert_eq!(
        panel_style.paint.box_shadows[0].blur_radius,
        bar_shadow.blur_radius
    );
    let mark_id = parts.panels[0].rows[0].texts.unwrap()[0];
    assert_eq!(
        context.world().layout_style(mark_id).unwrap().width,
        Some(LengthSpec::Px(24.0))
    );
}

#[test]
fn despawning_theme_scope_leaves_no_stale_handles() {
    use nana_ui_core::{ThemeAppearance, ThemePatch, builtin_theme_arc};

    let mut context = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = context.create_component(doc, Stack::column(0.0)).unwrap();
    let outer = context
        .create_component(
            doc,
            patched_stack(ThemePatch::default().with_color(SemanticColorRole::Accent, PURPLE)),
        )
        .unwrap();
    let inner = context
        .create_component(
            doc,
            patched_stack(ThemePatch::default().with_color(SemanticColorRole::Warning, ORANGE)),
        )
        .unwrap();
    let btn = context
        .create_component(doc, crate::Button::new("Scoped"))
        .unwrap();

    context.append_child(root, outer).unwrap();
    context.append_child(outer, inner).unwrap();
    context.append_child(inner, btn).unwrap();
    layout(&mut context, doc);
    assert_eq!(context.active_theme_scopes(), 2);

    let mut despawn = MutationQueue::new();
    despawn.despawn_subtree(outer.stable_id());
    context.commit_mutations(despawn).unwrap();
    assert_eq!(context.active_theme_scopes(), 0);

    context.set_preset_theme(ThemeAppearance::Light).unwrap();
    context
        .set_document_theme(doc, Some(builtin_theme_arc(ThemeAppearance::Dark)))
        .unwrap();
    assert_eq!(context.active_theme_scopes(), 0);
}

#[test]
fn noop_theme_scope_install_and_remove_rebinds_descendant_scopes() {
    use nana_ui_core::{ThemePatch, ThemeScopeId};

    let mut context = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = context.create_component(doc, Stack::column(0.0)).unwrap();
    let host = context.create_component(doc, Stack::column(0.0)).unwrap();
    let btn = context
        .create_component(doc, crate::Button::new("Plain"))
        .unwrap();
    let inner = context
        .create_component(
            doc,
            patched_stack(ThemePatch::default().with_color(SemanticColorRole::Accent, PURPLE)),
        )
        .unwrap();
    let inner_btn = context
        .create_component(doc, crate::Button::new("Inner"))
        .unwrap();

    context.append_child(root, host).unwrap();
    context.append_child(host, btn).unwrap();
    context.append_child(host, inner).unwrap();
    context.append_child(inner, inner_btn).unwrap();
    layout(&mut context, doc);

    assert_eq!(context.active_theme_scopes(), 1);
    assert_eq!(
        context
            .inspect_node_theme(btn.stable_id())
            .unwrap()
            .active_scope,
        ThemeScopeId::ROOT
    );
    assert_eq!(
        context
            .inspect_node_theme(inner_btn.stable_id())
            .unwrap()
            .scope_chain
            .len(),
        1
    );

    // Install a ThemeScope on `host` whose resolved theme matches parent (changed == NONE).
    let same_text = context.world().theme().palette().text;
    let noop_patch = Arc::new(ThemePatch::default().with_color(SemanticColorRole::Text, same_text));
    let mut install = MutationQueue::new();
    install.set_style(
        host.stable_id(),
        NodeStyle {
            theme_patch: Some(noop_patch),
            ..NodeStyle::default()
        },
    );
    context.commit_mutations(install).unwrap();

    assert_eq!(context.active_theme_scopes(), 2);
    let host_scope = context
        .inspect_node_theme(host.stable_id())
        .unwrap()
        .active_scope;
    assert!(!host_scope.is_root());
    assert_eq!(
        context
            .inspect_node_theme(btn.stable_id())
            .unwrap()
            .active_scope,
        host_scope,
        "descendant must rebind to the newly attached no-op scope"
    );
    assert_eq!(
        context
            .inspect_node_theme(inner_btn.stable_id())
            .unwrap()
            .scope_chain
            .len(),
        2,
        "nested child scope must re-parent under host_scope"
    );

    // Remove the no-op ThemeScope: descendants must rebind cleanly back to ROOT (no stale scope ID).
    let mut remove = MutationQueue::new();
    remove.set_style(host.stable_id(), NodeStyle::default());
    context.commit_mutations(remove).unwrap();

    assert_eq!(context.active_theme_scopes(), 1);
    assert_eq!(
        context
            .inspect_node_theme(btn.stable_id())
            .unwrap()
            .active_scope,
        ThemeScopeId::ROOT
    );
    assert_eq!(
        context
            .inspect_node_theme(inner_btn.stable_id())
            .unwrap()
            .scope_chain
            .len(),
        1
    );
}

#[test]
fn root_theme_metrics_and_recipe_switch_isolates_document_theme_overrides() {
    use nana_ui_core::{
        ButtonKind, ThemeAppearance, ThemeMetricsPatch, ThemePatch, builtin_theme_arc,
    };

    let mut context = AppContext::new();
    let doc1 = DocumentId::new(1).unwrap();
    let doc2 = DocumentId::new(2).unwrap();
    let base_dark = builtin_theme_arc(ThemeAppearance::Dark);
    context
        .set_document_theme(doc2, Some(Arc::clone(&base_dark)))
        .unwrap();

    let btn1 = context
        .create_component(doc1, crate::Button::new("Root").kind(ButtonKind::Primary))
        .unwrap();
    let btn2 = context
        .create_component(
            doc2,
            crate::Button::new("DocOverride").kind(ButtonKind::Primary),
        )
        .unwrap();
    layout(&mut context, doc1);
    layout(&mut context, doc2);

    // Hover the button inside doc2 so an active hover transition exists on doc2.
    context
        .set_pointer_hover(doc2, 1, Some(btn2.stable_id()))
        .unwrap();
    layout(&mut context, doc2);
    let hover_anim =
        crate::component_animation_id(crate::component_animation_kinds::HOVER, btn2.stable_id())
            .unwrap();
    assert!(context.world().animation_is_active(hover_anim));

    let height_before_doc1 = context.world().layout_box(btn1.stable_id()).unwrap().height;
    let height_before_doc2 = context.world().layout_box(btn2.stable_id()).unwrap().height;
    assert!((height_before_doc1 - height_before_doc2).abs() < 0.01);

    let mut next_variant = base_dark.recipes().button().variant(ButtonKind::Primary);
    next_variant.foreground = SemanticColorRole::Warning;
    let next_root = Arc::new(
        ThemePatch::default()
            .with_metrics_patch(ThemeMetricsPatch {
                control_height: Some(base_dark.metrics().control_height + 10.0),
                ..Default::default()
            })
            .with_button_variant(ButtonKind::Primary, next_variant)
            .apply_to_compiled(&base_dark)
            .unwrap(),
    );
    context.set_theme_tokens(next_root).unwrap();
    layout(&mut context, doc1);
    layout(&mut context, doc2);

    let height_after_doc1 = context.world().layout_box(btn1.stable_id()).unwrap().height;
    let height_after_doc2 = context.world().layout_box(btn2.stable_id()).unwrap().height;
    assert!(
        height_after_doc1 > height_before_doc1 + 5.0,
        "non-overridden doc1 must reproject and grow with root control_height"
    );
    assert!(
        (height_after_doc2 - height_before_doc2).abs() < 0.01,
        "overridden doc2 must not reproject when root theme changes metrics"
    );
    assert!(
        context.world().animation_is_active(hover_anim),
        "root theme switch must not cancel hover transitions in overridden doc2"
    );
    let doc2_status = context
        .inspect_node_theme(btn2.stable_id())
        .unwrap()
        .last_invalidation_status
        .expect("invalidation status present");
    assert!(!doc2_status.affected);
}

#[test]
fn nested_scope_metrics_patch_and_rapid_toggle_preserve_isolation() {
    use nana_ui_core::{ThemeMetricsPatch, ThemePatch, UI_METRICS};

    let mut context = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let root = context.create_component(doc, Stack::column(0.0)).unwrap();
    let root_btn = context
        .create_component(doc, crate::Button::new("RootBtn"))
        .unwrap();
    let outer = context
        .create_component(
            doc,
            patched_stack(ThemePatch::default().with_metrics_patch(ThemeMetricsPatch {
                control_height: Some(38.0),
                ..Default::default()
            })),
        )
        .unwrap();
    let outer_btn = context
        .create_component(doc, crate::Button::new("OuterBtn"))
        .unwrap();
    let inner = context.create_component(doc, Stack::column(0.0)).unwrap();
    let inner_btn = context
        .create_component(doc, crate::Button::new("InnerBtn"))
        .unwrap();

    context.append_child(root, root_btn).unwrap();
    context.append_child(root, outer).unwrap();
    context.append_child(outer, outer_btn).unwrap();
    context.append_child(outer, inner).unwrap();
    context.append_child(inner, inner_btn).unwrap();
    layout(&mut context, doc);

    let inner_patch = Arc::new(ThemePatch::default().with_metrics_patch(ThemeMetricsPatch {
        control_height: Some(46.0),
        ..Default::default()
    }));

    for step in 0..6 {
        let enable = step % 2 == 0;
        let mut q = MutationQueue::new();
        q.set_style(
            inner.stable_id(),
            if enable {
                NodeStyle {
                    theme_patch: Some(Arc::clone(&inner_patch)),
                    ..NodeStyle::default()
                }
            } else {
                NodeStyle::default()
            },
        );
        context.commit_mutations(q).unwrap();
        layout(&mut context, doc);

        let expected_scopes = if enable { 2 } else { 1 };
        let expected_inner_h = if enable { 46.0 } else { 38.0 };
        assert_eq!(context.active_theme_scopes(), expected_scopes);
        assert!(
            (context
                .world()
                .layout_box(inner_btn.stable_id())
                .unwrap()
                .height
                - expected_inner_h)
                .abs()
                < 0.01
        );
        assert!(
            (context
                .world()
                .layout_box(outer_btn.stable_id())
                .unwrap()
                .height
                - 38.0)
                .abs()
                < 0.01
        );
        assert!(
            (context
                .world()
                .layout_box(root_btn.stable_id())
                .unwrap()
                .height
                - UI_METRICS.control_height)
                .abs()
                < 0.01
        );
    }
}
