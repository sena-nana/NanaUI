#[cfg(feature = "charts")]
use nana_ui_runtime::TimeSeriesChart;
use std::sync::Arc;

use nana_ui_runtime::{
    ComputedStyle, CustomRenderNode, LayoutBox, NodeKind, NodeStyle, TextContent, TextMatchMarker,
};

use super::*;

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn primitive_icon(kind: &ScenePrimitiveKind) -> Option<nana_ui_core::Icon> {
    match kind {
        ScenePrimitiveKind::Icon { icon, .. } => Some(*icon),
        _ => None,
    }
}

fn style_mut(node: &mut ExtractedNode) -> &mut ComputedStyle {
    Arc::make_mut(&mut node.style)
}

fn node(value: u64, parent: Option<u64>, children: &[u64]) -> ExtractedNode {
    ExtractedNode {
        id: id(value),
        kind: Arc::new(NodeKind::Element { tag: "div".into() }),
        parent: parent.map(id),
        children: Arc::new(children.iter().copied().map(id).collect()),
        layout: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        },
        scroll_offset: nana_ui_runtime::ScrollOffset::default(),
        source_style: NodeStyle::default(),
        style: Arc::new(ComputedStyle::default()),
        text: None,
        text_metrics: None,
        z_index: 0,
        focused: false,
        ime: None,
        text_input: None,
        text_spans: Vec::new(),
        standard_visual: None,
        component_geometry: None,
        standard_visual_foreground: None,
        custom_render: None,
    }
}

#[test]
fn hidden_nodes_skip_scene_primitives() {
    let mut hidden = node(1, None, &[]);
    style_mut(&mut hidden).visible = false;
    style_mut(&mut hidden).background = Some([1.0, 0.0, 0.0, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([hidden], []);
    assert!(scene.primitives().all(|primitive| primitive.node != id(1)));
}

#[test]
fn workspace_resize_handle_is_not_clipped_by_its_region() {
    let mut region = node(1, None, &[2]);
    region.layout = LayoutBox {
        x: 200.0,
        y: 0.0,
        width: 180.0,
        height: 400.0,
    };
    let layout = Arc::make_mut(&mut region.source_style.layout);
    layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
    layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;

    let mut handle = node(2, Some(1), &[]);
    handle.kind = Arc::new(NodeKind::Element {
        tag: "workspace-resize-handle".into(),
    });
    handle.layout = LayoutBox {
        x: 196.0,
        y: 0.0,
        width: 8.0,
        height: 400.0,
    };
    style_mut(&mut handle).background = Some([0.5, 0.5, 0.5, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([region, handle], []);
    let painted = scene
        .primitives()
        .find(|primitive| primitive.node == id(2))
        .expect("handle quad");
    assert_eq!(painted.bounds.width, 8.0);
    assert!(
        painted.clips.is_empty(),
        "region overflow must not clip the overlay bar, got {:?}",
        painted.clips
    );
}

#[test]
fn apply_delta_refreshes_instance_on_node_changes_not_idle() {
    let mut scene = UiScene::new();
    let created = scene.instance_id();
    scene.apply_delta([], []);
    assert_eq!(
        scene.instance_id(),
        created,
        "empty apply_delta must keep the instance so idle paint caches hit"
    );

    scene.apply_delta([node(1, None, &[])], []);
    let after_insert = scene.instance_id();
    assert_ne!(
        after_insert, created,
        "inserting a node in place must refresh the instance"
    );

    let mut updated = node(1, None, &[]);
    updated.layout.width = 40.0;
    scene.apply_delta([updated], []);
    let after_update = scene.instance_id();
    assert_ne!(
        after_update, after_insert,
        "updating a node in place must refresh the instance"
    );

    scene.apply_delta([], []);
    assert_eq!(
        scene.instance_id(),
        after_update,
        "empty apply_delta after a real delta must keep the instance"
    );

    let cloned = scene.clone();
    assert_ne!(
        cloned.instance_id(),
        scene.instance_id(),
        "Clone still gets a distinct instance"
    );
}

#[test]
fn extracted_text_spans_travel_on_the_text_primitive() {
    let mut labeled = node(1, None, &[]);
    labeled.text = Some(TextContent {
        value: "fn main".into(),
    });
    labeled.text_spans = vec![nana_ui_runtime::ExtractedTextSpan {
        start: 0,
        end: 2,
        color: [0.2, 0.6, 1.0, 1.0],
    }];
    let mut scene = UiScene::new();
    scene.apply_delta([labeled], []);
    let Some(ScenePrimitiveKind::Text {
        ref content,
        ref spans,
        ..
    }) = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .map(|primitive| primitive.kind.clone())
    else {
        panic!("expected a text primitive");
    };
    assert_eq!(content, "fn main");
    assert_eq!(
        spans,
        &vec![SceneTextSpan {
            start: 0,
            end: 2,
            color: [0.2, 0.6, 1.0, 1.0],
        }]
    );
}

#[test]
fn generic_text_em_padding_uses_computed_font_size() {
    let mut labeled = node(1, None, &[]);
    labeled.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 80.0,
    };
    labeled.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            font_size: Some(32.0),
            padding: Some(nana_ui_core::LengthSpec::Em(1.0)),
            ..Default::default()
        }),
        ..Default::default()
    };
    labeled.text = Some(TextContent {
        value: "hello".into(),
    });
    let mut scene = UiScene::new();
    scene.apply_delta([labeled], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .expect("generic text");
    assert_eq!(
        primitive.bounds,
        SceneRect {
            x: 32.0,
            y: 32.0,
            width: 136.0,
            height: 16.0,
        },
        "1em padding at font-size 32px must inset text 32px, not 16px"
    );
}

#[test]
fn text_input_clip_em_padding_uses_computed_font_size() {
    let mut input = node(1, None, &[]);
    input.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 80.0,
    };
    input.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            font_size: Some(32.0),
            padding: Some(nana_ui_core::LengthSpec::Em(1.0)),
            ..Default::default()
        }),
        ..Default::default()
    };
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 32.0,
                y: 32.0,
                width: 136.0,
                height: 16.0,
            },
            content: Arc::from("hi"),
            color: Some([1.0; 4]),
            font_size: 32.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    let text = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .expect("text input text");
    assert_eq!(text.clips.len(), 1);
    assert_eq!(
        text.clips[0].bounds,
        SceneRect {
            x: 32.0,
            y: 32.0,
            width: 136.0,
            height: 16.0,
        },
        "1em padding at font-size 32px must clip the field 32px inset, not 16px"
    );
}

#[test]
fn extraction_preserves_custom_interleaving_and_removals() {
    let mut root = node(1, None, &[2]);
    root.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.1, 0.2, 0.3, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.custom_render = Some(CustomRenderNode::new("host-texture", "preview", 3));
    child.text = Some(TextContent {
        value: "caption".into(),
    });
    let mut scene = UiScene::new();
    let delta = scene.apply_delta([root, child], []);
    assert_eq!(delta.primitive_count, 3);
    let graph = scene.frame_graph(ResourceId(7)).unwrap();
    assert_eq!(
        graph
            .passes
            .iter()
            .flat_map(|pass| pass.operations.iter().cloned())
            .collect::<Vec<_>>(),
        vec![
            RenderOperation::PrepareExternal(PrimitiveId {
                node: id(2),
                slot: 1
            }),
            RenderOperation::Draw(PrimitiveId {
                node: id(1),
                slot: 0
            }),
            RenderOperation::InvokeCustom(PrimitiveId {
                node: id(2),
                slot: 1
            }),
            RenderOperation::Draw(PrimitiveId {
                node: id(2),
                slot: 2
            }),
        ]
    );
    assert_eq!(graph.passes.len(), 4);
    assert_eq!(graph.resources.len(), 2);
    assert_eq!(graph.resources[1].label, "preview");
    assert_eq!(graph.passes[0].label, "prepare:preview");
    assert_eq!(graph.passes[2].resources.len(), 2);
    assert!(graph.passes[2].dependencies.contains(&graph.passes[0].id));
    let root_before = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .unwrap()
        .clone();
    let mut changed_child = node(2, Some(1), &[]);
    changed_child.custom_render = Some(CustomRenderNode::new("host-texture", "preview", 4));
    let delta = scene.apply_delta([changed_child], []);
    assert!(!delta.order_rebuilt);
    assert_eq!(delta.rebuilt_primitives, 1);
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .unwrap(),
        &root_before,
        "a local extraction must not rebuild unrelated primitives"
    );
    let delta = scene.apply_delta([], [id(2)]);
    assert_eq!(delta.removed_nodes, 1);
    assert_eq!(delta.primitive_count, 1);
}

#[test]
fn selection_option_emits_surface_text_icon_and_focus_slots() {
    let mut option = node(7, None, &[]);
    option.layout = LayoutBox {
        x: 10.0,
        y: 20.0,
        width: 96.0,
        height: 26.0,
    };
    option.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.2, 0.2, 0.2, 1.0]),
            border_radius: Some(7.0),
            ..Default::default()
        }),
        ..Default::default()
    };
    option.standard_visual = Some(StandardVisual::SelectionOption {
        label: Arc::from("Preview"),
        icon: Some(nana_ui_core::Icon::Search),
        selected: true,
        disabled: false,
        size: ControlSize::Medium,
        show_focus_ring: true,
        indicator: false,
    });
    option.component_geometry = Some(ComponentGeometry::SelectionOption {
        icon: Some((
            nana_ui_core::Icon::Search,
            LayoutBox {
                x: 20.0,
                y: 26.0,
                width: 14.0,
                height: 14.0,
            },
            [0.8, 0.8, 0.8, 1.0],
        )),
        label: ComponentTextRegion {
            bounds: LayoutBox {
                x: 39.0,
                y: 20.0,
                width: 57.0,
                height: 26.0,
            },
            content: Arc::from("Preview"),
            color: Some([1.0, 1.0, 1.0, 1.0]),
            font_size: 13.0,
            font_weight: Some(500),
        },
        focus_ring: Some([0.2, 0.6, 1.0, 1.0]),
        indicator: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([option], []);
    for slot in [0, 2, 3, 7] {
        assert!(scene.primitive(PrimitiveId { node: id(7), slot }).is_some());
    }
    let focus = scene
        .primitive(PrimitiveId {
            node: id(7),
            slot: 7,
        })
        .unwrap();
    assert_eq!(
        focus.bounds,
        SceneRect {
            x: 6.0,
            y: 16.0,
            width: 104.0,
            height: 34.0,
        }
    );
    assert!(matches!(
        focus.kind,
        ScenePrimitiveKind::Quad {
            border_width: 2.0,
            corner_radius,
            ..
        } if corner_radius.iter().all(|r| (*r - 11.0).abs() < f32::EPSILON)
    ));
    assert!(matches!(
        scene.primitive(PrimitiveId { node: id(7), slot: 2 }).unwrap().kind,
        ScenePrimitiveKind::Text { ref content, wrap: false, .. } if content == "Preview"
    ));
    assert_eq!(
        primitive_icon(
            &scene
                .primitive(PrimitiveId {
                    node: id(7),
                    slot: 3
                })
                .unwrap()
                .kind
        ),
        Some(nana_ui_core::Icon::Search)
    );
}

#[test]
fn menu_surface_paints_row_icon_and_iconless_labels() {
    let mut menu = node(3, None, &[]);
    menu.layout = LayoutBox {
        x: 8.0,
        y: 12.0,
        width: 200.0,
        height: 72.0,
    };
    menu.standard_visual = Some(StandardVisual::MenuSurface {
        open: true,
        kind: nana_ui_runtime::MenuSurfaceKind::ContextMenu,
        trigger: None,
        trigger_icon: None,
        gap: 0.0,
        query: None,
        rows: Arc::from([
            nana_ui_runtime::SelectOptionData {
                label: Arc::from("Add"),
                hint: None,
                disabled: false,
                checked: false,
                icon: Some(nana_ui_core::Icon::Add),
            },
            nana_ui_runtime::SelectOptionData {
                label: Arc::from("Rename"),
                hint: None,
                disabled: false,
                checked: false,
                icon: None,
            },
        ]),
        highlighted: None,
    });
    menu.component_geometry = Some(ComponentGeometry::MenuSurface {
        trigger_surface: None,
        trigger: None,
        trigger_icon: None,
        surface: LayoutBox {
            x: 8.0,
            y: 12.0,
            width: 200.0,
            height: 72.0,
        },
        search: None,
        search_field: None,
        options: vec![
            nana_ui_runtime::SelectOptionGeometry {
                bounds: LayoutBox {
                    x: 12.0,
                    y: 16.0,
                    width: 192.0,
                    height: 26.0,
                },
                label: ComponentTextRegion {
                    bounds: LayoutBox {
                        x: 33.0,
                        y: 16.0,
                        width: 163.0,
                        height: 26.0,
                    },
                    content: Arc::from("Add"),
                    color: Some([1.0, 1.0, 1.0, 1.0]),
                    font_size: 12.0,
                    font_weight: Some(500),
                },
                selected: false,
                checked: false,
                disabled: false,
                background: None,
                icon: Some((
                    nana_ui_core::Icon::Add,
                    LayoutBox {
                        x: 20.0,
                        y: 22.0,
                        width: 13.0,
                        height: 13.0,
                    },
                    [0.7, 0.7, 0.7, 1.0],
                )),
            },
            nana_ui_runtime::SelectOptionGeometry {
                bounds: LayoutBox {
                    x: 12.0,
                    y: 43.0,
                    width: 192.0,
                    height: 26.0,
                },
                label: ComponentTextRegion {
                    bounds: LayoutBox {
                        x: 20.0,
                        y: 43.0,
                        width: 176.0,
                        height: 26.0,
                    },
                    content: Arc::from("Rename"),
                    color: Some([1.0, 1.0, 1.0, 1.0]),
                    font_size: 12.0,
                    font_weight: Some(500),
                },
                selected: false,
                checked: false,
                disabled: false,
                background: None,
                icon: None,
            },
        ],
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, 0.55],
            offset_x: 0.0,
            offset_y: 4.0,
            blur_radius: 18.0,
            spread_radius: 0.0,
            inset: false,
        },
        background: [0.1, 0.1, 0.1, 1.0],
        border: [0.2, 0.2, 0.2, 1.0],
    });
    let mut scene = UiScene::new();
    scene.apply_delta([menu], []);
    assert_eq!(
        primitive_icon(
            &scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 80
                })
                .unwrap()
                .kind
        ),
        Some(nana_ui_core::Icon::Add)
    );
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 40
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { .. }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 41
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { .. }
    ));
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 81
            })
            .is_none()
    );
}

#[test]
fn frame_graph_rejects_conflicting_revisions_of_one_external_resource() {
    let mut first = node(1, None, &[]);
    first.custom_render = Some(CustomRenderNode::new("nana.host-texture", "program", 7));
    let mut second = node(2, None, &[]);
    second.custom_render = Some(CustomRenderNode::new("nana.host-texture", "program", 8));
    let mut scene = UiScene::new();
    scene.apply_delta([first, second], []);

    assert_eq!(
        scene.frame_graph(ResourceId(1)),
        Err(GraphError::ConflictingExternalResource("program".into()))
    );
}

#[test]
fn rotate_pivot_follows_transform_origin() {
    let rotate_90 = nana_ui_core::PaintTransform {
        a: 0.0,
        b: 1.0,
        c: -1.0,
        d: 0.0,
        ..Default::default()
    };
    let mut centered = node(1, None, &[]);
    centered.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 20.0,
        height: 10.0,
    };
    centered.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
    centered.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            transform: Some(rotate_90),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut corner = node(2, None, &[]);
    corner.layout = centered.layout;
    corner.custom_render = Some(CustomRenderNode::new("test", "resource", 1));
    corner.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            transform: Some(rotate_90),
            transform_origin: Some(nana_ui_core::TransformOrigin {
                x: nana_ui_core::LengthSpec::Px(0.0),
                y: nana_ui_core::LengthSpec::Px(0.0),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([centered, corner], []);
    let center_tf = scene
        .primitives()
        .find(|primitive| primitive.node == id(1))
        .expect("center")
        .transform
        .0;
    let corner_tf = scene
        .primitives()
        .find(|primitive| primitive.node == id(2))
        .expect("corner")
        .transform
        .0;
    assert_eq!(center_tf, rotate_90.around_center(0.0, 0.0, 20.0, 10.0));
    assert_eq!(corner_tf, rotate_90.around_origin(0.0, 0.0, 0.0, 0.0));
    assert_ne!(center_tf, corner_tf);
}

#[test]
fn perspective_rotate_y_stores_projective_on_the_primitive() {
    let mat = nana_ui_core::PaintMat4::perspective(800.0)
        .expect("d")
        .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians()));
    let mut card = node(1, None, &[]);
    card.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 80.0,
    };
    card.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
    card.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            transform_3d: Some(mat),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([card], []);
    let primitive = scene
        .primitives()
        .find(|primitive| primitive.node == id(1))
        .expect("card");
    assert!(
        primitive.transform.is_projective(),
        "perspective+rotateY must not collapse to affine, persp={:?}",
        primitive.transform.1
    );
    let expected = mat
        .around_origin(0.0, 0.0, 100.0, 40.0)
        .planar_homography()
        .expect("homography");
    assert_eq!(primitive.transform.0, expected.0);
    assert_eq!(primitive.transform.1, expected.1);
}

#[test]
fn parent_preserve_3d_fail_closes_child_3d() {
    let mat = nana_ui_core::PaintMat4::perspective(800.0)
        .expect("d")
        .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians()));
    let mut parent = node(1, None, &[2]);
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            preserve_3d: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 80.0,
    };
    child.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
    child.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            transform_3d: Some(mat),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([parent, child], []);
    let primitive = scene
        .primitives()
        .find(|primitive| primitive.node == id(2))
        .expect("child");
    assert_eq!(primitive.transform, AffineTransform::IDENTITY);
}

#[test]
fn parent_perspective_fail_closes_child_3d() {
    let mat = nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians());
    let mut parent = node(1, None, &[2]);
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            css_perspective: Some(800.0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 80.0,
    };
    child.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
    child.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            transform_3d: Some(mat),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([parent, child], []);
    let primitive = scene
        .primitives()
        .find(|primitive| primitive.node == id(2))
        .expect("child");
    assert_eq!(primitive.transform, AffineTransform::IDENTITY);
}

#[test]
fn ancestor_clip_transform_and_opacity_are_composed() {
    let mut root = node(1, None, &[2]);
    root.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            overflow_x: nana_ui_core::OverflowSpec::Hidden,
            opacity: Some(0.5),
            transform: Some(nana_ui_core::PaintTransform {
                e: 4.0,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
    child.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            opacity: Some(0.5),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([root, child], []);
    let custom = scene
        .primitives()
        .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
        .unwrap();
    assert_eq!(custom.opacity, 0.5);
    assert_eq!(
        scene.opacity_groups(id(2)),
        vec![OpacityGroup {
            node: id(1),
            opacity: 0.5,
            filter: ColorFilter::default(),
            mix_blend: MixBlendMode::Normal,
            inset_shadow: None,
        }]
    );
    assert_eq!(custom.clips.len(), 1);
    assert_eq!(custom.transform.0[4], 4.0);
}

#[test]
fn leaf_opacity_stays_on_the_primitive() {
    let mut leaf = node(1, None, &[]);
    leaf.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            opacity: Some(0.5),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([leaf], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .unwrap();
    assert_eq!(primitive.opacity, 0.5);
    assert!(scene.opacity_groups(id(1)).is_empty());
}

#[test]
fn opacity_group_keeps_high_z_child_contiguous() {
    let mut parent = node(1, None, &[2]);
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 0.0, 1.0, 1.0]),
            opacity: Some(0.5),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.z_index = 10;
    child.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut sibling = node(3, None, &[]);
    sibling.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 1.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([parent, child, sibling], []);
    let order = scene
        .primitives()
        .map(|primitive| primitive.node.get())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![1, 2, 3],
        "translucent parent must isolate its high-z child from a later sibling"
    );
}

#[test]
fn losing_group_isolation_reorders_descendants_that_were_not_reextracted() {
    let solid = |color: [f32; 4], opacity: Option<f32>| NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some(color),
            opacity,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut parent = node(1, None, &[2]);
    parent.source_style = solid([0.0, 0.0, 1.0, 1.0], Some(0.5));
    let mut child = node(2, Some(1), &[]);
    child.z_index = 10;
    child.source_style = solid([1.0, 0.0, 0.0, 1.0], None);
    let mut sibling = node(3, None, &[]);
    sibling.source_style = solid([0.0, 1.0, 0.0, 1.0], None);
    let mut scene = UiScene::new();
    scene.apply_delta([parent.clone(), child, sibling.clone()], []);
    let order = |scene: &UiScene| {
        scene
            .primitives()
            .map(|primitive| primitive.node.get())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&scene), vec![1, 2, 3]);

    // Paint-only update: the child keeps its place without being extracted.
    sibling.source_style = solid([0.0, 1.0, 1.0, 1.0], None);
    scene.apply_delta([sibling], []);
    assert_eq!(
        order(&scene),
        vec![1, 2, 3],
        "a color-only update must not disturb paint order"
    );

    // The parent stops isolating, so the high-z child escapes the group and
    // sorts after the later sibling even though it was not re-extracted.
    parent.source_style = solid([0.0, 0.0, 1.0, 1.0], None);
    scene.apply_delta([parent], []);
    assert_eq!(
        order(&scene),
        vec![1, 3, 2],
        "losing group isolation must reorder the retained descendant"
    );
}

#[test]
fn positioned_z_index_keeps_high_z_child_contiguous() {
    let mut parent = node(1, None, &[2]);
    parent.z_index = 0;
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 0.0, 1.0, 1.0]),
            position: nana_ui_core::PositionSpec::Relative,
            z_index: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.z_index = 10;
    child.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            z_index: Some(10),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut sibling = node(3, None, &[]);
    sibling.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 1.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([parent, child, sibling], []);
    let order = scene
        .primitives()
        .map(|primitive| primitive.node.get())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![1, 2, 3],
        "positioned z-index parent must isolate its high-z child from a later sibling"
    );
}

#[test]
fn isolation_keeps_high_z_child_contiguous() {
    let mut parent = node(1, None, &[2]);
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 0.0, 1.0, 1.0]),
            isolation: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.z_index = 10;
    child.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut sibling = node(3, None, &[]);
    sibling.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 1.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([parent.clone(), child, sibling.clone()], []);
    let order = |scene: &UiScene| {
        scene
            .primitives()
            .map(|primitive| primitive.node.get())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&scene), vec![1, 2, 3]);

    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 0.0, 1.0, 1.0]),
            isolation: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    scene.apply_delta([parent], []);
    assert_eq!(
        order(&scene),
        vec![1, 3, 2],
        "losing isolation must reorder the retained high-z descendant"
    );
}

#[test]
fn text_primitive_preserves_content_box_and_paint_semantics() {
    let mut text = node(1, None, &[]);
    text.text = Some(TextContent {
        value: "Build".into(),
    });
    text.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            padding_top: Some(nana_ui_core::LengthSpec::Px(2.0)),
            padding_right: Some(nana_ui_core::LengthSpec::Px(8.0)),
            padding_bottom: Some(nana_ui_core::LengthSpec::Px(4.0)),
            padding_left: Some(nana_ui_core::LengthSpec::Px(10.0)),
            border_width: Some(1.0),
            white_space_nowrap: true,
            text_overflow_ellipsis: true,
            ..Default::default()
        }),
        text_horizontal_alignment: TextHorizontalAlignment::Center,
        text_vertical_alignment: TextVerticalAlignment::Center,
        ..Default::default()
    };
    style_mut(&mut text).line_height = Some(LineHeightSpec::Absolute(18.0));

    let mut scene = UiScene::new();
    scene.apply_delta([text], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .unwrap();
    assert_eq!(
        primitive.bounds,
        SceneRect {
            x: 11.0,
            y: 3.0,
            width: 80.0,
            height: 72.0,
        }
    );
    assert!(matches!(
        primitive.kind,
        ScenePrimitiveKind::Text {
            line_height: Some(LineHeightSpec::Absolute(18.0)),
            wrap: false,
            ellipsis: true,
            horizontal_alignment: TextHorizontalAlignment::Center,
            vertical_alignment: TextVerticalAlignment::Center,
            ..
        }
    ));
}

#[test]
fn text_input_editor_markers_and_line_labels_paint() {
    let mut input = node(1, None, &[]);
    input.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            padding_left: Some(nana_ui_core::LengthSpec::Px(40.0)),
            ..nana_ui_core::LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: true,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 40.0,
                y: 0.0,
                width: 84.0,
                height: 48.0,
            },
            content: Arc::from("a\nb\nc"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: vec![
            (
                LayoutBox {
                    x: 40.0,
                    y: 12.0,
                    width: 10.0,
                    height: 2.0,
                },
                [0.9, 0.1, 0.1, 1.0],
            ),
            (
                LayoutBox {
                    x: 40.0,
                    y: 30.0,
                    width: 10.0,
                    height: 2.0,
                },
                [0.9, 0.7, 0.1, 1.0],
            ),
        ],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        line_labels: vec![
            nana_ui_runtime::LineLabel {
                y: 0.0,
                height: 16.0,
                number: 1,
            },
            nana_ui_runtime::LineLabel {
                y: 16.0,
                height: 16.0,
                number: 2,
            },
        ],
        line_labels_color: [0.6, 0.6, 0.6, 1.0],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    // 每个标记一条 quad（颜色不同）。
    let marker = |slot: u8, y: f64| {
        scene
            .primitives()
            .find(|primitive| {
                primitive.id.slot == slot && (primitive.bounds.y - y as f32).abs() < 0.5
            })
            .expect("marker quad")
    };
    let error_quad = marker(20, 12.0);
    let ScenePrimitiveKind::Quad { background, .. } = &error_quad.kind else {
        panic!("expected quad");
    };
    assert_eq!(*background, Some([0.9, 0.1, 0.1, 1.0]));
    let warning_quad = marker(21, 30.0);
    let ScenePrimitiveKind::Quad { background, .. } = &warning_quad.kind else {
        panic!("expected quad");
    };
    assert_eq!(*background, Some([0.9, 0.7, 0.1, 1.0]));
    // 行号标签为右对齐文本图元。
    let label = scene
            .primitives()
            .find(|primitive| {
                primitive.id.slot == 41
                    && matches!(&primitive.kind, ScenePrimitiveKind::Text { content, .. } if content == "2")
            })
            .expect("line label");
    let ScenePrimitiveKind::Text { content, .. } = &label.kind else {
        unreachable!()
    };
    assert_eq!(&**content, "2");
}

#[test]
fn text_input_match_markers_paint_as_batches_and_current_match_emphasizes() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 84.0,
                height: 32.0,
            },
            content: Arc::from("ab ab"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: vec![(
            LayoutBox {
                x: 8.0,
                y: 12.0,
                width: 10.0,
                height: 2.0,
            },
            [0.9, 0.1, 0.1, 1.0],
        )],
        match_markers: vec![
            TextMatchMarker {
                rect: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 12.0,
                    height: 14.0,
                },
                color: [0.48, 0.73, 0.94, 0.20],
                current: false,
            },
            TextMatchMarker {
                rect: LayoutBox {
                    x: 8.0,
                    y: 16.0,
                    width: 12.0,
                    height: 14.0,
                },
                color: [0.48, 0.73, 0.94, 0.45],
                current: true,
            },
        ],
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    // 普通匹配为 slot 3 的 quad 批次，当前匹配为更强的 slot 6 批次。
    let batch = |slot: u8| {
        scene
            .primitive(PrimitiveId { node: id(1), slot })
            .expect("match batch")
    };
    let normal = batch(3);
    let ScenePrimitiveKind::QuadBatch {
        bounds, background, ..
    } = &normal.kind
    else {
        panic!("expected quad batch");
    };
    assert_eq!(bounds.len(), 1);
    assert_eq!(*background, Some([0.48, 0.73, 0.94, 0.20]));
    let current = batch(6);
    let ScenePrimitiveKind::QuadBatch {
        bounds, background, ..
    } = &current.kind
    else {
        panic!("expected quad batch");
    };
    assert_eq!(bounds.len(), 1);
    assert_eq!(*background, Some([0.48, 0.73, 0.94, 0.45]));
    // 诊断下划线（slot 20）与匹配高亮共存。
    let diagnostic = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 20,
        })
        .expect("diagnostic quad");
    let ScenePrimitiveKind::Quad { background, .. } = &diagnostic.kind else {
        panic!("expected quad");
    };
    assert_eq!(*background, Some([0.9, 0.1, 0.1, 1.0]));
}

#[test]
fn text_input_color_swatches_paint_as_one_per_item_color_batch_and_clear_with_feed() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 84.0,
                height: 32.0,
            },
            content: Arc::from("red green"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: vec![
            (
                LayoutBox {
                    x: 20.0,
                    y: 3.0,
                    width: 9.0,
                    height: 9.0,
                },
                [0.9, 0.2, 0.2, 0.5],
            ),
            (
                LayoutBox {
                    x: 60.0,
                    y: 19.0,
                    width: 9.0,
                    height: 9.0,
                },
                [0.2, 0.9, 0.3, 1.0],
            ),
        ],
        swatch_border_color: [0.5, 0.5, 0.5, 1.0],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input.clone()], []);
    // 全部 swatch 合并为一个 slot 23 的逐项颜色批次（数量与颜色种数都
    // 不占额外 slot），半透明颜色按宿主给定值直传，带 1px 细描边。
    let batch = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 23,
        })
        .expect("swatch batch");
    let ScenePrimitiveKind::QuadColorBatch {
        bounds,
        colors,
        border_color,
        border_width,
        ..
    } = &batch.kind
    else {
        panic!("expected quad color batch");
    };
    assert_eq!(bounds.len(), 2);
    assert_eq!(colors, &vec![[0.9, 0.2, 0.2, 0.5], [0.2, 0.9, 0.3, 1.0]]);
    assert_eq!(*border_color, Some([0.5, 0.5, 0.5, 1.0]));
    assert_eq!(*border_width, 1.0);

    // 清空宿主 feed 后 swatch 图元消失。
    if let Some(ComponentGeometry::TextInput { swatch_markers, .. }) =
        input.component_geometry.as_mut()
    {
        swatch_markers.clear();
    }
    scene.apply_delta([input], []);
    assert!(!scene.primitives().any(|primitive| primitive.id.slot == 23));
}

#[test]
fn text_input_minimap_paints_panel_bars_and_indicator_batches() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 80.0,
            },
            content: Arc::from("a\nb"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        minimap: Some(nana_ui_runtime::TextMinimapGeometry {
            panel: LayoutBox {
                x: 136.0,
                y: 0.0,
                width: 64.0,
                height: 80.0,
            },
            separator: LayoutBox {
                x: 135.0,
                y: 0.0,
                width: 1.0,
                height: 80.0,
            },
            bars: vec![
                LayoutBox {
                    x: 136.0,
                    y: 0.0,
                    width: 32.0,
                    height: 2.0,
                },
                LayoutBox {
                    x: 136.0,
                    y: 2.0,
                    width: 64.0,
                    height: 2.0,
                },
            ],
            indicator: Some(LayoutBox {
                x: 136.0,
                y: 4.0,
                width: 64.0,
                height: 12.0,
            }),
            panel_color: [0.12, 0.12, 0.14, 1.0],
            bar_color: [0.5, 0.5, 0.5, 1.0],
            indicator_color: [1.0, 0.2, 0.2, 0.2],
            stride: 1,
            line_count: 5,
        }),
        sticky_line: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        steppers: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([input], []);

    // 面板（70）与指示器（72）各一个 quad，行条 + 分隔线共享 slot 71
    // 的一个批次（faint 同色）。
    let panel = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 70,
        })
        .expect("minimap panel");
    let ScenePrimitiveKind::Quad { background, .. } = &panel.kind else {
        panic!("expected panel quad");
    };
    assert_eq!(*background, Some([0.12, 0.12, 0.14, 1.0]));
    assert_eq!(
        panel.bounds,
        SceneRect {
            x: 136.0,
            y: 0.0,
            width: 64.0,
            height: 80.0
        }
    );

    let bars = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 71,
        })
        .expect("minimap bars batch");
    let ScenePrimitiveKind::QuadBatch {
        bounds, background, ..
    } = &bars.kind
    else {
        panic!("expected bars batch");
    };
    assert_eq!(bounds.len(), 3, "separator + two bars share one batch");
    assert_eq!(
        bounds[0],
        SceneRect {
            x: 135.0,
            y: 0.0,
            width: 1.0,
            height: 80.0
        }
    );
    assert_eq!(*background, Some([0.5, 0.5, 0.5, 1.0]));

    let indicator = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 72,
        })
        .expect("minimap indicator");
    let ScenePrimitiveKind::Quad { background, .. } = &indicator.kind else {
        panic!("expected indicator quad");
    };
    assert_eq!(*background, Some([1.0, 0.2, 0.2, 0.2]));
}

#[test]
fn occurrence_whitespace_and_wrap_guides_paint_in_dedicated_slots() {
    let occurrence_color = [0.48, 0.73, 0.94, 0.14];
    let faint = [0.35, 0.35, 0.35, 1.0];
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 42.0,
            },
            content: Arc::from("a b\tc"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        // 出现高亮：两条淡底色填充（slot 11 批次）。
        occurrence_markers: vec![
            (
                LayoutBox {
                    x: 20.0,
                    y: 0.0,
                    width: 20.0,
                    height: 14.0,
                },
                occurrence_color,
            ),
            (
                LayoutBox {
                    x: 60.0,
                    y: 14.0,
                    width: 20.0,
                    height: 14.0,
                },
                occurrence_color,
            ),
        ],
        // 空白显示：两个空格（slot 16 圆点批次）+ 一个 Tab（slot 60+
        // 箭头图标）。
        whitespace_marks: vec![
            (
                LayoutBox {
                    x: 10.0,
                    y: 0.0,
                    width: 10.0,
                    height: 14.0,
                },
                nana_ui_runtime::TextWhitespaceKind::Space,
            ),
            (
                LayoutBox {
                    x: 40.0,
                    y: 0.0,
                    width: 10.0,
                    height: 14.0,
                },
                nana_ui_runtime::TextWhitespaceKind::Space,
            ),
            (
                LayoutBox {
                    x: 30.0,
                    y: 0.0,
                    width: 10.0,
                    height: 14.0,
                },
                nana_ui_runtime::TextWhitespaceKind::Tab,
            ),
        ],
        whitespace_color: faint,
        // wrap guide：列 5、10 的全高竖线（slot 17 批次）。
        wrap_guides: vec![
            (
                LayoutBox {
                    x: 50.0,
                    y: 0.0,
                    width: 1.0,
                    height: 42.0,
                },
                faint,
            ),
            (
                LayoutBox {
                    x: 100.0,
                    y: 0.0,
                    width: 1.0,
                    height: 42.0,
                },
                faint,
            ),
        ],
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    let batch = |slot: u8| {
        scene
            .primitive(PrimitiveId { node: id(1), slot })
            .unwrap_or_else(|| panic!("slot {slot} primitive"))
    };
    // 出现高亮批次：两条，共用淡底色。
    let ScenePrimitiveKind::QuadBatch {
        bounds, background, ..
    } = &batch(11).kind
    else {
        panic!("expected occurrence quad batch");
    };
    assert_eq!(bounds.len(), 2);
    assert_eq!(*background, Some(occurrence_color));
    // 空格圆点批次：两条小圆点。
    let ScenePrimitiveKind::QuadBatch { bounds, .. } = &batch(16).kind else {
        panic!("expected whitespace dot batch");
    };
    assert_eq!(bounds.len(), 2);
    // Tab 箭头：单一批次（slot 60），箭头图标图元。
    let tab = batch(60);
    let ScenePrimitiveKind::IconBatch {
        bounds: arrow_bounds,
        icon,
        color,
    } = &tab.kind
    else {
        panic!("expected tab arrow icon batch");
    };
    assert_eq!(arrow_bounds.len(), 1);
    assert_eq!(*icon, Icon::ArrowRight);
    assert_eq!(*color, Some(faint));
    // wrap guide 批次：两条全高竖线。
    let ScenePrimitiveKind::QuadBatch {
        bounds, background, ..
    } = &batch(17).kind
    else {
        panic!("expected wrap guide batch");
    };
    assert_eq!(bounds.len(), 2);
    assert_eq!(bounds[0].height, 42.0);
    assert_eq!(*background, Some(faint));
}

#[test]
fn text_input_without_editor_extras_paints_no_occurrence_whitespace_or_wrap_slots() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 14.0,
            },
            content: Arc::from("plain"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    for slot in [11, 16, 17, 60] {
        assert!(
            scene.primitive(PrimitiveId { node: id(1), slot }).is_none(),
            "slot {slot} must stay empty"
        );
    }
}

/// git gutter 批次测试的输入节点：多行 TextInput + 指定 git 几何。
fn git_gutter_input(node_id: u64, git: nana_ui_runtime::TextGitGutterGeometry) -> ExtractedNode {
    let mut input = node(node_id, None, &[]);
    // 宿主预留的 gutter 宽度（行号标签按它门控）。
    input.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            padding_left: Some(nana_ui_core::LengthSpec::Px(46.0)),
            ..nana_ui_core::LayoutStyle::default()
        }),
        ..NodeStyle::default()
    };
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: true,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 46.0,
                y: 0.0,
                width: 154.0,
                height: 56.0,
            },
            content: Arc::from("a\nb\nc\nd"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: vec![
            nana_ui_runtime::LineLabel {
                y: 0.0,
                height: 14.0,
                number: 1,
            },
            nana_ui_runtime::LineLabel {
                y: 14.0,
                height: 14.0,
                number: 2,
            },
        ],
        line_labels_color: [0.5, 0.5, 0.5, 1.0],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry {
            gutters: vec![nana_ui_runtime::TextFoldGutter {
                bounds: LayoutBox {
                    x: 2.0,
                    y: 0.0,
                    width: 14.0,
                    height: 14.0,
                },
                fold: nana_ui_runtime::TextCodeFold::new(0, 8),
                collapsed: true,
                color: [0.5, 0.5, 0.5, 0.4],
            }],
            markers: Vec::new(),
        },
        git_marks: git,
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });
    input
}

#[test]
fn text_input_git_gutter_renders_kind_batches_and_coexists_with_gutter_slots() {
    let git = nana_ui_runtime::TextGitGutterGeometry {
        added: vec![
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 14.0,
            },
            LayoutBox {
                x: 0.0,
                y: 28.0,
                width: 2.0,
                height: 14.0,
            },
        ],
        modified: vec![LayoutBox {
            x: 0.0,
            y: 14.0,
            width: 2.0,
            height: 14.0,
        }],
        deleted: vec![LayoutBox {
            x: 0.0,
            y: 42.0,
            width: 2.0,
            height: 14.0,
        }],
        added_color: [0.2, 0.8, 0.3, 1.0],
        modified_color: [0.9, 0.7, 0.2, 1.0],
        deleted_color: [0.9, 0.3, 0.3, 1.0],
    };
    let mut scene = UiScene::new();
    scene.apply_delta([git_gutter_input(1, git)], []);

    // 三类各一个 quad 批次（slot 18 新增 / 19 修改 / 8 删除），批次内
    // 同色合批、bounds 逐一对应。
    let batch = |slot: u8| scene.primitive(PrimitiveId { node: id(1), slot });
    let added = batch(18).expect("added batch");
    match &added.kind {
        ScenePrimitiveKind::QuadBatch {
            bounds,
            background,
            border_color,
            ..
        } => {
            assert_eq!(bounds.len(), 2);
            assert_eq!(*background, Some([0.2, 0.8, 0.3, 1.0]));
            assert_eq!(*border_color, None);
            assert_eq!(
                bounds[0],
                SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 2.0,
                    height: 14.0
                }
            );
            assert_eq!(
                bounds[1],
                SceneRect {
                    x: 0.0,
                    y: 28.0,
                    width: 2.0,
                    height: 14.0
                }
            );
        }
        _ => panic!("expected added git gutter quad batch"),
    }
    let modified = batch(19).expect("modified batch");
    match &modified.kind {
        ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } => {
            assert_eq!(bounds.len(), 1);
            assert_eq!(*background, Some([0.9, 0.7, 0.2, 1.0]));
        }
        _ => panic!("expected modified git gutter quad batch"),
    }
    let deleted = batch(8).expect("deleted batch");
    match &deleted.kind {
        ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } => {
            assert_eq!(bounds.len(), 1);
            assert_eq!(*background, Some([0.9, 0.3, 0.3, 1.0]));
        }
        _ => panic!("expected deleted git gutter quad batch"),
    }

    // 与行号（slot 40+）、折叠箭头（slot 14/15）共存，slot 互不冲突。
    assert!(batch(40).is_some(), "line number label");
    assert!(batch(14).is_some(), "collapsed fold gutter");
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 15
            })
            .is_none(),
        "no expanded fold gutter"
    );

    // 空种类不产生批次：只喂修改标记时 slot 8/18 无图元。
    let only_modified = nana_ui_runtime::TextGitGutterGeometry {
        modified: vec![LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 14.0,
        }],
        modified_color: [0.9, 0.7, 0.2, 1.0],
        ..nana_ui_runtime::TextGitGutterGeometry::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([git_gutter_input(1, only_modified)], []);
    for slot in [8, 18] {
        assert!(
            scene.primitive(PrimitiveId { node: id(1), slot }).is_none(),
            "empty kind slot {slot} must stay empty"
        );
    }
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 19
            })
            .is_some()
    );
}

#[test]
fn text_input_sticky_line_paints_panel_divider_and_head_text() {
    let mut scene = UiScene::new();
    let mut input = git_gutter_input(1, nana_ui_runtime::TextGitGutterGeometry::default());
    if let Some(ComponentGeometry::TextInput { sticky_line, .. }) = &mut input.component_geometry {
        *sticky_line = Some(nana_ui_runtime::TextStickyLineGeometry {
            panel: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 14.0,
            },
            divider: LayoutBox {
                x: 0.0,
                y: 13.0,
                width: 200.0,
                height: 1.0,
            },
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 14.0,
                },
                content: Arc::from("fn outer() {"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            background: [1.0, 1.0, 1.0, 1.0],
            divider_color: [0.2, 0.2, 0.2, 1.0],
        });
    }
    scene.apply_delta([input], []);

    // 背景条与底缘分割线各一个 quad，头行文本复用正文字形管线；
    // 无钉住几何时三个 slot 全部为空。
    let primitive = |slot: u8| scene.primitive(PrimitiveId { node: id(1), slot });
    let panel = primitive(80).expect("sticky panel");
    match &panel.kind {
        ScenePrimitiveKind::Quad { background, .. } => {
            assert_eq!(*background, Some([1.0, 1.0, 1.0, 1.0]))
        }
        other => panic!("expected panel quad, got {other:?}"),
    }
    let divider = primitive(81).expect("sticky divider");
    match &divider.kind {
        ScenePrimitiveKind::Quad { background, .. } => {
            assert_eq!(*background, Some([0.2, 0.2, 0.2, 1.0]))
        }
        other => panic!("expected divider quad, got {other:?}"),
    }
    let text = primitive(82).expect("sticky text");
    match &text.kind {
        ScenePrimitiveKind::Text { content, .. } => {
            assert_eq!(content, "fn outer() {")
        }
        other => panic!("expected sticky text primitive, got {other:?}"),
    }

    let mut scene = UiScene::new();
    scene.apply_delta(
        [git_gutter_input(
            1,
            nana_ui_runtime::TextGitGutterGeometry::default(),
        )],
        [],
    );
    for slot in [80, 81, 82] {
        assert!(
            scene.primitive(PrimitiveId { node: id(1), slot }).is_none(),
            "sticky slot {slot} must stay empty without geometry"
        );
    }
}

#[test]
fn fold_gutter_marks_paint_as_two_batches_and_survive_beyond_the_slot_cap() {
    const FOLDS: usize = 25;
    let mut gutters = Vec::with_capacity(FOLDS);
    for index in 0..FOLDS {
        gutters.push(nana_ui_runtime::TextFoldGutter {
            bounds: LayoutBox {
                x: 2.0,
                y: index as f32 * 14.0,
                width: 14.0,
                height: 14.0,
            },
            fold: nana_ui_runtime::TextCodeFold::new(index * 10, index * 10 + 8),
            collapsed: index % 2 == 0,
            color: [0.5, 0.5, 0.5, 0.4],
        });
    }
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 18.0,
                y: 0.0,
                width: 180.0,
                height: 350.0,
            },
            content: Arc::from("fn a() {}"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry {
            gutters,
            markers: Vec::new(),
        },
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    // 折叠态（slot 14，实心）与展开态（slot 15，描边）各一个批次，
    // 超过旧 slot 上限（21）后仍全部渲染。
    let batch = |slot: u8| {
        scene
            .primitive(PrimitiveId { node: id(1), slot })
            .expect("gutter batch")
    };
    let collapsed = batch(14);
    let collapsed_len = match &collapsed.kind {
        ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } => {
            assert_eq!(bounds.len(), 13);
            assert_eq!(*background, Some([0.5, 0.5, 0.5, 0.4]));
            assert_eq!(
                bounds[0],
                SceneRect {
                    x: 2.0,
                    y: 0.0,
                    width: 14.0,
                    height: 14.0,
                }
            );
            bounds.len()
        }
        _ => panic!("expected collapsed gutter quad batch"),
    };
    let expanded = batch(15);
    let expanded_len = match &expanded.kind {
        ScenePrimitiveKind::QuadBatch {
            bounds,
            background,
            border_color,
            border_width,
            ..
        } => {
            assert_eq!(bounds.len(), 12);
            assert_eq!(*background, None);
            assert_eq!(*border_color, Some([0.5, 0.5, 0.5, 0.4]));
            assert_eq!(*border_width, 1.0);
            assert_eq!(
                bounds[11],
                SceneRect {
                    x: 2.0,
                    y: 23.0 * 14.0,
                    width: 14.0,
                    height: 14.0,
                }
            );
            bounds.len()
        }
        _ => panic!("expected expanded gutter quad batch"),
    };
    // 全部 25 个箭头（>21）都渲染为批次内的 quad，不再互相覆盖。
    assert_eq!(collapsed_len + expanded_len, FOLDS);
}

#[test]
fn tab_arrows_paint_as_one_batch_and_survive_beyond_the_slot_cap() {
    const TABS: usize = 300;
    let mut marks = Vec::with_capacity(TABS);
    for index in 0..TABS {
        marks.push((
            LayoutBox {
                x: 10.0 + (index % 40) as f32 * 10.0,
                y: (index / 40) as f32 * 14.0,
                width: 10.0,
                height: 14.0,
            },
            nana_ui_runtime::TextWhitespaceKind::Tab,
        ));
    }
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 18.0,
                y: 0.0,
                width: 180.0,
                height: 350.0,
            },
            content: Arc::from("\t".repeat(TABS)),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        occurrence_markers: Vec::new(),
        whitespace_marks: marks,
        whitespace_color: [0.5, 0.5, 0.5, 1.0],
        wrap_guides: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    // 单一批次（slot 60）装下全部 300 个 Tab 箭头；旧实现按
    // 60 + index 分配 slot，超过 195 个后在 u8 上回绕互相覆盖。
    let batch = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 60,
        })
        .expect("tab arrow batch");
    let ScenePrimitiveKind::IconBatch {
        bounds: arrows,
        icon,
        color,
    } = &batch.kind
    else {
        panic!("expected tab arrow icon batch");
    };
    assert_eq!(arrows.len(), TABS);
    assert_eq!(*icon, Icon::ArrowRight);
    assert_eq!(*color, Some([0.5, 0.5, 0.5, 1.0]));
    for (index, arrow) in arrows.iter().enumerate() {
        let expected_cell_x = 10.0 + (index % 40) as f32 * 10.0;
        let expected_cell_y = (index / 40) as f32 * 14.0;
        // 箭头按字符单元高度的 0.55 居中放置。
        let extent = (14.0f32 * 0.55).clamp(6.0, 14.0);
        assert!((arrow.width - extent).abs() < f32::EPSILON);
        assert!(
            (arrow.x - (expected_cell_x + (10.0 - extent) / 2.0)).abs() < f32::EPSILON,
            "arrow {index} x mismatch"
        );
        assert!(
            (arrow.y - (expected_cell_y + (14.0 - extent) / 2.0)).abs() < f32::EPSILON,
            "arrow {index} y mismatch"
        );
    }
    // 批次外的任何 slot 都不承载 Tab 箭头。
    for slot in [61u8, 100, 200, 255] {
        assert!(
            scene.primitive(PrimitiveId { node: id(1), slot }).is_none(),
            "slot {slot} must stay empty"
        );
    }
}

#[test]
fn text_input_paints_additional_cursors_as_a_batch_beside_the_primary_caret() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 40.0,
                y: 0.0,
                width: 84.0,
                height: 48.0,
            },
            content: Arc::from("a\nb\nc"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: Some(LayoutBox {
            x: 48.0,
            y: 8.0,
            width: 1.0,
            height: 16.0,
        }),
        additional_carets: vec![
            LayoutBox {
                x: 8.0,
                y: 24.0,
                width: 1.0,
                height: 16.0,
            },
            LayoutBox {
                x: 20.0,
                y: 40.0,
                width: 1.0,
                height: 16.0,
            },
        ],
        additional_caret_color: [0.2, 0.2, 0.2, 0.55],
        preedit: Vec::new(),
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);

    // 主光标保持单矩形 slot 4。
    let caret = scene
        .primitives()
        .find(|primitive| primitive.id.slot == 4)
        .expect("primary caret");
    assert_eq!(caret.bounds.y, 8.0);

    // 附加光标合并为一个半透明 quad 批次（slot 13）。
    let batch = scene
        .primitives()
        .find(|primitive| primitive.id.slot == 13)
        .expect("additional caret batch");
    let ScenePrimitiveKind::QuadBatch {
        bounds: rects,
        background,
        ..
    } = &batch.kind
    else {
        panic!("expected quad batch");
    };
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].y, 24.0);
    assert_eq!(rects[1].y, 40.0);
    assert_eq!(*background, Some([0.2, 0.2, 0.2, 0.55]));
}

#[test]
fn text_input_editor_chrome_paints_caret_line_brackets_and_indent_guides() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: Some(Arc::from("\t")),
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 84.0,
                height: 48.0,
            },
            content: Arc::from("ab"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        // 当前行条与选区同层（slot 1）。
        caret_line: Some((
            LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 84.0,
                height: 16.0,
            },
            [0.18, 0.18, 0.18, 1.0],
        )),
        // 括号匹配两端共用 accent 描边。
        bracket_markers: vec![
            (
                LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 6.0,
                    height: 16.0,
                },
                [0.48, 0.73, 0.94, 1.0],
            ),
            (
                LayoutBox {
                    x: 20.0,
                    y: 16.0,
                    width: 6.0,
                    height: 16.0,
                },
                [0.48, 0.73, 0.94, 1.0],
            ),
        ],
        // 缩进参考线：两条 1px 竖线一个批次。
        indent_guides: vec![
            (
                LayoutBox {
                    x: 10.0,
                    y: 0.0,
                    width: 1.0,
                    height: 16.0,
                },
                [0.16, 0.16, 0.16, 1.0],
            ),
            (
                LayoutBox {
                    x: 10.0,
                    y: 16.0,
                    width: 1.0,
                    height: 16.0,
                },
                [0.16, 0.16, 0.16, 1.0],
            ),
        ],
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    let primitive = |slot: u8| {
        scene
            .primitive(PrimitiveId { node: id(1), slot })
            .expect("chrome primitive")
    };
    // 当前行条是 slot 1 的单个填充 quad。
    let line = primitive(1);
    let ScenePrimitiveKind::Quad { background, .. } = &line.kind else {
        panic!("expected caret line quad");
    };
    assert_eq!(*background, Some([0.18, 0.18, 0.18, 1.0]));
    assert_eq!(line.bounds.width, 84.0);
    // 缩进参考线是 slot 10 的填充批次，同一颜色合并。
    let guides = primitive(10);
    let ScenePrimitiveKind::QuadBatch {
        bounds, background, ..
    } = &guides.kind
    else {
        panic!("expected indent guide batch");
    };
    assert_eq!(bounds.len(), 2);
    assert_eq!(*background, Some([0.16, 0.16, 0.16, 1.0]));
    // 括号匹配是 slot 12 的描边批次（无填充，不遮挡字形）。
    let brackets = primitive(12);
    let ScenePrimitiveKind::QuadBatch {
        bounds,
        background,
        border_color,
        border_width,
        ..
    } = &brackets.kind
    else {
        panic!("expected bracket batch");
    };
    assert_eq!(bounds.len(), 2);
    assert_eq!(*background, None);
    assert_eq!(*border_color, Some([0.48, 0.73, 0.94, 1.0]));
    assert_eq!(*border_width, 1.0);
}

#[test]
fn text_input_geometry_paints_selection_text_caret_preedit_and_focus_in_order() {
    let mut input = node(1, None, &[]);
    input.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.1, 0.1, 0.1, 1.0]),
            border_width: Some(1.0),
            border_color: Some([0.3, 0.3, 0.3, 1.0]),
            border_radius: Some(6.0),
            overflow_x: nana_ui_core::OverflowSpec::Hidden,
            opacity: Some(0.5),
            transform: Some(nana_ui_core::PaintTransform {
                e: 4.0,
                f: 6.0,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 84.0,
                height: 32.0,
            },
            content: Arc::from("release/next"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: (0..10)
            .map(|line| LayoutBox {
                x: 8.0,
                y: 8.0 + line as f32 * 16.0,
                width: 40.0 + line as f32,
                height: 16.0,
            })
            .collect(),
        caret: Some(LayoutBox {
            x: 48.0,
            y: 8.0,
            width: 1.0,
            height: 16.0,
        }),
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: vec![
            LayoutBox {
                x: 48.0,
                y: 23.0,
                width: 18.0,
                height: 1.0,
            },
            LayoutBox {
                x: 8.0,
                y: 39.0,
                width: 24.0,
                height: 1.0,
            },
        ],
        background: Some([0.1, 0.1, 0.1, 1.0]),
        border: Some([0.3, 0.3, 0.3, 1.0]),
        border_width: 1.0,
        focus_ring: Some([0.2, 0.6, 1.0, 1.0]),
        selection_color: [0.2, 0.4, 0.7, 0.4],
        caret_color: [0.2, 0.6, 1.0, 1.0],
        preedit_color: [0.2, 0.6, 1.0, 1.0],
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    assert_eq!(scene.primitives().count(), 6);
    for slot in [0, 1, 2, 4, 5, 7] {
        assert!(scene.primitive(PrimitiveId { node: id(1), slot }).is_some());
    }
    assert!(matches!(
        scene.primitive(PrimitiveId { node: id(1), slot: 2 }).unwrap().kind,
        ScenePrimitiveKind::Text {
            ref content,
            wrap: true,
            vertical_alignment: TextVerticalAlignment::Top,
            ..
        } if content == "release/next"
    ));
    for slot in [1, 5] {
        let primitive = scene.primitive(PrimitiveId { node: id(1), slot }).unwrap();
        assert_eq!(primitive.transform.0[4..], [4.0, 6.0]);
        assert_eq!(primitive.opacity, 0.5);
        assert_eq!(primitive.clips.len(), 2);
        let expected_count = if slot == 1 { 10 } else { 2 };
        assert!(matches!(
            primitive.kind,
            ScenePrimitiveKind::QuadBatch { ref bounds, .. }
                if bounds.len() == expected_count
        ));
    }

    let mut single_line = node(2, None, &[]);
    single_line.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    single_line.component_geometry = input_component_geometry(false);
    scene.apply_delta([single_line], []);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            wrap: false,
            vertical_alignment: TextVerticalAlignment::Center,
            ..
        }
    ));
}

fn input_component_geometry(multiline: bool) -> Option<ComponentGeometry> {
    Some(ComponentGeometry::TextInput {
        multiline,
        text: nana_ui_runtime::ComponentTextRegion {
            bounds: LayoutBox {
                x: 8.0,
                y: 0.0,
                width: 84.0,
                height: 32.0,
            },
            content: Arc::from("release/next"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.2, 0.4, 0.7, 0.4],
        caret_color: [0.2, 0.6, 1.0, 1.0],
        preedit_color: [0.2, 0.6, 1.0, 1.0],
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
    })
}

#[test]
fn feedback_geometry_emits_semantic_quad_text_and_icon_primitives() {
    let region = |content: &'static str, y, size, weight| ComponentTextRegion {
        bounds: LayoutBox {
            x: 20.0,
            y,
            width: 120.0,
            height: 18.0,
        },
        content: Arc::from(content),
        color: Some([0.8, 0.9, 1.0, 1.0]),
        font_size: size,
        font_weight: weight,
    };
    let mut badge = node(1, None, &[]);
    badge.source_style.layout = Arc::new(nana_ui_core::LayoutStyle {
        background: Some([0.1, 0.2, 0.3, 1.0]),
        ..Default::default()
    });
    badge.standard_visual = Some(StandardVisual::StatusBadge {
        label: Arc::from("Online"),
        tone: nana_ui_core::StatusTone::Success,
        compact: true,
    });
    badge.component_geometry = Some(ComponentGeometry::StatusBadge {
        indicator: LayoutBox {
            x: 8.0,
            y: 8.0,
            width: 3.0,
            height: 3.0,
        },
        label: region("Online", 0.0, 11.0, Some(500)),
        background: [0.1, 0.8, 0.4, 0.12],
        foreground: [0.1, 0.8, 0.4, 1.0],
    });

    let mut validation = node(2, None, &[]);
    validation.standard_visual = Some(StandardVisual::ValidationMessage {
        message: Arc::from("Required"),
        intent: nana_ui_core::ValidationIntent::Danger,
        compact: true,
    });
    validation.component_geometry = Some(ComponentGeometry::ValidationMessage {
        indicator: LayoutBox {
            x: 4.0,
            y: 7.0,
            width: 5.0,
            height: 5.0,
        },
        label: region("Required", 0.0, 11.0, None),
        foreground: [0.9, 0.2, 0.2, 1.0],
    });

    let mut empty = node(3, None, &[]);
    empty.standard_visual = Some(StandardVisual::EmptyState {
        title: Arc::from("No files"),
        message: Some(Arc::from("Create one")),
        icon: Some(nana_ui_core::Icon::Folder),
        compact: false,
        action: None,
    });
    empty.component_geometry = Some(ComponentGeometry::EmptyState {
        root_clip: empty.layout,
        content_clip: LayoutBox {
            x: 16.0,
            y: 24.0,
            width: 68.0,
            height: 52.0,
        },
        icon: Some((
            nana_ui_core::Icon::Folder,
            LayoutBox {
                x: 40.0,
                y: 2.0,
                width: 22.0,
                height: 22.0,
            },
            [0.5, 0.5, 0.5, 1.0],
        )),
        title: region("No files", 26.0, 13.0, Some(600)),
        message: Some(region("Create one", 46.0, 12.0, None)),
        action: None,
    });

    let mut labeled = node(4, None, &[]);
    labeled.standard_visual = Some(StandardVisual::LabeledValue {
        label: Arc::from("Revision"),
        value: Arc::from("42"),
        value_role: nana_ui_core::SemanticColorRole::Text,
        value_weight: 600,
        compact: true,
        action: None,
    });
    labeled.component_geometry = Some(ComponentGeometry::LabeledValue {
        label: region("Revision", 0.0, 11.0, None),
        value: region("42", 14.0, 12.0, Some(600)),
        action: None,
    });

    let mut compact_empty = node(5, None, &[]);
    compact_empty.standard_visual = Some(StandardVisual::EmptyState {
        title: Arc::from("空状态"),
        message: None,
        icon: None,
        compact: true,
        action: None,
    });
    compact_empty.component_geometry = Some(ComponentGeometry::EmptyState {
        root_clip: compact_empty.layout,
        content_clip: LayoutBox {
            x: 6.0,
            y: 8.0,
            width: 88.0,
            height: 84.0,
        },
        icon: None,
        title: region("空状态", 2.0, 12.0, Some(600)),
        message: None,
        action: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([badge, validation, empty, labeled, compact_empty], []);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            background: Some([_, _, _, 0.12]),
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            background: Some(_),
            border_width: 0.0,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 3
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            background: None,
            border_width: 1.0,
            ..
        }
    ));
    assert_eq!(
        primitive_icon(
            &scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 3
                })
                .unwrap()
                .kind
        ),
        Some(nana_ui_core::Icon::Folder)
    );
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 4
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { size: 12.0, .. }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            wrap: true,
            horizontal_alignment: TextHorizontalAlignment::Start,
            vertical_alignment: TextVerticalAlignment::Top,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(5),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            wrap: true,
            horizontal_alignment: TextHorizontalAlignment::Start,
            vertical_alignment: TextVerticalAlignment::Top,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(4),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            size: 11.0,
            weight: None,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(4),
                slot: 3
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            size: 12.0,
            weight: Some(600),
            ..
        }
    ));
}

#[test]
fn labeled_value_and_card_text_primitives_enable_ellipsis() {
    // 长文本区域(label/value/标题)必须开启省略截断,否则 Start/End 对齐
    // 的溢出会盖过属性行对侧内容(如完整文件路径)。
    let long_path: Arc<str> = Arc::from("/Users/dev/workspace/very-long-project/assets/textures");
    let region = |content: Arc<str>, y| ComponentTextRegion {
        bounds: LayoutBox {
            x: 12.0,
            y,
            width: 96.0,
            height: 16.0,
        },
        content,
        color: None,
        font_size: 12.0,
        font_weight: None,
    };

    let mut labeled = node(1, None, &[]);
    labeled.standard_visual = Some(StandardVisual::LabeledValue {
        label: Arc::from("Source"),
        value: long_path.clone(),
        value_role: nana_ui_core::SemanticColorRole::Text,
        value_weight: 600,
        compact: false,
        action: None,
    });
    labeled.component_geometry = Some(ComponentGeometry::LabeledValue {
        label: region(Arc::from("Source"), 0.0),
        value: region(long_path.clone(), 14.0),
        action: None,
    });

    let mut card = node(2, None, &[]);
    card.standard_visual = Some(StandardVisual::Card {
        title: Some(Arc::from("渲染管线")),
        kind: nana_ui_core::CardKind::Surface,
        loading: false,
        loading_phase: 0.0,
    });
    card.component_geometry = Some(ComponentGeometry::Card {
        title: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 10.0,
                y: 8.0,
                width: 60.0,
                height: 18.0,
            },
            content: Arc::from("渲染管线 / Render Pipeline Settings"),
            color: None,
            font_size: 13.0,
            font_weight: Some(600),
        }),
        content: LayoutBox {
            x: 10.0,
            y: 36.0,
            width: 80.0,
            height: 34.0,
        },
        elevation: None,
        spinner: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([labeled, card], []);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            ellipsis: true,
            horizontal_alignment: TextHorizontalAlignment::Start,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            ellipsis: true,
            horizontal_alignment: TextHorizontalAlignment::End,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { ellipsis: true, .. }
    ));
}

#[test]
fn action_menu_item_label_and_hint_text_primitives_enable_ellipsis() {
    let region = |content: &'static str, x, width| ComponentTextRegion {
        bounds: LayoutBox {
            x,
            y: 4.0,
            width,
            height: 18.0,
        },
        content: Arc::from(content),
        color: None,
        font_size: 13.0,
        font_weight: None,
    };

    let mut item = node(1, None, &[]);
    item.standard_visual = Some(StandardVisual::ActionMenuItem {
        label: Arc::from("Reveal in Finder"),
        hint: Some(Arc::from("/Users/dev/very-long-project-folder-name")),
        icon: Some(nana_ui_core::Icon::Folder),
        danger: false,
        active: false,
        disabled: false,
        size: nana_ui_core::ControlSize::Medium,
    });
    item.component_geometry = Some(ComponentGeometry::ActionMenuItem {
        icon: Some((
            nana_ui_core::Icon::Folder,
            LayoutBox {
                x: 8.0,
                y: 6.0,
                width: 16.0,
                height: 16.0,
            },
            [0.8, 0.8, 0.8, 1.0],
        )),
        label: region("/Users/dev/very-long-project-folder-name", 32.0, 100.0),
        hint: Some(region(
            "/Users/dev/very-long-project-folder-name",
            132.0,
            60.0,
        )),
        background: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([item], []);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            ellipsis: true,
            horizontal_alignment: TextHorizontalAlignment::Start,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 4
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            ellipsis: true,
            horizontal_alignment: TextHorizontalAlignment::End,
            ..
        }
    ));
}

#[test]
fn modal_frame_emits_distinct_scrim_surface_and_intrinsic_text_slots() {
    let mut modal = node(50, None, &[]);
    modal.standard_visual = Some(StandardVisual::ModalFrame {
        title: Arc::from("Delete project"),
        description: Some(Arc::from("This cannot be undone")),
        body_text: None,
        kind: nana_ui_runtime::ModalSurfaceKind::Confirm(nana_ui_core::DialogSize::Compact),
        busy: false,
        danger: false,
        slots: nana_ui_runtime::ModalSlots::default(),
    });
    let text = |content: &'static str, y, height, size, weight| ComponentTextRegion {
        bounds: LayoutBox {
            x: 206.0,
            y,
            width: 388.0,
            height,
        },
        content: Arc::from(content),
        color: Some([1.0; 4]),
        font_size: size,
        font_weight: weight,
    };
    modal.component_geometry = Some(ComponentGeometry::ModalFrame {
        scrim: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        surface: LayoutBox {
            x: 190.0,
            y: 72.0,
            width: 420.0,
            height: 180.0,
        },
        body: LayoutBox {
            x: 206.0,
            y: 130.0,
            width: 388.0,
            height: 80.0,
        },
        title: text("Delete project", 86.0, 20.0, 14.0, Some(600)),
        description: Some(text("This cannot be undone", 110.0, 18.0, 12.0, None)),
        body_text: None,
        background: [0.1, 0.1, 0.1, 1.0],
        border: [0.3, 0.3, 0.3, 1.0],
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, 0.24],
            offset_x: 0.0,
            offset_y: 8.0,
            blur_radius: 24.0,
            spread_radius: 0.0,
            inset: false,
        },
    });
    let mut scene = UiScene::default();
    scene.apply_delta([modal], []);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(50),
                slot: 10
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            background: Some([0.0, 0.0, 0.0, 0.45]),
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(50),
                slot: 11
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            border_color: None,
            border_width: 0.0,
            corner_radius,
            shadow: Some(_),
            ..
        } if corner_radius
            .iter()
            .all(|r| (*r - UI_METRICS.radius_md).abs() < f32::EPSILON)
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(50),
                slot: 12
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            wrap: true,
            horizontal_alignment: TextHorizontalAlignment::Start,
            ..
        }
    ));
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(50),
                slot: 13
            })
            .is_some()
    );
}

#[test]
fn command_palette_title_and_query_sort_above_surface_quads() {
    let mut palette = node(60, None, &[]);
    palette.standard_visual = Some(StandardVisual::CommandPalette {
        title: Arc::from("命令"),
        query: Arc::from("工作区"),
        placeholder: Arc::from("搜索操作"),
        empty: None,
        rows: Arc::from([]),
    });
    let text = |content: &'static str, y, height, size, weight| ComponentTextRegion {
        bounds: LayoutBox {
            x: 24.0,
            y,
            width: 360.0,
            height,
        },
        content: Arc::from(content),
        color: Some([1.0; 4]),
        font_size: size,
        font_weight: weight,
    };
    palette.component_geometry = Some(ComponentGeometry::CommandPalette {
        scrim: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        surface: LayoutBox {
            x: 160.0,
            y: 80.0,
            width: 480.0,
            height: 240.0,
        },
        title: text("命令", 96.0, 22.0, 16.0, Some(600)),
        input: text("工作区", 132.0, 32.0, 13.0, None),
        empty: Some(text("没有可用操作", 176.0, 40.0, 12.0, None)),
        rows: Vec::new(),
        background: [0.1, 0.1, 0.1, 1.0],
        input_background: [0.08, 0.08, 0.08, 1.0],
        input_border: [0.3, 0.3, 0.3, 1.0],
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, 0.4],
            offset_x: 0.0,
            offset_y: 12.0,
            blur_radius: 24.0,
            spread_radius: 0.0,
            inset: false,
        },
    });

    let mut scene = UiScene::default();
    scene.apply_delta([palette], []);
    let node = id(60);
    let ordered = scene
        .primitives()
        .filter(|primitive| primitive.node == node)
        .collect::<Vec<_>>();
    let position = |slot: u8| {
        ordered
            .iter()
            .position(|primitive| primitive.id.slot == slot)
            .unwrap_or_else(|| panic!("missing command-palette slot {slot}"))
    };
    let surface = scene.primitive(PrimitiveId { node, slot: 11 }).unwrap();
    let input_quad = scene.primitive(PrimitiveId { node, slot: 12 }).unwrap();
    let title = scene.primitive(PrimitiveId { node, slot: 20 }).unwrap();
    let query = scene.primitive(PrimitiveId { node, slot: 21 }).unwrap();
    let empty = scene.primitive(PrimitiveId { node, slot: 22 }).unwrap();

    assert!(matches!(surface.kind, ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(input_quad.kind, ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(
        &title.kind,
        ScenePrimitiveKind::Text { content, .. } if content == "命令"
    ));
    assert!(matches!(
        &query.kind,
        ScenePrimitiveKind::Text { content, .. } if content == "工作区"
    ));
    assert!(matches!(&empty.kind, ScenePrimitiveKind::Text { .. }));
    assert_eq!(title.z_index, surface.z_index);
    assert_eq!(query.z_index, input_quad.z_index);
    assert_eq!(title.document_order, surface.document_order);
    assert_eq!(query.document_order, input_quad.document_order);
    assert!(
        position(20) > position(11) && position(20) > position(12),
        "title must sort after surface and input quads"
    );
    assert!(
        position(21) > position(11) && position(21) > position(12),
        "query must sort after surface and input quads"
    );
    assert!(
        position(22) > position(11) && position(22) > position(12),
        "empty text must sort after surface and input quads"
    );
    assert!(scene.primitive(PrimitiveId { node, slot: 2 }).is_none());
    assert!(scene.primitive(PrimitiveId { node, slot: 3 }).is_none());
    assert!(scene.primitive(PrimitiveId { node, slot: 4 }).is_none());
}

#[test]
fn docked_drawer_extends_the_flush_edge_so_clipping_squares_that_side() {
    let mut drawer = node(52, None, &[]);
    drawer.standard_visual = Some(StandardVisual::ModalFrame {
        title: Arc::from("Inspector"),
        description: None,
        body_text: None,
        kind: nana_ui_runtime::ModalSurfaceKind::Drawer(DrawerSide::Right),
        busy: false,
        danger: false,
        slots: nana_ui_runtime::ModalSlots::default(),
    });
    drawer.component_geometry = Some(ComponentGeometry::ModalFrame {
        scrim: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 420.0,
            height: 240.0,
        },
        surface: LayoutBox {
            x: 60.0,
            y: 0.0,
            width: 360.0,
            height: 240.0,
        },
        body: LayoutBox {
            x: 76.0,
            y: 64.0,
            width: 328.0,
            height: 160.0,
        },
        title: ComponentTextRegion {
            bounds: LayoutBox {
                x: 76.0,
                y: 14.0,
                width: 280.0,
                height: 17.0,
            },
            content: Arc::from("Inspector"),
            color: Some([1.0; 4]),
            font_size: 14.0,
            font_weight: Some(600),
        },
        description: None,
        body_text: None,
        background: [0.1, 0.1, 0.1, 1.0],
        border: [0.0; 4],
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, 0.45],
            offset_x: 0.0,
            offset_y: 14.0,
            blur_radius: 30.0,
            spread_radius: 0.0,
            inset: false,
        },
    });
    let mut scene = UiScene::default();
    scene.apply_delta([drawer], []);
    let surface = scene
        .primitive(PrimitiveId {
            node: id(52),
            slot: 11,
        })
        .unwrap();
    assert!((surface.bounds.width - (360.0 + UI_METRICS.radius_md)).abs() < f32::EPSILON);
    assert!((surface.bounds.x - 60.0).abs() < f32::EPSILON);
    assert_eq!(surface.clips.len(), 1);
    assert!((surface.clips[0].bounds.width - 420.0).abs() < f32::EPSILON);
}

#[test]
fn confirm_action_scene_restores_label_after_busy_spinner_clears() {
    let mut action = node(51, None, &[]);
    let label = ComponentTextRegion {
        bounds: LayoutBox {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 24.0,
        },
        content: Arc::from("Delete"),
        color: Some([1.0; 4]),
        font_size: 13.0,
        font_weight: Some(500),
    };
    action.standard_visual = Some(StandardVisual::Button {
        label: Arc::from("Delete"),
        kind: nana_ui_core::ButtonKind::Danger,
        size: nana_ui_core::ControlSize::Medium,
        loading: true,
        loading_phase: 0.5,
        invalid: false,
    });
    action.component_geometry = Some(ComponentGeometry::Button {
        label: label.clone(),
        spinner: Some(LayoutBox {
            x: 42.0,
            y: 14.0,
            width: 16.0,
            height: 16.0,
        }),
        background: Some([0.8, 0.1, 0.1, 1.0]),
        border: None,
        border_width: 0.0,
        focus_ring: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([action.clone()], []);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(51),
                slot: 3
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Spinner { .. }
    ));

    action.standard_visual = Some(StandardVisual::Button {
        label: Arc::from("Delete"),
        kind: nana_ui_core::ButtonKind::Primary,
        size: nana_ui_core::ControlSize::Medium,
        loading: false,
        loading_phase: 0.0,
        invalid: false,
    });
    action.component_geometry = Some(ComponentGeometry::Button {
        label,
        spinner: None,
        background: Some([0.2, 0.4, 0.8, 1.0]),
        border: None,
        border_width: 0.0,
        focus_ring: None,
    });
    scene.apply_delta([action], []);
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(51),
                slot: 3
            })
            .is_none()
    );
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(51),
                slot: 2
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { ref content, .. } if content == "Delete"
    ));
}

#[test]
fn empty_state_separates_intrinsic_clip_from_focused_action_root_clip() {
    let content_clip = LayoutBox {
        x: 8.0,
        y: 9.0,
        width: 1.0,
        height: 2.0,
    };
    let mut empty = node(10, None, &[11]);
    empty.standard_visual = Some(StandardVisual::EmptyState {
        title: Arc::from("Title"),
        message: Some(Arc::from("Message")),
        icon: Some(nana_ui_core::Icon::Folder),
        compact: false,
        action: Some(id(11)),
    });
    empty.component_geometry = Some(ComponentGeometry::EmptyState {
        root_clip: empty.layout,
        content_clip,
        icon: Some((
            nana_ui_core::Icon::Folder,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 22.0,
                height: 22.0,
            },
            [0.5, 0.5, 0.5, 1.0],
        )),
        title: ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 22.0,
                width: 40.0,
                height: 30.0,
            },
            content: Arc::from("Title"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: Some(600),
        },
        message: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 58.0,
                width: 60.0,
                height: 40.0,
            },
            content: Arc::from("Message"),
            color: Some([0.7; 4]),
            font_size: 12.0,
            font_weight: None,
        }),
        action: Some(LayoutBox {
            x: 20.0,
            y: 53.0,
            width: 60.0,
            height: 24.0,
        }),
    });
    let mut action = node(11, Some(10), &[]);
    action.layout = LayoutBox {
        x: 20.0,
        y: 53.0,
        width: 60.0,
        height: 24.0,
    };
    action.focused = true;
    action.standard_visual = Some(StandardVisual::Button {
        label: Arc::from("Action"),
        kind: nana_ui_core::ButtonKind::Primary,
        size: nana_ui_core::ControlSize::Medium,
        loading: false,
        loading_phase: 0.0,
        invalid: false,
    });
    action.component_geometry = Some(ComponentGeometry::Button {
        label: ComponentTextRegion {
            bounds: LayoutBox {
                x: 20.0,
                y: 53.0,
                width: 60.0,
                height: 24.0,
            },
            content: Arc::from("Action"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        spinner: None,
        background: Some([0.2, 0.4, 0.8, 1.0]),
        border: None,
        border_width: 0.0,
        focus_ring: Some([0.3, 0.6, 1.0, 1.0]),
    });

    let mut scene = UiScene::new();
    scene.apply_delta([empty, action], []);
    let content = ClipRegion {
        bounds: scene_rect(content_clip),
        transform: AffineTransform::IDENTITY,
        corner_radius: 0.0,
        polygon_clip: None,
    };
    for primitive in [
        PrimitiveId {
            node: id(10),
            slot: 2,
        },
        PrimitiveId {
            node: id(10),
            slot: 3,
        },
        PrimitiveId {
            node: id(10),
            slot: 4,
        },
    ] {
        assert!(scene.primitive(primitive).unwrap().clips.contains(&content));
    }
    let root = ClipRegion {
        bounds: SceneRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        },
        transform: AffineTransform::IDENTITY,
        corner_radius: 0.0,
        polygon_clip: None,
    };
    for primitive in [
        PrimitiveId {
            node: id(11),
            slot: 2,
        },
        PrimitiveId {
            node: id(11),
            slot: 7,
        },
    ] {
        let primitive = scene.primitive(primitive).unwrap();
        assert!(primitive.clips.contains(&root));
        assert!(!primitive.clips.contains(&content));
    }
    let ring = scene
        .primitive(PrimitiveId {
            node: id(11),
            slot: 7,
        })
        .unwrap();
    assert_eq!(ring.bounds.y + ring.bounds.height, root.bounds.height);
}

#[test]
fn standard_control_visuals_expand_without_backend_tag_matching() {
    let mut checkbox = node(1, None, &[]);
    checkbox.text = Some(TextContent {
        value: "Notifications".into(),
    });
    checkbox.standard_visual = Some(StandardVisual::Checkbox {
        checked: true,
        indeterminate: false,
        size: nana_ui_core::ControlSize::Medium,
    });
    style_mut(&mut checkbox).background = Some([0.2, 0.5, 0.9, 1.0]);
    style_mut(&mut checkbox).border_color = Some([0.1, 0.2, 0.3, 1.0]);

    let mut slider = node(2, None, &[]);
    slider.standard_visual = Some(StandardVisual::Range {
        label: None,
        value: Arc::from("25"),
        unit: None,
        size: nana_ui_core::ControlSize::Medium,
        ratio: 0.25,
        invalid: false,
    });
    style_mut(&mut slider).background = Some([0.2, 0.5, 0.9, 1.0]);
    style_mut(&mut slider).border_color = Some([0.4, 0.4, 0.4, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([checkbox, slider], []);
    assert_eq!(scene.primitives().count(), 6);
    let checkbox_text = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .unwrap();
    assert_eq!(checkbox_text.bounds.x, 24.0);
    assert_eq!(checkbox_text.bounds.width, 76.0);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 4,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { ref content, .. } if content == "✓"
    ));
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 4,
            })
            .unwrap()
            .bounds
            .width,
        21.5
    );
}

#[test]
fn scrollbar_chrome_paints_ordinary_quads_over_the_scrollport() {
    let mut scroller = node(1, None, &[]);
    scroller.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 120.0,
    };
    {
        let layout = Arc::make_mut(&mut scroller.source_style.layout);
        layout.overflow_x = nana_ui_core::OverflowSpec::Scroll;
        layout.overflow_y = nana_ui_core::OverflowSpec::Scroll;
    }
    scroller.standard_visual = Some(StandardVisual::Scrollbar {
        axes: nana_ui_runtime::ScrollAxes::Vertical,
        visibility: nana_ui_core::ScrollbarVisibility::Always,
        revealed: true,
        dragging: None,
    });
    scroller.component_geometry = Some(ComponentGeometry::Scrollbar {
        horizontal: None,
        vertical: Some(nana_ui_runtime::ScrollbarBar {
            track: LayoutBox {
                x: 188.0,
                y: 0.0,
                width: 12.0,
                height: 120.0,
            },
            thumb: LayoutBox {
                x: 191.0,
                y: 0.0,
                width: 6.0,
                height: 72.0,
            },
            track_background: Some([0.1, 0.1, 0.1, 1.0]),
            thumb_background: [0.6, 0.6, 0.6, 1.0],
            thumb_radius: 3.0,
            max_offset: 80.0,
        }),
    });

    let mut scene = UiScene::new();
    scene.apply_delta([scroller], []);
    let track = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 3,
        })
        .expect("resident track");
    let thumb = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 4,
        })
        .expect("thumb");
    assert!(matches!(
        track.kind,
        ScenePrimitiveKind::Quad {
            background: Some([0.1, 0.1, 0.1, 1.0]),
            ..
        }
    ));
    assert!(matches!(
        thumb.kind,
        ScenePrimitiveKind::Quad {
            background: Some([0.6, 0.6, 0.6, 1.0]),
            corner_radius,
            ..
        } if corner_radius.iter().all(|r| (*r - 3.0).abs() < f32::EPSILON)
    ));
    assert_eq!(thumb.bounds.x, 191.0);
    assert_eq!(thumb.bounds.height, 72.0);
    // Both axes clip, so chrome shares the scrollport overflow clip and is
    // not cut by a tighter content clip.
    assert_eq!(
        track.clips.as_ref(),
        [ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 120.0,
            },
            transform: AffineTransform::default(),
            corner_radius: 0.0,
            polygon_clip: None,
        }]
    );
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 5,
            })
            .is_none(),
        "a vertical-only container emits no horizontal bar"
    );
}

#[test]
fn scrollbar_skin_thickness_still_paints_ordinary_quads() {
    let mut scroller = node(1, None, &[]);
    scroller.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 120.0,
    };
    scroller.standard_visual = Some(StandardVisual::Scrollbar {
        axes: nana_ui_runtime::ScrollAxes::Vertical,
        visibility: nana_ui_core::ScrollbarVisibility::Always,
        revealed: true,
        dragging: None,
    });
    scroller.component_geometry = Some(ComponentGeometry::Scrollbar {
        horizontal: None,
        vertical: Some(nana_ui_runtime::ScrollbarBar {
            track: LayoutBox {
                x: 192.0,
                y: 0.0,
                width: 8.0,
                height: 120.0,
            },
            thumb: LayoutBox {
                x: 194.0,
                y: 8.0,
                width: 4.0,
                height: 48.0,
            },
            track_background: Some([0.2, 0.2, 0.2, 1.0]),
            thumb_background: [1.0, 0.0, 0.0, 1.0],
            thumb_radius: 2.0,
            max_offset: 80.0,
        }),
    });

    let mut scene = UiScene::new();
    scene.apply_delta([scroller], []);
    let track = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 3,
        })
        .expect("skinned track");
    let thumb = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 4,
        })
        .expect("skinned thumb");
    assert!(matches!(
        track.kind,
        ScenePrimitiveKind::Quad {
            background: Some([0.2, 0.2, 0.2, 1.0]),
            ..
        }
    ));
    assert_eq!(track.bounds.width, 8.0);
    assert!(matches!(
        thumb.kind,
        ScenePrimitiveKind::Quad {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            corner_radius,
            ..
        } if corner_radius.iter().all(|r| (*r - 2.0).abs() < f32::EPSILON)
    ));
    assert_eq!(thumb.bounds.width, 4.0);
    assert_eq!(thumb.bounds.x, 194.0);
}

#[test]
fn focused_icon_does_not_paint_an_external_ring() {
    let mut icon = node(1, None, &[]);
    icon.layout = LayoutBox {
        x: 10.0,
        y: 20.0,
        width: 28.0,
        height: 28.0,
    };
    icon.focused = true;
    icon.standard_visual = Some(StandardVisual::Icon {
        icon: nana_ui_core::Icon::Settings,
        size: 16.0,
        tooltip: None,
    });
    style_mut(&mut icon).border_color = Some([0.2, 0.6, 1.0, 1.0]);
    style_mut(&mut icon).background = Some([0.2, 0.2, 0.2, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([icon], []);
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3
            })
            .is_some()
    );
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 7
            })
            .is_none()
    );
}

#[test]
fn compact_leading_icon_centers_on_the_parent_text_line() {
    let mut row = node(1, None, &[2]);
    row.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 160.0,
        height: 28.0,
    };
    row.text = Some(TextContent {
        value: "舞台".into(),
    });
    row.standard_visual = Some(StandardVisual::ListItem {
        leading: Some(id(2)),
        content: None,
        trailing: None,
        detail: None,
    });
    row.source_style.text_vertical_alignment = TextVerticalAlignment::Center;
    style_mut(&mut row).font_size = 12.0;
    style_mut(&mut row).line_height = Some(LineHeightSpec::Absolute(12.0));

    let mut icon = node(2, Some(1), &[]);
    icon.layout = LayoutBox {
        x: 8.0,
        y: 0.0,
        width: 12.0,
        height: 12.0,
    };
    icon.standard_visual = Some(StandardVisual::Icon {
        icon: nana_ui_core::Icon::Workspace,
        size: 12.0,
        tooltip: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([row, icon], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 3,
        })
        .expect("leading icon");
    assert_eq!(
        primitive.bounds,
        SceneRect {
            x: 8.0,
            y: 8.0,
            width: 12.0,
            height: 12.0,
        }
    );
}

#[test]
fn an_icon_trigger_paints_a_centered_glyph_instead_of_label_text() {
    let mut menu = node(1, None, &[]);
    menu.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 28.0,
        height: 28.0,
    };
    menu.standard_visual = Some(StandardVisual::MenuSurface {
        open: false,
        kind: nana_ui_runtime::MenuSurfaceKind::ActionMenu,
        trigger: None,
        trigger_icon: Some(nana_ui_core::Icon::Add),
        gap: 0.0,
        query: None,
        rows: Arc::from([]),
        highlighted: None,
    });
    menu.component_geometry = Some(ComponentGeometry::MenuSurface {
        trigger_surface: Some(nana_ui_runtime::ComponentTriggerSurface {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 28.0,
                height: 28.0,
            },
            background: Some([0.18, 0.18, 0.2, 1.0]),
            border: Some([0.4, 0.4, 0.45, 1.0]),
        }),
        trigger: None,
        trigger_icon: Some((
            nana_ui_core::Icon::Add,
            LayoutBox {
                x: 7.5,
                y: 7.5,
                width: 13.0,
                height: 13.0,
            },
        )),
        surface: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        },
        search: None,
        search_field: None,
        options: Vec::new(),
        elevation: ComponentElevation {
            color: [0.0, 0.0, 0.0, 0.0],
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 0.0,
            spread_radius: 0.0,
            inset: false,
        },
        background: [0.1, 0.1, 0.1, 1.0],
        border: [0.3, 0.3, 0.3, 1.0],
    });

    let mut scene = UiScene::new();
    scene.apply_delta([menu], []);
    let glyph = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .expect("icon trigger glyph");
    assert_eq!(
        glyph.bounds,
        SceneRect {
            x: 7.5,
            y: 7.5,
            width: 13.0,
            height: 13.0,
        }
    );
    assert_eq!(primitive_icon(&glyph.kind), Some(nana_ui_core::Icon::Add));
}

#[test]
fn a_focused_range_adds_a_thumb_focus_ring_and_keeps_the_rail_untouched() {
    let range = |focused: bool| {
        let mut range = node(1, None, &[]);
        range.focused = focused;
        range.standard_visual = Some(StandardVisual::Range {
            label: None,
            value: Arc::from("50"),
            unit: None,
            size: nana_ui_core::ControlSize::Medium,
            ratio: 0.5,
            invalid: false,
        });
        style_mut(&mut range).border_color = Some([0.4, 0.4, 0.45, 1.0]);
        range.standard_visual_foreground = Some([0.2, 0.5, 1.0, 1.0]);
        range
    };

    let mut idle = UiScene::new();
    idle.apply_delta([range(false)], []);
    assert!(
        idle.primitive(PrimitiveId {
            node: id(1),
            slot: 6
        })
        .is_none(),
        "an unfocused range paints no focus ring"
    );

    let mut focused = UiScene::new();
    focused.apply_delta([range(true)], []);
    let ring = focused
        .primitive(PrimitiveId {
            node: id(1),
            slot: 6,
        })
        .expect("focused range paints a thumb focus ring");
    // Thumb centre for ratio 0.5 on the fallback track band (7..93).
    assert_eq!(
        ring.bounds,
        SceneRect {
            x: 40.0,
            y: 30.0,
            width: 20.0,
            height: 20.0,
        }
    );
    match &ring.kind {
        ScenePrimitiveKind::Quad {
            background: None,
            border_color: Some([0.2, 0.5, 1.0, 1.0]),
            border_width: 2.0,
            ..
        } => {}
        other => panic!("focus ring must outline the thumb in accent, got {other:?}"),
    }
    // The rail (slot 3) keeps the interaction-free border colour.
    match &focused
        .primitive(PrimitiveId {
            node: id(1),
            slot: 3,
        })
        .expect("range rail")
        .kind
    {
        ScenePrimitiveKind::Quad {
            background: Some([0.4, 0.4, 0.45, 1.0]),
            ..
        } => {}
        other => panic!("rail must stay on the base border colour, got {other:?}"),
    }
}

#[test]
fn migrated_components_consume_runtime_subregion_geometry() {
    let mut icon = node(1, None, &[]);
    icon.layout = LayoutBox {
        x: 10.0,
        y: 20.0,
        width: 32.0,
        height: 32.0,
    };
    icon.standard_visual = Some(StandardVisual::Icon {
        icon: nana_ui_core::Icon::Search,
        size: 16.0,
        tooltip: None,
    });
    style_mut(&mut icon).background = Some([0.2, 0.3, 0.4, 1.0]);
    style_mut(&mut icon).border_color = Some([0.4, 0.5, 0.6, 1.0]);
    style_mut(&mut icon).color = Some([0.9, 0.9, 0.9, 1.0]);
    icon.standard_visual_foreground = Some([0.1, 0.6, 0.9, 1.0]);

    let mut switch = node(2, None, &[]);
    switch.standard_visual = Some(StandardVisual::Switch {
        thumb_progress: 1.0,
        label: Arc::from("Enabled"),
        hint: Some(Arc::from("Starts with the workspace")),
        checked: true,
        control_position: nana_ui_core::SwitchControlPosition::End,
        size: nana_ui_core::ControlSize::Medium,
        loading: false,
        loading_phase: 0.0,
        invalid: false,
    });
    switch.component_geometry = Some(ComponentGeometry::Switch {
        thumb_progress: 1.0,
        label: ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 8.0,
                width: 100.0,
                height: 18.0,
            },
            content: Arc::from("Enabled"),
            color: Some([0.9, 0.9, 0.9, 1.0]),
            font_size: 13.0,
            font_weight: Some(500),
        },
        hint: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 28.0,
                width: 150.0,
                height: 16.0,
            },
            content: Arc::from("Starts with the workspace"),
            color: Some([0.6, 0.6, 0.6, 1.0]),
            font_size: 12.0,
            font_weight: Some(400),
        }),
        control: LayoutBox {
            x: 170.0,
            y: 18.0,
            width: 30.0,
            height: 16.0,
        },
        track_background: [0.2, 0.5, 0.9, 1.0],
        track_border: [0.1, 0.4, 0.8, 1.0],
        thumb_background: [1.0, 1.0, 1.0, 1.0],
    });
    switch.layout.width = 200.0;
    switch.layout.height = 52.0;
    switch.focused = true;

    let mut range = node(3, None, &[]);
    range.standard_visual = Some(StandardVisual::Range {
        label: Some(Arc::from("Opacity")),
        value: Arc::from("25"),
        unit: Some(Arc::from("%")),
        size: nana_ui_core::ControlSize::Medium,
        ratio: 0.25,
        invalid: false,
    });
    range.component_geometry = Some(ComponentGeometry::Range {
        label: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 11.0,
                width: 52.0,
                height: 18.0,
            },
            content: Arc::from("Opacity"),
            color: None,
            font_size: 13.0,
            font_weight: Some(500),
        }),
        value: ComponentTextRegion {
            bounds: LayoutBox {
                x: 210.0,
                y: 11.0,
                width: 20.0,
                height: 18.0,
            },
            content: Arc::from("25"),
            color: None,
            font_size: 13.0,
            font_weight: None,
        },
        unit: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 232.0,
                y: 11.0,
                width: 8.0,
                height: 18.0,
            },
            content: Arc::from("%"),
            color: None,
            font_size: 13.0,
            font_weight: None,
        }),
        track: LayoutBox {
            x: 80.0,
            y: 12.0,
            width: 120.0,
            height: 16.0,
        },
    });
    range.layout.width = 240.0;
    range.layout.height = 40.0;
    style_mut(&mut range).background = Some([0.2, 0.5, 0.9, 1.0]);
    style_mut(&mut range).border_color = Some([0.4, 0.4, 0.4, 1.0]);

    let mut card = node(4, None, &[]);
    card.standard_visual = Some(StandardVisual::Card {
        title: Some(Arc::from("Actions")),
        kind: nana_ui_core::CardKind::Surface,
        loading: true,
        loading_phase: 0.5,
    });
    style_mut(&mut card).background = Some([0.12, 0.12, 0.12, 1.0]);
    style_mut(&mut card).border_color = Some([0.3, 0.3, 0.3, 1.0]);
    card.component_geometry = Some(ComponentGeometry::Card {
        title: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 10.0,
                y: 8.0,
                width: 50.0,
                height: 18.0,
            },
            content: Arc::from("Actions"),
            color: None,
            font_size: 13.0,
            font_weight: Some(600),
        }),
        content: LayoutBox {
            x: 10.0,
            y: 36.0,
            width: 80.0,
            height: 34.0,
        },
        elevation: Some(ComponentElevation {
            color: [0.0, 0.0, 0.0, 0.25],
            offset_x: 0.0,
            offset_y: 3.0,
            blur_radius: 8.0,
            spread_radius: 0.0,
            inset: false,
        }),
        spinner: Some(LayoutBox {
            x: 68.0,
            y: 10.0,
            width: 14.0,
            height: 14.0,
        }),
    });

    let mut list_item = node(5, None, &[]);
    list_item.text = Some(TextContent {
        value: "Project".into(),
    });
    list_item.standard_visual = Some(StandardVisual::ListItem {
        leading: None,
        content: None,
        trailing: None,
        detail: None,
    });
    list_item.component_geometry = Some(ComponentGeometry::ListItem {
        leading: None,
        content: Some(LayoutBox {
            x: 30.0,
            y: 6.0,
            width: 55.0,
            height: 22.0,
        }),
        trailing: None,
        detail: None,
    });
    style_mut(&mut list_item).background = Some([0.15, 0.15, 0.15, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([icon, switch, range, card, list_item], []);

    let icon = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 3,
        })
        .unwrap();
    assert_eq!(
        icon.bounds,
        SceneRect {
            x: 18.0,
            y: 28.0,
            width: 16.0,
            height: 16.0,
        }
    );
    assert_eq!(primitive_icon(&icon.kind), Some(nana_ui_core::Icon::Search));
    match &icon.kind {
        ScenePrimitiveKind::Icon {
            color: Some(color), ..
        } => assert_eq!(*color, [0.1, 0.6, 0.9, 1.0]),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            background: Some([0.2, 0.3, 0.4, 1.0]),
            border_color: Some([0.4, 0.5, 0.6, 1.0]),
            ..
        }
    ));

    let switch_track = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 4,
        })
        .unwrap();
    assert_eq!(switch_track.bounds.width, 30.0);
    assert_eq!(switch_track.bounds.height, 16.0);
    assert_eq!(switch_track.bounds.x, 170.0);
    let switch_thumb = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 5,
        })
        .unwrap();
    assert_eq!(switch_thumb.bounds.width, 10.0);
    assert_eq!(switch_thumb.bounds.x, 187.0);
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2,
            })
            .unwrap()
            .bounds
            .height,
        18.0
    );
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 3,
            })
            .unwrap()
            .bounds
            .y,
        28.0
    );
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 7,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            border_width: 2.0,
            ..
        }
    ));

    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 4,
            })
            .unwrap()
            .bounds
            .width,
        30.0
    );
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 5,
            })
            .unwrap()
            .bounds
            .x,
        103.0
    );
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(3),
                slot: 6,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text {
            horizontal_alignment: TextHorizontalAlignment::End,
            ..
        }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(4),
                slot: 3,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Spinner { phase: 4, .. }
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(4),
                slot: 0,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Quad {
            shadow: Some(ComponentElevation {
                offset_x: 0.0,
                offset_y: 3.0,
                blur_radius: 8.0,
                ..
            }),
            ..
        }
    ));
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(4),
                slot: 2,
            })
            .unwrap()
            .bounds
            .y,
        8.0
    );
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(4),
                slot: 2,
            })
            .unwrap()
            .kind,
        ScenePrimitiveKind::Text { ellipsis: true, .. }
    ));
    assert_eq!(
        scene
            .primitive(PrimitiveId {
                node: id(5),
                slot: 2,
            })
            .unwrap()
            .bounds,
        SceneRect {
            x: 30.0,
            y: 6.0,
            width: 55.0,
            height: 22.0,
        }
    );
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(5),
                slot: 0,
            })
            .is_some()
    );
}

#[test]
fn subtree_membership_follows_retained_parent_links() {
    let scene = {
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                node(1, None, &[2]),
                node(2, Some(1), &[3]),
                node(3, Some(2), &[]),
            ],
            [],
        );
        scene
    };
    assert!(scene.is_node_in_subtree(id(1), id(3)));
    assert!(scene.is_node_in_subtree(id(2), id(2)));
    assert!(!scene.is_node_in_subtree(id(3), id(1)));
}

#[test]
fn scroll_offset_transforms_descendants_but_not_viewport_clip() {
    let mut scroller = node(1, None, &[2]);
    scroller.layout.height = 50.0;
    scroller.scroll_offset = nana_ui_runtime::ScrollOffset { x: 0.0, y: 60.0 };
    scroller.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            overflow_y: nana_ui_core::OverflowSpec::Scroll,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.layout.y = 80.0;
    child.text = Some(TextContent {
        value: "Visible".into(),
    });

    let mut scene = UiScene::new();
    scene.apply_delta([scroller, child], []);
    let text = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 2,
        })
        .unwrap();
    assert_eq!(text.transform.0[5], -60.0);
    assert_eq!(text.bounds.y, 80.0);
    assert_eq!(text.clips.len(), 1);
    assert_eq!(text.clips[0].bounds.height, 50.0);
    assert_eq!(text.clips[0].transform, AffineTransform::IDENTITY);
}

#[test]
fn scroll_offset_delta_rebuilds_descendant_primitives_without_reextracting_them() {
    let mut scroller = node(1, None, &[2]);
    scroller.layout.height = 50.0;
    scroller.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            overflow_y: nana_ui_core::OverflowSpec::Scroll,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.layout.y = 80.0;
    child.text = Some(TextContent {
        value: "Visible".into(),
    });

    let mut scene = UiScene::new();
    scene.apply_delta([scroller.clone(), child], []);
    scroller.scroll_offset = nana_ui_runtime::ScrollOffset { x: 0.0, y: 60.0 };
    let delta = scene.apply_delta([scroller], []);
    assert_eq!(delta.updated_nodes, 1);
    let text = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 2,
        })
        .unwrap();
    assert_eq!(
        text.transform.0[5], -60.0,
        "extracting only the scroller must recompose descendant paint transforms"
    );
    assert_eq!(text.bounds.y, 80.0);
}

#[test]
#[cfg(feature = "graph-canvas")]
fn graph_canvas_high_slots_stay_in_paint_order_across_incremental_updates() {
    let edge = vec![[8.0, 20.0], [48.0, 22.0]];
    let geometry = |edges: Vec<(Vec<[f32; 2]>, [f32; 4])>| ComponentGeometry::GraphCanvas {
        nodes: Vec::new(),
        separators: Vec::new(),
        ports: Vec::new(),
        port_labels: Vec::new(),
        edges,
        edge_labels: Vec::new(),
        grid: Vec::new(),
        background: [0.1, 0.1, 0.1, 1.0],
        grid_color: [0.2, 0.2, 0.2, 0.5],
        separator_color: [0.2, 0.2, 0.2, 1.0],
    };
    let mut canvas = node(1, None, &[]);
    canvas.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 120.0,
    };
    canvas.standard_visual = Some(StandardVisual::GraphCanvas {
        nodes: Arc::from([]),
        ports: Arc::from([]),
        edges: Arc::from([]),
        connecting: None,
        grid_spacing: 24.0,
        viewport_offset_x: 0.0,
        viewport_offset_y: 0.0,
        viewport_zoom: 1.0,
    });
    canvas.component_geometry = Some(geometry(vec![(edge.clone(), [0.5, 0.5, 0.5, 1.0])]));

    let mut scene = UiScene::new();
    scene.apply_delta([canvas.clone()], []);
    assert!(scene.primitives().any(|primitive| primitive.id.slot == 12));
    assert!(!scene.primitives().any(|primitive| primitive.id.slot == 13));

    canvas.component_geometry = Some(geometry(vec![
        (edge.clone(), [0.5, 0.5, 0.5, 1.0]),
        (edge.clone(), [0.2, 0.6, 1.0, 1.0]),
    ]));
    scene.apply_delta([canvas.clone()], []);
    assert!(
        scene.primitives().any(|primitive| primitive.id.slot == 12),
        "base edge batch must remain in paint order"
    );
    assert!(
        scene.primitives().any(|primitive| primitive.id.slot == 13),
        "selected/connecting overlay must enter paint order on the next extract"
    );

    canvas.component_geometry = Some(geometry(vec![(edge, [0.5, 0.5, 0.5, 1.0])]));
    scene.apply_delta([canvas], []);
    assert!(scene.primitives().any(|primitive| primitive.id.slot == 12));
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 13
            })
            .is_none(),
        "unused high slots must be removed instead of leaving a stale overlay"
    );
}

#[test]
#[cfg(feature = "charts")]
#[cfg(feature = "rich-text")]
fn new_component_geometry_paints_owned_quads_and_skips_generic_text() {
    let mut chart = node(1, None, &[]);
    chart.standard_visual = Some(StandardVisual::TimeSeriesChart {
        values: Arc::from([0.0, 1.0]),
    });
    chart.component_geometry = Some(ComponentGeometry::TimeSeriesChart {
        grid: vec![LayoutBox {
            x: 8.0,
            y: 10.0,
            width: 92.0,
            height: 1.0,
        }],
        area: vec![LayoutBox {
            x: 8.0,
            y: 40.0,
            width: 2.0,
            height: 70.0,
        }],
        line: vec![[8.0, 40.0], [54.0, 40.0]],
        grid_color: [0.2, 0.2, 0.2, 0.55],
        area_color: [0.3, 0.5, 0.8, 0.16],
        line_color: [0.3, 0.5, 0.9, 1.0],
    });

    let mut markdown = node(2, None, &[]);
    markdown.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 120.0,
        height: 48.0,
    };
    markdown.text = Some(TextContent {
        value: "hello".into(),
    });
    markdown.standard_visual = Some(StandardVisual::NativeMarkdown {
        text: Arc::from("hello"),
        selection: Some((0, 5)),
    });
    markdown.component_geometry = Some(ComponentGeometry::NativeMarkdown {
        text: ComponentTextRegion {
            bounds: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 48.0,
            },
            content: Arc::from("hello"),
            color: Some([1.0, 1.0, 1.0, 1.0]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: vec![LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 48.0,
        }],
        selection_color: [0.2, 0.4, 0.8, 0.14],
    });

    let mut scene = UiScene::new();
    scene.apply_delta([chart, markdown], []);

    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 10
            })
            .map(|primitive| &primitive.kind),
        Some(ScenePrimitiveKind::QuadBatch { .. })
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 11
            })
            .map(|primitive| &primitive.kind),
        Some(ScenePrimitiveKind::QuadBatch { .. })
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 12
            })
            .map(|primitive| &primitive.kind),
        Some(ScenePrimitiveKind::Stroke {
            width,
            points,
            widths,
            cap: StrokeCap::Round,
            pattern: None,
            ..
        }) if (*width - TimeSeriesChart::LINE_WIDTH).abs() < f32::EPSILON
            && points.len() == 2
            && widths.is_empty()
    ));
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2
            })
            .is_none(),
        "time series does not emit generic text"
    );

    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 1
            })
            .map(|primitive| &primitive.kind),
        Some(ScenePrimitiveKind::QuadBatch {
            background: Some([0.2, 0.4, 0.8, 0.14]),
            ..
        })
    ));
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2
            })
            .map(|primitive| &primitive.kind),
        Some(ScenePrimitiveKind::Text {
            content,
            wrap: true,
            ..
        }) if content == "hello"
    ));
    let text_primitives = scene
        .primitives()
        .filter(|primitive| {
            primitive.node == id(2) && matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
        })
        .count();
    assert_eq!(
        text_primitives, 1,
        "markdown must not double-paint generic text"
    );
}

fn visible_text_count(scene: &UiScene, nodes: &[StableNodeId]) -> usize {
    scene
        .primitives()
        .filter(|primitive| {
            nodes.contains(&primitive.node)
                && matches!(
                    &primitive.kind,
                    ScenePrimitiveKind::Text { content, .. } if !content.trim().is_empty()
                )
        })
        .count()
}

fn text_node(id: u64, parent: u64, value: &str) -> ExtractedNode {
    let mut child = node(id, Some(parent), &[]);
    child.kind = Arc::new(NodeKind::Text);
    child.text = Some(TextContent {
        value: value.into(),
    });
    child
}

#[test]
fn host_and_child_text_extract_one_visible_text_primitive() {
    let mut button = node(1, None, &[2]);
    button.kind = Arc::new(NodeKind::Element {
        tag: "button".into(),
    });
    button.text = Some(TextContent {
        value: "Open".into(),
    });
    let label = ComponentTextRegion {
        bounds: LayoutBox {
            x: 8.0,
            y: 8.0,
            width: 48.0,
            height: 20.0,
        },
        content: Arc::from("Open"),
        color: Some([0.1, 0.1, 0.1, 1.0]),
        font_size: 13.0,
        font_weight: Some(500),
    };
    button.standard_visual = Some(StandardVisual::Button {
        label: Arc::from("Open"),
        kind: nana_ui_core::ButtonKind::Ghost,
        size: nana_ui_core::ControlSize::Medium,
        loading: false,
        loading_phase: 0.0,
        invalid: false,
    });
    button.component_geometry = Some(ComponentGeometry::Button {
        label,
        spinner: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([button, text_node(2, 1, "Open")], []);
    assert_eq!(visible_text_count(&scene, &[id(1), id(2)]), 1);

    let mut heading = node(3, None, &[4]);
    heading.kind = Arc::new(NodeKind::Element { tag: "h1".into() });
    heading.text = Some(TextContent {
        value: "Title".into(),
    });
    scene.apply_delta([heading, text_node(4, 3, "Title")], []);
    assert_eq!(visible_text_count(&scene, &[id(3), id(4)]), 1);
}

#[test]
fn card_child_list_item_keeps_its_label() {
    let mut card = node(1, None, &[2]);
    card.text = Some(TextContent {
        value: "Outputs".into(),
    });
    card.standard_visual = Some(StandardVisual::Card {
        title: Some(Arc::from("Outputs")),
        kind: nana_ui_core::CardKind::Surface,
        loading: false,
        loading_phase: 0.0,
    });
    card.component_geometry = Some(ComponentGeometry::Card {
        title: Some(ComponentTextRegion {
            bounds: LayoutBox {
                x: 10.0,
                y: 8.0,
                width: 80.0,
                height: 18.0,
            },
            content: Arc::from("Outputs"),
            color: None,
            font_size: 13.0,
            font_weight: Some(600),
        }),
        content: LayoutBox {
            x: 10.0,
            y: 36.0,
            width: 160.0,
            height: 36.0,
        },
        elevation: None,
        spinner: None,
    });

    let mut item = node(2, Some(1), &[]);
    item.kind = Arc::new(NodeKind::Element {
        tag: "list-item".into(),
    });
    item.text = Some(TextContent {
        value: "Window".into(),
    });
    item.standard_visual = Some(StandardVisual::ListItem {
        leading: None,
        content: None,
        trailing: None,
        detail: None,
    });
    item.component_geometry = Some(ComponentGeometry::ListItem {
        leading: None,
        content: Some(LayoutBox {
            x: 10.0,
            y: 36.0,
            width: 160.0,
            height: 36.0,
        }),
        trailing: None,
        detail: None,
    });

    let mut scene = UiScene::new();
    scene.apply_delta([card, item], []);
    assert_eq!(visible_text_count(&scene, &[id(1), id(2)]), 2);
    assert!(matches!(
        scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2,
            })
            .map(|primitive| &primitive.kind),
        Some(ScenePrimitiveKind::Text { content, .. }) if content == "Window"
    ));
}

#[test]
fn css_gradient_and_clip_path_surface_paint_travels_on_quad() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 1.0, 1.0, 0.5]),
            paint: nana_ui_core::PaintStyle {
                background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                    nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                        angle_deg: 180.0,
                        stops: vec![
                            nana_ui_core::GradientStop {
                                position: 0.0,
                                color: [1.0, 1.0, 1.0, 1.0],
                            },
                            nana_ui_core::GradientStop {
                                position: 1.0,
                                color: [1.0, 1.0, 1.0, 0.0],
                            },
                        ],
                    }),
                )),
                clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                    top: nana_ui_core::LengthSpec::Percent(50.0),
                    right: nana_ui_core::LengthSpec::Percent(50.0),
                    bottom: nana_ui_core::LengthSpec::Percent(50.0),
                    left: nana_ui_core::LengthSpec::Percent(50.0),
                    round: None,
                })),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 0.5]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    assert_eq!(
        primitive.clips.len(),
        1,
        "inset clip-path adds a clip region"
    );
    let clip = &primitive.clips[0];
    assert!((clip.bounds.width - 0.0).abs() < 0.01);
    assert!((clip.bounds.height - 0.0).abs() < 0.01);
    match &primitive.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            assert!(surface.background_image.is_some());
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn css_clip_path_inset_round_applies_surface_corner_radius() {
    let mut painted = node(1, None, &[]);
    painted.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 80.0,
    };
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                    top: nana_ui_core::LengthSpec::Px(10.0),
                    right: nana_ui_core::LengthSpec::Px(10.0),
                    bottom: nana_ui_core::LengthSpec::Px(10.0),
                    left: nana_ui_core::LengthSpec::Px(10.0),
                    round: Some(nana_ui_core::LengthSpec::Px(8.0)),
                })),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    assert!(
        (primitive.clips[0].corner_radius - 8.0).abs() < f32::EPSILON,
        "inset round travels on clip region"
    );
    match &primitive.kind {
        ScenePrimitiveKind::Quad { corner_radius, .. } => {
            assert!(
                corner_radius
                    .iter()
                    .all(|r| (*r - 8.0).abs() < f32::EPSILON),
                "inset round applies to owning quad radii, got {corner_radius:?}"
            );
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn css_clip_path_inset_clips_text_child() {
    let mut parent = node(1, None, &[2]);
    parent.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 80.0,
    };
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                    top: nana_ui_core::LengthSpec::Px(10.0),
                    right: nana_ui_core::LengthSpec::Px(10.0),
                    bottom: nana_ui_core::LengthSpec::Px(10.0),
                    left: nana_ui_core::LengthSpec::Px(10.0),
                    round: None,
                })),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut child = node(2, Some(1), &[]);
    child.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 80.0,
    };
    child.text = Some(TextContent {
        value: "child".into(),
    });

    let mut scene = UiScene::new();
    scene.apply_delta([parent, child], []);
    let text = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 2,
        })
        .expect("text child");
    assert_eq!(text.clips.len(), 1);
    let clip = &text.clips[0];
    assert!((clip.bounds.x - 10.0).abs() < 0.01);
    assert!((clip.bounds.y - 10.0).abs() < 0.01);
    assert!((clip.bounds.width - 80.0).abs() < 0.01);
    assert!((clip.bounds.height - 60.0).abs() < 0.01);
}

#[test]
fn css_filter_group_omits_leaf_shader_on_parent_quad() {
    let mut parent = node(1, None, &[2]);
    parent.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 40.0,
    };
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                filter: Some(nana_ui_core::ColorFilter {
                    brightness: 0.5,
                    saturate: 1.0,
                    contrast: 1.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut parent).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut child = node(2, Some(1), &[]);
    child.layout = LayoutBox {
        x: 0.0,
        y: 40.0,
        width: 100.0,
        height: 20.0,
    };

    let mut scene = UiScene::new();
    scene.apply_delta([parent, child], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("parent quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            assert!(surface.filter.is_none(), "filter group owns parent filter");
        }
        other => panic!("expected quad, got {other:?}"),
    }
    assert_eq!(scene.filter_groups(id(2)).len(), 1);
    assert_eq!(
        scene.opacity_groups(id(2)),
        vec![OpacityGroup {
            node: id(1),
            opacity: 1.0,
            filter: nana_ui_core::ColorFilter {
                brightness: 0.5,
                saturate: 1.0,
                contrast: 1.0,
                ..Default::default()
            },
            mix_blend: MixBlendMode::Normal,
            inset_shadow: None,
        }]
    );
}

#[test]
fn css_mix_blend_and_element_blur_isolate_dest_groups() {
    let mut blended = node(1, None, &[]);
    blended.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                mix_blend: MixBlendMode::Multiply,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut blended).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([blended], []);
    assert_eq!(
        scene.opacity_groups(id(1)),
        vec![OpacityGroup {
            node: id(1),
            opacity: 1.0,
            filter: ColorFilter::default(),
            mix_blend: MixBlendMode::Multiply,
            inset_shadow: None,
        }]
    );

    let mut blurred = node(2, None, &[]);
    blurred.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.0, 1.0, 0.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                filter: Some(nana_ui_core::ColorFilter {
                    blur_radius: 8.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut blurred).background = Some([0.0, 1.0, 0.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([blurred], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(2),
            slot: 0,
        })
        .expect("blur quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            assert!(surface.filter.is_none(), "element blur is dest-group owned");
        }
        other => panic!("expected quad, got {other:?}"),
    }
    assert_eq!(scene.filter_groups(id(2)).len(), 1);
    assert_eq!(scene.opacity_groups(id(2))[0].filter.blur_radius, 8.0);
}

#[test]
fn css_drop_shadow_isolates_dest_group_not_box_shadow() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                filter: Some(nana_ui_core::ColorFilter {
                    drop_shadow: Some(nana_ui_core::FilterDropShadow {
                        offset_x: 4.0,
                        offset_y: 6.0,
                        blur_radius: 8.0,
                        color: [0.0, 0.0, 0.0, 0.5],
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("drop-shadow quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad {
            surface, shadow, ..
        } => {
            assert!(
                surface.filter.is_none(),
                "drop-shadow is dest-group owned, not leaf shader"
            );
            assert!(
                shadow.is_none(),
                "drop-shadow must not reuse box-shadow quads"
            );
        }
        other => panic!("expected quad, got {other:?}"),
    }
    assert_eq!(scene.filter_groups(id(1)).len(), 1);
    let group = &scene.opacity_groups(id(1))[0];
    let shadow = group.filter.drop_shadow.expect("dest-group drop-shadow");
    assert!((shadow.offset_x - 4.0).abs() < 0.01);
    assert!((shadow.offset_y - 6.0).abs() < 0.01);
    assert!((shadow.blur_radius - 8.0).abs() < 0.01);
}

#[test]
fn css_box_shadow_layers_outline_and_line_clamp_travel() {
    let mut painted = node(1, None, &[]);
    painted.text = Some(TextContent {
        value: "clamped".into(),
    });
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 1.0, 1.0, 1.0]),
            line_clamp: Some(2),
            text_overflow_ellipsis: true,
            paint: nana_ui_core::PaintStyle {
                box_shadows: vec![
                    nana_ui_core::BoxShadowSpec {
                        offset_x: 2.0,
                        offset_y: 2.0,
                        blur_radius: 4.0,
                        spread_radius: 0.0,
                        color: [0.0, 0.0, 0.0, 1.0],
                        inset: true,
                    },
                    nana_ui_core::BoxShadowSpec {
                        offset_x: 0.0,
                        offset_y: 4.0,
                        blur_radius: 8.0,
                        spread_radius: 0.0,
                        color: [0.0, 0.0, 0.0, 0.5],
                        inset: false,
                    },
                ],
                outline: nana_ui_core::OutlineSpec {
                    width: 2.0,
                    color: Some([1.0, 0.0, 0.0, 1.0]),
                    style: nana_ui_core::OutlineStyle::Solid,
                },
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("shadow quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad {
            shadow, surface, ..
        } => {
            assert!(shadow.is_some_and(|s| s.inset));
            assert_eq!(surface.extra_shadows.len(), 1);
            assert!(!surface.extra_shadows[0].inset);
            assert!((surface.outline_width - 2.0).abs() < f32::EPSILON);
        }
        other => panic!("expected quad, got {other:?}"),
    }
    let text = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 2,
        })
        .expect("text");
    match &text.kind {
        ScenePrimitiveKind::Text {
            max_lines,
            ellipsis,
            wrap,
            ..
        } => {
            assert_eq!(*max_lines, Some(2));
            assert!(*ellipsis);
            assert!(*wrap);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn inset_box_shadow_on_a_leaf_is_not_an_outset_elevation() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 1.0, 1.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                box_shadows: vec![nana_ui_core::BoxShadowSpec {
                    offset_x: 2.0,
                    offset_y: 4.0,
                    blur_radius: 6.0,
                    spread_radius: 1.0,
                    color: [0.0, 0.0, 0.0, 0.5],
                    inset: true,
                }],
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("leaf quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad {
            shadow: Some(elevation),
            ..
        } => {
            assert!(
                elevation.inset,
                "leaf inset must travel as inset, not an outset drop shadow"
            );
            assert!((elevation.offset_y - 4.0).abs() < f32::EPSILON);
        }
        other => panic!("expected inset shadow quad, got {other:?}"),
    }
    assert!(
        scene.opacity_groups(id(1)).is_empty(),
        "a leaf inset does not open a dest group"
    );
}

#[test]
fn inset_box_shadow_with_children_is_a_dest_group_not_parent_quad() {
    let mut parent = node(1, None, &[2]);
    parent.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 40.0,
    };
    parent.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                box_shadows: vec![nana_ui_core::BoxShadowSpec {
                    offset_x: 0.0,
                    offset_y: 2.0,
                    blur_radius: 4.0,
                    spread_radius: 0.0,
                    color: [0.0, 0.0, 0.0, 0.4],
                    inset: true,
                }],
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut parent).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut child = node(2, Some(1), &[]);
    child.layout = LayoutBox {
        x: 4.0,
        y: 4.0,
        width: 20.0,
        height: 20.0,
    };
    style_mut(&mut child).background = Some([0.0, 1.0, 0.0, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([parent, child], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("parent quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad {
            shadow: Some(elevation),
            ..
        } => {
            assert!(
                elevation.inset,
                "origin paints inset on the parent quad (PAINT_SHADOW_INSET)"
            );
            assert!((elevation.offset_y - 2.0).abs() < f32::EPSILON);
        }
        other => panic!("expected inset shadow quad, got {other:?}"),
    }
    assert!(
        scene.opacity_groups(id(2)).is_empty(),
        "inset-only parents are not dest groups; origin opacity stacking is filter/opacity/mix-blend"
    );
}

#[test]
fn css_mask_and_gradient_both_travel_on_quad() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.2, 0.2, 0.2, 1.0]),
            paint: nana_ui_core::PaintStyle {
                background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                    nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                        angle_deg: 90.0,
                        stops: vec![nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [1.0, 0.0, 0.0, 1.0],
                        }],
                    }),
                )),
                mask: Some(nana_ui_core::MaskImage::Gradient(
                    nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                        angle_deg: 180.0,
                        stops: vec![nana_ui_core::GradientStop {
                            position: 0.0,
                            color: [0.0, 0.0, 0.0, 1.0],
                        }],
                    }),
                )),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([0.2, 0.2, 0.2, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    match &primitive.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            assert!(surface.background_image.is_some());
            assert!(surface.mask.is_some());
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn css_backdrop_filter_travels_on_quad_surface() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 1.0, 1.0, 0.4]),
            paint: nana_ui_core::PaintStyle {
                backdrop_filter: Some(nana_ui_core::BackdropFilter {
                    blur_radius: 16.0,
                    saturate: 1.2,
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 0.4]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    match &primitive.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            let backdrop = surface
                .backdrop_filter
                .expect("backdrop-filter must travel to scene quad");
            assert!((backdrop.blur_radius - 16.0).abs() < 0.01);
            assert!((backdrop.saturate - 1.2).abs() < 0.01);
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn img_content_image_and_two_background_layers_travel_on_quad() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                background_image: Some(nana_ui_core::BackgroundImage::url("fg.png")),
                background_layers: vec![nana_ui_core::BackgroundImage::url("bg.png")],
                content_image: Some(nana_ui_core::BackgroundImage::url_with_fit(
                    "photo.png",
                    nana_ui_core::BackgroundImageFit::Contain,
                )),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([0.1, 0.1, 0.1, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    match &primitive.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            assert_eq!(
                surface
                    .background_image
                    .as_ref()
                    .and_then(|image| image.url_str()),
                Some("fg.png")
            );
            assert_eq!(surface.background_layers.len(), 1);
            assert_eq!(
                surface
                    .content_image
                    .as_ref()
                    .and_then(|image| image.url_str()),
                Some("photo.png")
            );
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn border_image_travels_on_quad() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                border_image: Some(nana_ui_core::BorderImageSpec {
                    source: nana_ui_core::BackgroundImage::url("frame.png"),
                    slice: [nana_ui_core::BorderImageSlice::Number(30.0); 4],
                    fill: true,
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([0.1, 0.1, 0.1, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    match &primitive.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            let spec = surface.border_image.as_ref().expect("border-image");
            assert_eq!(spec.source.url_str(), Some("frame.png"));
            assert!(spec.fill);
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn unsupported_border_image_does_not_travel_on_quad() {
    let mut painted = node(1, None, &[]);
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                unsupported_border_image: true,
                border_image: Some(nana_ui_core::BorderImageSpec {
                    source: nana_ui_core::BackgroundImage::url("frame.png"),
                    slice: [nana_ui_core::BorderImageSlice::Number(30.0); 4],
                    fill: true,
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([0.1, 0.1, 0.1, 1.0]);

    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    match &primitive.kind {
        ScenePrimitiveKind::Quad { surface, .. } => {
            assert!(
                surface.border_image.is_none(),
                "sticky unsupported must not project a 9-slice"
            );
        }
        other => panic!("expected quad, got {other:?}"),
    }
}

#[test]
fn host_texture_custom_carries_css_mask() {
    let mut painted = node(1, None, &[]);
    painted.custom_render = Some(CustomRenderNode::new("nana.host-texture", "preview", 1));
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            paint: nana_ui_core::PaintStyle {
                mask: Some(nana_ui_core::MaskImage::Gradient(
                    nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                        angle_deg: 180.0,
                        stops: vec![
                            nana_ui_core::GradientStop {
                                position: 0.0,
                                color: [1.0, 1.0, 1.0, 1.0],
                            },
                            nana_ui_core::GradientStop {
                                position: 1.0,
                                color: [1.0, 1.0, 1.0, 0.0],
                            },
                        ],
                    }),
                )),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let primitive = scene
        .primitives()
        .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
        .expect("host texture custom primitive");
    match &primitive.kind {
        ScenePrimitiveKind::Custom { mask, node } => {
            assert_eq!(node.renderer.as_ref(), "nana.host-texture");
            assert!(mask.is_some(), "mask must travel on the Custom primitive");
        }
        other => panic!("expected custom, got {other:?}"),
    }
}

#[test]
fn css_mask_url_travels_on_quad_and_host_texture() {
    let mut painted = node(1, None, &[]);
    painted.custom_render = Some(CustomRenderNode::new("nana.host-texture", "preview", 1));
    painted.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            paint: nana_ui_core::PaintStyle {
                mask: Some(nana_ui_core::MaskImage::Url("fade.png".into())),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut painted).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([painted], []);
    let quad = scene
        .primitive(PrimitiveId {
            node: id(1),
            slot: 0,
        })
        .expect("surface quad");
    match &quad.kind {
        ScenePrimitiveKind::Quad { surface, .. } => match &surface.mask {
            Some(nana_ui_core::MaskImage::Url(url)) => assert_eq!(url, "fade.png"),
            other => panic!("expected url mask on quad, got {other:?}"),
        },
        other => panic!("expected quad, got {other:?}"),
    }
    let custom = scene
        .primitives()
        .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
        .expect("host texture custom");
    match &custom.kind {
        ScenePrimitiveKind::Custom { mask, .. } => match mask {
            Some(nana_ui_core::MaskImage::Url(url)) => assert_eq!(url, "fade.png"),
            other => panic!("expected url mask on custom, got {other:?}"),
        },
        other => panic!("expected custom, got {other:?}"),
    }
}

#[test]
fn rasterized_svg_host_texture_skips_vector_children() {
    let mut svg = node(1, None, &[2]);
    svg.kind = Arc::new(NodeKind::Element { tag: "svg".into() });
    svg.custom_render = Some(CustomRenderNode::new("nana.host-texture", "svg:1", 1));
    let mut path = node(2, Some(1), &[]);
    path.kind = Arc::new(NodeKind::Element { tag: "path".into() });
    path.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut path).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([svg, path], []);
    assert!(
        scene.primitives().any(|primitive| matches!(
            primitive.kind,
            ScenePrimitiveKind::Custom { .. }
        ) && primitive.node == id(1)),
        "svg root still samples HostTexture"
    );
    assert!(
        scene.primitives().all(|primitive| primitive.node != id(2)),
        "path children of a rasterized svg must not paint as boxes"
    );
}

#[test]
fn icon_visual_skips_vector_children() {
    let mut icon = node(1, None, &[2]);
    icon.standard_visual = Some(StandardVisual::Icon {
        icon: nana_ui_core::Icon::Search,
        size: 16.0,
        tooltip: None,
    });
    let mut path = node(2, Some(1), &[]);
    path.kind = Arc::new(NodeKind::Element { tag: "path".into() });
    path.source_style = NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        }),
        ..Default::default()
    };
    style_mut(&mut path).background = Some([1.0, 0.0, 0.0, 1.0]);
    let mut scene = UiScene::new();
    scene.apply_delta([icon, path], []);
    assert!(
        scene.primitives().any(|primitive| {
            primitive.node == id(1)
                && matches!(
                    primitive.kind,
                    ScenePrimitiveKind::Icon { icon, .. }
                        if icon == nana_ui_core::Icon::Search
                )
        }),
        "icon root still paints the atlas glyph"
    );
    assert!(
        scene.primitives().all(|primitive| primitive.node != id(2)),
        "path children of an Icon visual must not paint as boxes"
    );
}

#[test]
fn completion_and_hover_overlays_paint_above_editor_layers() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    let row_rect = |index: usize| LayoutBox {
        x: 10.0,
        y: 20.0 + index as f32 * 14.0,
        width: 120.0,
        height: 14.0,
    };
    let text_region = |content: &str, bounds: LayoutBox| ComponentTextRegion {
        bounds,
        content: Arc::from(content),
        color: Some([0.9, 0.9, 0.9, 1.0]),
        font_size: 12.0,
        font_weight: None,
    };
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: text_region("fn", LayoutBox::default()),
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: Some(nana_ui_runtime::TextCompletionPopup {
            panel: LayoutBox {
                x: 8.0,
                y: 18.0,
                width: 120.0,
                height: 32.0,
            },
            selected: 1,
            first_row: 0,
            rows: (0..2)
                .map(|index| nana_ui_runtime::TextCompletionRow {
                    bounds: row_rect(index),
                    label: text_region("label", row_rect(index)),
                    detail: None,
                    kind: Some(text_region("fn", row_rect(index))),
                    doc: None,
                })
                .collect(),
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            selected_background: [0.2, 0.2, 0.2, 1.0],
            label_color: [1.0; 4],
            detail_color: [0.5; 4],
            kind_color: [0.4; 4],
        }),
        hover_popup: Some(nana_ui_runtime::TextHoverPopup {
            panel: LayoutBox {
                x: 8.0,
                y: 80.0,
                width: 120.0,
                height: 28.0,
            },
            title: text_region("hover", row_rect(0)),
            body_rows: vec![text_region("body", row_rect(1))],
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            title_color: [1.0; 4],
            body_color: [0.6; 4],
        }),
        signature_popup: Some(nana_ui_runtime::TextSignaturePopup {
            panel: LayoutBox {
                x: 8.0,
                y: 140.0,
                width: 160.0,
                height: 32.0,
            },
            prefix: text_region("mix(", row_rect(0)),
            active: Some(text_region("a", row_rect(0))),
            suffix: text_region(", b)", row_rect(0)),
            doc: Some(text_region("blend", row_rect(1))),
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            active_background: [0.2, 0.2, 0.2, 1.0],
        }),
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([input], []);

    let kind = |slot: u8| {
        scene
            .primitive(PrimitiveId { node: id(1), slot })
            .map(|primitive| primitive.kind.clone())
            .expect("overlay primitive")
    };
    // 面板底 + 选中行高亮 + 行文本（label/kind 各一层；detail 为空
    // 的候选不产生文本层）。
    assert!(matches!(kind(90), ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(kind(91), ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(kind(92), ScenePrimitiveKind::Text { content, .. } if content == "label"));
    assert!(matches!(kind(93), ScenePrimitiveKind::Text { content, .. } if content == "label"));
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 100
            })
            .is_none()
    );
    assert!(matches!(kind(108), ScenePrimitiveKind::Text { content, .. } if content == "fn"));
    // hover 浮窗：面板 + 标题 + 正文。
    assert!(matches!(kind(120), ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(kind(121), ScenePrimitiveKind::Text { content, .. } if content == "hover"));
    assert!(matches!(kind(122), ScenePrimitiveKind::Text { content, .. } if content == "body"));
    // 签名帮助 slot 140+：与 hover 120-131、补全 doc 132-139 不相交。
    assert!(matches!(kind(140), ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(kind(141), ScenePrimitiveKind::Quad { .. }));
    assert!(matches!(kind(142), ScenePrimitiveKind::Text { content, .. } if content == "mix("));
    assert!(matches!(kind(143), ScenePrimitiveKind::Text { content, .. } if content == "a"));
    assert!(matches!(kind(144), ScenePrimitiveKind::Text { content, .. } if content == ", b)"));
    assert!(matches!(kind(145), ScenePrimitiveKind::Text { content, .. } if content == "blend"));
    // 两行之外没有多余文本层。
    assert!(
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 94
            })
            .is_none()
    );
}

/// 文档行 slot 带(132+)与 hover 浮窗(120-131)独立共存:满 8 行带
/// doc 的候选 + hover 同屏时,全部 doc 图元与 hover 图元俱在——锁
/// 定"insert_primitive 按 (node,slot) 覆盖"下带不相交的约束。
#[test]
fn completion_doc_rows_and_hover_overlay_coexist_without_slot_clashes() {
    let mut input = node(1, None, &[]);
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: false,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    let row_rect = |index: usize| LayoutBox {
        x: 10.0,
        y: 20.0 + index as f32 * 28.0,
        width: 120.0,
        height: 28.0,
    };
    let text_region = |content: &str, bounds: LayoutBox| ComponentTextRegion {
        bounds,
        content: Arc::from(content),
        color: Some([0.9, 0.9, 0.9, 1.0]),
        font_size: 12.0,
        font_weight: None,
    };
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: text_region("fn", LayoutBox::default()),
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        line_labels_color: [0.0; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: Some(nana_ui_runtime::TextCompletionPopup {
            panel: LayoutBox {
                x: 8.0,
                y: 18.0,
                width: 120.0,
                height: 224.0,
            },
            selected: 0,
            first_row: 0,
            rows: (0..8)
                .map(|index| nana_ui_runtime::TextCompletionRow {
                    bounds: row_rect(index),
                    label: text_region("label", row_rect(index)),
                    detail: None,
                    kind: None,
                    doc: Some(text_region("doc", row_rect(index))),
                })
                .collect(),
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            selected_background: [0.2, 0.2, 0.2, 1.0],
            label_color: [1.0; 4],
            detail_color: [0.5; 4],
            kind_color: [0.4; 4],
        }),
        hover_popup: Some(nana_ui_runtime::TextHoverPopup {
            panel: LayoutBox {
                x: 8.0,
                y: 260.0,
                width: 120.0,
                height: 28.0,
            },
            title: text_region("hover", row_rect(0)),
            body_rows: vec![text_region("body", row_rect(1))],
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            title_color: [1.0; 4],
            body_color: [0.6, 0.6, 0.6, 1.0],
        }),
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });
    let mut scene = UiScene::new();
    scene.apply_delta([input], []);

    // slot 形参不写死类型:经 PrimitiveId 推断,在 slot u8 与 u32 两种
    // 基线下都编译(他人 slot 拓宽重构在途,本测试不得绑定其任一侧)。
    let content = |slot| {
        scene
            .primitive(PrimitiveId { node: id(1), slot })
            .map(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Text { content, .. } => Some(content.clone()),
                _ => None,
            })
            .flatten()
    };
    // 满 8 行 doc 全部在场(132..139),不被 hover 覆盖。
    for slot in 132..=139 {
        assert_eq!(content(slot).as_deref(), Some("doc"), "doc slot {slot} 在场");
    }
    // hover 图元同屏俱在(面板/标题/正文)。
    assert!(
        matches!(
            scene
                .primitive(PrimitiveId { node: id(1), slot: 120 })
                .map(|primitive| primitive.kind.clone()),
            Some(ScenePrimitiveKind::Quad { .. })
        )
    );
    assert_eq!(content(121).as_deref(), Some("hover"));
    assert_eq!(content(122).as_deref(), Some("body"));
}

#[cfg(feature = "rich-text")]
#[test]
fn markdown_keeps_and_removes_more_than_256_scene_primitives() {
    let source = (0..300)
        .map(|index| format!("Paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let view = nana_ui_runtime::NativeMarkdown::from_source(&source);
    let mut markdown = node(991, None, &[]);
    markdown.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 9000.0,
    };
    markdown.standard_visual = Some(StandardVisual::NativeMarkdown {
        blocks: view.blocks().to_vec().into(),
        text: source.clone().into(),
        selection: None,
    });
    markdown.component_geometry = Some(ComponentGeometry::NativeMarkdown {
        drawing: view.drawing(markdown.layout),
        text: ComponentTextRegion {
            bounds: markdown.layout,
            content: source.into(),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        selection_color: [0.0; 4],
    });
    let mut scene = UiScene::new();
    scene.apply_delta([markdown], []);
    let primitives = scene
        .primitives()
        .filter(|p| p.node == id(991))
        .collect::<Vec<_>>();
    assert_eq!(primitives.len(), 300);
    assert_eq!(
        primitives
            .iter()
            .map(|p| p.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        300
    );
    scene.apply_delta([], [id(991)]);
    assert!(scene.primitives().all(|p| p.node != id(991)));
}

#[cfg(feature = "rich-text")]
#[test]
fn markdown_formulas_and_diagrams_emit_svg_surfaces_with_theme_ink() {
    let source = "$$\\frac{1}{\\sqrt{x^2+1}}$$\n\n```mermaid\nflowchart TD\nA-->B\n```";
    let view = nana_ui_runtime::NativeMarkdown::from_source(source);
    let mut markdown = node(992, None, &[]);
    markdown.layout = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 430.0,
        height: 900.0,
    };
    markdown.standard_visual = Some(StandardVisual::NativeMarkdown {
        blocks: view.blocks().to_vec().into(),
        text: view.plain_text().into(),
        selection: None,
    });
    markdown.component_geometry = Some(ComponentGeometry::NativeMarkdown {
        drawing: view.drawing(markdown.layout),
        text: ComponentTextRegion {
            bounds: markdown.layout,
            content: view.plain_text().into(),
            color: Some([1.0, 1.0, 1.0, 1.0]),
            font_size: 13.0,
            font_weight: None,
        },
        selection: Vec::new(),
        selection_color: [0.0; 4],
    });
    let mut scene = UiScene::new();
    scene.apply_delta([markdown], []);
    let images = scene
        .primitives()
        .filter_map(|p| match &p.kind {
            ScenePrimitiveKind::Quad { surface, .. } => surface
                .content_image
                .as_ref()
                .map(|image| (&p.bounds, image)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 2);
    for (bounds, image) in images {
        assert!(bounds.width > 0.0 && bounds.width <= 430.0 && bounds.height > 20.0);
        let BackgroundImage::Url { url, .. } = image else {
            panic!("expected native SVG source")
        };
        assert!(url.starts_with("data:image/svg+xml,"));
        assert!(url.contains("rgb%28255%2C255%2C255%29"));
        assert!(!url.contains("%23010203"));
    }
}

/// Wave 4b-1 守卫端点：TextInput 主文本区域携带折叠重映射后的显示空间
/// span（值串 ≠ 显示串时不再整批丢弃）；同一节点的行号标签区域不承载
/// 编辑器 span（内容与 span 空间不一致，整批丢弃）。
#[test]
fn text_input_main_text_region_keeps_display_space_spans_but_labels_do_not() {
    let value = "fn a() {\n    x();\n    y();\n}\nfn b() {}";
    let display = "fn a() { …3\nfn b() {}";
    let text_region = |content: &str| ComponentTextRegion {
        bounds: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 60.0,
        },
        content: Arc::from(content),
        color: Some([0.9, 0.9, 0.9, 1.0]),
        font_size: 12.0,
        font_weight: None,
    };
    let mut input = node(1, None, &[]);
    input.text = Some(TextContent {
        value: value.into(),
    });
    // 行号标签绘制在左内边距区域：预留 gutter 宽度让标签层真实发出。
    Arc::make_mut(&mut input.source_style.layout).padding_left = Some(
        nana_ui_core::LengthSpec::Px(46.0),
    );
    input.standard_visual = Some(StandardVisual::TextInput {
        placeholder: Arc::from(""),
        size: nana_ui_core::ControlSize::Medium,
        secure: false,
        invalid: false,
        steppers: false,
        diagnostics: Arc::from([]),
        matches: Arc::from([]),
        color_swatches: Arc::from([]),
        line_numbers: true,
        indent_guides: None,
        folds: Arc::from([]),
        git_marks: Arc::from([]),
        editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
    });
    input.component_geometry = Some(ComponentGeometry::TextInput {
        multiline: true,
        text: text_region(display),
        selection: Vec::new(),
        caret: None,
        additional_carets: Vec::new(),
        additional_caret_color: [0.0; 4],
        preedit: Vec::new(),
        diagnostic_markers: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color: [0.0; 4],
        caret_line: None,
        bracket_markers: Vec::new(),
        drop_indicator: None,
        indent_guides: Vec::new(),
        line_labels: vec![nana_ui_runtime::LineLabel {
            y: 0.0,
            height: 14.0,
            number: 1,
        }],
        line_labels_color: [0.5; 4],
        line_labels_font_size: 11.0,
        folds: nana_ui_runtime::TextFoldGeometry::default(),
        git_marks: nana_ui_runtime::TextGitGutterGeometry::default(),
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        background: None,
        border: None,
        border_width: 0.0,
        focus_ring: None,
        selection_color: [0.0; 4],
        caret_color: [0.0; 4],
        preedit_color: [0.0; 4],
        occurrence_markers: Vec::new(),
        whitespace_marks: Vec::new(),
        whitespace_color: [0.0; 4],
        wrap_guides: Vec::new(),
        steppers: None,
        minimap: None,
        sticky_line: None,
    });
    input.text_spans = vec![
        nana_ui_runtime::ExtractedTextSpan {
            start: 0,
            end: 4,
            color: [1.0, 0.0, 0.0, 1.0],
        },
        nana_ui_runtime::ExtractedTextSpan {
            start: 13,
            end: 16,
            color: [0.0, 1.0, 0.0, 1.0],
        },
    ];

    let mut scene = UiScene::new();
    scene.apply_delta([input], []);
    // slot 形参经 PrimitiveId 推断,u8/u32 基线都编译(同前,不绑定
    // 在途的 slot 拓宽重构)。
    let spans_of = |slot| {
        scene
            .primitive(PrimitiveId {
                node: id(1),
                slot,
            })
            .map(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Text { spans, .. } => spans.clone(),
                _ => panic!("text primitive"),
            })
            .expect("text primitive")
    };
    // 主文本区域（slot 2）：显示空间 span 原样生效（旧守卫在此整批丢弃）。
    assert_eq!(
        spans_of(2),
        vec![
            SceneTextSpan {
                start: 0,
                end: 4,
                color: [1.0, 0.0, 0.0, 1.0]
            },
            SceneTextSpan {
                start: 13,
                end: 16,
                color: [0.0, 1.0, 0.0, 1.0]
            }
        ]
    );
    // 行号标签区域（slot 40）：内容 "1" 与 span 空间不一致，不带 span。
    assert!(
        spans_of(40).is_empty(),
        "标签区域不承载编辑器显示空间 span"
    );
}
