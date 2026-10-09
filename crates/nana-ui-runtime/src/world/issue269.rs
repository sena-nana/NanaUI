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

#![cfg(test)]

use nana_ui_core::{
    DirSpec, FlexDirection, I18nCounters, LayoutStyle, LengthSpec, VirtualListLayout,
};

use super::issue262::{port, viewport};
use super::issue268::{App, MESSAGES, cost, difference, section, tag};
use super::reflow_oracle::{bundled_face_shaper, node, product_frame, styled};
use super::{DocumentId, NodeKind, StableNodeId};
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
    context.set_message_catalog(Some(super::issue268::catalog()));
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
pub(super) struct LocalizedFeed {
    pub(super) context: AppContext,
    shaper: crate::NanaTextEngineShaper,
    document: DocumentId,
    pub(super) items: VirtualListItems<u64, Text>,
}

impl LocalizedFeed {
    pub(super) fn new(rows: usize) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        context.set_message_catalog(Some(super::issue268::catalog()));
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
    pub(super) fn switch(
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
