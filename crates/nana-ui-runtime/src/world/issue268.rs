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

#![cfg(test)]

use std::sync::Arc;

use nana_text::font::LanguageTag;
use nana_ui_core::{DirSpec, FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

use super::reflow_oracle::{
    bundled_face_shaper, node, product_frame, product_frame_seeded, styled,
};
use super::{DocumentId, NodeKind, StableNodeId};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{
    AppContext, LayoutViewport, Locale, LocaleScope, LocalizedText, MessageTable, MutationQueue,
    NanaTextEngineShaper, TextContent,
};

/// Messages the windows say: `item.0` .. `item.49`.
pub(super) const MESSAGES: u64 = 50;

pub(super) fn tag(tag: &str) -> LanguageTag {
    LanguageTag::new(tag).unwrap()
}

/// English, British English and Chinese for every message; the British
/// strings of the first twenty messages equal the American ones.
pub(super) fn catalog() -> Arc<MessageTable> {
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
pub(super) struct Window {
    pub(super) document: DocumentId,
    pub(super) localized: Vec<StableNodeId>,
    /// The message each localized node says: `item.{n}`.
    pub(super) messages: Vec<u64>,
    pub(super) literal: Vec<StableNodeId>,
}

impl Window {
    /// Localized nodes whose British English reads as the American: the
    /// first twenty messages.
    pub(super) fn same_in_british(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| **message < 20)
            .count()
    }
}

/// Sections of a window: section `index` of the window whose ids start at
/// `base`.
pub(super) fn section(base: u64, index: u64) -> StableNodeId {
    node(base + 2 + index)
}

const SECTIONS: u64 = 10;

/// A window of about `nodes` nodes whose ids start at `base`, with at most
/// `cap` localized nodes and boxes for the rest.
pub(super) fn window_with(
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
                        LocalizedText::new(&format!("item.{}", localized.len() as u64 % MESSAGES))
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
pub(super) fn labelled_window(
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
pub(super) struct App {
    pub(super) context: AppContext,
    pub(super) windows: Vec<Window>,
    shaper: NanaTextEngineShaper,
    guarded: bool,
}

impl App {
    pub(super) fn new(sizes: &[u64], guarded: bool) -> Self {
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
    pub(super) fn labelled(nodes: u64, labels: usize, guarded: bool) -> Self {
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
    pub(super) fn frame(&mut self, at: usize) -> WorkCounters {
        let _unguarded = (!self.guarded).then(skip_layout_verify);
        let document = self.windows[at].document;
        product_frame(&mut self.context, document, viewport(), &mut self.shaper)
    }

    /// [`Self::frame`], and the nodes its layout passes were seeded at.
    pub(super) fn frame_seeded(
        &mut self,
        at: usize,
    ) -> (WorkCounters, std::collections::HashSet<StableNodeId>) {
        let _unguarded = (!self.guarded).then(skip_layout_verify);
        let document = self.windows[at].document;
        product_frame_seeded(&mut self.context, document, viewport(), &mut self.shaper)
    }

    pub(super) fn text(&self, id: StableNodeId) -> String {
        self.context
            .world()
            .text(id)
            .unwrap_or_default()
            .to_string()
    }

    /// The localization counters a change left for the next drain.
    pub(super) fn pending(&self) -> nana_ui_core::I18nCounters {
        self.context.last_work_counters().i18n
    }
}

/// What every work counter did from `before` to `after`. The index sizes,
/// which every snapshot carries, stay zero.
pub(super) fn difference(
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
        resolved_content_changed: after.resolved_content_changed - before.resolved_content_changed,
        resolved_content_unchanged: after.resolved_content_unchanged
            - before.resolved_content_unchanged,
        language_changed_nodes: after.language_changed_nodes - before.language_changed_nodes,
        direction_changed_scopes: after.direction_changed_scopes - before.direction_changed_scopes,
        layout_seeds: after.layout_seeds - before.layout_seeds,
        virtual_rows_resolved: after.virtual_rows_resolved - before.virtual_rows_resolved,
        switch_commits: after.switch_commits - before.switch_commits,
        format_requests: after.format_requests - before.format_requests,
        format_cache_hits: after.format_cache_hits - before.format_cache_hits,
        format_cache_misses: after.format_cache_misses - before.format_cache_misses,
        message_patterns_compiled: after.message_patterns_compiled
            - before.message_patterns_compiled,
        args_revisions: after.args_revisions - before.args_revisions,
        formatted_output_changed: after.formatted_output_changed - before.formatted_output_changed,
        formatted_output_unchanged: after.formatted_output_unchanged
            - before.formatted_output_unchanged,
        catalog_messages_invalidated: after.catalog_messages_invalidated
            - before.catalog_messages_invalidated,
        formatter_allocations: after.formatter_allocations - before.formatter_allocations,
        ..nana_ui_core::I18nCounters::default()
    }
}

/// What `change` cost localization, read before the next drain.
pub(super) fn cost(app: &mut App, change: impl FnOnce(&mut App)) -> nana_ui_core::I18nCounters {
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
