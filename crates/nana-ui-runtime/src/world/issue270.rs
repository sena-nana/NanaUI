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

#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::WorkCounters;

use super::issue268::{App, MESSAGES, catalog, cost, section};
use super::reflow_oracle::node;
use super::{NodeKind, StableNodeId};
use crate::{Locale, LocalizedText, MessageId, MessageTable, MutationQueue, TextContent};

/// Issue #268's catalog with the counter's messages.
pub(super) fn counting_catalog() -> Arc<MessageTable> {
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
    let updated = Arc::new(
        (*catalog())
            .clone()
            .with("en-us", "item.7", "Updated item of {count}"),
    );
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
