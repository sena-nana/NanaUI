//! Issue #259: the performance contract of incremental reflow.
//!
//! One fixture, the scaling matrix. A page of groups; each group's body holds
//! the formatting contexts the scenarios edit -- a sequential list of rows, a
//! fixed card with a label, a fixed flex row, a fixed grid with auto tracks
//! and a fixed inline formatting context. Groups repeat to 1k, 10k and 100k
//! nodes; the body sits directly in its group (shallow) or under eight
//! wrappers; the page is LTR, RTL or vertical-rl. A case makes one edit in 1,
//! 8 or 100 groups at the head, middle or tail of the page, in one frame.
//!
//! The gates read [`WorkCounters`]:
//!
//! - paint-only: every layout counter is zero;
//! - local-contained: the frontier, the edges it walked, the boxes placed,
//!   the children walked and the subtrees measured are the same at 1k, 10k
//!   and 100k nodes;
//! - sequential: a frame places the boxes that moved and the way down to the
//!   edit, and measures the same subtrees at every size. At the flow end
//!   whose growth moves nothing after it, all of that is a constant;
//! - text: a label at the head of a page, in a column it does not fill,
//!   takes a longer string that still fits: the frame lays out once, as for
//!   a label that cannot wrap, and nothing below the label moves, at 1k and
//!   10k nodes;
//! - batch: a hundred seeds walk each edge of their union closure at most
//!   twice and cost no more than a hundred single edits;
//! - memory: 10k edits on 100k nodes leave the retained cache what the tree
//!   gives it, keep no full-layout copy, and no frame's scratch outgrows one
//!   edit's closure;
//! - virtualization: a row of a 100k and of a 1M row virtual list growing
//!   costs the list its window, the same at both sizes (Issue #262);
//! - responsive: a container resized within its rules' buckets, across one
//!   breakpoint and back, and eight nested containers resized across
//!   theirs, cost the same at 1k, 10k and 100k nodes (Issue #265);
//! - locale: forty localized labels at the end of a window switch from
//!   American English to Chinese, with their language named to British
//!   English (half of them reading the same), and as a scope of their own to
//!   Arabic, right to left, at the same cost in 1k, 10k and 100k node
//!   windows; a localized virtual list switches at the same cost at 100k and
//!   1M rows (Issues #268, #269);
//! - formatting: a counter whose count changes, a catalog update of one
//!   message, and a switch to a locale whose formatters are not built yet,
//!   cost the same at 10k and 100k nodes (Issue #270).
//!
//! Every page ends against the cold oracle ([`super::reflow_oracle`]); 1k
//! pages also run the per-pass guard on every frame.

#![cfg(test)]

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{
    DirSpec, DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, SemanticColorRole,
    WorkCounters, WritingModeSpec,
};

use super::reflow_oracle::{
    assert_matches_cold, bundled_face_shaper, node, product_frame, resize_frame, styled,
};
use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{
    AppContext, LayoutBox, LayoutFrontierStats, LayoutViewport, MutationQueue,
    NanaTextEngineShaper, NodeStyle, TextContent,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 600.0)
}

fn column() -> NodeStyle {
    styled(LayoutStyle {
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    })
}

const ROWS: u64 = 8;
const ITEMS: u64 = 4;
const CELLS: u64 = 6;
const ATOMS: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Depth {
    Shallow,
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Writing {
    Ltr,
    Rtl,
    VerticalRl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Scenario {
    PaintOnly,
    FixedContent,
    Sequential,
    ParentConstraint,
    Flex,
    Grid,
    Inline,
    /// The card names a language for its label (Issue #260): the label
    /// shapes again in it, and lays out again only if its metrics moved.
    Language,
    /// The card sets a typography scale for its label (Issue #266): the label
    /// lays out again at the new size inside the fixed card.
    Typography,
}

impl Scenario {
    const ALL: [Self; 9] = [
        Self::PaintOnly,
        Self::FixedContent,
        Self::Sequential,
        Self::ParentConstraint,
        Self::Flex,
        Self::Grid,
        Self::Inline,
        Self::Language,
        Self::Typography,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Position {
    Head,
    Middle,
    Tail,
}

impl Position {
    const ALL: [Self; 3] = [Self::Head, Self::Middle, Self::Tail];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    depth: Depth,
    writing: Writing,
}

impl Shape {
    /// The shallow left-to-right page most single-shape gates run on.
    const SHALLOW_LTR: Self = Self {
        depth: Depth::Shallow,
        writing: Writing::Ltr,
    };

    fn wrappers(self) -> u64 {
        match self.depth {
            Depth::Shallow => 0,
            Depth::Nested => 8,
        }
    }

    fn group_nodes(self) -> u64 {
        self.wrappers() + 9 + ROWS + ITEMS + CELLS + ATOMS
    }

    fn group(self, group: u64) -> StableNodeId {
        node(3 + group * self.group_nodes())
    }

    fn wrapper(self, group: u64, depth: u64) -> StableNodeId {
        node(self.group(group).get() + 1 + depth)
    }

    fn body(self, group: u64) -> StableNodeId {
        node(self.group(group).get() + 1 + self.wrappers())
    }

    fn list(self, group: u64) -> StableNodeId {
        node(self.body(group).get() + 1)
    }

    fn row(self, group: u64, row: u64) -> StableNodeId {
        node(self.list(group).get() + 1 + row)
    }

    fn card(self, group: u64) -> StableNodeId {
        node(self.list(group).get() + 1 + ROWS)
    }

    fn label(self, group: u64) -> StableNodeId {
        node(self.card(group).get() + 1)
    }

    fn flex(self, group: u64) -> StableNodeId {
        node(self.label(group).get() + 1)
    }

    fn item(self, group: u64, item: u64) -> StableNodeId {
        node(self.flex(group).get() + 1 + item)
    }

    fn grid(self, group: u64) -> StableNodeId {
        node(self.flex(group).get() + 1 + ITEMS)
    }

    fn cell(self, group: u64, cell: u64) -> StableNodeId {
        node(self.grid(group).get() + 1 + cell)
    }

    fn para(self, group: u64) -> StableNodeId {
        node(self.grid(group).get() + 1 + CELLS)
    }

    fn atom(self, group: u64, atom: u64) -> StableNodeId {
        node(self.para(group).get() + 1 + atom)
    }

    fn text(self, group: u64) -> StableNodeId {
        node(self.para(group).get() + 1 + ATOMS)
    }

    /// The node `scenario` edits in `group`.
    fn target(self, scenario: Scenario, group: u64) -> StableNodeId {
        match scenario {
            Scenario::PaintOnly
            | Scenario::ParentConstraint
            | Scenario::Language
            | Scenario::Typography => self.card(group),
            Scenario::FixedContent => self.label(group),
            Scenario::Sequential => self.row(group, 0),
            Scenario::Flex => self.item(group, 0),
            Scenario::Grid => self.cell(group, 0),
            Scenario::Inline => self.atom(group, 0),
        }
    }

    /// A box `inline` long on the page's inline axis and `block` long on its
    /// block axis.
    fn sized(self, inline: Option<f32>, block: Option<f32>) -> LayoutStyle {
        let (width, height) = if self.writing == Writing::VerticalRl {
            (block, inline)
        } else {
            (inline, block)
        };
        LayoutStyle {
            width: width.map(LengthSpec::Px),
            height: height.map(LengthSpec::Px),
            ..LayoutStyle::default()
        }
    }

    fn page_style(self) -> NodeStyle {
        let mut layout = self.sized(Some(320.0), None);
        layout.direction = Some(FlexDirection::Column);
        match self.writing {
            Writing::Ltr => {}
            Writing::Rtl => layout.dir = Some(DirSpec::Rtl),
            Writing::VerticalRl => layout.writing_mode = Some(WritingModeSpec::VerticalRl),
        }
        styled(layout)
    }

    fn row_style(self, grown: bool) -> NodeStyle {
        styled(self.sized(Some(300.0), Some(if grown { 30.0 } else { 20.0 })))
    }

    fn card_style(self, narrowed: bool, painted: bool) -> NodeStyle {
        let mut layout = self.sized(Some(if narrowed { 180.0 } else { 200.0 }), Some(40.0));
        layout.direction = Some(FlexDirection::Row);
        NodeStyle {
            layout: Arc::new(layout),
            background: painted.then_some(SemanticColorRole::Selected),
            ..NodeStyle::default()
        }
    }

    fn flex_style(self) -> NodeStyle {
        let mut layout = self.sized(Some(300.0), Some(20.0));
        layout.direction = Some(FlexDirection::Row);
        styled(layout)
    }

    /// Growth is shared by `flex-grow`; doubling one item's share moves
    /// every item on the line.
    fn item_style(self, grown: bool) -> NodeStyle {
        let mut layout = self.sized(None, Some(20.0));
        layout.flex_grow = Some(if grown { 2.0 } else { 1.0 });
        styled(layout)
    }

    fn grid_style(self) -> NodeStyle {
        let mut layout = self.sized(Some(300.0), Some(60.0));
        layout.display = Some(DisplaySpec::Grid);
        layout.grid_columns = Some(vec![GridTrack::Auto; 3]);
        styled(layout)
    }

    fn cell_style(self, grown: bool) -> NodeStyle {
        styled(self.sized(Some(if grown { 50.0 } else { 30.0 }), Some(20.0)))
    }

    fn para_style(self) -> NodeStyle {
        let mut layout = self.sized(Some(300.0), Some(60.0));
        layout.display = Some(DisplaySpec::Block);
        styled(layout)
    }

    fn atom_style(self, grown: bool) -> NodeStyle {
        let mut layout = self.sized(Some(if grown { 100.0 } else { 80.0 }), Some(20.0));
        layout.display = Some(DisplaySpec::InlineBlock);
        styled(layout)
    }
}

fn inline_text_style() -> NodeStyle {
    styled(LayoutStyle {
        display: Some(DisplaySpec::Inline),
        ..LayoutStyle::default()
    })
}

fn card_language(edited: bool) -> Option<crate::LanguageTag> {
    edited.then(|| crate::LanguageTag::new("ja").unwrap())
}

fn card_text_scale(edited: bool) -> Option<f32> {
    edited.then_some(1.25)
}

fn label_text(edited: bool) -> &'static str {
    if edited { "seventeen" } else { "one" }
}

/// The (scenario, group) edits in effect. Every node is built from this set,
/// so a page built cold from the same set has the same inputs.
#[derive(Debug, Default, Clone)]
struct Edits(HashSet<(Scenario, u64)>);

impl Edits {
    fn on(&self, scenario: Scenario, group: u64) -> bool {
        self.0.contains(&(scenario, group))
    }

    fn toggle(&mut self, scenario: Scenario, group: u64) {
        if !self.0.remove(&(scenario, group)) {
            self.0.insert((scenario, group));
        }
    }
}

/// A page's mutations, each node at the id its [`Shape`] gives it.
struct Tree {
    queue: MutationQueue,
    document: DocumentId,
}

impl Tree {
    fn new(document: DocumentId) -> Self {
        Self {
            queue: MutationQueue::new(),
            document,
        }
    }

    fn element(&mut self, parent: StableNodeId, id: StableNodeId, tag: &str, style: NodeStyle) {
        self.queue
            .create(id, self.document, NodeKind::Element { tag: tag.into() });
        self.queue.insert(parent, id, None);
        self.queue.set_style(id, style);
    }

    fn text(&mut self, parent: StableNodeId, id: StableNodeId, value: &str) {
        self.queue.create(id, self.document, NodeKind::Text);
        self.queue.insert(parent, id, None);
        self.queue.set_text(
            id,
            TextContent {
                value: value.into(),
            },
        );
    }
}

fn build_group(tree: &mut Tree, shape: Shape, edits: &Edits, group: u64) {
    let mut parent = shape.group(group);
    tree.element(node(2), parent, "group", column());
    for depth in 0..shape.wrappers() {
        let wrapper = shape.wrapper(group, depth);
        tree.element(parent, wrapper, "wrapper", column());
        parent = wrapper;
    }
    let body = shape.body(group);
    tree.element(parent, body, "body", column());
    tree.element(body, shape.list(group), "list", column());
    for row in 0..ROWS {
        let style = shape.row_style(row == 0 && edits.on(Scenario::Sequential, group));
        tree.element(shape.list(group), shape.row(group, row), "row", style);
    }
    let card = shape.card(group);
    let style = shape.card_style(
        edits.on(Scenario::ParentConstraint, group),
        edits.on(Scenario::PaintOnly, group),
    );
    tree.element(body, card, "card", style);
    if edits.on(Scenario::Language, group) {
        tree.queue.set_language(card, card_language(true));
    }
    if edits.on(Scenario::Typography, group) {
        tree.queue.set_text_scale(card, card_text_scale(true));
    }
    let label = label_text(edits.on(Scenario::FixedContent, group));
    tree.text(card, shape.label(group), label);
    tree.element(body, shape.flex(group), "flex", shape.flex_style());
    for item in 0..ITEMS {
        let style = shape.item_style(item == 0 && edits.on(Scenario::Flex, group));
        tree.element(shape.flex(group), shape.item(group, item), "item", style);
    }
    tree.element(body, shape.grid(group), "grid", shape.grid_style());
    for cell in 0..CELLS {
        let style = shape.cell_style(cell == 0 && edits.on(Scenario::Grid, group));
        tree.element(shape.grid(group), shape.cell(group, cell), "cell", style);
    }
    tree.element(body, shape.para(group), "para", shape.para_style());
    for atom in 0..ATOMS {
        let style = shape.atom_style(atom == 0 && edits.on(Scenario::Inline, group));
        tree.element(shape.para(group), shape.atom(group, atom), "atom", style);
    }
    tree.text(shape.para(group), shape.text(group), "one two");
    tree.queue.set_style(shape.text(group), inline_text_style());
}

/// Whether a page checks every layout pass against a full layout. Pages of
/// 10k nodes and more check against the cold oracle at the end instead: a
/// full layout per frame would cost more than the frames under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guard {
    EveryPass,
    ColdOnly,
}

struct Page {
    shape: Shape,
    groups: u64,
    edits: Edits,
    guard: Guard,
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
}

impl Page {
    fn new(shape: Shape, nodes: u64, guard: Guard) -> Self {
        Self::with_edits(shape, nodes, Edits::default(), guard)
    }

    /// About `nodes` nodes with `edits` in effect, laid out in one frame.
    fn with_edits(shape: Shape, nodes: u64, edits: Edits, guard: Guard) -> Self {
        let groups = (nodes.saturating_sub(2) / shape.group_nodes()).max(1);
        let document = DocumentId::new(1).unwrap();
        let mut tree = Tree::new(document);
        tree.queue.create(node(1), document, NodeKind::Document);
        tree.element(node(1), node(2), "page", shape.page_style());
        for group in 0..groups {
            build_group(&mut tree, shape, &edits, group);
        }
        let mut context = AppContext::new();
        context.compat_world_mut().commit(tree.queue).unwrap();
        let mut page = Self {
            shape,
            groups,
            edits,
            guard,
            context,
            document,
            shaper: bundled_face_shaper(),
        };
        page.frame();
        page
    }

    /// The same inputs, laid out once from nothing.
    fn cold(&self) -> Self {
        let nodes = self.groups * self.shape.group_nodes() + 2;
        Self::with_edits(self.shape, nodes, self.edits.clone(), Guard::ColdOnly)
    }

    fn assert_matches_cold(&mut self) {
        let mut cold = self.cold();
        assert_matches_cold(&mut self.context, &mut cold.context, self.document);
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    /// `seeds` consecutive groups at `position`.
    fn groups_at(&self, position: Position, seeds: u64) -> Vec<u64> {
        let seeds = seeds.min(self.groups);
        let start = match position {
            Position::Head => 0,
            Position::Middle => (self.groups - seeds) / 2,
            Position::Tail => self.groups - seeds,
        };
        (start..start + seeds).collect()
    }

    /// Toggle `scenario`'s edit in every group of `groups`, as one commit.
    fn edit(&mut self, scenario: Scenario, groups: &[u64]) {
        let shape = self.shape;
        let mut queue = MutationQueue::new();
        for &group in groups {
            self.edits.toggle(scenario, group);
            let on = self.edits.on(scenario, group);
            let target = shape.target(scenario, group);
            match scenario {
                Scenario::PaintOnly | Scenario::ParentConstraint => queue.set_style(
                    target,
                    shape.card_style(
                        self.edits.on(Scenario::ParentConstraint, group),
                        self.edits.on(Scenario::PaintOnly, group),
                    ),
                ),
                Scenario::FixedContent => queue.set_text(
                    target,
                    TextContent {
                        value: label_text(on).into(),
                    },
                ),
                Scenario::Sequential => queue.set_style(target, shape.row_style(on)),
                Scenario::Flex => queue.set_style(target, shape.item_style(on)),
                Scenario::Grid => queue.set_style(target, shape.cell_style(on)),
                Scenario::Inline => queue.set_style(target, shape.atom_style(on)),
                Scenario::Language => queue.set_language(target, card_language(on)),
                Scenario::Typography => queue.set_text_scale(target, card_text_scale(on)),
            }
        }
        self.context.compat_world_mut().commit(queue).unwrap();
    }

    fn frame(&mut self) -> WorkCounters {
        let _unguarded = (self.guard == Guard::ColdOnly).then(skip_layout_verify);
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    /// One frame of a window resize to `viewport`.
    fn resize(&mut self, viewport: LayoutViewport) -> WorkCounters {
        let _unguarded = (self.guard == Guard::ColdOnly).then(skip_layout_verify);
        resize_frame(&mut self.context, self.document, viewport, &mut self.shaper)
    }

    fn boxes(&self) -> Vec<Option<LayoutBox>> {
        let world = self.world();
        world
            .document_order(self.document)
            .into_iter()
            .map(|id| world.layout_box(id))
            .collect()
    }
}

/// What one frame cost, in the units the gates name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Cost {
    seeds: usize,
    frontier_measure: usize,
    frontier_placement: usize,
    contexts: usize,
    edges: usize,
    /// Nodes that computed a used size: subtree walks, and fixed-size boxes
    /// resolving their own style.
    measured: usize,
    placed: usize,
    origin_only: usize,
    children_walked: usize,
    /// Children result publication checked or rebuilt.
    result_children: usize,
    full_subtrees: usize,
    plan_misses: usize,
    local_fallbacks: usize,
    full_fallbacks: usize,
}

impl From<WorkCounters> for Cost {
    fn from(counters: WorkCounters) -> Self {
        Self {
            seeds: counters.layout_frontier_seeds,
            frontier_measure: counters.layout_frontier_nodes_measure,
            frontier_placement: counters.layout_frontier_nodes_placement,
            contexts: counters.layout_frontier_contexts,
            edges: counters.layout_dependency_edges_visited,
            measured: counters.layout_measure_nodes,
            placed: counters.layout_placement_nodes,
            origin_only: counters.layout_origin_only_updates,
            children_walked: counters.layout_children_measured,
            result_children: counters.layout_result_children_visited,
            full_subtrees: counters.intrinsic_measure_full_subtrees,
            plan_misses: counters.layout_plan_misses,
            local_fallbacks: counters.layout_local_subtree_fallbacks,
            full_fallbacks: counters.layout_full_document_fallbacks,
        }
    }
}

impl Cost {
    /// Everything but `measured`. A fixed-size box resolving its own style
    /// counts there, and which retained slot answers it first can follow
    /// hash order; it never walks children, so that count is bounded, not
    /// exact.
    fn structural(self) -> Self {
        Self {
            measured: 0,
            ..self
        }
    }

    /// Everything but `result_children`. When what ends the page changes
    /// its extent, the page republishes its own result, which holds a
    /// placement for every group: that one copy follows the page's size,
    /// everything else is constant.
    fn without_page_result(self) -> Self {
        Self {
            result_children: 0,
            ..self
        }
    }

    /// The frontier a pass built and walked.
    fn frontier(self) -> (usize, usize, usize, usize, usize) {
        (
            self.seeds,
            self.frontier_measure,
            self.frontier_placement,
            self.contexts,
            self.edges,
        )
    }
}

/// One case run on a page: what the frame cost and how many boxes it moved.
struct Case {
    cost: Cost,
    stats: LayoutFrontierStats,
    moved: usize,
}

fn run_case(page: &mut Page, scenario: Scenario, position: Position, seeds: u64) -> Case {
    let groups = page.groups_at(position, seeds);
    let before = page.boxes();
    page.edit(scenario, &groups);
    let counters = page.frame();
    let moved = before
        .iter()
        .zip(page.boxes())
        .filter(|(old, new)| **old != *new)
        .count();
    Case {
        cost: Cost::from(counters),
        stats: page.context.layout_frontier_stats(page.document),
        moved,
    }
}

/// The flow end whose growth moves nothing after it: the tail of a forward
/// block axis, the head of vertical-rl's, which runs from the right edge
/// while the page grows to the right.
fn quiet_end(writing: Writing) -> Position {
    match writing {
        Writing::Ltr | Writing::Rtl => Position::Tail,
        Writing::VerticalRl => Position::Head,
    }
}

/// The gates one case meets, on a 1k page (`small`) and a 10k page (`large`).
fn check_case(
    shape: Shape,
    scenario: Scenario,
    position: Position,
    edits: usize,
    small: &Case,
    large: &Case,
    label: &str,
) {
    // The document node, the page, the group, its wrappers and the body:
    // placement walks down them to reach an edit it does not move, once for
    // every edited group.
    let path = (shape.wrappers() as usize + 4) * edits;
    for case in [small, large] {
        let cost = case.cost;
        assert_eq!(
            (cost.local_fallbacks, cost.full_fallbacks),
            (0, 0),
            "{label}: {cost:?}"
        );
        if scenario == Scenario::PaintOnly {
            assert_eq!(cost, Cost::default(), "{label}: paint reached layout");
            assert_eq!(case.moved, 0, "{label}");
            continue;
        }
        // Placement pays for the boxes that moved and for the way down to
        // the edit, and for nothing else.
        assert!(
            cost.placed >= case.moved,
            "{label}: {} boxes moved, {} placed",
            case.moved,
            cost.placed
        );
        assert!(
            cost.placed <= case.moved + cost.frontier_placement + path,
            "{label}: {} boxes moved, {} placed: {cost:?}",
            case.moved,
            cost.placed
        );
    }
    if scenario == Scenario::PaintOnly {
        return;
    }
    let (small_cost, large_cost) = (small.cost, large.cost);
    if scenario == Scenario::Sequential {
        // Moving later siblings is real output and grows with them. What
        // was built, walked and measured to find them does not.
        assert_eq!(small_cost.frontier(), large_cost.frontier(), "{label}");
        assert_eq!(
            small_cost.full_subtrees, large_cost.full_subtrees,
            "{label}"
        );
        if position == quiet_end(shape.writing) {
            // The page's own placement of the edited group did change.
            assert_eq!(small.moved, large.moved, "{label}");
            assert_eq!(
                small_cost.structural().without_page_result(),
                large_cost.structural().without_page_result(),
                "{label}"
            );
        }
    } else {
        assert_eq!(small.moved, large.moved, "{label}");
        assert_eq!(small_cost.structural(), large_cost.structural(), "{label}");
    }
    assert!(
        large_cost.measured <= 2 * small_cost.measured + 8,
        "{label}: {} nodes measured at 1k, {} at 10k",
        small_cost.measured,
        large_cost.measured
    );
}

fn matrix(writing: Writing) {
    for depth in [Depth::Shallow, Depth::Nested] {
        let shape = Shape { depth, writing };
        let mut small = Page::new(shape, 1_000, Guard::EveryPass);
        let mut large = Page::new(shape, 10_000, Guard::ColdOnly);
        assert!(large.world().len() >= 9_900);
        for scenario in Scenario::ALL {
            for position in Position::ALL {
                for seeds in [1, 8] {
                    let label = format!("{writing:?} {depth:?} {scenario:?} {position:?} x{seeds}");
                    let small_case = run_case(&mut small, scenario, position, seeds);
                    let large_case = run_case(&mut large, scenario, position, seeds);
                    check_case(
                        shape,
                        scenario,
                        position,
                        seeds as usize,
                        &small_case,
                        &large_case,
                        &label,
                    );
                }
            }
        }
        small.assert_matches_cold();
        large.assert_matches_cold();
    }
}

/// Every scenario at the head, middle and tail of 1k and 10k pages, shallow
/// and eight levels deep, with one seed and with eight.
#[test]
fn issue259_matrix_ltr() {
    matrix(Writing::Ltr);
}

#[test]
fn issue259_matrix_rtl() {
    matrix(Writing::Rtl);
}

#[test]
fn issue259_matrix_vertical_rl() {
    matrix(Writing::VerticalRl);
}

const CONTAINED: [Scenario; 7] = [
    Scenario::FixedContent,
    Scenario::ParentConstraint,
    Scenario::Flex,
    Scenario::Grid,
    Scenario::Inline,
    Scenario::Language,
    Scenario::Typography,
];

/// A hundred edits in a hundred groups build one union closure: every edge
/// of it is walked at most twice, and nothing costs more than a hundred
/// single edits.
#[test]
fn issue259_a_hundred_seeds_cost_their_union_closure() {
    let shape = Shape {
        depth: Depth::Nested,
        writing: Writing::Ltr,
    };
    let mut page = Page::new(shape, 10_000, Guard::ColdOnly);
    assert!(page.groups >= 100);
    for scenario in CONTAINED {
        let single = run_case(&mut page, scenario, Position::Head, 1);
        let batch = run_case(&mut page, scenario, Position::Middle, 100);
        let label = format!("{scenario:?}");
        assert_eq!(batch.cost.seeds, 100 * single.cost.seeds, "{label}");
        assert!(
            batch.cost.edges <= 2 * batch.stats.graph_edges,
            "{label}: {} edge walks over a {}-edge closure",
            batch.cost.edges,
            batch.stats.graph_edges
        );
        for (name, one, hundred) in [
            (
                "frontier",
                single.cost.frontier_placement,
                batch.cost.frontier_placement,
            ),
            ("edges", single.cost.edges, batch.cost.edges),
            ("placed", single.cost.placed, batch.cost.placed),
            (
                "full subtrees",
                single.cost.full_subtrees,
                batch.cost.full_subtrees,
            ),
            (
                "scratch",
                single.stats.scratch_entries,
                batch.stats.scratch_entries,
            ),
        ] {
            assert!(
                hundred <= 100 * one,
                "{label}: {name} {hundred} for a hundred edits, {one} for one"
            );
        }
        assert_eq!(batch.cost.full_fallbacks, 0, "{label}");
    }
    page.assert_matches_cold();
}

/// Local edits, and a sequential edit at the quiet end, cost the same on a
/// 100k page as on a 10k one, one seed or a hundred.
#[test]
fn issue259_contained_edits_cost_the_same_at_100k() {
    let shape = Shape::SHALLOW_LTR;
    let mut large = Page::new(shape, 10_000, Guard::ColdOnly);
    let mut huge = Page::new(shape, 100_000, Guard::ColdOnly);
    assert!(huge.world().len() >= 99_000);
    for scenario in CONTAINED.into_iter().chain([Scenario::Sequential]) {
        let position = if scenario == Scenario::Sequential {
            quiet_end(shape.writing)
        } else {
            Position::Middle
        };
        for seeds in [1, 100] {
            let label = format!("{scenario:?} x{seeds}");
            let large_case = run_case(&mut large, scenario, position, seeds);
            let huge_case = run_case(&mut huge, scenario, position, seeds);
            assert_eq!(large_case.moved, huge_case.moved, "{label}");
            let compared = |cost: Cost| {
                if scenario == Scenario::Sequential {
                    cost.structural().without_page_result()
                } else {
                    cost.structural()
                }
            };
            assert_eq!(
                compared(large_case.cost),
                compared(huge_case.cost),
                "{label}"
            );
            assert!(
                huge_case.cost.measured <= 2 * large_case.cost.measured + 8,
                "{label}: {} nodes measured at 10k, {} at 100k",
                large_case.cost.measured,
                huge_case.cost.measured
            );
            assert_eq!(
                large_case.stats.scratch_entries, huge_case.stats.scratch_entries,
                "{label}: scratch follows the closure, not the document"
            );
        }
    }
    large.assert_matches_cold();
}

/// Ten thousand edits on a 100k page. The retained cache stays what the
/// tree gives it: the same boxes and plans, no full-layout copy, intrinsic
/// facts within their per-node budget. No frame's scratch outgrows one
/// edit's closure. The page then matches a cold layout.
#[test]
fn issue259_ten_thousand_edits_on_100k_nodes_keep_the_retained_cache_flat() {
    use crate::layout_engine::RetainedLayoutFootprint;
    let mut page = Page::new(Shape::SHALLOW_LTR, 100_000, Guard::ColdOnly);
    let nodes = page.world().len();
    assert!(nodes >= 99_000);
    let kinds: Vec<Scenario> = CONTAINED
        .into_iter()
        .chain([Scenario::PaintOnly, Scenario::Sequential])
        .collect();
    let tail = page.groups - 1;
    // Each edit kind on and off once, so every plan the soak touches exists.
    for &scenario in &kinds {
        for _ in 0..2 {
            page.edit(scenario, &[tail]);
            page.frame();
        }
    }
    let settled = page.context.retained_layout_footprint(page.document);
    assert_eq!(
        settled.full_snapshots, 0,
        "the product frame keeps no full layout"
    );
    // What the tree gives the cache. Intrinsic facts and the last frontier
    // are held to their budgets instead.
    let flat = |footprint: RetainedLayoutFootprint| RetainedLayoutFootprint {
        intrinsics: 0,
        intrinsic_metrics: 0,
        last_frontier: 0,
        ..footprint
    };

    let mut group = 1u64;
    let mut peak_scratch = 0usize;
    for edit in 0..10_000usize {
        let scenario = kinds[edit % kinds.len()];
        // A sequential edit at the quiet end moves a constant; elsewhere it
        // would be the legitimate O(N) the matrix covers, not a soak frame.
        let target = if scenario == Scenario::Sequential {
            tail
        } else {
            group = (group * 7_919 + 13) % page.groups;
            group
        };
        page.edit(scenario, &[target]);
        let counters = page.frame();
        assert_eq!(counters.layout_full_document_fallbacks, 0, "edit {edit}");
        peak_scratch = peak_scratch.max(counters.layout_scratch_entries);
        if edit % 1_000 == 999 {
            let footprint = page.context.retained_layout_footprint(page.document);
            assert_eq!(flat(footprint), flat(settled), "after {} edits", edit + 1);
            assert!(footprint.intrinsics <= 2 * nodes, "{footprint:?}");
            assert!(footprint.intrinsic_metrics <= 4 * nodes, "{footprint:?}");
            assert!(footprint.last_frontier <= 64, "{footprint:?}");
        }
    }
    assert!(
        peak_scratch <= 64,
        "a frame of one edit held {peak_scratch} scratch entries"
    );
    page.assert_matches_cold();
}

/// The timing half of the contract, for the benchmark machine: p50, p95 and
/// p99 of a local edit's frame at 1k, 10k and 100k nodes, and the p95 growth
/// from 1k to 100k against the 1.75x target. PR CI gates on the counters
/// above; timings are recorded where the machine is fixed:
///
/// `cargo test --release -p nana-ui-runtime --lib issue259_local_frame_time -- --ignored --nocapture`
#[test]
#[ignore = "fixed-machine timing: run with --release -- --ignored"]
fn issue259_local_frame_time_scales_flat_to_100k() {
    use std::time::{Duration, Instant};
    let shape = Shape::SHALLOW_LTR;
    let mut p95 = Vec::new();
    for nodes in [1_000u64, 10_000, 100_000] {
        let mut page = Page::new(shape, nodes, Guard::ColdOnly);
        let middle = page.groups / 2;
        for scenario in CONTAINED {
            for _ in 0..4 {
                page.edit(scenario, &[middle]);
                page.frame();
            }
        }
        let mut samples: Vec<Duration> = Vec::with_capacity(500);
        for edit in 0..500u64 {
            let scenario = CONTAINED[edit as usize % CONTAINED.len()];
            page.edit(scenario, &[(edit * 7_919 + 13) % page.groups]);
            let started = Instant::now();
            page.frame();
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let at = |quantile: f64| samples[((samples.len() - 1) as f64 * quantile).round() as usize];
        eprintln!(
            "{nodes} nodes: p50 {:?}, p95 {:?}, p99 {:?}",
            at(0.50),
            at(0.95),
            at(0.99)
        );
        p95.push(at(0.95));
    }
    let growth = p95[2].as_secs_f64() / p95[0].as_secs_f64();
    eprintln!("p95 at 100k / p95 at 1k: {growth:.2}");
    assert!(
        growth <= 1.75,
        "a local edit's p95 frame grew {growth:.2}x from 1k to 100k nodes"
    );
}

/// The widths of a window resize storm: out by 120 pixels and back to the
/// page's own viewport, one a frame.
fn storm_widths() -> impl Iterator<Item = f32> {
    (1..=240).map(|step: u32| {
        let out = if step <= 120 { step } else { 240 - step };
        viewport().width + out as f32
    })
}

/// A window resize storm (Issue #264): 240 viewport widths. The page is a
/// fixed column, so each resize reaches the document root and no group: the
/// cost is the same at 1k, 10k and 100k nodes, and nothing falls back.
#[test]
fn issue259_a_viewport_resize_storm_stays_at_the_root() {
    let shape = Shape::SHALLOW_LTR;
    let mut pages = [
        Page::new(shape, 1_000, Guard::EveryPass),
        Page::new(shape, 10_000, Guard::ColdOnly),
        Page::new(shape, 100_000, Guard::ColdOnly),
    ];
    assert!(pages[2].world().len() >= 99_000);
    for width in storm_widths() {
        let viewport = LayoutViewport::new(width, viewport().height);
        let costs: Vec<Cost> = pages
            .iter_mut()
            .map(|page| Cost::from(page.resize(viewport)))
            .collect();
        assert_eq!(
            (costs[0].local_fallbacks, costs[0].full_fallbacks),
            (0, 0),
            "{width}"
        );
        assert_eq!(costs[0], costs[1], "{width}");
        assert_eq!(costs[1], costs[2], "{width}");
    }
    for page in &mut pages {
        page.assert_matches_cold();
    }
}

/// The same storm on a vertical-rl page, whose block axis runs from the
/// right edge: each pass matches a full layout.
#[test]
fn issue259_a_vertical_rl_resize_storm_matches_a_full_layout() {
    let shape = Shape {
        depth: Depth::Nested,
        writing: Writing::VerticalRl,
    };
    let mut page = Page::new(shape, 1_000, Guard::EveryPass);
    for width in storm_widths().step_by(9).chain([viewport().width]) {
        let counters = page.resize(LayoutViewport::new(width, viewport().height));
        assert_eq!(counters.layout_full_document_fallbacks, 0, "{width}");
    }
    page.assert_matches_cold();
}

/// Theme workloads (Issue #261): a Light/Dark switch is a palette, and a
/// metrics install reaches only boxes that declare design intent, which
/// these pages have none of. Neither lays anything out, at any size.
#[test]
fn issue259_theme_switches_lay_nothing_out_at_any_size() {
    for nodes in [1_000, 10_000, 100_000] {
        let guard = if nodes == 1_000 {
            Guard::EveryPass
        } else {
            Guard::ColdOnly
        };
        let mut page = Page::new(Shape::SHALLOW_LTR, nodes, guard);
        for mode in [
            nana_ui_core::ThemeAppearance::Light,
            nana_ui_core::ThemeAppearance::Dark,
        ] {
            page.context.set_preset_theme(mode).unwrap();
            let counters = page.frame();
            assert_eq!(Cost::from(counters), Cost::default(), "{nodes} {mode:?}");
            assert_eq!(counters.theme_palette_layout_invalidations, 0);
        }
        super::issue261::widen_control_padding(&mut page.context, 4.0);
        let counters = page.frame();
        assert_eq!(Cost::from(counters), Cost::default(), "{nodes} metrics");
        assert_eq!(counters.theme_to_layout_seeds, 0);
        assert_eq!(counters.theme_metric_dependents_invalidated, 0);
    }
}

/// Replaced content workloads (Issue #263), appended to the end of 1k, 10k
/// and 100k pages: a video's frames lay nothing out, an image's natural size
/// lays out its box, and a shared image reaches the twenty nodes showing it.
/// Each costs the same at every size.
#[test]
fn issue259_replaced_content_costs_the_same_at_any_size() {
    use super::issue263::{image_layout, url};
    use crate::{CustomRenderNode, HOST_TEXTURE_RENDERER, ReplacedMetadata};
    let image = |name: &str, side: Option<f32>| styled(image_layout(name, side, side));
    let mut costs: Vec<Vec<Cost>> = Vec::new();
    for nodes in [1_000, 10_000, 100_000] {
        let guard = if nodes == 1_000 {
            Guard::EveryPass
        } else {
            Guard::ColdOnly
        };
        let mut page = Page::new(Shape::SHALLOW_LTR, nodes, guard);
        let base = 10_000_000u64;
        let media = node(base);
        let video = node(base + 1);
        let mut tree = Tree::new(page.document);
        tree.element(node(2), media, "media", column());
        tree.element(
            media,
            video,
            "video",
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(160.0)),
                height: Some(LengthSpec::Px(90.0)),
                ..LayoutStyle::default()
            }),
        );
        tree.queue.set_custom_render(
            video,
            Some(CustomRenderNode::new(HOST_TEXTURE_RENDERER, "video:1", 0)),
        );
        tree.element(media, node(base + 2), "img", image("hero", None));
        for index in 0..20u64 {
            let side = (index % 2 == 0).then_some(40.0);
            tree.element(media, node(base + 10 + index), "img", image("shared", side));
        }
        page.context.commit_mutations(tree.queue).unwrap();
        page.frame();
        let mut row = Vec::new();
        for frame in 1..=30u64 {
            let mut queue = MutationQueue::new();
            queue.set_custom_render(
                video,
                Some(CustomRenderNode::new(
                    HOST_TEXTURE_RENDERER,
                    "video:1",
                    frame,
                )),
            );
            page.context.commit_mutations(queue).unwrap();
            let counters = page.frame();
            assert_eq!(
                Cost::from(counters),
                Cost::default(),
                "{nodes}: frame {frame}"
            );
        }
        for (resource, size) in [(url("hero"), (320.0, 180.0)), (url("shared"), (24.0, 16.0))] {
            let mut queue = MutationQueue::new();
            queue.set_replaced_metadata(resource, Some(ReplacedMetadata::new(size.0, size.1)));
            page.context.commit_mutations(queue).unwrap();
            let counters = page.frame();
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            row.push(Cost::from(counters).without_page_result());
        }
        costs.push(row);
    }
    assert_eq!(costs[0], costs[1]);
    assert_eq!(costs[1], costs[2]);
}

/// Issue #262's feed of 100k and of 1M rows: a visible row growing by 8 px
/// costs the same at both sizes -- the list's window, never its rows. Only
/// the row index's depth, O(log C) entries over C chunks, follows the
/// collection.
#[test]
fn issue259_a_virtual_row_growing_costs_the_same_at_100k_and_1m() {
    use super::issue262::{Feed, Pass, grow};
    let mut costs = Vec::new();
    for rows in [100_000, 1_000_000] {
        let mut feed = Feed::new(rows);
        feed.scroll_to_row(rows / 2);
        let growth = grow(&mut feed, 10);
        assert!(
            growth.pass.index_updates <= 512 + 32,
            "{rows}: {:?}",
            growth.pass
        );
        costs.push((
            Pass {
                index_updates: 0,
                ..growth.pass
            },
            growth.lookups,
            Cost::from(growth.laid),
            Cost::from(growth.placed),
        ));
    }
    assert_eq!(costs[0], costs[1]);
}

/// Issue #265's panel of a hundred responsive rows and its eight nested
/// containers, at the end of 1k, 10k and 100k node pages: a resize within
/// the rules' buckets, one across a breakpoint and one back, and a nested
/// resize across every level's, cost the same at every size.
#[test]
fn issue259_responsive_rules_cost_the_same_at_any_size() {
    use super::issue265::{Panel, Queries};
    let mut costs: Vec<Vec<(Queries, Cost)>> = Vec::new();
    for nodes in [1_000, 10_000, 100_000] {
        let guarded = nodes == 1_000;
        let mut row = Vec::new();
        let mut panel = Panel::new(nodes, 800.0, guarded);
        for width in [790.0, 480.0, 790.0] {
            let counters = panel.resize(width);
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            row.push((
                Queries::from(counters),
                Cost::from(counters).without_page_result(),
            ));
        }
        let mut nest = Panel::nest(800.0, nodes, guarded);
        for width in [300.0, 800.0] {
            let counters = nest.resize(width);
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            row.push((
                Queries::from(counters),
                Cost::from(counters).without_page_result(),
            ));
        }
        costs.push(row);
    }
    assert_eq!(costs[0], costs[1]);
    assert_eq!(costs[1], costs[2]);
}

/// Issues #268 and #269: forty localized labels in a bar of a fixed height
/// at the end of 1k, 10k and 100k node windows switch locale at the same
/// cost: left to right to left to right (en-US to zh-CN), and an equal
/// translation (en-US to en-GB with the language named) whose unchanged
/// strings do no text work. The rest of the window is never visited.
#[test]
fn issue259_a_locale_switch_costs_the_same_at_any_size() {
    use super::issue268::{App, cost, tag};
    use crate::Locale;
    let mut costs = Vec::new();
    for nodes in [1_000, 10_000, 100_000] {
        let mut app = App::labelled(nodes, 40, nodes == 1_000);
        assert_eq!(app.windows[0].localized.len(), 40, "{nodes}");
        let mut row = Vec::new();
        let mut measure = |app: &mut App, change: &dyn Fn(&mut App)| {
            let switch = cost(app, change);
            let counters = app.frame(0);
            assert_eq!(counters.layout_full_document_fallbacks, 0, "{nodes}");
            row.push((
                switch.direction_changed_scopes,
                switch.scope_dependents_notified,
                switch.nodes_resolved,
                switch.resolved_content_changed,
                switch.resolved_content_unchanged,
                switch.literal_nodes,
                counters.text_shaped,
                Cost::from(counters).without_page_result(),
            ));
        };
        let pinned = |messages: &str| Locale::parse(messages).unwrap().with_language(tag("en"));
        for locale in [
            Locale::parse("zh-cn"),
            Some(pinned("en-us")),
            Some(pinned("en-gb")),
        ] {
            measure(&mut app, &|app| {
                app.context.set_default_locale(locale.clone())
            });
        }
        // The bar becomes a scope of its own, then turns right to left:
        // the bar lays out again, and nothing outside it.
        let bar = node(1_000_000 + 999_000);
        for locale in [Locale::parse("en-us"), Locale::parse("ar")] {
            measure(&mut app, &|app| {
                let mut queue = MutationQueue::new();
                queue.set_locale(bar, locale.clone());
                app.context.commit_mutations(queue).unwrap();
            });
        }
        costs.push(row);
    }
    assert_eq!(costs[0], costs[1]);
    assert_eq!(costs[1], costs[2]);
}

/// Issue #269: a virtual list of localized rows switches locale at the same
/// cost at 100k and at 1M rows: its mounted rows resolve, no logical row is
/// looked up.
#[test]
fn issue259_a_localized_virtual_list_switches_at_the_same_cost_at_any_size() {
    use super::issue269::LocalizedFeed;
    use crate::Locale;
    let mut costs = Vec::new();
    for rows in [100_000, 1_000_000] {
        let mut feed = LocalizedFeed::new(rows);
        let mounted = feed.items.mounted_keys().len();
        let (switch, scanned, frame) = feed.switch(Locale::parse("zh-cn"));
        assert_eq!(scanned, 0);
        assert!(
            switch.virtual_rows_resolved > 0 && switch.virtual_rows_resolved <= mounted,
            "{rows}: {} virtual rows resolved, {mounted} mounted",
            switch.virtual_rows_resolved
        );
        costs.push((
            switch.nodes_resolved,
            switch.virtual_rows_resolved,
            frame.text_shaped,
            Cost::from(frame),
        ));
    }
    assert_eq!(costs[0], costs[1]);
}

/// Issue #270: in 10k and 100k node windows ending in forty localized labels
/// and a counter, the counter's count changing, a catalog update of one
/// message, and a switch to German, whose formatters are built then, cost
/// the same: formatting reaches the nodes whose output it is.
#[test]
fn issue259_formatting_costs_the_same_at_any_size() {
    use super::issue268::{App, cost};
    use super::issue270::counting_catalog;
    use crate::{Locale, LocalizedText, MessageId};
    let mut costs = Vec::new();
    for nodes in [10_000, 100_000] {
        let mut app = App::labelled(nodes, 40, false);
        app.context.set_message_catalog(Some(counting_catalog()));
        let counter = node(1_000_000 + 999_000 + 500);
        let mut queue = MutationQueue::new();
        queue.create(counter, app.windows[0].document, NodeKind::Text);
        queue.insert(node(1_000_000 + 999_000), counter, None);
        queue.set_localized_text(
            counter,
            Some(LocalizedText::new("files").arg("count", 1u32)),
        );
        app.context.commit_mutations(queue).unwrap();
        app.frame(0);
        let mut row = Vec::new();
        let mut measure = |app: &mut App, change: &dyn Fn(&mut App)| {
            let update = cost(app, change);
            let counters = app.frame(0);
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            row.push((
                update.format_requests,
                update.nodes_resolved,
                update.catalog_messages_invalidated,
                update.message_patterns_compiled,
                update.formatter_allocations,
                update.formatted_output_changed,
                counters.text_shaped,
                Cost::from(counters).without_page_result(),
            ));
        };
        for count in [2u32, 3, 40] {
            measure(&mut app, &|app| {
                let mut queue = MutationQueue::new();
                queue.set_localized_text(
                    counter,
                    Some(LocalizedText::new("files").arg("count", count)),
                );
                app.context.commit_mutations(queue).unwrap();
            });
        }
        measure(&mut app, &|app| {
            let updated = Arc::new((*counting_catalog()).clone().with(
                "en-us",
                "item.7",
                "Updated item of {count}",
            ));
            app.context
                .compat_world_mut()
                .update_message_catalog(updated, &[MessageId::new("item.7")]);
        });
        measure(&mut app, &|app| {
            app.context.set_default_locale(Locale::parse("de"))
        });
        costs.push(row);
    }
    assert_eq!(costs[0], costs[1]);
}

/// A label at the head of a page, in a column 1200 px wide that it does not
/// fill, above 1k and 10k nodes of cards, takes a longer string that still
/// fits the column. Its box was only as wide as the string it held, so the
/// new one is not wrapped to that width: the frame lays out once, the label
/// on one line at its new width, and nothing below it moves -- what the same
/// edit costs a label that cannot wrap, and the same at both sizes.
#[test]
fn issue259_a_longer_label_moves_nothing_below_it() {
    use super::reflow_oracle::{self, Builder, FILLER_GROUP_NODES};
    const SHORT: &str = "Short";
    const LONGER: &str = "A much longer label that still fits on one line of its column";
    let viewport = LayoutViewport::new(1200.0, 800.0);
    // The cards sit in a column of their own: the page holds two boxes at
    // any size.
    let build = |nodes: u64, wraps: bool, value: &str| {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 1200.0);
        let head = b.element(page, reflow_oracle::column(Some(1200.0)));
        let label = b.label(head, value);
        if !wraps {
            b.queue.set_style(
                label,
                styled(LayoutStyle {
                    white_space_nowrap: true,
                    ..LayoutStyle::default()
                }),
            );
        }
        let body = b.element(page, reflow_oracle::column(None));
        b.filler(body, nodes / FILLER_GROUP_NODES);
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport, &mut shaper);
        (context, document, label, shaper)
    };
    let mut costs = Vec::new();
    for nodes in [1_000, 10_000] {
        let _unguarded = (nodes > 1_000).then(skip_layout_verify);
        let mut row = Vec::new();
        for wraps in [false, true] {
            let case = format!("{nodes} nodes, wraps {wraps}");
            let (mut context, document, label, mut shaper) = build(nodes, wraps, SHORT);
            let order = context.world().document_order(document);
            let boxes = |context: &AppContext| {
                order
                    .iter()
                    .map(|id| context.world().layout_box(*id))
                    .collect::<Vec<_>>()
            };
            let before = boxes(&context);
            let passes = context.layout_invocations();
            let mut queue = MutationQueue::new();
            queue.set_text(
                label,
                TextContent {
                    value: LONGER.into(),
                },
            );
            context.commit_mutations(queue).unwrap();
            let counters = product_frame(&mut context, document, viewport, &mut shaper);
            let moved: Vec<StableNodeId> = order
                .iter()
                .zip(before.iter().zip(boxes(&context)))
                .filter(|(_, (old, new))| **old != *new)
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(moved, [label], "{case}");
            // Nor did anything move and move back on the way.
            assert_eq!(counters.layout_origin_only_updates, 0, "{case}");
            assert_eq!(context.layout_invocations() - passes, 1, "{case}");
            let (mut cold, ..) = build(nodes, wraps, LONGER);
            assert_matches_cold(&mut context, &mut cold, document);
            row.push(Cost::from(counters).structural());
        }
        assert_eq!(row[0], row[1], "{nodes} nodes: wrapping costs extra");
        costs.push(row[1]);
    }
    assert_eq!(costs[0], costs[1]);
}
