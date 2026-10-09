//! Issue #261: style, theme and component state reach layout only through
//! the footprint of what they move.
//!
//! One document of mixed controls: rows of an auto-sized `Button`, a
//! `TextInput`, a `Card`, a `ListItem`, and a `Button` in a fixed cell.
//!
//! - Gate A: hovering from button to button lays nothing out.
//! - Gate B: a Light/Dark switch -- a palette, no metric -- shapes no text and
//!   lays nothing out.
//! - Gate C: moving the control padding step seeds exactly the buttons that
//!   use it; text inputs, cards and list rows are not measured, and a button
//!   in a fixed cell moves nothing above the cell.
//! - Gate D: one button's padding step moves its own closure, the same at 1k
//!   and 10k controls, without a fallback.
//! - Gate E: a thousand writes equal in effect -- spelled differently or
//!   not -- seed nothing and build no frontier.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{
    ControlPadding, FlexDirection, LayoutStyle, LengthSpec, ThemeAppearance, WorkCounters,
};

use super::reflow_oracle::{
    assert_matches_cold, bundled_face_shaper, measured, node, product_frame, styled,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{
    AppContext, Button, Card, LayoutViewport, ListItem, MutationQueue, NanaTextEngineShaper,
    TextInput,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(1000.0, 800.0)
}

/// The controls of one row, as they sit in the world.
struct Row {
    button: StableNodeId,
    input: StableNodeId,
    card: StableNodeId,
    list_item: StableNodeId,
    cell: StableNodeId,
    row: StableNodeId,
}

struct Controls {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    rows: Vec<Row>,
}

impl Controls {
    /// `rows` rows of five controls each.
    fn new(rows: u64) -> Self {
        Self::build(rows, 0.0, None)
    }

    /// The same, laid out first under a control padding step `delta` wider
    /// and, for a `roomy` row, with its free button on the roomy padding
    /// step.
    fn build(rows: u64, delta: f32, roomy: Option<usize>) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document, NodeKind::Document);
        queue.create(node(2), document, NodeKind::Element { tag: "page".into() });
        queue.insert(node(1), node(2), None);
        queue.set_style(
            node(2),
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(1000.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
        );
        let mut next = 3;
        let mut take = || {
            next += 1;
            node(next - 1)
        };
        let mut shells = Vec::new();
        for _ in 0..rows {
            let (row, cell) = (take(), take());
            queue.create(row, document, NodeKind::Element { tag: "row".into() });
            queue.insert(node(2), row, None);
            queue.set_style(
                row,
                styled(LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    ..LayoutStyle::default()
                }),
            );
            queue.create(cell, document, NodeKind::Element { tag: "cell".into() });
            queue.set_style(
                cell,
                styled(LayoutStyle {
                    width: Some(LengthSpec::Px(140.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    direction: Some(FlexDirection::Row),
                    ..LayoutStyle::default()
                }),
            );
            shells.push((row, cell));
        }
        context.commit_mutations(queue).unwrap();
        let mut rows = Vec::new();
        let mut queue = MutationQueue::new();
        for (index, (row, cell)) in shells.into_iter().enumerate() {
            let button = context
                .create_component(document, Button::new(format!("Act {index}")))
                .unwrap()
                .stable_id();
            let input = context
                .create_component(document, TextInput::new(format!("v{index}")))
                .unwrap()
                .stable_id();
            let card = context
                .create_component(document, Card::new())
                .unwrap()
                .stable_id();
            let list_item = context
                .create_component(document, ListItem::new(format!("Row {index}")))
                .unwrap()
                .stable_id();
            for id in [button, input, card, list_item, cell] {
                queue.insert(row, id, None);
            }
            let fixed_button = context
                .create_component(document, Button::new("Go"))
                .unwrap()
                .stable_id();
            queue.insert(cell, fixed_button, None);
            rows.push(Row {
                button,
                input,
                card,
                list_item,
                cell,
                row,
            });
        }
        context.commit_mutations(queue).unwrap();
        let mut controls = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            rows,
        };
        if delta != 0.0 {
            widen_control_padding(&mut controls.context, delta);
        }
        if let Some(row) = roomy {
            controls.make_roomy(controls.rows[row].button);
        }
        controls.frame();
        controls
    }

    /// Put `button` on the roomy padding step: design intent alone.
    fn make_roomy(&mut self, button: StableNodeId) {
        let mut style = self.world().node_style(button).unwrap().clone();
        style.control_padding_x = Some(ControlPadding::Roomy);
        let mut queue = MutationQueue::new();
        queue.set_style(button, style);
        self.context.commit_mutations(queue).unwrap();
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }
}

/// Install the current theme with the control padding step `delta` wider.
pub(super) fn widen_control_padding(context: &mut AppContext, delta: f32) {
    let mut model = context.world().style_model();
    model.metrics.control_padding_x += delta;
    let theme = Arc::new(
        context
            .world()
            .installed_theme()
            .clone()
            .with_style_model(model),
    );
    context.set_theme_tokens(theme).unwrap();
}

/// What a frame cost layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Cost {
    seeds: usize,
    frontier_measure: usize,
    frontier_placement: usize,
    measured: usize,
    placed: usize,
    local_fallbacks: usize,
    full_fallbacks: usize,
}

impl From<WorkCounters> for Cost {
    fn from(counters: WorkCounters) -> Self {
        Self {
            seeds: counters.layout_frontier_seeds,
            frontier_measure: counters.layout_frontier_nodes_measure,
            frontier_placement: counters.layout_frontier_nodes_placement,
            measured: counters.layout_measure_nodes,
            placed: counters.layout_placement_nodes,
            local_fallbacks: counters.layout_local_subtree_fallbacks,
            full_fallbacks: counters.layout_full_document_fallbacks,
        }
    }
}

/// Gate A. Hover moves from button to button across 2,000 controls: paint
/// work on the two it leaves and enters, and no layout at all.
#[test]
fn issue261_hover_paints_and_lays_nothing_out() {
    let _unguarded = skip_layout_verify();
    let mut controls = Controls::new(400);
    let buttons: Vec<StableNodeId> = controls.rows.iter().map(|row| row.button).collect();
    for (step, button) in buttons.iter().take(120).enumerate() {
        controls
            .context
            .set_pointer_hover_at(
                controls.document,
                1,
                Some(*button),
                Duration::from_millis(16 * step as u64),
            )
            .unwrap();
        let counters = controls.frame();
        assert_eq!(Cost::from(counters), Cost::default(), "hover {step}");
        assert_eq!(counters.component_state_layout_seeds, 0);
        assert_eq!(counters.theme_palette_layout_invalidations, 0);
        // The button it left, the one it entered, and the ancestors their
        // styles inherit from: a row, the page, the document.
        let styles = controls
            .world()
            .last_theme_work_counters()
            .style_nodes_considered;
        assert!(styles <= 2 * 4, "hover {step}: {styles} styles resolved");
    }
}

/// Gate B. Light and dark share every metric: a switch is paint. No text
/// shapes, no box is measured or placed, and the palette queues no layout.
#[test]
fn issue261_a_palette_switch_is_paint_only() {
    let _unguarded = skip_layout_verify();
    let mut controls = Controls::new(2_000);
    assert!(controls.world().len() >= 12_000);
    for mode in [
        ThemeAppearance::Light,
        ThemeAppearance::Dark,
        ThemeAppearance::Light,
    ] {
        controls.context.set_preset_theme(mode).unwrap();
        let counters = controls.frame();
        assert_eq!(Cost::from(counters), Cost::default(), "{mode:?}");
        assert_eq!(counters.theme_palette_layout_invalidations, 0);
        assert_eq!(counters.theme_to_layout_seeds, 0);
        let text = controls.world().last_text_work_counters();
        assert_eq!(text.text_nodes_shaped, 0, "{mode:?}");
        assert_eq!(counters.text_constraint_relayouts, 0, "{mode:?}");
    }
}

/// Gate C. The control padding step moves: exactly the buttons that use it
/// are seeded. Text inputs, cards and list rows keep their measurement, and
/// a button in a fixed cell moves nothing above its cell.
#[test]
fn issue261_a_metric_token_reaches_only_its_consumers() {
    let _unguarded = skip_layout_verify();
    let mut controls = Controls::new(2_000);
    let rows = controls.rows.len();
    widen_control_padding(&mut controls.context, 4.0);
    let counters = controls.frame();
    // Two buttons a row use the step: the free one and the one in the cell.
    assert_eq!(counters.theme_metric_dependents_invalidated, 2 * rows);
    assert_eq!(counters.theme_to_layout_seeds, 2 * rows);
    assert_eq!(counters.layout_full_document_fallbacks, 0);
    for row in &controls.rows {
        for other in [row.input, row.card, row.list_item] {
            assert!(!measured(&controls.context, other), "{other:?} measured");
        }
        // The cell keeps its size whatever the button in it does.
        assert!(
            !measured(&controls.context, row.cell),
            "cell {:?} measured",
            row.cell
        );
    }
    let mut cold = Controls::build(2_000, 4.0, None);
    assert_matches_cold(&mut controls.context, &mut cold.context, controls.document);
}

/// Gate D. One button's padding step moves: its row and the way up to it,
/// the same at 1k and 10k controls, with no fallback and no scan of the rest.
#[test]
fn issue261_one_state_geometry_change_costs_its_closure() {
    let _unguarded = skip_layout_verify();
    let mut costs = Vec::new();
    for rows in [200u64, 2_000] {
        let mut controls = Controls::new(rows);
        let row = &controls.rows[controls.rows.len() / 2];
        let (button, row_id) = (row.button, row.row);
        let before = controls.world().layout_box(button).unwrap();
        controls.make_roomy(button);
        let counters = controls.frame();
        assert_eq!(counters.style_to_layout_seeds, 1);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        let after = controls.world().layout_box(button).unwrap();
        assert!(after.width > before.width, "{before:?} -> {after:?}");
        let styles = controls
            .world()
            .last_theme_work_counters()
            .style_nodes_considered;
        assert!(styles <= 4, "{rows} rows: {styles} styles resolved");
        assert!(measured(&controls.context, row_id));
        costs.push(Cost::from(counters));
    }
    assert_eq!(costs[0], costs[1]);
}

/// Gate E. A thousand writes equal in effect: the same style again, and the
/// same layout spelled another way. Nothing is seeded and no frontier built.
#[test]
fn issue261_equivalent_writes_seed_nothing() {
    let mut controls = Controls::new(200);
    // A row's own style again, and the page's column axis spelled `None`
    // and `Column` by turns: both lay out as the style already in place.
    let row = controls.rows[10].row;
    let row_style = controls.world().node_style(row).unwrap().clone();
    let page = node(2);
    let page_style = controls.world().node_style(page).unwrap().clone();
    let mut page_respelled = page_style.clone();
    Arc::make_mut(&mut page_respelled.layout).direction = None;
    let before = controls.context.last_work_counters();
    for step in 0..1_000 {
        let mut queue = MutationQueue::new();
        queue.set_style(row, row_style.clone());
        queue.set_style(
            page,
            if step % 2 == 0 {
                page_respelled.clone()
            } else {
                page_style.clone()
            },
        );
        controls.context.commit_mutations(queue).unwrap();
    }
    // Nothing to do: no layout seed, no style to resolve, nothing to paint.
    let work = controls.context.take_system_work();
    assert!(work.layout_frontier_seeds.is_empty());
    assert!(work.style.is_empty());
    assert!(work.render_extraction.is_empty());
    // An idle drain leaves the last counters and folds what the writes
    // counted onto them.
    let after = controls.context.last_work_counters();
    assert_eq!(
        after.equivalent_style_layout_skips - before.equivalent_style_layout_skips,
        2_000
    );
    assert_eq!(after.style_to_layout_seeds, before.style_to_layout_seeds);
}

/// A recipe patch reprojects the buttons that read it, and changes only the
/// roles they paint with: nothing is laid out.
#[test]
fn issue261_a_button_recipe_patch_lays_nothing_out() {
    let _unguarded = skip_layout_verify();
    let mut controls = Controls::new(400);
    let kind = nana_ui_core::ButtonKind::Ghost;
    let mut definition = nana_ui_core::ThemeDefinition::NANA_DARK;
    let variant = nana_ui_core::ButtonVariantDraft {
        hovered_background: Some(nana_ui_core::SemanticColorRole::Warning),
        ..definition.components.button.variant(kind)
    };
    definition.components.button = definition.components.button.with(kind, variant);
    assert!(
        controls
            .context
            .set_theme_definition(&definition.bump())
            .unwrap()
    );
    let counters = controls.frame();
    assert_eq!(Cost::from(counters), Cost::default());
    assert_eq!(counters.style_to_layout_seeds, 0);
    assert_eq!(counters.theme_to_layout_seeds, 0);
    assert_eq!(
        controls
            .world()
            .node_style(controls.rows[0].button)
            .unwrap()
            .interaction
            .hovered
            .background,
        Some(nana_ui_core::SemanticColorRole::Warning)
    );
}

/// A metrics install and a write of intent alone, each pass checked against
/// a full layout, and the end against the same inputs laid out once.
#[test]
fn issue261_metric_and_intent_changes_match_a_full_layout_every_pass() {
    let mut controls = Controls::new(30);
    widen_control_padding(&mut controls.context, 6.0);
    controls.frame();
    controls.make_roomy(controls.rows[7].button);
    controls.frame();
    let mut cold = Controls::build(30, 6.0, Some(7));
    assert_matches_cold(&mut controls.context, &mut cold.context, controls.document);
}
