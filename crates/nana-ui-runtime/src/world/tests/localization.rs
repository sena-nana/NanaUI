//! CJK line decisions, text/editable metrics, typography scale, i18n catalogs, locale switches, and message formatting.

#![cfg(test)]

mod cjk_line_break {
    //! Issue #211: the CJK line decision reaches Runtime text through inherited
    //! style.
    //!
    //! A column of two scopes, each a 101 px box around one paragraph of Chinese
    //! with punctuation. Chinese and Japanese close their punctuation up at
    //! fixed amounts unless `text-spacing-trim` says otherwise; every other
    //! language, and every other property, starts off. A typography change lays
    //! the text out again and never shapes it again.

    use nana_text::font::LanguageTag;
    use nana_ui_core::{
        LayoutStyle, LengthSpec, TextJustifySpec, TextSpacingTrimSpec, TextTypography, WorkCounters,
    };

    use crate::world::reflow_oracle::{
        Builder, bundled_face_shaper, column, product_frame, styled,
    };
    use crate::world::{DocumentId, StableNodeId};
    use crate::{AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper};

    const TEXT: &str = "排版，「测试」。中文排版测试中文排版测试中文";

    struct Page {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        scope: StableNodeId,
        text: StableNodeId,
    }

    fn scope_style(typography: TextTypography) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(101.0)),
            text_typography: typography,
            ..LayoutStyle::default()
        }
    }

    impl Page {
        fn new(language: &str, typography: TextTypography) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 400.0);
            let outer = b.element(page, column(None));
            let scope = b.element(outer, scope_style(typography));
            let text = b.label(scope, TEXT);
            b.queue.set_language(outer, LanguageTag::new(language));
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut page = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                scope,
                text,
            };
            page.frame();
            page
        }

        fn frame(&mut self) -> WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                LayoutViewport::new(800.0, 600.0),
                &mut self.shaper,
            )
        }

        fn restyle(&mut self, typography: TextTypography) -> WorkCounters {
            let mut queue = MutationQueue::new();
            queue.set_style(self.scope, styled(scope_style(typography)));
            self.context.commit_mutations(queue).unwrap();
            self.frame()
        }

        /// Each line's width and text.
        fn lines(&self) -> Vec<(f32, String)> {
            let (_, layout) = self.context.world().text_layout(self.text).unwrap();
            layout
                .lines
                .iter()
                .map(|line| (line.metrics.width_px, TEXT[line.source.clone()].to_owned()))
                .collect()
        }

        fn text_width(&self) -> f32 {
            self.lines().iter().map(|(width, _)| *width).sum()
        }
    }

    fn trim(spec: TextSpacingTrimSpec) -> TextTypography {
        TextTypography {
            spacing_trim: Some(spec),
            ..TextTypography::INHERIT
        }
    }

    /// Chinese closes the comma against the opening bracket and the closing
    /// bracket against the full stop; English does not, nor does Chinese that
    /// asks for `space-all`. Japanese is Chinese's default.
    #[test]
    fn issue211_chinese_and_japanese_close_punctuation_up_by_default() {
        let chinese = Page::new("zh-Hans", TextTypography::INHERIT);
        let japanese = Page::new("ja", TextTypography::INHERIT);
        let english = Page::new("en", TextTypography::INHERIT);
        let spaced = Page::new("zh-Hans", trim(TextSpacingTrimSpec::SpaceAll));
        assert!(
            chinese.text_width() < english.text_width() - 1.0,
            "{:?} {:?}",
            chinese.lines(),
            english.lines()
        );
        assert_eq!(japanese.text_width(), chinese.text_width());
        assert_eq!(spaced.lines(), english.lines());
    }

    /// A scope's typography reaches the text under it: justified, every line
    /// but the last fills the box. Turning it on and off lays the text out again
    /// and shapes nothing.
    #[test]
    fn issue211_inherited_typography_relays_text_out_without_reshaping() {
        let mut page = Page::new("zh-Hans", TextTypography::INHERIT);
        let ragged = page.lines();
        assert!(ragged.len() > 1);
        let justified = TextTypography {
            justify_lines: Some(true),
            justify: Some(TextJustifySpec::InterCharacter),
            ..TextTypography::INHERIT
        };
        let counters = page.restyle(justified);
        assert_eq!(counters.text_shaped_runs, 0, "{counters:?}");
        assert!(counters.text_constraint_relayouts >= 1, "{counters:?}");
        let lines = page.lines();
        let (last, filled) = lines.split_last().unwrap();
        for (width, line) in filled {
            assert!((width - 101.0).abs() < 0.05, "{line}: {width}");
        }
        assert!(last.0 < 101.0);

        let counters = page.restyle(TextTypography::INHERIT);
        assert_eq!(counters.text_shaped_runs, 0, "{counters:?}");
        assert_eq!(page.lines(), ragged);
    }

    /// `text-spacing-trim: auto` turns cost fitting on: a line a little short
    /// closes its punctuation up rather than break, so it never takes more lines
    /// than the fixed default.
    #[test]
    fn issue211_auto_trim_never_takes_more_lines_than_the_default() {
        let mut page = Page::new("zh-Hans", TextTypography::INHERIT);
        let fixed = page.lines();
        page.restyle(trim(TextSpacingTrimSpec::Auto));
        let fitted = page.lines();
        assert!(fitted.len() <= fixed.len());
        for (width, _) in &fitted {
            assert!(*width <= 101.05);
        }
    }
}

mod text_metrics {
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

    use crate::world::reflow_oracle::{
        Builder, FILLER_GROUP_NODES, admitted_nodes, assert_matches_cold, bundled_face_shaper,
        column, fixed, measured, node, product_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
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
        let (paragraph_box, paragraph, other) =
            (doc.paragraph_box, doc.paragraph, doc.other_paragraph);
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
        assert!(
            doc.world().last_text_work_counters().text_nodes_shaped <= SCOPE_LABELS as usize + 1
        );

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
}

mod typography_scale {
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

    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        Builder, FILLER_GROUP_NODES, admitted, assert_matches_cold, bundled_face_shaper, column,
        fixed, measured, product_frame,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        AppContext, Button, LayoutBox, LayoutViewport, MutationQueue, NanaTextEngineShaper,
        TextContent,
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
}

pub(super) mod catalog_switching {
    //! Issue #268: localized text, its catalog and its scopes.
    //!
    //! A window is a page of ten sections of groups of a hundred: in each group
    //! one literal text node, a localized text node every twentieth node, and
    //! fixed boxes for the rest. Five percent of a 100k window is localized.
    //!
    //! - Gate A: a steady frame looks up, visits and resolves nothing.
    //! - Gate B: the messages' locale moves and nothing else: the localized
    //!   nodes are notified, and no literal node, language or direction is.
    //! - Gate C: of two windows, switching one leaves every node of the other
    //!   untouched.
    //! - Gate D: the index holds what is registered; ten thousand mounts and
    //!   unmounts leave it where it was.
    //! - Gate E: a switch resolves no literal text.

    use std::sync::Arc;

    use nana_text::font::LanguageTag;
    use nana_ui_core::{DirSpec, FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        bundled_face_shaper, node, product_frame, product_frame_seeded, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId};
    use crate::{
        AppContext, LayoutViewport, Locale, LocaleScope, LocalizedText, MessageTable,
        MutationQueue, NanaTextEngineShaper, TextContent,
    };

    /// Messages the windows say: `item.0` .. `item.49`.
    pub(in crate::world::tests) const MESSAGES: u64 = 50;

    pub(in crate::world::tests) fn tag(tag: &str) -> LanguageTag {
        LanguageTag::new(tag).unwrap()
    }

    /// English, British English and Chinese for every message; the British
    /// strings of the first twenty messages equal the American ones.
    pub(in crate::world::tests) fn catalog() -> Arc<MessageTable> {
        let mut table = MessageTable::new();
        for message in 0..MESSAGES {
            let key = format!("item.{message}");
            table = table
                .with("en-us", &key, &format!("Item {message} of {{count}}"))
                .with(
                    "en-gb",
                    &key,
                    &if message < 20 {
                        format!("Item {message} of {{count}}")
                    } else {
                        format!("Entry {message} of {{count}}")
                    },
                )
                .with("zh-cn", &key, &format!("第 {message} 项，共 {{count}} 项"))
                .with("ar", &key, &format!("العنصر {message} من {{count}}"));
        }
        Arc::new(table)
    }

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(1200.0, 800.0)
    }

    /// What one window holds.
    pub(in crate::world::tests) struct Window {
        pub(in crate::world::tests) document: DocumentId,
        pub(in crate::world::tests) localized: Vec<StableNodeId>,
        /// The message each localized node says: `item.{n}`.
        pub(in crate::world::tests) messages: Vec<u64>,
        pub(in crate::world::tests) literal: Vec<StableNodeId>,
    }

    impl Window {
        /// Localized nodes whose British English reads as the American: the
        /// first twenty messages.
        pub(in crate::world::tests) fn same_in_british(&self) -> usize {
            self.messages
                .iter()
                .filter(|message| **message < 20)
                .count()
        }
    }

    /// Sections of a window: section `index` of the window whose ids start at
    /// `base`.
    pub(in crate::world::tests) fn section(base: u64, index: u64) -> StableNodeId {
        node(base + 2 + index)
    }

    const SECTIONS: u64 = 10;

    /// A window of about `nodes` nodes whose ids start at `base`, with at most
    /// `cap` localized nodes and boxes for the rest.
    pub(in crate::world::tests) fn window_with(
        queue: &mut MutationQueue,
        document: DocumentId,
        base: u64,
        nodes: u64,
        cap: usize,
    ) -> Window {
        // Sections and groups are as wide as the page: a string that grows
        // widens no container, so what a switch lays out is the text's own.
        let column = || {
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(1200.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            })
        };
        let root = node(base);
        queue.create(root, document, NodeKind::Document);
        let page = node(base + 1);
        queue.create(page, document, NodeKind::Element { tag: "page".into() });
        queue.insert(root, page, None);
        queue.set_style(page, column());
        for index in 0..SECTIONS {
            let id = section(base, index);
            queue.create(
                id,
                document,
                NodeKind::Element {
                    tag: "section".into(),
                },
            );
            queue.insert(page, id, None);
            queue.set_style(id, column());
        }
        let mut localized = Vec::new();
        let mut messages = Vec::new();
        let mut literal = Vec::new();
        let mut next = base + 2 + SECTIONS;
        let mut index = 0u64;
        let mut groups = 0u64;
        while next < base + nodes {
            let group = node(next);
            next += 1;
            queue.create(
                group,
                document,
                NodeKind::Element {
                    tag: "group".into(),
                },
            );
            queue.insert(section(base, groups % SECTIONS), group, None);
            queue.set_style(group, column());
            groups += 1;
            for slot in 0..99u64 {
                let id = node(next);
                next += 1;
                index += 1;
                if slot == 0 {
                    queue.create(id, document, NodeKind::Text);
                    queue.insert(group, id, None);
                    queue.set_text(
                        id,
                        TextContent {
                            value: "literal".into(),
                        },
                    );
                    literal.push(id);
                } else if index.is_multiple_of(20) && localized.len() < cap {
                    queue.create(id, document, NodeKind::Text);
                    queue.insert(group, id, None);
                    queue.set_localized_text(
                        id,
                        Some(
                            LocalizedText::new(&format!(
                                "item.{}",
                                localized.len() as u64 % MESSAGES
                            ))
                            .arg("count", 3u32),
                        ),
                    );
                    messages.push(localized.len() as u64 % MESSAGES);
                    localized.push(id);
                } else {
                    queue.create(id, document, NodeKind::Element { tag: "box".into() });
                    queue.insert(group, id, None);
                    queue.set_style(
                        id,
                        styled(LayoutStyle {
                            width: Some(LengthSpec::Px(40.0)),
                            height: Some(LengthSpec::Px(2.0)),
                            ..LayoutStyle::default()
                        }),
                    );
                }
            }
        }
        Window {
            document,
            localized,
            messages,
            literal,
        }
    }

    /// A window of about `nodes` nodes of literal filler ending in a bar of a
    /// fixed height holding `labels` localized labels: a label that measures
    /// differently moves nothing outside the bar.
    pub(in crate::world::tests) fn labelled_window(
        queue: &mut MutationQueue,
        document: DocumentId,
        base: u64,
        nodes: u64,
        labels: usize,
    ) -> Window {
        let mut window = window_with(queue, document, base, nodes, 0);
        let bar = node(base + 999_000);
        queue.create(bar, document, NodeKind::Element { tag: "bar".into() });
        queue.insert(node(base + 1), bar, None);
        queue.set_style(
            bar,
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(1200.0)),
                height: Some(LengthSpec::Px(2000.0)),
                direction: Some(FlexDirection::Column),
                flex_shrink: Some(0.0),
                ..LayoutStyle::default()
            }),
        );
        for label in 0..labels as u64 {
            let id = node(base + 999_001 + label);
            queue.create(id, document, NodeKind::Text);
            queue.insert(bar, id, None);
            let message = label % MESSAGES;
            queue.set_localized_text(
                id,
                Some(LocalizedText::new(&format!("item.{message}")).arg("count", 3u32)),
            );
            window.localized.push(id);
            window.messages.push(message);
        }
        window
    }

    /// An application with one window per entry of `sizes`, in en-US.
    pub(in crate::world::tests) struct App {
        pub(in crate::world::tests) context: AppContext,
        pub(in crate::world::tests) windows: Vec<Window>,
        shaper: NanaTextEngineShaper,
        guarded: bool,
    }

    impl App {
        pub(in crate::world::tests) fn new(sizes: &[u64], guarded: bool) -> Self {
            Self::build(guarded, |queue| {
                sizes
                    .iter()
                    .enumerate()
                    .map(|(at, nodes)| {
                        let document = DocumentId::new(at as u64 + 1).unwrap();
                        window_with(
                            queue,
                            document,
                            (at as u64 + 1) * 1_000_000,
                            *nodes,
                            usize::MAX,
                        )
                    })
                    .collect()
            })
        }

        /// One window of about `nodes` nodes ending in `labels` localized
        /// labels; see [`labelled_window`].
        pub(in crate::world::tests) fn labelled(nodes: u64, labels: usize, guarded: bool) -> Self {
            Self::build(guarded, |queue| {
                let document = DocumentId::new(1).unwrap();
                vec![labelled_window(queue, document, 1_000_000, nodes, labels)]
            })
        }

        /// The windows `windows` queues, in en-US, each laid out once.
        fn build(guarded: bool, windows: impl FnOnce(&mut MutationQueue) -> Vec<Window>) -> Self {
            let mut context = AppContext::new();
            context.set_message_catalog(Some(catalog()));
            context.set_default_locale(Locale::parse("en-us"));
            let mut queue = MutationQueue::new();
            let windows = windows(&mut queue);
            context.compat_world_mut().commit(queue).unwrap();
            let mut app = Self {
                context,
                windows,
                shaper: bundled_face_shaper(),
                guarded,
            };
            for at in 0..app.windows.len() {
                app.frame(at);
            }
            app
        }

        /// One frame of window `at`.
        pub(in crate::world::tests) fn frame(&mut self, at: usize) -> WorkCounters {
            let _unguarded = (!self.guarded).then(skip_layout_verify);
            let document = self.windows[at].document;
            product_frame(&mut self.context, document, viewport(), &mut self.shaper)
        }

        /// [`Self::frame`], and the nodes its layout passes were seeded at.
        pub(in crate::world::tests) fn frame_seeded(
            &mut self,
            at: usize,
        ) -> (WorkCounters, std::collections::HashSet<StableNodeId>) {
            let _unguarded = (!self.guarded).then(skip_layout_verify);
            let document = self.windows[at].document;
            product_frame_seeded(&mut self.context, document, viewport(), &mut self.shaper)
        }

        pub(in crate::world::tests) fn text(&self, id: StableNodeId) -> String {
            self.context
                .world()
                .text(id)
                .unwrap_or_default()
                .to_string()
        }

        /// The localization counters a change left for the next drain.
        pub(in crate::world::tests) fn pending(&self) -> nana_ui_core::I18nCounters {
            self.context.last_work_counters().i18n
        }
    }

    /// What every work counter did from `before` to `after`. The index sizes,
    /// which every snapshot carries, stay zero.
    pub(in crate::world::tests) fn difference(
        after: nana_ui_core::I18nCounters,
        before: nana_ui_core::I18nCounters,
    ) -> nana_ui_core::I18nCounters {
        nana_ui_core::I18nCounters {
            literal_nodes: after.literal_nodes - before.literal_nodes,
            catalog_lookups: after.catalog_lookups - before.catalog_lookups,
            catalog_cache_hits: after.catalog_cache_hits - before.catalog_cache_hits,
            catalog_cache_misses: after.catalog_cache_misses - before.catalog_cache_misses,
            scope_dependents_notified: after.scope_dependents_notified
                - before.scope_dependents_notified,
            switch_transactions: after.switch_transactions - before.switch_transactions,
            nodes_resolved: after.nodes_resolved - before.nodes_resolved,
            resolved_content_changed: after.resolved_content_changed
                - before.resolved_content_changed,
            resolved_content_unchanged: after.resolved_content_unchanged
                - before.resolved_content_unchanged,
            language_changed_nodes: after.language_changed_nodes - before.language_changed_nodes,
            direction_changed_scopes: after.direction_changed_scopes
                - before.direction_changed_scopes,
            layout_seeds: after.layout_seeds - before.layout_seeds,
            virtual_rows_resolved: after.virtual_rows_resolved - before.virtual_rows_resolved,
            switch_commits: after.switch_commits - before.switch_commits,
            format_requests: after.format_requests - before.format_requests,
            format_cache_hits: after.format_cache_hits - before.format_cache_hits,
            format_cache_misses: after.format_cache_misses - before.format_cache_misses,
            message_patterns_compiled: after.message_patterns_compiled
                - before.message_patterns_compiled,
            args_revisions: after.args_revisions - before.args_revisions,
            formatted_output_changed: after.formatted_output_changed
                - before.formatted_output_changed,
            formatted_output_unchanged: after.formatted_output_unchanged
                - before.formatted_output_unchanged,
            catalog_messages_invalidated: after.catalog_messages_invalidated
                - before.catalog_messages_invalidated,
            formatter_allocations: after.formatter_allocations - before.formatter_allocations,
            ..nana_ui_core::I18nCounters::default()
        }
    }

    /// What `change` cost localization, read before the next drain.
    pub(in crate::world::tests) fn cost(
        app: &mut App,
        change: impl FnOnce(&mut App),
    ) -> nana_ui_core::I18nCounters {
        let before = app.pending();
        change(app);
        difference(app.pending(), before)
    }

    #[test]
    fn issue268_gate_a_a_steady_frame_resolves_nothing() {
        for nodes in [10_000, 100_000] {
            let mut app = App::new(&[nodes], nodes == 10_000);
            let before = app.pending();
            let passes = app.context.layout_invocations();
            app.frame(0);
            assert!(app.context.take_system_work().is_empty(), "{nodes}");
            assert_eq!(app.context.layout_invocations(), passes, "{nodes}");
            assert_eq!(
                difference(app.pending(), before),
                Default::default(),
                "{nodes}"
            );
        }
    }

    #[test]
    fn issue268_gate_b_a_messages_only_change_reaches_the_localized_nodes() {
        let mut app = App::new(&[100_000], false);
        let localized = app.windows[0].localized.len();
        let same = app.windows[0].same_in_british();
        // About five percent of the window.
        assert!((4_800..=5_000).contains(&localized), "{localized}");
        // Messages move; the language and direction are named and stay.
        let pinned = |messages: &str| {
            Locale::parse(messages)
                .unwrap()
                .with_language(tag("en"))
                .with_direction(DirSpec::Ltr)
        };
        app.context.set_default_locale(Some(pinned("en-us")));
        app.frame(0);
        let scope = LocaleScope::Document(app.windows[0].document);
        let generations = app.context.world().locale_generations(scope);
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Some(pinned("en-gb")));
        });
        assert_eq!(switch.scope_dependents_notified, localized, "{switch:?}");
        assert_eq!(switch.nodes_resolved, localized);
        assert_eq!(switch.literal_nodes, 0);
        assert_eq!(switch.language_changed_nodes, 0, "the language is named");
        assert_eq!(switch.direction_changed_scopes, 0, "the direction is named");
        // Thirty of fifty messages read differently in British English.
        assert_eq!(switch.resolved_content_changed, localized - same);
        assert_eq!(switch.resolved_content_unchanged, same);
        // Fifty messages, one lookup each before the cache answers.
        assert_eq!(switch.catalog_cache_misses, MESSAGES as usize);
        assert_eq!(
            switch.catalog_cache_hits,
            localized - MESSAGES as usize,
            "{switch:?}"
        );
        let after = app.context.world().locale_generations(scope);
        assert_eq!(after.messages, generations.messages + 1);
        assert_eq!(after.language, generations.language);
        assert_eq!(after.direction, generations.direction);
    }

    #[test]
    fn issue268_gate_c_switching_one_window_leaves_the_other_alone() {
        let mut app = App::new(&[20_000, 20_000], false);
        let (a, b) = (app.windows[0].document, app.windows[1].document);
        let localized = app.windows[0].localized.len();
        let texts_b: Vec<String> = app.windows[1]
            .localized
            .iter()
            .map(|id| app.text(*id))
            .collect();
        let generations_b = app
            .context
            .world()
            .locale_generations(LocaleScope::Document(b));
        let switch = cost(&mut app, |app| {
            app.context.set_document_locale(a, Locale::parse("zh-cn"));
        });
        assert_eq!(
            switch.scope_dependents_notified, localized,
            "window A's alone"
        );
        assert_eq!(switch.nodes_resolved, localized);
        // Everything the switch left for the frame is window A's.
        let pending = app.context.take_system_work();
        let world = app.context.world();
        for id in pending
            .text
            .iter()
            .chain(pending.style.iter())
            .chain(pending.layout_frontier_seeds.iter().map(|seed| &seed.node))
        {
            assert_eq!(world.document_of(*id), Some(a), "{id:?} is not window A's");
        }
        let texts: Vec<String> = app.windows[1]
            .localized
            .iter()
            .map(|id| app.text(*id))
            .collect();
        assert_eq!(texts, texts_b);
        assert_eq!(
            app.context
                .world()
                .locale_generations(LocaleScope::Document(b)),
            generations_b
        );
        assert!(app.text(app.windows[0].localized[0]).starts_with('第'));
    }

    #[test]
    fn issue268_gate_d_the_index_follows_registration_not_history() {
        let mut app = App::new(&[100_000], false);
        let document = app.windows[0].document;
        let localized = app.windows[0].localized.len();
        let (nodes, links, scopes, lookups) = app.context.world().i18n.footprint();
        assert_eq!((nodes, links, scopes), (localized, localized, 0));
        assert!(lookups <= MESSAGES as usize * 3);
        let holder = node(9_000_000);
        let mut queue = MutationQueue::new();
        queue.create(
            holder,
            document,
            NodeKind::Element {
                tag: "holder".into(),
            },
        );
        queue.insert(section(1_000_000, 0), holder, None);
        app.context.commit_mutations(queue).unwrap();
        for round in 0..10_000u64 {
            let id = node(9_000_001 + round);
            let mut queue = MutationQueue::new();
            queue.create(id, document, NodeKind::Text);
            queue.insert(holder, id, None);
            queue.set_localized_text(id, Some(LocalizedText::new("item.7").arg("count", round)));
            app.context.commit_mutations(queue).unwrap();
            let mut queue = MutationQueue::new();
            queue.despawn_subtree(id);
            app.context.commit_mutations(queue).unwrap();
        }
        let after = app.context.world().i18n.footprint();
        assert_eq!((after.0, after.1, after.2), (localized, localized, 0));
        assert!(after.3 <= MESSAGES as usize * 3);
    }

    #[test]
    fn issue268_gate_e_a_switch_resolves_no_literal_text() {
        let mut app = App::new(&[10_000], true);
        let literal = app.windows[0].literal.clone();
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Locale::parse("zh-cn"));
        });
        assert_eq!(switch.literal_nodes, 0);
        assert_eq!(switch.nodes_resolved, app.windows[0].localized.len());
        let pending = app.context.take_system_work();
        for id in &literal {
            assert!(!pending.text.contains(id), "literal {id:?} shapes again");
            assert_eq!(app.text(*id), "literal");
        }
    }
}

pub(super) mod virtual_feed {
    //! Issue #269: a locale switch is one transaction, and layout hears of it
    //! only through what moved.
    //!
    //! - Gate A: en-US to zh-CN in a 100k window with 5k localized nodes
    //!   resolves the 5k and no literal node; both are left to right, so no
    //!   scope's direction moves; the frame shapes and lays out no more than the
    //!   strings that changed, with no full-document fallback.
    //! - Gate B: of 5k localized nodes, 2k read the same in the new locale and
    //!   shape in the same language: the frame shapes only the 3k.
    //! - Gate C: a thousand fixed-size buttons change language: their labels
    //!   lay out again inside them, and nothing around them measures.
    //! - Gate D: a right-to-left switch of a subtree scope lays out that subtree
    //!   and nothing outside it.
    //! - Gate E: a million-row virtual list of localized rows resolves its
    //!   mounted rows, each counted as a virtual row, and looks up no logical
    //!   row.
    //! - Gate F: a switch is one transaction, landed whole before the frame.

    use nana_ui_core::{
        DirSpec, FlexDirection, I18nCounters, LayoutStyle, LengthSpec, VirtualListLayout,
    };

    use crate::world::reflow_oracle::{bundled_face_shaper, node, product_frame, styled};
    use crate::world::tests::incremental_work::virtual_scroll::{port, viewport};
    use crate::world::tests::localization::catalog_switching::{
        App, MESSAGES, cost, difference, section, tag,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId};
    use crate::{
        AppContext, LayoutViewport, Locale, LocaleScope, LocalizedText, MutationQueue, Text,
        VirtualListItems,
    };

    #[test]
    fn issue269_gate_a_en_to_zh_resolves_the_localized_nodes_and_lays_out_what_moved() {
        let mut app = App::new(&[100_000], false);
        let localized = app.windows[0].localized.len();
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Locale::parse("zh-cn"));
        });
        assert_eq!(switch.nodes_resolved, localized, "{switch:?}");
        assert_eq!(switch.literal_nodes, 0);
        assert_eq!(switch.direction_changed_scopes, 0, "both are left to right");
        assert_eq!(switch.resolved_content_changed, localized);
        assert_eq!(switch.switch_transactions, 1);
        assert_eq!(switch.switch_commits, 1);
        let (frame, seeded) = app.frame_seeded(0);
        assert_eq!(frame.layout_full_document_fallbacks, 0);
        assert!(
            frame.text_shaped <= localized,
            "{} shaped",
            frame.text_shaped
        );
        // Layout is seeded at the localized text that changed and at the
        // container each sits in, nowhere else: a string shapes again at its box
        // and may seed again, its group places it.
        let world = app.context.world();
        let localized_nodes: std::collections::HashSet<StableNodeId> =
            app.windows[0].localized.iter().copied().collect();
        let containers: std::collections::HashSet<StableNodeId> = localized_nodes
            .iter()
            .filter_map(|id| world.parent_id(*id))
            .collect();
        assert!(
            seeded
                .iter()
                .all(|id| localized_nodes.contains(id) || containers.contains(id)),
            "a seed outside the localized text and its containers"
        );
        assert!(
            seeded.len() <= localized + containers.len(),
            "{} seeded",
            seeded.len()
        );
    }

    #[test]
    fn issue269_gate_b_an_equal_translation_does_no_text_work() {
        let mut app = App::new(&[100_000], false);
        let localized = app.windows[0].localized.len();
        let same = app.windows[0].same_in_british();
        // The language is named, so it holds across the switch.
        let pinned = |messages: &str| Locale::parse(messages).unwrap().with_language(tag("en"));
        app.context.set_default_locale(Some(pinned("en-us")));
        app.frame(0);
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Some(pinned("en-gb")));
        });
        assert_eq!(switch.resolved_content_unchanged, same, "{switch:?}");
        assert_eq!(switch.resolved_content_changed, localized - same);
        assert_eq!(switch.language_changed_nodes, 0);
        let frame = app.frame(0);
        assert_eq!(
            frame.text_shaped,
            localized - same,
            "only the strings that changed shape"
        );
    }

    #[test]
    fn issue269_gate_c_fixed_buttons_keep_the_layout_around_them() {
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        context.set_message_catalog(Some(
            crate::world::tests::localization::catalog_switching::catalog(),
        ));
        context.set_default_locale(Locale::parse("en-us"));
        let mut queue = MutationQueue::new();
        queue.create(node(1), document, NodeKind::Document);
        queue.create(node(2), document, NodeKind::Element { tag: "bar".into() });
        queue.insert(node(1), node(2), None);
        queue.set_style(
            node(2),
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(1200.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
        );
        for button in 0..1_000u64 {
            let id = node(10 + 2 * button);
            queue.create(
                id,
                document,
                NodeKind::Element {
                    tag: "button".into(),
                },
            );
            queue.insert(node(2), id, None);
            queue.set_style(
                id,
                styled(LayoutStyle {
                    width: Some(LengthSpec::Px(160.0)),
                    height: Some(LengthSpec::Px(28.0)),
                    flex_shrink: Some(0.0),
                    ..LayoutStyle::default()
                }),
            );
            let label = node(11 + 2 * button);
            queue.create(label, document, NodeKind::Text);
            queue.insert(id, label, None);
            queue.set_localized_text(
                label,
                Some(LocalizedText::new(&format!("item.{}", button % MESSAGES)).arg("count", 2u32)),
            );
        }
        context.compat_world_mut().commit(queue).unwrap();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(1200.0, 800.0);
        product_frame(&mut context, document, viewport, &mut shaper);
        let buttons: Vec<_> = (0..1_000u64)
            .map(|button| context.world().layout_box(node(10 + 2 * button)))
            .collect();
        context.set_default_locale(Locale::parse("zh-cn"));
        let frame = product_frame(&mut context, document, viewport, &mut shaper);
        // The labels lay out again inside their buttons, and each button
        // resolves its own fixed size without walking anything: nothing around
        // the buttons measures.
        assert!(
            frame.layout_measure_nodes <= 2 * 1_000,
            "{} measured",
            frame.layout_measure_nodes
        );
        assert!(
            frame.intrinsic_measure_full_subtrees <= 2 * 1_000,
            "{} full subtrees",
            frame.intrinsic_measure_full_subtrees
        );
        assert!(frame.layout_frontier_nodes_measure <= 2 * 1_000);
        assert_eq!(frame.layout_full_document_fallbacks, 0);
        let after: Vec<_> = (0..1_000u64)
            .map(|button| context.world().layout_box(node(10 + 2 * button)))
            .collect();
        assert_eq!(after, buttons, "no button moved or changed size");
    }

    #[test]
    fn issue269_gate_d_a_right_to_left_scope_lays_out_only_its_subtree() {
        let mut app = App::new(&[100_000], false);
        let document = app.windows[0].document;
        // The first of the page's ten sections, a tenth of it, is scope A.
        let scope = section(1_000_000, 0);
        let mut queue = MutationQueue::new();
        queue.set_locale(scope, Locale::parse("en-us"));
        app.context.commit_mutations(queue).unwrap();
        app.frame(0);
        let switch = cost(&mut app, |app| {
            let mut queue = MutationQueue::new();
            queue.set_locale(scope, Locale::parse("ar"));
            app.context.commit_mutations(queue).unwrap();
        });
        assert_eq!(switch.direction_changed_scopes, 1, "{switch:?}");
        assert_eq!(
            app.context.world().locale_scope(scope),
            Some(LocaleScope::Node(scope))
        );
        let pending = app.context.take_system_work();
        let world = app.context.world();
        let inside = |id: StableNodeId| {
            let mut current = Some(id);
            while let Some(node) = current {
                if node == scope {
                    return true;
                }
                current = world.parent_id(node);
            }
            false
        };
        for id in pending
            .text
            .iter()
            .chain(pending.layout_frontier_seeds.iter().map(|seed| &seed.node))
        {
            assert!(inside(*id), "{id:?} is outside the scope");
            assert_eq!(world.document_of(*id), Some(document));
        }
    }

    /// A virtual list of `rows` localized rows in a ScrollView 40 rows tall, in
    /// en-US, with its window mounted and laid out.
    pub(in crate::world::tests) struct LocalizedFeed {
        pub(in crate::world::tests) context: AppContext,
        shaper: crate::NanaTextEngineShaper,
        document: DocumentId,
        pub(in crate::world::tests) items: VirtualListItems<u64, Text>,
    }

    impl LocalizedFeed {
        pub(in crate::world::tests) fn new(rows: usize) -> Self {
            let document = DocumentId::new(1).unwrap();
            let mut context = AppContext::new();
            context.set_message_catalog(Some(
                crate::world::tests::localization::catalog_switching::catalog(),
            ));
            context.set_default_locale(Locale::parse("en-us"));
            let (scroll, list) = port(&mut context);
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document, viewport(), &mut shaper);
            let layout = VirtualListLayout::uniform(rows, 24.0);
            let mut items = VirtualListItems::<u64, Text>::default();
            context
                .sync_virtual_list_retained_in(
                    scroll,
                    list,
                    &mut items,
                    &layout,
                    8.0 * 24.0,
                    0,
                    &[],
                    |index| index as u64,
                    |key| usize::try_from(*key).ok(),
                    |index, _| {
                        Text::localized(
                            LocalizedText::new(&format!("item.{}", index as u64 % MESSAGES))
                                .arg("count", index),
                        )
                    },
                )
                .unwrap();
            product_frame(&mut context, document, viewport(), &mut shaper);
            Self {
                context,
                shaper,
                document,
                items,
            }
        }

        /// Switch the application to `locale`: what localization did, the
        /// logical rows the list looked up meanwhile, and the frame that
        /// followed.
        pub(in crate::world::tests) fn switch(
            &mut self,
            locale: Option<Locale>,
        ) -> (I18nCounters, usize, crate::WorkCounters) {
            let before = self.context.last_work_counters();
            self.context.set_default_locale(locale);
            let after = self.context.last_work_counters();
            let frame = product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            );
            (
                difference(after.i18n, before.i18n),
                after.virtual_logical_rows_scanned - before.virtual_logical_rows_scanned,
                frame,
            )
        }
    }

    #[test]
    fn issue269_gate_e_a_million_virtual_rows_resolve_what_is_mounted() {
        let mut feed = LocalizedFeed::new(1_000_000);
        let mounted = feed.items.mounted_keys().len();
        assert!(mounted > 0 && mounted <= 64, "{mounted} rows");
        let (switch, scanned, frame) = feed.switch(Locale::parse("zh-cn"));
        assert_eq!(
            switch.nodes_resolved, mounted,
            "the mounted rows, not the million"
        );
        assert_eq!(
            switch.virtual_rows_resolved, mounted,
            "each of them a virtual row"
        );
        assert_eq!(scanned, 0, "no logical row is looked up");
        assert_eq!(frame.layout_full_document_fallbacks, 0);
        let key = feed.items.mounted_keys()[3];
        let row = feed.items.entity(&key).unwrap().stable_id();
        assert!(
            feed.context.world().text(row).unwrap().starts_with('第'),
            "a mounted row shows the new locale"
        );
    }

    #[test]
    fn issue269_gate_f_a_switch_lands_whole_before_the_frame() {
        let mut app = App::new(&[10_000], true);
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Locale::parse("zh-cn"));
        });
        assert_eq!(switch.switch_transactions, 1);
        assert_eq!(switch.switch_commits, 1);
        // Before any frame: every localized node already says the new locale.
        for id in &app.windows[0].localized {
            assert!(app.text(*id).starts_with('第'), "{id:?}: {}", app.text(*id));
        }
        // A direction switch lands with it.
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Some(
                Locale::parse("zh-cn").unwrap().with_direction(DirSpec::Rtl),
            ));
        });
        assert_eq!(switch.switch_transactions, 1);
        assert_eq!(switch.direction_changed_scopes, 1);
    }

    /// A localized node taken out of a scope reads its window; put under
    /// another section, it reads that section's locale.
    #[test]
    fn issue269_a_node_moved_out_of_a_scope_reads_where_it_is() {
        let mut app = App::new(&[10_000], true);
        let scope = section(1_000_000, 0);
        let mut queue = MutationQueue::new();
        queue.set_locale(scope, Locale::parse("zh-cn"));
        app.context.commit_mutations(queue).unwrap();
        let world = app.context.world();
        let id = *app.windows[0]
            .localized
            .iter()
            .find(|id| world.locale_scope(**id) == Some(LocaleScope::Node(scope)))
            .expect("a localized node in the scope");
        assert!(app.text(id).starts_with('第'), "{}", app.text(id));
        let group = app.context.world().parent_id(id).unwrap();
        let mut queue = MutationQueue::new();
        queue.detach(group);
        app.context.commit_mutations(queue).unwrap();
        assert!(app.text(id).starts_with("Item"), "{}", app.text(id));
        let mut queue = MutationQueue::new();
        queue.insert(section(1_000_000, 1), group, None);
        app.context.commit_mutations(queue).unwrap();
        assert_eq!(
            app.context.world().locale_scope(id),
            Some(LocaleScope::Document(app.windows[0].document))
        );
        assert!(app.text(id).starts_with("Item"));
        // Back into the scope: Chinese again.
        let mut queue = MutationQueue::new();
        queue.insert(scope, group, None);
        app.context.commit_mutations(queue).unwrap();
        assert!(app.text(id).starts_with('第'), "{}", app.text(id));
    }

    /// A switch that moves the messages and names a language shapes the
    /// localized text in that language, not in the locale the old messages
    /// came from.
    #[test]
    fn issue269_a_switch_that_names_a_language_shapes_in_it() {
        let mut app = App::new(&[1_000], true);
        let id = app.windows[0].localized[0];
        let language = |app: &App| {
            app.context
                .world()
                .computed_style(id)
                .and_then(|style| style.language.clone())
        };
        app.context.set_default_locale(Locale::parse("zh-cn"));
        app.frame(0);
        assert_eq!(language(&app), Some(tag("zh-cn")));
        app.context.set_default_locale(Some(
            Locale::parse("en-us").unwrap().with_language(tag("en")),
        ));
        app.frame(0);
        assert_eq!(language(&app), Some(tag("en")));
    }

    /// A switch the application makes outside a commit moves the world's
    /// generation when it changed what shows, as a commit does, so a host that
    /// draws when the generation moves draws it; a switch that changes nothing
    /// leaves it.
    #[test]
    fn issue269_a_switch_outside_a_commit_moves_the_generation() {
        let mut app = App::new(&[1_000], true);
        let generation = app.context.world().generation();
        app.context.set_default_locale(Locale::parse("zh-cn"));
        let switched = app.context.world().generation();
        assert_ne!(switched, generation, "the text changed");
        app.context.set_default_locale(Locale::parse("zh-cn"));
        assert_eq!(
            app.context.world().generation(),
            switched,
            "nothing changed"
        );
    }
}

pub(super) mod message_formatting {
    //! Issue #270: messages format through compiled patterns and kept
    //! formatters, and a changed argument reaches only its own node.
    //!
    //! The windows are Issue #268's, with one more node: a counter saying
    //! `{count, plural, one {# file} other {# files}}`.
    //!
    //! - Gate A: the counter's count changes every frame among 10k localized
    //!   nodes: one format request, no catalog lookup missed, nothing else
    //!   formatted, and only the counter shapes.
    //! - Gate B: an argument change the integer style absorbs formats to the
    //!   same string: no text or layout work.
    //! - Gate C: ten thousand numbers formatted in one locale compile their
    //!   pattern once and build one formatter: the formatter cache answers 99%.
    //! - Gate D: a catalog update of one message reaches the nodes saying it
    //!   and no other.
    //! - Gate E: a locale switch compiles each message once, not each node.
    //!
    //! Gate F, allocation, is `tests/i18n_alloc.rs`.

    use std::sync::Arc;

    use nana_ui_core::WorkCounters;

    use crate::world::reflow_oracle::node;
    use crate::world::tests::localization::catalog_switching::{
        App, MESSAGES, catalog, cost, section,
    };
    use crate::world::{NodeKind, StableNodeId};
    use crate::{Locale, LocalizedText, MessageId, MessageTable, MutationQueue, TextContent};

    /// Issue #268's catalog with the counter's messages.
    pub(in crate::world::tests) fn counting_catalog() -> Arc<MessageTable> {
        Arc::new(
            (*catalog())
                .clone()
                .with(
                    "en-us",
                    "files",
                    "{count, plural, one {# file} other {# files}}",
                )
                .with("zh-cn", "files", "{count} 个文件")
                .with("en-us", "rounded", "{n, number, integer} items")
                .with("en-us", "amount", "{n, number}")
                .with("de", "amount", "{n, number}"),
        )
    }

    const COUNTER: u64 = 9_500_000;

    /// An app of `nodes` nodes with the counter in its first section.
    fn counting(nodes: u64, guarded: bool) -> App {
        let mut app = App::new(&[nodes], guarded);
        app.context.set_message_catalog(Some(counting_catalog()));
        let document = app.windows[0].document;
        let mut queue = MutationQueue::new();
        queue.create(node(COUNTER), document, NodeKind::Text);
        queue.insert(section(1_000_000, 0), node(COUNTER), None);
        queue.set_localized_text(
            node(COUNTER),
            Some(LocalizedText::new("files").arg("count", 1u32)),
        );
        app.context.commit_mutations(queue).unwrap();
        app.frame(0);
        app
    }

    fn say(app: &mut App, id: StableNodeId, text: LocalizedText) {
        let mut queue = MutationQueue::new();
        queue.set_localized_text(id, Some(text));
        app.context.commit_mutations(queue).unwrap();
    }

    #[test]
    fn issue270_gate_a_a_changing_count_formats_one_message() {
        let mut app = counting(10_000, true);
        assert_eq!(app.text(node(COUNTER)), "1 file");
        for count in 2..=60u32 {
            let update = cost(&mut app, |app| {
                say(
                    app,
                    node(COUNTER),
                    LocalizedText::new("files").arg("count", count),
                );
            });
            let frame: WorkCounters = app.frame(0);
            assert_eq!(update.format_requests, 1, "{count}: {update:?}");
            assert_eq!(update.nodes_resolved, 1);
            assert_eq!(update.args_revisions, 1);
            assert_eq!(update.catalog_lookups, 1);
            assert_eq!(update.catalog_cache_misses, 0, "no catalog traversal");
            assert_eq!(update.message_patterns_compiled, 0);
            assert_eq!(update.formatter_allocations, 0);
            assert_eq!(update.formatted_output_changed, 1);
            assert_eq!(frame.text_shaped, 1, "{count}: only the counter shapes");
            assert_eq!(app.text(node(COUNTER)), format!("{count} files"));
        }
    }

    #[test]
    fn issue270_gate_b_an_absorbed_argument_change_does_no_text_work() {
        let mut app = counting(10_000, true);
        say(
            &mut app,
            node(COUNTER),
            LocalizedText::new("rounded").arg("n", 1.2),
        );
        app.frame(0);
        assert_eq!(app.text(node(COUNTER)), "1 items");
        let passes = app.context.layout_invocations();
        let update = cost(&mut app, |app| {
            say(
                app,
                node(COUNTER),
                LocalizedText::new("rounded").arg("n", 1.4),
            );
        });
        assert_eq!(update.args_revisions, 1);
        assert_eq!(update.formatted_output_unchanged, 1, "{update:?}");
        assert_eq!(update.formatted_output_changed, 0);
        // The string held: nothing is left for the frame to shape or lay out.
        assert!(app.context.take_system_work().is_empty());
        app.frame(0);
        assert_eq!(app.context.layout_invocations(), passes, "nothing lays out");
    }

    #[test]
    fn issue270_gate_c_one_locale_one_pattern_one_formatter() {
        let mut app = counting(1_000, true);
        let document = app.windows[0].document;
        let mut queue = MutationQueue::new();
        let numbers: Vec<StableNodeId> = (0..10_000u64).map(|at| node(9_600_000 + at)).collect();
        for (at, id) in numbers.iter().enumerate() {
            queue.create(*id, document, NodeKind::Text);
            queue.insert(section(1_000_000, 1), *id, None);
            queue.set_localized_text(
                *id,
                Some(LocalizedText::new("amount").arg("n", 1_000.5 + at as f64)),
            );
        }
        let created = cost(&mut app, |app| {
            app.context.commit_mutations(queue).unwrap();
        });
        assert_eq!(created.format_requests, 10_000);
        assert_eq!(created.message_patterns_compiled, 1, "{created:?}");
        assert!(created.formatter_allocations <= 1, "{created:?}");
        // German formats them all again: one more formatter, the pattern kept
        // per locale asked.
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Locale::parse("de"));
        });
        let hits = switch.format_cache_hits as f64;
        let asked = (switch.format_cache_hits + switch.format_cache_misses) as f64;
        assert!(hits / asked >= 0.99, "{switch:?}");
        assert!(switch.formatter_allocations <= 1, "{switch:?}");
        assert_eq!(app.text(numbers[0]), "1.000,5");
    }

    #[test]
    fn issue270_gate_d_a_catalog_update_reaches_the_nodes_saying_its_message() {
        let mut app = App::new(&[100_000], false);
        let window = &app.windows[0];
        let saying: Vec<StableNodeId> = window
            .localized
            .iter()
            .zip(&window.messages)
            .filter(|(_, message)| **message == 7)
            .map(|(id, _)| *id)
            .collect();
        assert!((90..=110).contains(&saying.len()), "{}", saying.len());
        let updated = Arc::new((*catalog()).clone().with(
            "en-us",
            "item.7",
            "Updated item of {count}",
        ));
        let update = cost(&mut app, |app| {
            app.context
                .compat_world_mut()
                .update_message_catalog(updated, &[MessageId::new("item.7")]);
        });
        assert_eq!(update.catalog_messages_invalidated, saying.len());
        assert_eq!(update.nodes_resolved, saying.len(), "{update:?}");
        assert_eq!(update.formatted_output_changed, saying.len());
        for id in &saying {
            assert_eq!(app.text(*id), "Updated item of 3");
        }
        let other = app.windows[0]
            .localized
            .iter()
            .zip(&app.windows[0].messages)
            .find(|(_, message)| **message == 8)
            .map(|(id, _)| *id)
            .unwrap();
        assert_eq!(app.text(other), "Item 8 of 3");
    }

    #[test]
    fn issue270_gate_e_a_locale_switch_compiles_each_message_once() {
        let mut app = App::new(&[100_000], false);
        let localized = app.windows[0].localized.len();
        let switch = cost(&mut app, |app| {
            app.context.set_default_locale(Locale::parse("zh-cn"));
        });
        assert_eq!(switch.format_requests, localized);
        assert_eq!(
            switch.message_patterns_compiled, MESSAGES as usize,
            "{switch:?}"
        );
        assert!(switch.formatter_allocations <= 1, "{switch:?}");
    }

    #[test]
    fn issue270_literal_text_written_over_a_localized_node_stays() {
        let mut app = counting(1_000, true);
        let mut queue = MutationQueue::new();
        queue.set_text(
            node(COUNTER),
            TextContent {
                value: "written".into(),
            },
        );
        app.context.commit_mutations(queue).unwrap();
        assert_eq!(app.context.world().localized_text(node(COUNTER)), None);
        app.context.set_default_locale(Locale::parse("zh-cn"));
        assert_eq!(app.text(node(COUNTER)), "written");
    }
}
