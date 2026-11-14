//! Application models that nest are walked without running out of stack:
//! a panel layout as deep as `MAX_SPLIT_DEPTH` allows, on a 512 KiB thread
//! in any build, and one an application built deeper itself, which the
//! framework folds to that depth as it takes it in. (A tree view's data has
//! no depth bound at all; its walks are tested in `tree_view`.) See
//! `docs/reference/resource-budgets.md`.

use nana_ui_core::SplitAxis;
use nana_ui_runtime::{
    AppContext, Area, AreaAxis, AreaLayout, AreaLayoutModel, AreaNode, Dock, DockAxis, DockNode,
    DockWorkspace, DocumentId, LayoutBox, LayoutViewport, MAX_SPLIT_DEPTH, PaneTree, PaneTreeNode,
};

/// Far deeper than `MAX_SPLIT_DEPTH`: an application's own tree, built in
/// a loop. Unfolded, the framework's walks over it run out of stack.
const APPLICATION_DEPTH: usize = 5_000;

fn deep_dock(depth: usize) -> DockNode {
    let mut node = DockNode::item("leaf", None);
    for n in 0..depth {
        node = DockNode::split(
            DockAxis::Horizontal,
            0.5,
            DockNode::item(format!("beside-{n}"), None),
            node,
        );
    }
    node
}

fn deep_panes(depth: usize) -> PaneTreeNode {
    let mut node = PaneTreeNode::leaf("leaf");
    for n in 0..depth {
        node = PaneTreeNode::split(
            format!("split-{n}"),
            SplitAxis::Horizontal,
            0.5,
            PaneTreeNode::leaf(format!("beside-{n}")),
            node,
        );
    }
    node
}

/// An area layout `depth` splits deep, written as the struct it is.
fn deep_areas(depth: usize) -> AreaLayoutModel {
    let mut root = AreaNode::Area { id: "leaf".into() };
    let mut areas = vec![Area {
        id: "leaf".into(),
        editor: "editor".into(),
        state: None,
    }];
    for n in 0..depth {
        let id = format!("beside-{n}");
        areas.push(Area {
            id: id.clone(),
            editor: "editor".into(),
            state: None,
        });
        root = AreaNode::Split {
            axis: AreaAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(AreaNode::Area { id }),
            second: Box::new(root),
        };
    }
    AreaLayoutModel {
        root,
        areas,
        maximized: Some("leaf".into()),
    }
}

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn on_a_small_stack(run: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(run)
        .unwrap()
        .join()
        .expect("a walk of the model ran out of stack");
}

#[test]
fn a_dock_as_deep_as_allowed_clones_compares_persists_and_drops() {
    on_a_small_stack(|| {
        let mut node = DockNode::item("leaf", None);
        for n in 0..MAX_SPLIT_DEPTH {
            node = DockNode::split(
                DockAxis::Horizontal,
                0.5,
                DockNode::item(format!("beside-{n}"), None),
                node,
            );
        }
        assert_eq!(node.split_depth(), MAX_SPLIT_DEPTH);
        let copy = node.clone();
        assert!(copy == node);
        assert_eq!(node.flatten().len(), MAX_SPLIT_DEPTH + 1);
        let workspace = DockWorkspace::new(node);
        let json = workspace.layout_json().unwrap();
        let restored = DockWorkspace::from_layout_json(&json).unwrap();
        assert_eq!(restored.main.split_depth(), MAX_SPLIT_DEPTH);
        drop((workspace, restored, copy));
    });
}

#[test]
fn an_area_layout_as_deep_as_allowed_places_persists_and_drops() {
    on_a_small_stack(|| {
        let mut model = AreaLayoutModel::single("a0", "editor");
        let mut last = String::from("a0");
        for n in 1..=MAX_SPLIT_DEPTH {
            let next = format!("a{n}");
            assert!(model.split(&last, AreaAxis::Horizontal, 0.5, next.as_str()));
            last = next;
        }
        assert_eq!(model.leaves().len(), MAX_SPLIT_DEPTH + 1);
        let bounds = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 600.0,
        };
        let _ = model.geometry(bounds, 2.0, 0.0);
        let json = serde_json::to_string(&model).unwrap();
        let restored: AreaLayoutModel = serde_json::from_str(&json).unwrap();
        assert!(restored == model);
        drop((model, restored));
    });
}

#[test]
fn a_pane_tree_as_deep_as_allowed_visits_clones_and_drops() {
    on_a_small_stack(|| {
        let mut node = PaneTreeNode::leaf("leaf");
        for n in 0..MAX_SPLIT_DEPTH {
            node = PaneTreeNode::split(
                format!("split-{n}"),
                SplitAxis::Horizontal,
                0.5,
                PaneTreeNode::leaf(format!("beside-{n}")),
                node,
            );
        }
        assert_eq!(node.split_depth(), MAX_SPLIT_DEPTH);
        let mut leaves = 0;
        node.visit_leaves(|_| leaves += 1);
        assert_eq!(leaves, MAX_SPLIT_DEPTH + 1);
        let copy = node.clone();
        assert!(copy == node);
        drop((node, copy));
    });
}

#[test]
fn the_deepest_dock_layout_json_can_hold_still_restores() {
    // Hand-edited or foreign, a saved layout can nest past MAX_SPLIT_DEPTH,
    // up to what `serde_json` reads; restoring it must not run out of stack.
    on_a_small_stack(|| {
        let deepest = (MAX_SPLIT_DEPTH..)
            .take_while(|&depth| DockWorkspace::from_layout_json(&dock_json(depth)).is_ok())
            .last()
            .expect("a layout at the split depth restores");
        assert!(deepest > MAX_SPLIT_DEPTH);
        // It restores folded to the split depth, every item kept.
        let restored = DockWorkspace::from_layout_json(&dock_json(deepest)).unwrap();
        assert_eq!(restored.main.split_depth(), MAX_SPLIT_DEPTH);
        assert_eq!(restored.main.flatten().len(), deepest + 1);
    });
}

/// A saved dock layout `depth` splits deep, written out by hand.
fn dock_json(depth: usize) -> String {
    let mut node = String::from(r#"{"kind":"item","id":"leaf"}"#);
    for n in 0..depth {
        node = format!(
            r#"{{"kind":"split","axis":"horizontal","ratio":0.5,"first":{{"kind":"item","id":"beside-{n}"}},"second":{node}}}"#
        );
    }
    format!(r#"{{"version":1,"main":{node}}}"#)
}

#[test]
fn a_dock_an_application_built_too_deep_is_folded_into_tabs_and_shown() {
    on_a_small_stack(|| {
        // Through the constructor: every item survives, in one tab group.
        let dock = Dock::new(deep_dock(APPLICATION_DEPTH));
        assert_eq!(dock.root.split_depth(), MAX_SPLIT_DEPTH);
        let items = dock.root.flatten();
        assert_eq!(items.len(), APPLICATION_DEPTH + 1);
        assert_eq!(items.first().map(|id| id.as_ref()), Some("beside-4999"));
        assert_eq!(items.last().map(|id| id.as_ref()), Some("leaf"));

        // Past the constructor, written into the public field, then created
        // and laid out; and written again in an update.
        let mut cx = AppContext::new();
        let mut written = Dock::new(DockNode::item("only", None));
        written.root = deep_dock(APPLICATION_DEPTH);
        let entity = cx.create_component(document(), written).unwrap();
        cx.layout_document(document(), LayoutViewport::new(1200.0, 800.0))
            .unwrap();
        cx.update_component(entity, |dock, _| {
            dock.root = deep_dock(APPLICATION_DEPTH);
        })
        .unwrap();
        cx.layout_document(document(), LayoutViewport::new(1200.0, 800.0))
            .unwrap();
        let workspace = DockWorkspace::new(deep_dock(APPLICATION_DEPTH));
        assert_eq!(workspace.main.split_depth(), MAX_SPLIT_DEPTH);
        drop((dock, workspace, cx));
    });
}

#[test]
fn a_pane_tree_an_application_built_too_deep_keeps_its_first_panes() {
    on_a_small_stack(|| {
        let tree = PaneTree::new(deep_panes(APPLICATION_DEPTH));
        assert_eq!(tree.root.split_depth(), MAX_SPLIT_DEPTH);
        let mut cx = AppContext::new();
        let mut written = PaneTree::new(PaneTreeNode::leaf("only"));
        written.root = deep_panes(APPLICATION_DEPTH);
        let entity = cx.create_component(document(), written).unwrap();
        cx.layout_document(document(), LayoutViewport::new(1200.0, 800.0))
            .unwrap();
        cx.update_component(entity, |tree, _| tree.root = deep_panes(APPLICATION_DEPTH))
            .unwrap();
        drop((tree, cx));
    });
}

#[test]
fn an_area_layout_an_application_built_too_deep_keeps_its_first_areas() {
    on_a_small_stack(|| {
        let mut model = deep_areas(APPLICATION_DEPTH);
        assert!(model.fold_past_split_depth());
        assert_eq!(model.split_depth(), MAX_SPLIT_DEPTH);
        // Every leaf left is an area, and only those: the layout holds.
        assert_eq!(model.leaves().len(), model.areas.len());
        assert_eq!(model.leaves().len(), MAX_SPLIT_DEPTH + 1);
        // The maximized area was folded away.
        assert_eq!(model.maximized, None);
        let mut cx = AppContext::new();
        let mut written = AreaLayout::new(AreaLayoutModel::single("only", "editor"));
        written.model = deep_areas(APPLICATION_DEPTH);
        cx.create_component(document(), written).unwrap();
        cx.layout_document(document(), LayoutViewport::new(1200.0, 800.0))
            .unwrap();
        drop((model, cx));
    });
}
