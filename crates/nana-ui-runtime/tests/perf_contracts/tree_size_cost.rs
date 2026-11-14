//! What one node costs must not depend on how many others there are.
//!
//! Two paths used to: removing a node walked every per-component table of
//! the framework (2.4 ms to remove one button among 100,000), and keying a
//! new child under an assembled parent copied that parent's whole key table
//! (6 s to assemble 16,000 children). Both are measured here against the
//! size of the tree rather than against a budget, so the checks hold on any
//! machine.

use crate::ALLOCATED_GROWTH as ALLOCATED;
use std::cell::Cell;
use std::time::Instant;

use nana_ui_runtime::{AppContext, Button, DocumentId, Entity, List, Text};

/// Bytes allocated per child while a parent assembles `children` keyed
/// children for the first time.
fn assembly_bytes_per_child(children: usize) -> f64 {
    let document = DocumentId::new(1).unwrap();
    let mut context = AppContext::new();
    let parent = context.create_component(document, List::new()).unwrap();
    let before = ALLOCATED.with(Cell::get);
    context
        .mount(parent, |scope| {
            for index in 0..children {
                scope.child(format!("row-{index}"), Text::new("row"))?;
            }
            Ok(())
        })
        .unwrap();
    (ALLOCATED.with(Cell::get) - before) as f64 / children as f64
}

#[test]
fn a_keyed_child_costs_the_same_under_a_small_parent_and_a_large_one() {
    let small = assembly_bytes_per_child(200);
    let large = assembly_bytes_per_child(3_200);
    // Sixteen times the children. A table copied per child allocates sixteen
    // times as much for each; tables that only grow allocate about the same.
    assert!(
        large < small * 3.0,
        "a child of 3,200 allocates {large:.0} bytes, a child of 200 allocates {small:.0}"
    );
}

/// World commits an assembly makes when it declares its children again
/// without `dropped` of them.
fn commits_to_drop(dropped: usize) -> u64 {
    const CHILDREN: usize = 120;
    let document = DocumentId::new(1).unwrap();
    let mut context = AppContext::new();
    let parent = context.create_component(document, List::new()).unwrap();
    let assemble = |context: &mut AppContext, from: usize| {
        context
            .mount(parent, |scope| {
                for index in from..CHILDREN {
                    scope.child(format!("row-{index}"), Text::new("row"))?;
                }
                Ok(())
            })
            .unwrap();
    };
    assemble(&mut context, 0);
    let before = context.world().generation();
    assemble(&mut context, dropped);
    assert_eq!(
        context
            .world()
            .node(parent.stable_id())
            .unwrap()
            .children
            .len(),
        CHILDREN - dropped
    );
    context.world().generation() - before
}

#[test]
fn an_assembly_drops_its_children_in_one_commit_however_many() {
    let one = commits_to_drop(1);
    let many = commits_to_drop(60);
    // A commit per dropped child settled the parent's remaining children
    // once for each of them.
    assert_eq!(
        many, one,
        "dropping 60 children took {many} commits, dropping one took {one}"
    );
}

/// The least time, in nanoseconds, removing one button takes in a tree of
/// `nodes` of them. The last child is the one removed, so its siblings do
/// not move; the least of many removals, so a busy machine does not count.
fn removal_nanos(nodes: usize) -> u128 {
    let document = DocumentId::new(1).unwrap();
    let mut context = AppContext::new();
    let root = context.create_component(document, List::new()).unwrap();
    let mut buttons: Vec<Entity<Button>> = (0..nodes)
        .map(|index| {
            let button = context
                .create_component(document, Button::new(format!("row {index}")))
                .unwrap();
            context.append_child(root, button).unwrap();
            button
        })
        .collect();
    (0..200)
        .map(|_| {
            let button = buttons.pop().unwrap();
            let started = Instant::now();
            context.remove_view(button).unwrap();
            started.elapsed().as_nanos()
        })
        .min()
        .unwrap()
}

#[test]
fn removing_a_node_costs_the_same_in_a_small_tree_and_a_large_one() {
    let small = removal_nanos(1_000);
    let large = removal_nanos(32_000);
    // Thirty-two times the nodes. Walking a table of them all made the
    // removal thirty-two times slower; looked up, it only meets colder
    // memory.
    assert!(
        large < small * 8,
        "removing one of 32,000 takes {large} ns, one of 1,000 takes {small} ns"
    );
}
