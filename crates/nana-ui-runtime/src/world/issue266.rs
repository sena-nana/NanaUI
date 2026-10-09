//! Issue #266: a typography scale reaches the text that inherits it, and
//! layout follows only the metrics that moved.
//!
//! A scale is a retained, scoped value: a node's own
//! ([`MutationQueue::set_text_scale`]), else its parent's, else its
//! window's, else the application's. It multiplies the font size and an
//! absolute line height; box lengths keep the size before it.
//!
//! - Gate A: a fixed panel of 2,000 labels in a document of 10k and 100k
//!   nodes goes from 1 to 1.25. The change visits the panel's own nodes and
//!   reaches its 2,000 labels; a nested scope pinned at 1 is not visited; no
//!   text, style or measurement outside the panel moves, and the frame costs
//!   the same at both sizes.
//! - Gate B: a thousand fixed-size buttons take the scale. Their labels lay
//!   out again inside them; no parent reflows and no sibling is laid out.
//! - Gate C: a row that aligns its boxes by baseline lays out again; a row
//!   that does not, and a baseline row outside the scope, do not measure.
//! - Gate D: setting the scale a scope, a window or the application already
//!   has does no work at all.
//! - Gate E: two windows of 10k nodes; one's scale reaches none of the
//!   other's text or layout.

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{AlignSpec, FlexDirection, LayoutStyle, WorkCounters};

use super::reflow_oracle::{
    Builder, FILLER_GROUP_NODES, admitted, assert_matches_cold, bundled_face_shaper, column, fixed,
    measured, product_frame,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{
    AppContext, Button, LayoutBox, LayoutViewport, MutationQueue, NanaTextEngineShaper, TextContent,
};

const SCALE: f32 = 1.25;
const SCOPE_ROWS: u64 = 1_000;
const PINNED_LABELS: u64 = 100;
const BUTTONS: u64 = 1_000;
const BASELINE_BOXES: u64 = 20;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(320.0, 480.0)
}

fn row(align_items: AlignSpec) -> LayoutStyle {
    LayoutStyle {
        direction: Some(FlexDirection::Row),
        align_items,
        ..LayoutStyle::default()
    }
}

/// A context with one or more documents and the bundled-face shaper.
struct Doc {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
}

impl Doc {
    fn new(context: AppContext, document: DocumentId) -> Self {
        Self {
            context,
            document,
            shaper: bundled_face_shaper(),
        }
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
        self.frame_of(self.document)
    }

    fn frame_of(&mut self, document: DocumentId) -> WorkCounters {
        product_frame(&mut self.context, document, viewport(), &mut self.shaper)
    }

    fn order(&self, document: DocumentId) -> Vec<StableNodeId> {
        self.world().document_order(document)
    }

    fn is_text(&self, id: StableNodeId) -> bool {
        self.world()
            .nodes
            .get(id)
            .is_some_and(|record| matches!(record.kind.as_ref(), NodeKind::Text))
    }

    /// Every text node of `document` with its shape revision.
    fn shape_revisions(&self, document: DocumentId) -> Vec<(StableNodeId, u32)> {
        self.order(document)
            .into_iter()
            .filter(|id| self.is_text(*id))
            .map(|id| (id, self.world().text_revisions(id).unwrap().shape))
            .collect()
    }

    /// The text whose shape revision moved since `before`.
    fn reshaped(&self, before: &[(StableNodeId, u32)]) -> HashSet<StableNodeId> {
        before
            .iter()
            .filter(|(id, shape)| self.world().text_revisions(*id).unwrap().shape != *shape)
            .map(|(id, _)| *id)
            .collect()
    }

    fn boxes(&self, document: DocumentId) -> Vec<(StableNodeId, Option<LayoutBox>)> {
        self.order(document)
            .into_iter()
            .map(|id| (id, self.world().layout_box(id)))
            .collect()
    }

    fn within(&self, id: StableNodeId, root: StableNodeId) -> bool {
        std::iter::successors(Some(id), |id| self.world().parent_id(*id)).any(|id| id == root)
    }

    /// `root` and every node under it that inherits its scale.
    fn scope_of(&self, root: StableNodeId) -> HashSet<StableNodeId> {
        let mut scope = HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            scope.insert(id);
            let children = self.world().node(id).unwrap().children;
            stack.extend(
                children
                    .into_iter()
                    .filter(|child| self.world().node_text_scale(*child).is_none()),
            );
        }
        scope
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
    children_walked: usize,
    result_children: usize,
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
            children_walked: counters.layout_children_measured,
            result_children: counters.layout_result_children_visited,
            local_fallbacks: counters.layout_local_subtree_fallbacks,
            full_fallbacks: counters.layout_full_document_fallbacks,
        }
    }
}

/// Gate A's document: filler around a fixed 300x400 panel -- the scope --
/// of [`SCOPE_ROWS`] rows of two labels, the last of its children a nested
/// scope of [`PINNED_LABELS`] fixed cards pinned at scale 1.
struct PanelDoc {
    doc: Doc,
    panel: StableNodeId,
    pinned: StableNodeId,
}

impl PanelDoc {
    fn new(nodes: u64, scale: Option<f32>) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut builder, page) = Builder::page(document, 1, 320.0);
        let panel_nodes = 1 + 3 * SCOPE_ROWS + 1 + 2 * PINNED_LABELS;
        let groups = (nodes.saturating_sub(2 + panel_nodes) / FILLER_GROUP_NODES).max(2);
        builder.filler(page, groups / 2);
        let panel = builder.element(
            page,
            LayoutStyle {
                direction: Some(FlexDirection::Column),
                ..fixed(300.0, 400.0)
            },
        );
        for _ in 0..SCOPE_ROWS {
            let line = builder.element(panel, row(AlignSpec::Start));
            builder.label(line, "scaled");
            builder.label(line, "text");
        }
        let pinned = builder.element(panel, column(None));
        for _ in 0..PINNED_LABELS {
            let card = builder.element(pinned, fixed(200.0, 40.0));
            builder.label(card, "pinned");
        }
        builder.filler(page, groups - groups / 2);
        builder.queue.set_text_scale(pinned, Some(1.0));
        if scale.is_some() {
            builder.queue.set_text_scale(panel, scale);
        }
        let mut context = AppContext::new();
        context.commit_mutations(builder.queue).unwrap();
        let mut doc = Doc::new(context, document);
        doc.frame();
        Self { doc, panel, pinned }
    }
}

/// Gate A at `nodes` nodes: what the switch cost, checked against the
/// gate's exact counts.
fn panel_switch(nodes: u64) -> Cost {
    let mut panel = PanelDoc::new(nodes, None);
    let doc = &mut panel.doc;
    let document = doc.document;
    assert!(doc.world().len() as u64 >= nodes * 99 / 100);
    let scope = doc.scope_of(panel.panel);
    let labels = 2 * SCOPE_ROWS as usize;
    assert_eq!(scope.len(), 1 + 3 * SCOPE_ROWS as usize);
    let before = doc.shape_revisions(document);
    let boxes = doc.boxes(document);

    let target = panel.panel;
    doc.commit(|queue| queue.set_text_scale(target, Some(SCALE)));
    let counters = doc.frame();

    // The change visited the scope and reached its labels: not the pinned
    // scope, not one node outside.
    assert_eq!(counters.typography_scale_scope_nodes_scanned, scope.len());
    assert_eq!(counters.typography_scale_equivalent_skips, 0);
    assert_eq!(counters.typography_scale_dependents_notified, labels);
    assert_eq!(counters.typography_scale_text_relayouts, labels);
    // Each label's row sizes to it: the metric delta reaches the row.
    assert_eq!(counters.typography_scale_parent_reflows, labels);
    let reshaped = doc.reshaped(&before);
    assert_eq!(reshaped.len(), labels);
    assert!(reshaped.iter().all(|id| scope.contains(id)));
    assert!(!reshaped.iter().any(|id| doc.within(*id, panel.pinned)));
    // Style: the scope, and the way up to it.
    let considered = doc
        .world()
        .last_theme_work_counters()
        .style_nodes_considered;
    assert!(
        considered >= scope.len() && considered <= scope.len() + 3,
        "{considered} styles considered for a scope of {}",
        scope.len()
    );
    // Layout: the panel is fixed, so nothing outside it measures and
    // nothing outside it moves.
    for id in doc.order(document) {
        if !doc.within(id, panel.panel) {
            assert!(
                !measured(&doc.context, id),
                "{id:?} outside the panel measured"
            );
        }
    }
    for ((id, old), (_, new)) in boxes.iter().zip(doc.boxes(document)) {
        if !doc.within(*id, panel.panel) || *id == panel.panel {
            assert_eq!(*old, new, "{id:?} outside the panel moved");
        }
    }
    let cost = Cost::from(counters);
    assert_eq!(
        (cost.local_fallbacks, cost.full_fallbacks),
        (0, 0),
        "{cost:?}"
    );

    let mut cold = PanelDoc::new(nodes, Some(SCALE));
    assert_matches_cold(&mut doc.context, &mut cold.doc.context, document);
    cost
}

/// Gate A. A panel's scale reaches its 2,000 labels and nothing outside it,
/// and costs the same in a document of 10k nodes as of 100k.
#[test]
fn issue266_a_scope_scale_reaches_its_text_and_nothing_outside() {
    // A full layout after each pass of a 100k document would cost more than
    // the frames under test; both sizes end against the cold oracle.
    let _unguarded = skip_layout_verify();
    let large = panel_switch(10_000);
    let huge = panel_switch(100_000);
    assert_eq!(large, huge);
}

/// Gate B's document: a column of [`BUTTONS`] fixed 96x28 buttons, each
/// followed by a fixed spacer, after some filler.
struct ButtonDoc {
    doc: Doc,
    bar: StableNodeId,
    buttons: Vec<StableNodeId>,
    spacers: Vec<StableNodeId>,
}

impl ButtonDoc {
    fn new(scale: Option<f32>) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut builder, page) = Builder::page(document, 1, 320.0);
        builder.filler(page, 20);
        let bar = builder.element(page, column(None));
        let spacers: Vec<StableNodeId> = (0..BUTTONS)
            .map(|_| builder.element(bar, fixed(96.0, 4.0)))
            .collect();
        if scale.is_some() {
            builder.queue.set_text_scale(bar, scale);
        }
        let mut context = AppContext::new();
        context.commit_mutations(builder.queue).unwrap();
        let mut queue = MutationQueue::new();
        let mut buttons = Vec::new();
        for (index, spacer) in spacers.iter().enumerate() {
            let button = context
                .create_component(
                    document,
                    Button::new(format!("Action {index}")).layout(Arc::new(fixed(96.0, 28.0))),
                )
                .unwrap()
                .stable_id();
            queue.insert(bar, button, Some(*spacer));
            buttons.push(button);
        }
        context.commit_mutations(queue).unwrap();
        let mut doc = Doc::new(context, document);
        doc.frame();
        Self {
            doc,
            bar,
            buttons,
            spacers,
        }
    }
}

/// Gate B. The buttons' labels take the scale inside their fixed boxes: no
/// parent reflows, no spacer is laid out, no box moves.
#[test]
fn issue266_fixed_buttons_keep_a_label_scale_inside() {
    let mut buttons = ButtonDoc::new(None);
    let doc = &mut buttons.doc;
    let document = doc.document;
    let boxes = doc.boxes(document);
    let bar = buttons.bar;
    doc.commit(|queue| queue.set_text_scale(bar, Some(SCALE)));
    let counters = doc.frame();

    assert_eq!(
        counters.typography_scale_dependents_notified,
        BUTTONS as usize
    );
    assert_eq!(counters.typography_scale_text_relayouts, BUTTONS as usize);
    assert_eq!(counters.typography_scale_parent_reflows, 0);
    assert_eq!(counters.text_parent_reflows, 0);
    assert_eq!(counters.text_external_metric_changes, BUTTONS as usize);
    // Nothing above the buttons measures; no spacer is laid out at all.
    for id in doc.order(document) {
        if !buttons.buttons.contains(&id) {
            assert!(!measured(&doc.context, id), "{id:?} measured");
        }
    }
    for spacer in &buttons.spacers {
        assert!(
            !admitted(&doc.context, *spacer),
            "spacer {spacer:?} laid out"
        );
    }
    assert_eq!(doc.boxes(document), boxes, "a box moved");
    let cost = Cost::from(counters);
    assert_eq!(
        (cost.local_fallbacks, cost.full_fallbacks),
        (0, 0),
        "{cost:?}"
    );
    // The buttons themselves and the way up to them, nothing more.
    assert!(
        cost.placed <= BUTTONS as usize + 4,
        "{} boxes placed for {BUTTONS} buttons: {cost:?}",
        cost.placed
    );

    let mut cold = ButtonDoc::new(Some(SCALE));
    assert_eq!(cold.buttons, buttons.buttons);
    assert_matches_cold(&mut doc.context, &mut cold.doc.context, document);
}

/// `count` fixed 64x28 buttons, created in order and appended to `row`.
fn button_row(
    context: &mut AppContext,
    document: DocumentId,
    row: StableNodeId,
    count: u64,
    label: &str,
) -> Vec<StableNodeId> {
    let mut queue = MutationQueue::new();
    let buttons = (0..count)
        .map(|index| {
            let button = context
                .create_component(
                    document,
                    Button::new(format!("{label}{index}")).layout(Arc::new(fixed(64.0, 28.0))),
                )
                .unwrap()
                .stable_id();
            queue.insert(row, button, None);
            button
        })
        .collect();
    context.commit_mutations(queue).unwrap();
    buttons
}

/// Gate C's document: a scope holding a row that aligns fixed buttons by
/// baseline -- one of them pinned at scale 1 -- and a row of fixed buttons
/// that does not; after the scope, a baseline row of buttons it does not
/// reach. A labelled button's baseline is its label's.
struct BaselineDoc {
    doc: Doc,
    scope: StableNodeId,
    baseline_row: StableNodeId,
    pinned: StableNodeId,
    plain_row: StableNodeId,
    plain: Vec<StableNodeId>,
    outside_row: StableNodeId,
    outside: Vec<StableNodeId>,
}

impl BaselineDoc {
    fn new(scale: Option<f32>) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut builder, page) = Builder::page(document, 1, 320.0);
        builder.filler(page, 10);
        let scope = builder.element(page, column(None));
        let baseline_row = builder.element(scope, row(AlignSpec::Baseline));
        let plain_row = builder.element(scope, row(AlignSpec::Start));
        let outside_row = builder.element(page, row(AlignSpec::Baseline));
        builder.filler(page, 10);
        if scale.is_some() {
            builder.queue.set_text_scale(scope, scale);
        }
        let mut context = AppContext::new();
        context.commit_mutations(builder.queue).unwrap();
        let baseline = button_row(&mut context, document, baseline_row, BASELINE_BOXES, "b");
        let plain = button_row(&mut context, document, plain_row, BASELINE_BOXES, "p");
        let outside = button_row(&mut context, document, outside_row, BASELINE_BOXES, "o");
        let pinned = baseline[BASELINE_BOXES as usize / 2];
        let mut queue = MutationQueue::new();
        queue.set_text_scale(pinned, Some(1.0));
        context.commit_mutations(queue).unwrap();
        let mut doc = Doc::new(context, document);
        doc.frame();
        Self {
            doc,
            scope,
            baseline_row,
            pinned,
            plain_row,
            plain,
            outside_row,
            outside,
        }
    }
}

/// Gate C. Labels that grow move their buttons' baselines. The row that
/// aligns by them lays out again and aligns the pinned button anew; the row
/// that does not keeps every box, and the baseline row the scale does not
/// reach is not measured.
#[test]
fn issue266_baseline_changes_reach_only_baseline_aligned_rows() {
    let mut fixture = BaselineDoc::new(None);
    let doc = &mut fixture.doc;
    let document = doc.document;
    let pinned_before = doc.world().layout_box(fixture.pinned).unwrap();
    let plain_before: Vec<_> = fixture
        .plain
        .iter()
        .map(|id| doc.world().layout_box(*id))
        .collect();
    let outside_text = fixture
        .outside
        .iter()
        .map(|id| doc.world().text_revisions(*id).unwrap().shape)
        .collect::<Vec<_>>();
    let scope = fixture.scope;
    doc.commit(|queue| queue.set_text_scale(scope, Some(SCALE)));
    let counters = doc.frame();

    let scaled_baseline = BASELINE_BOXES as usize - 1;
    assert_eq!(
        counters.typography_scale_dependents_notified,
        scaled_baseline + BASELINE_BOXES as usize
    );
    // A button its row aligns by baseline exports its label's baseline: the
    // row lays out again. A button in the plain row keeps the change.
    assert_eq!(counters.typography_scale_parent_reflows, scaled_baseline);
    assert!(measured(&doc.context, fixture.baseline_row));
    let pinned_after = doc.world().layout_box(fixture.pinned).unwrap();
    assert!(
        pinned_after.y > pinned_before.y,
        "the pinned button was not aligned to the grown baselines: {pinned_before:?} -> {pinned_after:?}"
    );
    // The plain row: not measured, no box moved.
    assert!(!measured(&doc.context, fixture.plain_row));
    let plain_after: Vec<_> = fixture
        .plain
        .iter()
        .map(|id| doc.world().layout_box(*id))
        .collect();
    assert_eq!(plain_after, plain_before);
    // The baseline row outside the scope: its labels did not move, and it
    // and its buttons were not measured.
    assert!(!measured(&doc.context, fixture.outside_row));
    for (id, shape) in fixture.outside.iter().zip(outside_text) {
        assert!(
            !measured(&doc.context, *id),
            "{id:?} outside the scope measured"
        );
        assert_eq!(doc.world().text_revisions(*id).unwrap().shape, shape);
    }
    let cost = Cost::from(counters);
    assert_eq!(
        (cost.local_fallbacks, cost.full_fallbacks),
        (0, 0),
        "{cost:?}"
    );

    let mut cold = BaselineDoc::new(Some(SCALE));
    assert_eq!(cold.outside, fixture.outside);
    assert_matches_cold(&mut doc.context, &mut cold.doc.context, document);
}

/// Gate D. Setting the scale a scope, a window or the application already
/// has leaves nothing to do.
#[test]
fn issue266_setting_the_same_scale_does_nothing() {
    let mut panel = PanelDoc::new(10_000, None);
    let doc = &mut panel.doc;
    let document = doc.document;
    let target = panel.panel;
    doc.commit(|queue| queue.set_text_scale(target, Some(SCALE)));
    doc.frame();
    let revisions = doc.shape_revisions(document);
    let boxes = doc.boxes(document);

    let skips = |doc: &Doc| {
        doc.context
            .last_work_counters()
            .typography_scale_equivalent_skips
    };
    let expect_nothing = |doc: &mut Doc, label: &str| {
        assert!(
            doc.context.take_system_work().is_empty(),
            "{label}: work queued"
        );
        assert_eq!(doc.shape_revisions(document), revisions, "{label}");
        assert_eq!(doc.boxes(document), boxes, "{label}");
    };

    // The scope's own scale again.
    let before = skips(doc);
    doc.commit(|queue| queue.set_text_scale(target, Some(SCALE)));
    expect_nothing(doc, "scope");
    assert_eq!(skips(doc), before + 1);

    // The window and the application: an override equal to what the window
    // already inherits, and the default it already has.
    doc.context.set_document_text_scale(document, Some(1.0));
    expect_nothing(doc, "window");
    assert_eq!(skips(doc), before + 2);
    doc.context.set_default_text_scale(1.0);
    expect_nothing(doc, "application");
    assert_eq!(skips(doc), before + 3);
    // An invalid scale changes nothing either.
    doc.context.set_default_text_scale(f32::NAN);
    doc.context.set_default_text_scale(0.0);
    expect_nothing(doc, "invalid");

    // The scope again, inside a batch that does other work: the scale part
    // reaches no text.
    let filler_label = doc
        .order(document)
        .into_iter()
        .find(|id| doc.is_text(*id) && !doc.within(*id, target))
        .unwrap();
    doc.commit(|queue| {
        queue.set_text_scale(target, Some(SCALE));
        queue.set_text(
            filler_label,
            TextContent {
                value: "two".into(),
            },
        );
    });
    let counters = doc.frame();
    assert_eq!(counters.typography_scale_equivalent_skips, 1);
    assert_eq!(counters.typography_scale_scope_nodes_scanned, 0);
    assert_eq!(counters.typography_scale_dependents_notified, 0);
    assert_eq!(counters.typography_scale_text_relayouts, 0);
    assert_eq!(
        doc.reshaped(&revisions),
        HashSet::from([filler_label]),
        "only the edited label shaped again"
    );
}

/// Gate E's world: two windows of about 10k nodes each.
struct Windows {
    doc: Doc,
    first: DocumentId,
    second: DocumentId,
}

impl Windows {
    fn new(first_scale: Option<f32>) -> Self {
        let first = DocumentId::new(1).unwrap();
        let second = DocumentId::new(2).unwrap();
        let mut context = AppContext::new();
        if first_scale.is_some() {
            context.set_document_text_scale(first, first_scale);
        }
        for (document, base) in [(first, 1), (second, 1_000_000)] {
            let (mut builder, page) = Builder::page(document, base, 320.0);
            builder.filler(page, 10_000 / FILLER_GROUP_NODES);
            context.commit_mutations(builder.queue).unwrap();
        }
        let mut doc = Doc::new(context, first);
        doc.frame_of(first);
        doc.frame_of(second);
        Self { doc, first, second }
    }
}

/// Gate E. One window's scale reaches its own text and layout and none of
/// the other window's; the application's scale then reaches only the window
/// that inherits it.
#[test]
fn issue266_a_window_scale_stays_in_its_window() {
    let _unguarded = skip_layout_verify();
    let mut windows = Windows::new(None);
    let (first, second) = (windows.first, windows.second);
    let doc = &mut windows.doc;
    let first_nodes = doc.order(first).len();
    let first_text = doc.shape_revisions(first);
    let second_text = doc.shape_revisions(second);
    let second_boxes = doc.boxes(second);
    assert!(first_nodes >= 9_900 && second_text.len() >= 4_500);

    doc.context.set_document_text_scale(first, Some(SCALE));
    let counters = doc.frame_of(first);
    assert_eq!(counters.typography_scale_scope_nodes_scanned, first_nodes);
    assert_eq!(
        counters.typography_scale_dependents_notified,
        first_text.len()
    );
    assert_eq!(counters.typography_scale_text_relayouts, first_text.len());
    // Every label sits in a fixed card: none reflows its parent.
    assert_eq!(counters.typography_scale_parent_reflows, 0);
    assert_eq!(doc.reshaped(&first_text).len(), first_text.len());
    // The other window: no text, no style, no layout.
    assert!(doc.reshaped(&second_text).is_empty());
    assert!(doc.context.take_system_work().is_empty());
    assert!(doc.context.take_layout_frontier_seeds(second).is_empty());
    assert_eq!(doc.boxes(second), second_boxes);

    let mut cold = Windows::new(Some(SCALE));
    assert_matches_cold(&mut doc.context, &mut cold.doc.context, first);

    // The application's scale: the first window keeps its own.
    let first_text = doc.shape_revisions(first);
    let second_nodes = doc.order(second).len();
    doc.context.set_default_text_scale(1.5);
    let counters = doc.frame_of(second);
    assert_eq!(counters.typography_scale_scope_nodes_scanned, second_nodes);
    assert_eq!(
        counters.typography_scale_dependents_notified,
        second_text.len()
    );
    assert!(doc.reshaped(&first_text).is_empty());
    assert!(doc.context.take_layout_frontier_seeds(first).is_empty());
}
