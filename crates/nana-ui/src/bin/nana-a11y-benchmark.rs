//! Synthetic semantic projection diagnostic; excludes Runtime layout and native UIA.
use nana_ui::AccessTreeProjector;
use nana_ui_runtime::{
    AccessibilityDelta, AccessibilityNode, AccessibilityRole, LayoutBox, StableNodeId,
};
use serde_json::json;
use std::{hint::black_box, time::Instant};

fn node(value: u64) -> AccessibilityNode {
    AccessibilityNode {
        id: StableNodeId::new(value).unwrap(),
        parent: (value != 1).then(|| StableNodeId::new(1).unwrap()),
        children: Vec::new(),
        role: AccessibilityRole::Generic,
        label: None,
        value: None,
        description: None,
        role_description: None,
        disabled: false,
        display_only: false,
        inert: false,
        checked: None,
        mixed: false,
        orientation: None,
        selected: None,
        level: None,
        expanded: None,
        active_descendant: None,
        position_in_set: None,
        set_size: None,
        multiselectable: false,
        has_popup: false,
        multiline: false,
        editable: false,
        selection: None,
        modal: false,
        busy: false,
        invalid: false,
        live: None,
        numeric_minimum: None,
        numeric_maximum: None,
        numeric_step: None,
        numeric_value: None,
        scroll_x: None,
        scroll_y: None,
        focused: false,
        bounds: LayoutBox::default(),
        transform: None,
        scroll: [0.0, 0.0],
        clip: None,
        writing: Default::default(),
    }
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

fn branched_tree(retained: u64) -> (Vec<AccessibilityNode>, u64, Vec<StableNodeId>) {
    const GROUP_SIZE: u64 = 100;
    let group_count = (retained / GROUP_SIZE).max(1);
    let mut nodes = Vec::with_capacity(retained as usize);
    let mut root = node(1);
    root.parent = None;
    root.children = (2..=1 + group_count)
        .map(|id| StableNodeId::new(id).unwrap())
        .collect();
    nodes.push(root);

    let first_leaf = 2 + group_count;
    let total_leaves = retained.saturating_sub(1 + group_count);
    let base_per_group = total_leaves / group_count;
    let extra = total_leaves % group_count;
    let mut next_leaf = first_leaf;
    let mut last_group_children = Vec::new();
    for group_idx in 0..group_count {
        let group_id = 2 + group_idx;
        let leaf_count = base_per_group + u64::from(group_idx < extra);
        let mut group = node(group_id);
        group.parent = Some(StableNodeId::new(1).unwrap());
        let children = (next_leaf..next_leaf + leaf_count)
            .map(|id| StableNodeId::new(id).unwrap())
            .collect::<Vec<_>>();
        next_leaf += leaf_count;
        if group_idx + 1 == group_count {
            last_group_children = children.clone();
        }
        group.children = children;
        nodes.push(group);
    }

    let mut leaf_cursor = first_leaf;
    for group_idx in 0..group_count {
        let group_id = StableNodeId::new(2 + group_idx).unwrap();
        let leaf_count = base_per_group + u64::from(group_idx < extra);
        for _ in 0..leaf_count {
            let mut leaf = node(leaf_cursor);
            leaf.parent = Some(group_id);
            if leaf_cursor == first_leaf {
                leaf.role = AccessibilityRole::TextInput;
                leaf.editable = true;
                leaf.value = Some("Sample".into());
            }
            nodes.push(leaf);
            leaf_cursor += 1;
        }
    }
    (nodes, 1 + group_count, last_group_children)
}

fn main() {
    const WARMUP: usize = 100;
    let samples = std::env::args()
        .nth(1)
        .map_or(2_000, |value| value.parse::<usize>().expect("sample count"));
    assert!(samples > 0);
    let mut rows = Vec::new();
    let mut structural_rows = Vec::new();
    let structural_warmup = 5.min(WARMUP);
    let structural_samples = samples.min(50);
    for retained in [10_000_u64, 50_000, 100_000] {
        let mut nodes = (1..=retained).map(node).collect::<Vec<_>>();
        nodes[0].children = (2..=retained)
            .map(|id| StableNodeId::new(id).unwrap())
            .collect();
        // Updating the last sibling detects accidental linear parent-child searches.
        let editor = nodes.last_mut().unwrap();
        editor.role = AccessibilityRole::TextInput;
        editor.editable = true;
        let mut editor = editor.clone();
        let mut projector = AccessTreeProjector::new(nodes, true, 1.0);
        let mut times = Vec::with_capacity(samples);
        for iteration in 0..WARMUP + samples {
            editor.value = Some(if iteration % 2 == 0 { "Even" } else { "Odd" }.into());
            editor.focused = iteration % 2 == 0;
            let delta = AccessibilityDelta {
                generation: iteration as u64 + 1,
                updated: vec![editor.clone()],
                removed: Vec::new(),
            };
            let start = Instant::now();
            let update = projector.apply_delta(delta).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1_000_000.0;
            assert_eq!(update.nodes.len(), 2);
            assert!(update.tree.is_none());
            black_box(&update);
            if iteration >= WARMUP {
                times.push(elapsed);
            }
        }
        times.sort_by(f64::total_cmp);
        rows.push(json!({
            "retained_semantic_nodes": retained, "emitted_nodes_per_delta": 2,
            "p50_us": percentile(&times, 50), "p95_us": percentile(&times, 95), "p99_us": percentile(&times, 99),
        }));

        let (branched, group_id, base_children) = branched_tree(retained);
        let mut structural_projector = AccessTreeProjector::new(branched, true, 1.0);
        let full_start = Instant::now();
        let full = structural_projector.full_update();
        let full_update_us = full_start.elapsed().as_secs_f64() * 1_000_000.0;
        assert_eq!(full.nodes.len(), retained as usize + 1);
        black_box(&full);

        let mut insert_times = Vec::with_capacity(structural_samples);
        let mut remove_times = Vec::with_capacity(structural_samples);
        let mut generation = 1u64;
        for iteration in 0..structural_warmup + structural_samples {
            let new_id = retained + 10 + iteration as u64;
            let new_stable = StableNodeId::new(new_id).unwrap();
            let mut inserted = node(new_id);
            inserted.parent = Some(StableNodeId::new(group_id).unwrap());
            inserted.role = AccessibilityRole::Button;
            inserted.label = Some("Item".into());

            let mut parent_with_child = node(group_id);
            parent_with_child.parent = Some(StableNodeId::new(1).unwrap());
            parent_with_child.children = base_children.clone();
            parent_with_child.children.push(new_stable);

            let insert_delta = AccessibilityDelta {
                generation,
                updated: vec![parent_with_child, inserted],
                removed: Vec::new(),
            };
            generation += 1;
            let start = Instant::now();
            let update = structural_projector.apply_delta(insert_delta).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1_000_000.0;
            assert_eq!(update.nodes.len(), 2);
            assert!(update.tree.is_none());
            black_box(&update);
            if iteration >= structural_warmup {
                insert_times.push(elapsed);
            }

            let remove_delta = AccessibilityDelta {
                generation,
                updated: Vec::new(),
                removed: vec![new_stable],
            };
            generation += 1;
            let start = Instant::now();
            let update = structural_projector.apply_delta(remove_delta).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1_000_000.0;
            assert_eq!(update.nodes.len(), 1);
            assert!(update.tree.is_none());
            black_box(&update);
            if iteration >= structural_warmup {
                remove_times.push(elapsed);
            }
        }
        insert_times.sort_by(f64::total_cmp);
        remove_times.sort_by(f64::total_cmp);
        structural_rows.push(json!({
            "retained_semantic_nodes": retained,
            "full_update_us": full_update_us,
            "insert_p50_us": percentile(&insert_times, 50),
            "insert_p95_us": percentile(&insert_times, 95),
            "insert_p99_us": percentile(&insert_times, 99),
            "remove_p50_us": percentile(&remove_times, 50),
            "remove_p95_us": percentile(&remove_times, 95),
            "remove_p99_us": percentile(&remove_times, 99),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "scope": "AccessKit semantic delta only; not a frame or presentation benchmark",
            "warmup": WARMUP,
            "samples_per_size": samples,
            "structural_warmup": structural_warmup,
            "structural_samples_per_size": structural_samples,
            "rows": rows,
            "structural_rows": structural_rows,
        }))
        .unwrap()
    );
}
