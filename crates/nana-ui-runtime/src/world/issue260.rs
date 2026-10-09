//! Issue #260: text, editable text and language reach layout only through
//! the metrics they export.
//!
//! One document of labelled fixed cards, at 1k or 10k nodes. Its middle
//! group holds the nodes the gates edit: a fixed 200x32 box with a text
//! input, two paragraphs in boxes of their own, and a language scope of a
//! hundred labels with a nested scope that names its own language.
//!
//! - Gate A: typing into the fixed input lays out nothing above its box and
//!   no sibling, and costs the same at 1k and 10k nodes.
//! - Gate B: "99" -> "98" keeps the label's exported metrics: no text seed,
//!   no parent reflow, no frontier, no changed result.
//! - Gate C: 240 width changes of a paragraph's box lay the text out again
//!   from the runs it shaped once; the other paragraph does no text work and
//!   is never measured.
//! - Gate D: a scope's language reaches its hundred labels and no other
//!   text; the nested scope keeps its shape; the application's language and
//!   the engine's fallback reach exactly the text that inherits them.
//! - Gate E: caret and selection moves lay nothing out.

use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

use super::reflow_oracle::{
    Builder, FILLER_GROUP_NODES, admitted_nodes, assert_matches_cold, bundled_face_shaper, column,
    fixed, measured, node, product_frame, styled,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{
    AppContext, LanguageTag, LayoutViewport, MutationQueue, NanaTextEngineShaper, TextContent,
    TextInput, TextSelection, TextShaper,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(320.0, 480.0)
}

/// The box Gate C narrows and widens: a fixed height, so its text rewraps
/// inside it and nothing outside moves.
fn paragraph_box_layout(width: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(120.0)),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    }
}

fn language(tag: &str) -> Option<LanguageTag> {
    LanguageTag::new(tag)
}

const SCOPE_LABELS: u64 = 100;
const NESTED_LABELS: u64 = 10;
const PARAGRAPH: &str = "a paragraph long enough to wrap over a few lines of the box it is in";

struct Doc {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    groups: u64,
    editor_box: StableNodeId,
    editor: StableNodeId,
    paragraph_box: StableNodeId,
    paragraph: StableNodeId,
    other_paragraph: StableNodeId,
    scope: StableNodeId,
    nested_scope: StableNodeId,
}

impl Doc {
    /// Group `group` of the filler, which takes the ids from 3 on.
    fn group(group: u64) -> StableNodeId {
        node(3 + group * FILLER_GROUP_NODES)
    }

    fn card(group: u64, card: u64) -> StableNodeId {
        node(Self::group(group).get() + 1 + 2 * card)
    }

    fn label(group: u64, card: u64) -> StableNodeId {
        node(Self::card(group, card).get() + 1)
    }

    fn new(nodes: u64) -> Self {
        Self::with_input(nodes, "")
    }

    /// About `nodes` nodes, the input holding `value`.
    fn with_input(nodes: u64, value: &str) -> Self {
        let extras = 6 + 2 * (SCOPE_LABELS + NESTED_LABELS);
        let groups = (nodes.saturating_sub(2 + extras) / FILLER_GROUP_NODES).max(1);
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 320.0);
        b.filler(page, groups);
        let middle = Self::group(groups / 2);
        let editor_box = b.element(middle, fixed(200.0, 32.0));
        let paragraph_box = b.element(middle, paragraph_box_layout(200.0));
        let paragraph = b.label(paragraph_box, PARAGRAPH);
        let other_box = b.element(middle, column(Some(200.0)));
        let other_paragraph = b.label(other_box, PARAGRAPH);
        let scope = b.element(middle, column(None));
        for _ in 0..SCOPE_LABELS {
            let card = b.element(scope, fixed(200.0, 40.0));
            b.label(card, "scoped");
        }
        let nested_scope = b.element(scope, column(None));
        b.queue.set_language(nested_scope, language("ko"));
        for _ in 0..NESTED_LABELS {
            let card = b.element(nested_scope, fixed(200.0, 40.0));
            b.label(card, "nested");
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let editor = context
            .create_component(document, TextInput::new(value.to_owned()))
            .unwrap()
            .stable_id();
        let mut queue = MutationQueue::new();
        queue.insert(editor_box, editor, None);
        context.commit_mutations(queue).unwrap();
        let mut doc = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            groups,
            editor_box,
            editor,
            paragraph_box,
            paragraph,
            other_paragraph,
            scope,
            nested_scope,
        };
        doc.frame();
        doc
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    fn commit(&mut self, edit: impl FnOnce(&mut MutationQueue)) {
        let mut queue = MutationQueue::new();
        edit(&mut queue);
        self.context.commit_mutations(queue).unwrap();
    }

    fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn within(&self, id: StableNodeId, root: StableNodeId) -> bool {
        std::iter::successors(Some(id), |id| self.world().parent_id(*id)).any(|id| id == root)
    }

    /// Text nodes of the document, in document order.
    fn text_nodes(&self) -> Vec<StableNodeId> {
        self.world()
            .document_order(self.document)
            .into_iter()
            .filter(|id| {
                self.world()
                    .nodes
                    .get(*id)
                    .is_some_and(|record| matches!(record.kind.as_ref(), NodeKind::Text))
            })
            .collect()
    }

    fn shape_revisions(&self) -> Vec<(StableNodeId, u32)> {
        self.text_nodes()
            .into_iter()
            .map(|id| (id, self.world().text_revisions(id).unwrap().shape))
            .collect()
    }
}

/// What a frame cost layout, in the units the gates name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cost {
    seeds: usize,
    frontier_measure: usize,
    frontier_placement: usize,
    edges: usize,
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
            edges: counters.layout_dependency_edges_visited,
            measured: counters.layout_measure_nodes,
            placed: counters.layout_placement_nodes,
            local_fallbacks: counters.layout_local_subtree_fallbacks,
            full_fallbacks: counters.layout_full_document_fallbacks,
        }
    }
}

/// Type `chars` characters into the fixed input, one frame each.
fn typed(doc: &mut Doc, chars: usize) -> Vec<Cost> {
    let editor = doc.editor;
    let mut costs = Vec::with_capacity(chars);
    for typed in 0..chars {
        doc.commit(|queue| queue.replace_text_selection(editor, "a"));
        let counters = doc.frame();
        let cost = Cost::from(counters);
        assert_eq!(
            (cost.local_fallbacks, cost.full_fallbacks),
            (0, 0),
            "{cost:?}"
        );
        for id in admitted_nodes(&doc.context, doc.document) {
            assert!(
                doc.within(id, doc.editor_box),
                "character {typed}: {id:?} outside the input's fixed box was laid out"
            );
        }
        costs.push(cost);
    }
    costs
}

/// Gate A. Each character the input takes lays out its own fixed box at
/// most, and the same work at 1k and 10k nodes.
#[test]
fn issue260_typing_into_a_fixed_input_lays_out_nothing_above_its_box() {
    let mut small = Doc::new(1_000);
    let mut large = Doc::new(10_000);
    assert!(large.world().len() >= 9_900);
    let small_costs = typed(&mut small, 50);
    let large_costs = typed(&mut large, 50);
    assert_eq!(small_costs, large_costs);
    assert_eq!(
        large.world().text_input(large.editor).unwrap().value.len(),
        50
    );
}

/// Gate A over a thousand characters in the 10k document.
#[test]
fn issue260_a_thousand_characters_stay_inside_the_fixed_input() {
    // A full layout after each of a thousand frames would cost more than the
    // frames; the document is checked against a cold layout at the end.
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    let mut doc = Doc::new(10_000);
    typed(&mut doc, 1_000);
    let mut cold = Doc::with_input(10_000, &"a".repeat(1_000));
    assert_matches_cold(&mut doc.context, &mut cold.context, doc.document);
}

fn labels_cost(nodes: u64) -> (WorkCounters, bool) {
    let mut doc = Doc::new(nodes);
    let label = Doc::label(doc.groups / 2, 5);
    doc.commit(|queue| queue.set_text(label, TextContent { value: "99".into() }));
    doc.frame();
    let box_before = doc.world().layout_box(label);
    doc.commit(|queue| queue.set_text(label, TextContent { value: "98".into() }));
    let counters = doc.frame();
    (counters, doc.world().layout_box(label) == box_before)
}

/// Gate B. "99" and "98" export the same metrics: the text shapes again,
/// queues no layout seed, reflows no parent, and no result changes.
#[test]
fn issue260_text_with_the_same_external_metrics_reflows_no_parent() {
    let (small, small_box) = labels_cost(1_000);
    let (large, large_box) = labels_cost(10_000);
    for (counters, box_held) in [(small, small_box), (large, large_box)] {
        assert!(box_held);
        assert_eq!(counters.text_external_metric_unchanged, 1, "{counters:?}");
        assert_eq!(counters.text_external_metric_changes, 0);
        assert_eq!(counters.text_reflow_seeds, 0);
        assert_eq!(counters.text_parent_reflows, 0);
        assert_eq!(counters.layout_frontier_seeds, 0);
        assert_eq!(counters.layout_result_changed, 0);
    }
    assert_eq!(Cost::from(small), Cost::from(large));
}

/// Gate C. The paragraph's box changes width 240 times. Its text lays out
/// again from the runs it shaped once; the other paragraph does no work.
#[test]
fn issue260_width_only_changes_reuse_the_shaped_runs() {
    let mut doc = Doc::new(1_000);
    let (paragraph_box, paragraph, other) = (doc.paragraph_box, doc.paragraph, doc.other_paragraph);
    let other_revisions = doc.world().text_revisions(other).unwrap();
    let mut relayouts = 0;
    for step in 0..240u32 {
        let width = 120.0 + (step % 80) as f32;
        doc.commit(|queue| queue.set_style(paragraph_box, styled(paragraph_box_layout(width))));
        let counters = doc.frame();
        let text = doc.world().last_text_work_counters();
        assert_eq!(
            text.text_nodes_shaped, 0,
            "width {width}: the text shaped again"
        );
        assert_eq!(text.shape_cache_misses.unwrap_or(0), 0, "width {width}");
        relayouts += counters.text_constraint_relayouts;
        // A sizing write does not say which axis it moved, so the frontier
        // hands later siblings a placement check; the other paragraph is
        // never measured, shaped or laid out as text.
        assert!(
            !measured(&doc.context, other),
            "width {width}: the other paragraph measured"
        );
        assert_eq!(doc.world().text_revisions(other), Some(other_revisions));
        assert_eq!(
            doc.world()
                .layout_box(paragraph)
                .map(|layout| layout.width <= width),
            Some(true)
        );
    }
    assert!(relayouts > 0, "no relayout reached the paragraph");
}

/// Gate D. Languages reach exactly the text that inherits them.
#[test]
fn issue260_a_language_change_reaches_only_the_text_that_inherits_it() {
    let mut doc = Doc::new(10_000);
    let (scope, nested) = (doc.scope, doc.nested_scope);
    let outside: Vec<StableNodeId> = doc
        .text_nodes()
        .into_iter()
        .filter(|id| !doc.within(*id, scope))
        .collect();
    assert!(outside.len() > 400);

    // The scope's language: its hundred labels, not the nested ten.
    let before = doc.shape_revisions();
    doc.commit(|queue| queue.set_language(scope, language("ja")));
    let counters = doc.frame();
    assert_eq!(
        counters.text_language_scope_invalidations,
        SCOPE_LABELS as usize
    );
    assert_eq!(counters.text_literal_nodes_invalidated_by_language, 0);
    let moved: Vec<StableNodeId> = before
        .iter()
        .zip(doc.shape_revisions())
        .filter(|((_, old), (_, new))| old != new)
        .map(|((id, _), _)| *id)
        .collect();
    assert_eq!(moved.len(), SCOPE_LABELS as usize);
    assert!(
        moved
            .iter()
            .all(|id| doc.within(*id, scope) && !doc.within(*id, nested))
    );
    assert!(doc.world().last_text_work_counters().text_nodes_shaped <= SCOPE_LABELS as usize + 1);

    // The application's language: every text outside the scope.
    doc.context.set_default_language(language("fr"));
    let counters = doc.frame();
    assert_eq!(counters.text_language_scope_invalidations, outside.len());
    assert_eq!(counters.text_literal_nodes_invalidated_by_language, 0);

    // The engine's fallback sits under the application's: nothing moves.
    // An idle frame leaves the last counters in place, so the revisions say
    // whether any text was invalidated.
    let engine = doc.shaper.text_engine().unwrap();
    nana_text::lock_text_engine(&engine).set_language(language("de"));
    let before = doc.shape_revisions();
    doc.frame();
    assert_eq!(doc.shape_revisions(), before);
    // With no application language, the fallback is the root's again.
    doc.context.set_default_language(None);
    let counters = doc.frame();
    assert_eq!(counters.text_language_scope_invalidations, outside.len());
    assert_eq!(counters.text_literal_nodes_invalidated_by_language, 0);

    // Writing the language the scope already has is no change at all.
    let before = doc.shape_revisions();
    doc.commit(|queue| queue.set_language(scope, language("ja")));
    doc.frame();
    assert_eq!(doc.shape_revisions(), before);
}

/// Gate E. Moving the caret and the selection lays nothing out.
#[test]
fn issue260_caret_and_selection_moves_lay_nothing_out() {
    let mut doc = Doc::new(1_000);
    let editor = doc.editor;
    doc.commit(|queue| queue.replace_text_selection(editor, "hello world"));
    doc.frame();
    let len = "hello world".len();
    let selections = (0..=len)
        .map(TextSelection::caret)
        .chain((0..len).map(|start| TextSelection {
            anchor: start,
            focus: len,
            ..TextSelection::caret(start)
        }));
    for selection in selections {
        doc.commit(|queue| queue.set_text_selection(editor, selection));
        let counters = doc.frame();
        let cost = Cost::from(counters);
        assert_eq!(
            (cost.seeds, cost.frontier_measure, cost.frontier_placement),
            (0, 0, 0),
            "{selection:?}: {cost:?}"
        );
        assert_eq!(
            (cost.measured, cost.placed),
            (0, 0),
            "{selection:?}: {cost:?}"
        );
        assert_eq!(counters.text_reflow_seeds, 0);
    }
}
