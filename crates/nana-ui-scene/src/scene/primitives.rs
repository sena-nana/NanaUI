//! Scene primitives projection.

use super::*;
#[cfg(feature = "charts")]
use nana_ui_runtime::TimeSeriesChart;

#[cfg(feature = "calendar")]
mod calendar;
#[cfg(feature = "charts")]
mod charts;
#[cfg(feature = "controls")]
mod controls;
mod controls_base;
#[cfg(feature = "graph-canvas")]
mod graph_canvas;
#[cfg(feature = "image-viewer")]
mod image_viewer;
mod overlays;
#[cfg(feature = "rich-text")]
mod rich_text;
mod text;

struct GeometryPaintContext<'a> {
    node: &'a ExtractedNode,
    bounds: SceneRect,
    transform: AffineTransform,
    clips: &'a Arc<[ClipRegion]>,
    parent_clips: &'a Arc<[ClipRegion]>,
    text_input_clips: &'a Arc<[ClipRegion]>,
    empty_state_content_clips: &'a Arc<[ClipRegion]>,
    opacity: f32,
    node_order: usize,
}

impl UiScene {
    pub(super) fn rebuild_node_primitives(&mut self, id: StableNodeId) -> usize {
        self.remove_node_primitives(id);
        let Some(node) = self.nodes.get(&id).cloned() else {
            return 0;
        };
        if is_descendant_of_rasterized_svg(&self.nodes, &node)
            || is_descendant_of_icon_visual(&self.nodes, &node)
        {
            return 0;
        }
        let before = self.primitives.len();
        let (parent_transform, parent_opacity, parent_clips, parent_blocks_3d) =
            self.ancestor_state(&node);
        let layout = node.layout;
        let bounds = SceneRect {
            x: layout.x,
            y: layout.y,
            width: layout.width,
            height: layout.height,
        };
        let local_transform =
            node_scene_transform(node.source_style.layout.as_ref(), layout, parent_blocks_3d);
        let transform = parent_transform.then(local_transform);
        let local_opacity = local_opacity(&node);
        let opacity = if is_opacity_group(&self.nodes, &node) {
            parent_opacity
        } else {
            parent_opacity * local_opacity
        };
        let style = node.source_style.layout.as_ref();
        let clips: Arc<[ClipRegion]> = {
            let mut chain = if let Some((x, y, w, h)) = node.source_style.layout.overflow_clip_box(
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            ) {
                let mut own = parent_clips.to_vec();
                own.push(ClipRegion::axis_aligned(
                    SceneRect {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    transform,
                ));
                own
            } else {
                parent_clips.to_vec()
            };
            if let Some(region) = clip_path_region(style, bounds, transform) {
                chain.push(region);
            }
            chain.into()
        };
        let surface_clips: Arc<[ClipRegion]> = Arc::clone(&clips);
        let empty_state_content_clips: Arc<[ClipRegion]> =
            if let Some(ComponentGeometry::EmptyState { content_clip, .. }) =
                node.component_geometry.as_ref()
            {
                let mut content_clips = clips.to_vec();
                content_clips.push(ClipRegion {
                    bounds: scene_rect(*content_clip),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                content_clips.into()
            } else {
                Arc::clone(&clips)
            };
        let text_input_clips: Arc<[ClipRegion]> =
            if matches!(node.standard_visual, Some(StandardVisual::TextInput { .. })) {
                let padding = node
                    .source_style
                    .layout
                    .resolved_padding_against(Some(bounds.width));
                let border = node.source_style.layout.resolved_border_width();
                let mut text_input_clips = clips.to_vec();
                text_input_clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: bounds.x + border + padding.left,
                        y: bounds.y + border + padding.top,
                        width: (bounds.width - border * 2.0 - padding.left - padding.right)
                            .max(0.0),
                        height: (bounds.height - border * 2.0 - padding.top - padding.bottom)
                            .max(0.0),
                    },
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                text_input_clips.into()
            } else {
                Arc::clone(&clips)
            };
        let node_order = self.node_order.get(&id).copied().unwrap_or_default();
        if node.style.visible && opacity > 0.0 {
            let standard_visual_uses_root_surface = matches!(
                node.standard_visual,
                Some(
                    StandardVisual::Button { .. }
                        | StandardVisual::TextInput { .. }
                        | StandardVisual::Icon { .. }
                        | StandardVisual::Switch { .. }
                        | StandardVisual::Card { .. }
                        | StandardVisual::ListItem { .. }
                        | StandardVisual::StatusBadge { .. }
                        | StandardVisual::SelectionOption { .. }
                        | StandardVisual::ModalFrame { .. }
                        | StandardVisual::Toast { .. }
                        | StandardVisual::XYPad { .. }
                        | StandardVisual::Select { .. }
                        | StandardVisual::ActionMenuItem { .. }
                        | StandardVisual::TreeView { .. }
                        | StandardVisual::CommandPalette { .. }
                )
            );
            let surface_border_color =
                if matches!(node.standard_visual, Some(StandardVisual::Switch { .. })) {
                    None
                } else {
                    node.style.border_color
                };
            let (surface_background, surface_border_color, surface_border_width) =
                match node.component_geometry.as_ref() {
                    Some(ComponentGeometry::Button {
                        background,
                        border,
                        border_width,
                        ..
                    }) => (*background, *border, *border_width),
                    Some(ComponentGeometry::TextInput {
                        background,
                        border,
                        border_width,
                        ..
                    }) => (*background, *border, *border_width),
                    Some(ComponentGeometry::StatusBadge { background, .. }) => {
                        (Some(*background), None, 0.0)
                    }
                    _ => {
                        if matches!(node.standard_visual, Some(StandardVisual::Switch { .. })) {
                            (node.style.background, None, 0.0)
                        } else {
                            let edges = style.paint_border_edges();
                            (
                                node.style.background,
                                style.resolved_border_color().or(surface_border_color),
                                edges.top.max(edges.right).max(edges.bottom).max(edges.left),
                            )
                        }
                    }
                };
            if !matches!(
                node.standard_visual,
                Some(StandardVisual::MenuSurface { .. })
            ) && (style.has_surface_paint()
                || style.paints_any_border()
                || ((node.standard_visual.is_none() || standard_visual_uses_root_surface)
                    && (node.style.background.is_some()
                        || node.style.border_color.is_some()
                        || style.paints_any_border())))
            {
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 0 },
                    node: id,
                    bounds,
                    transform,
                    clips: Arc::clone(&surface_clips),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: surface_background,
                        border_color: surface_border_color,
                        border_width: surface_border_width,
                        corner_radius: surface_corner_radii(style, bounds.width, bounds.height),
                        shadow: style
                            .paint
                            .primary_box_shadow()
                            .map(ComponentElevation::from_box_shadow)
                            .or(match node.component_geometry.as_ref() {
                                Some(ComponentGeometry::Card { elevation, .. }) => *elevation,
                                _ => None,
                            }),
                        surface: {
                            let mut surface =
                                quad_surface_from_style(style, bounds.width, bounds.height);
                            if is_filter_group(&self.nodes, &node) {
                                surface.filter = None;
                            }
                            let component_owns_border = matches!(
                                node.component_geometry.as_ref(),
                                Some(
                                    ComponentGeometry::Button { .. }
                                        | ComponentGeometry::TextInput { .. }
                                )
                            ) || matches!(
                                node.standard_visual,
                                Some(StandardVisual::Switch { .. })
                            );
                            if !component_owns_border {
                                let edges = style.paint_border_edges();
                                surface.border_widths =
                                    [edges.top, edges.right, edges.bottom, edges.left];
                                surface.border_colors = style.paint_border_edge_colors();
                                surface.border_styles = style.paint_border_style_codes();
                            }
                            surface
                        },
                    },
                });
            }
            if let Some(custom) = node.custom_render.clone() {
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 1 },
                    node: id,
                    bounds,
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Custom {
                        node: custom,
                        mask: style.paint.mask.clone(),
                    },
                });
            }
            let component_owns_text =
                component_geometry_owns_text(node.component_geometry.as_ref());
            if let Some(text) = node.text.as_ref().filter(|text| {
                !text.value.is_empty()
                    && !component_owns_text
                    && !self.parent_already_paints_text(&node)
            }) {
                let padding = style.resolved_padding_against(Some(bounds.width));
                let border = style.resolved_border_width();
                let leading_visual = match node.standard_visual {
                    Some(StandardVisual::Checkbox { size, .. }) => {
                        size.indicator_size() + size.indicator_gap()
                    }
                    Some(StandardVisual::Switch {
                        control_position: SwitchControlPosition::Start,
                        ..
                    }) => 38.0,
                    _ => 0.0,
                };
                let trailing_visual = match node.standard_visual {
                    Some(StandardVisual::Switch {
                        control_position: SwitchControlPosition::End,
                        ..
                    }) => 38.0,
                    _ => 0.0,
                };
                let mut text_bounds = SceneRect {
                    x: bounds.x + border + padding.left + leading_visual,
                    y: bounds.y + border + padding.top,
                    width: (bounds.width
                        - border * 2.0
                        - padding.left
                        - padding.right
                        - leading_visual
                        - trailing_visual)
                        .max(0.0),
                    height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
                };
                if let Some(ComponentGeometry::ListItem {
                    content: Some(content),
                    ..
                }) = node.component_geometry
                {
                    text_bounds = scene_rect(content);
                }
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 2 },
                    node: id,
                    bounds: text_bounds,
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Text {
                        content: text.value.clone(),
                        color: node.style.color,
                        size: node.style.font_size,
                        weight: node.style.font_weight,
                        family: node.style.font_family.as_deref().map(str::to_owned),
                        line_height: node.style.line_height,
                        letter_spacing: node.style.letter_spacing,
                        wrap: style.text_wraps(),
                        ellipsis: style.uses_text_ellipsis(),
                        max_lines: style.resolved_line_clamp(),
                        shaping: if node.text_input.is_some() {
                            TextShaping::Advanced
                        } else {
                            TextShaping::Auto
                        },
                        horizontal_alignment: node.source_style.text_horizontal_alignment,
                        vertical_alignment: node.source_style.text_vertical_alignment,
                        spans: scene_text_spans(&node, None, &text.value),
                        text_shadow: style.paint.text_shadow,
                        underline: style.text_decoration.is_some_and(|d| d.underline),
                        line_through: style.text_decoration.is_some_and(|d| d.line_through),
                        font_features: style.font_features.clone().unwrap_or_default(),
                        italic: node.style.italic,
                        wrap_break: style.text_wrap_break(),
                        opentype: SceneTextOpenType::from_computed(&node.style),
                    },
                });
                if let Some(deco) = style.text_decoration.filter(|d| d.is_active()) {
                    insert_text_decoration_strokes(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        text_bounds,
                        node.style.color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        deco,
                        |primitive| self.insert_primitive(primitive),
                    );
                }
            }
            let context = GeometryPaintContext {
                node: &node,
                bounds,
                transform,
                clips: &clips,
                parent_clips: &parent_clips,
                text_input_clips: &text_input_clips,
                empty_state_content_clips: &empty_state_content_clips,
                opacity,
                node_order,
            };
            let mut emit = |primitive| self.insert_primitive(primitive);
            match node.component_geometry.as_ref() {
                Some(ComponentGeometry::ModalFrame { .. }) => overlays::build(&context, &mut emit),
                Some(ComponentGeometry::Button { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::TextInput { .. }) => text::build(&context, &mut emit),
                Some(ComponentGeometry::Switch { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::Range { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::Card { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::ListItem { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::StatusBadge { .. }) => text::build(&context, &mut emit),
                Some(ComponentGeometry::ValidationMessage { .. }) => {
                    text::build(&context, &mut emit)
                }
                Some(ComponentGeometry::EmptyState { .. }) => text::build(&context, &mut emit),
                Some(ComponentGeometry::LabeledValue { .. }) => text::build(&context, &mut emit),
                Some(ComponentGeometry::SelectionOption { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::Progress { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::FormField { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::Toast { .. }) => overlays::build(&context, &mut emit),
                Some(ComponentGeometry::XYPad { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::QrCode { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::Select { .. }) => controls_base::build(&context, &mut emit),
                Some(ComponentGeometry::ActionMenuItem { .. }) => {
                    overlays::build(&context, &mut emit)
                }
                Some(ComponentGeometry::TreeView { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::CommandPalette { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::MenuSurface { .. }) => overlays::build(&context, &mut emit),
                #[cfg(feature = "calendar")]
                Some(ComponentGeometry::CalendarHeatmap { .. }) => {
                    calendar::build(&context, &mut emit)
                }
                #[cfg(feature = "charts")]
                Some(ComponentGeometry::TimeSeriesChart { .. }) => {
                    charts::build(&context, &mut emit)
                }
                #[cfg(feature = "controls")]
                Some(ComponentGeometry::ReorderList { .. }) => controls::build(&context, &mut emit),
                #[cfg(feature = "rich-text")]
                Some(ComponentGeometry::NativeMarkdown { .. })
                | Some(ComponentGeometry::SelectableRichText { .. }) => {
                    rich_text::build(&context, &mut emit)
                }
                #[cfg(feature = "graph-canvas")]
                Some(ComponentGeometry::GraphCanvas { .. }) => {
                    graph_canvas::build(&context, &mut emit)
                }
                #[cfg(feature = "graph-canvas")]
                Some(ComponentGeometry::GraphMinimap { .. }) => {
                    graph_canvas::build(&context, &mut emit)
                }
                #[cfg(feature = "image-viewer")]
                Some(ComponentGeometry::ImageViewer { .. }) => {
                    image_viewer::build(&context, &mut emit)
                }
                Some(ComponentGeometry::KeyCaptureLayer { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                Some(ComponentGeometry::KeymapLayer { .. }) => {
                    controls_base::build(&context, &mut emit)
                }
                // Scrollbar chrome is emitted with the StandardVisual slots
                // below so it draws over the node's own content.
                Some(ComponentGeometry::Scrollbar { .. }) | None => {}
                #[cfg(not(feature = "calendar"))]
                Some(ComponentGeometry::CalendarHeatmap { .. }) => {}
                #[cfg(not(feature = "charts"))]
                Some(ComponentGeometry::TimeSeriesChart { .. }) => {}
                #[cfg(not(feature = "controls"))]
                Some(ComponentGeometry::ReorderList { .. }) => {}
                #[cfg(not(feature = "rich-text"))]
                Some(ComponentGeometry::NativeMarkdown { .. }) => {}
                #[cfg(not(feature = "rich-text"))]
                Some(ComponentGeometry::SelectableRichText { .. }) => {}
                #[cfg(not(feature = "graph-canvas"))]
                Some(ComponentGeometry::GraphCanvas { .. }) => {}
                #[cfg(not(feature = "graph-canvas"))]
                Some(ComponentGeometry::GraphMinimap { .. }) => {}
                #[cfg(not(feature = "image-viewer"))]
                Some(ComponentGeometry::ImageViewer { .. }) => {}
            }
            let visual_context = VisualPrimitiveContext {
                node: id,
                transform,
                clips: if matches!(node.standard_visual, Some(StandardVisual::TextInput { .. })) {
                    &text_input_clips
                } else {
                    &clips
                },
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            match node.standard_visual {
                Some(StandardVisual::Button { loading_phase, .. }) => {
                    if let Some(ComponentGeometry::Button {
                        spinner,
                        focus_ring,
                        ..
                    }) = node.component_geometry.as_ref()
                    {
                        if let Some(spinner) = spinner {
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId { node: id, slot: 3 },
                                node: id,
                                bounds: scene_rect(*spinner),
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Spinner {
                                    phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                    color: node.standard_visual_foreground.or(node.style.color),
                                },
                            });
                        }
                        if let Some(color) = focus_ring {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                7,
                                SceneRect {
                                    x: bounds.x - 3.0,
                                    y: bounds.y - 3.0,
                                    width: bounds.width + 6.0,
                                    height: bounds.height + 6.0,
                                },
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*color),
                                    border_width: 2.0,
                                    corner_radius: focus_ring_corner_radius(
                                        style,
                                        SceneRect {
                                            x: bounds.x - 3.0,
                                            y: bounds.y - 3.0,
                                            width: bounds.width + 6.0,
                                            height: bounds.height + 6.0,
                                        },
                                        3.0,
                                    ),
                                },
                            ));
                        }
                    }
                }
                Some(StandardVisual::TextInput { .. }) => {
                    if let Some(ComponentGeometry::TextInput {
                        caret,
                        additional_carets,
                        additional_caret_color,
                        preedit,
                        focus_ring,
                        caret_color,
                        preedit_color,
                        diagnostic_markers,
                        match_markers,
                        swatch_markers,
                        swatch_border_color,
                        caret_line,
                        bracket_markers,
                        occurrence_markers,
                        drop_indicator,
                        whitespace_marks,
                        whitespace_color,
                        wrap_guides,
                        indent_guides,
                        line_labels,
                        line_labels_color,
                        line_labels_font_size,
                        folds,
                        git_marks,
                        completion_popup,
                        hover_popup,
                        signature_popup,
                        minimap,
                        sticky_line,
                        ..
                    }) = node.component_geometry.as_ref()
                    {
                        // git gutter 标记：gutter 最左侧 2px 竖条按种类各一个
                        // quad 批次（slot 18 新增 / 19 修改 / 8 删除），与折叠
                        // 箭头同用外层裁剪；空种类不产生批次。位置与颜色由
                        // 世界按行几何与语义令牌解析。
                        if !git_marks.added.is_empty()
                            || !git_marks.modified.is_empty()
                            || !git_marks.deleted.is_empty()
                        {
                            let git_context = VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            };
                            for (slot, rects, color) in [
                                (18, &git_marks.added, git_marks.added_color),
                                (19, &git_marks.modified, git_marks.modified_color),
                                (8, &git_marks.deleted, git_marks.deleted_color),
                            ] {
                                if rects.is_empty() {
                                    continue;
                                }
                                self.insert_primitive(visual_quad_batch(
                                    &git_context,
                                    slot,
                                    rects.iter().map(|rect| scene_rect(*rect)),
                                    VisualQuadStyle::solid(color),
                                ));
                            }
                        }
                        // 折叠 gutter 标记：折叠态（实心，slot 14）与展开态
                        // （描边，slot 15）各一个 quad 批次，与行号同级的外层
                        // 裁剪；合批后数量不受 slot 上限约束，点击切换由
                        // Runtime 指针路径处理。
                        if !folds.gutters.is_empty() {
                            let gutter_context = VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            };
                            let collapsed: Vec<&TextFoldGutter> = folds
                                .gutters
                                .iter()
                                .filter(|gutter| gutter.collapsed)
                                .collect();
                            let expanded: Vec<&TextFoldGutter> = folds
                                .gutters
                                .iter()
                                .filter(|gutter| !gutter.collapsed)
                                .collect();
                            if !collapsed.is_empty() {
                                self.insert_primitive(visual_quad_batch(
                                    &gutter_context,
                                    14,
                                    collapsed.iter().map(|gutter| scene_rect(gutter.bounds)),
                                    VisualQuadStyle::solid(collapsed[0].color),
                                ));
                            }
                            if !expanded.is_empty() {
                                self.insert_primitive(visual_quad_batch(
                                    &gutter_context,
                                    15,
                                    expanded.iter().map(|gutter| scene_rect(gutter.bounds)),
                                    VisualQuadStyle {
                                        background: None,
                                        border_color: Some(expanded[0].color),
                                        border_width: 1.0,
                                        corner_radius: corner_radii(0.0),
                                    },
                                ));
                            }
                        }
                        // 行号标签绘制在左内边距区域，使用外层裁剪。
                        if !line_labels.is_empty() {
                            let padding = node
                                .source_style
                                .layout
                                .resolved_padding_against(Some(bounds.width));
                            let border = node.source_style.layout.resolved_border_width();
                            let gutter_width = padding.left;
                            if gutter_width > 4.0 {
                                for (label_index, label) in line_labels.iter().enumerate() {
                                    let region = ComponentTextRegion {
                                        bounds: LayoutBox {
                                            x: bounds.x + border + 2.0,
                                            y: label.y,
                                            width: (gutter_width - 4.0).max(0.0),
                                            height: label.height,
                                        },
                                        content: Arc::from(label.number.to_string().as_str()),
                                        color: Some(*line_labels_color),
                                        font_size: *line_labels_font_size,
                                        font_weight: None,
                                    };
                                    self.insert_primitive(component_text_primitive(
                                        id,
                                        40 + label_index as u8,
                                        &region,
                                        TextHorizontalAlignment::End,
                                        false,
                                        &node,
                                        transform,
                                        std::sync::Arc::clone(&clips),
                                        opacity,
                                        node_order,
                                    ));
                                }
                            }
                        }
                        for (marker_index, (rect, color)) in diagnostic_markers.iter().enumerate() {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                20 + marker_index as u8,
                                scene_rect(*rect),
                                VisualQuadStyle::solid(*color),
                            ));
                        }
                        // 查找匹配高亮：普通匹配（slot 3，文本之上、光标之
                        // 下）与当前匹配（slot 6，更强）各一个 quad 批次，
                        // 同类共用世界解析出的统一颜色。
                        let (normal_matches, current_matches): (Vec<_>, Vec<_>) =
                            match_markers.iter().partition(|marker| !marker.current);
                        if !normal_matches.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                3,
                                normal_matches.iter().map(|marker| scene_rect(marker.rect)),
                                VisualQuadStyle::solid(normal_matches[0].color),
                            ));
                        }
                        if !current_matches.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                6,
                                current_matches.iter().map(|marker| scene_rect(marker.rect)),
                                VisualQuadStyle::solid(current_matches[0].color),
                            ));
                        }
                        // 颜色装饰 swatch：单个 QuadColorBatch（slot 23，诊断
                        // 下划线之上、行号之下）承载全部方块，数量与颜色种数
                        // 都不占用额外 slot（同 IconBatch 的合批先例）；1px
                        // 细描边，位置与颜色由世界按 span 末行几何解析。
                        if !swatch_markers.is_empty() {
                            self.insert_primitive(visual_quad_color_batch(
                                &visual_context,
                                23,
                                swatch_markers
                                    .iter()
                                    .map(|(rect, color)| (scene_rect(*rect), *color)),
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*swatch_border_color),
                                    border_width: 1.0,
                                    corner_radius: corner_radii(2.0),
                                },
                            ));
                        }
                        // 当前行条：slot 1 与选区同一层级（互斥：选区收起时
                        // 才有当前行条），绘制在文本之下。
                        if let Some((rect, color)) = caret_line {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                1,
                                scene_rect(*rect),
                                VisualQuadStyle::solid(*color),
                            ));
                        }
                        // 缩进参考线：1px 竖线批次，低对比结构标记。
                        if !indent_guides.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                10,
                                indent_guides.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle::solid(indent_guides[0].1),
                            ));
                        }
                        // 出现高亮：淡底色填充批次（slot 11，缩进参考线之
                        // 上、括号描边之下），弱于查找匹配的两级强调。
                        if !occurrence_markers.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                11,
                                occurrence_markers.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle::solid(occurrence_markers[0].1),
                            ));
                        }
                        // 空白字符显示：空格画小圆点（slot 16 单一批次），
                        // Tab 画箭头图标（slot 60 单一批次，镜像折叠箭头
                        // 14/15 的合批先例——数量不受 slot 上限约束）。
                        if !whitespace_marks.is_empty() {
                            let dots: Vec<&LayoutBox> = whitespace_marks
                                .iter()
                                .filter_map(|(rect, kind)| {
                                    (*kind == TextWhitespaceKind::Space).then_some(rect)
                                })
                                .collect();
                            if !dots.is_empty() {
                                // 圆点直径随行号字号缩放，钳在小尺寸带，
                                // 保持"标点"观感而不遮挡字形。
                                let extent = (*line_labels_font_size * 0.2).clamp(2.0, 3.0);
                                self.insert_primitive(visual_quad_batch(
                                    &visual_context,
                                    16,
                                    dots.iter().map(|rect| {
                                        let mut bounds = scene_rect(**rect);
                                        bounds.width = extent;
                                        bounds.height = extent;
                                        bounds.x += (scene_rect(**rect).width - extent) / 2.0;
                                        bounds.y += (scene_rect(**rect).height - extent) / 2.0;
                                        bounds
                                    }),
                                    VisualQuadStyle {
                                        background: Some(*whitespace_color),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(999.0),
                                    },
                                ));
                            }
                            let arrows: Vec<SceneRect> = whitespace_marks
                                .iter()
                                .filter(|(_, kind)| *kind == TextWhitespaceKind::Tab)
                                .map(|(rect, _)| {
                                    let cell = scene_rect(*rect);
                                    // 箭头尺寸按字符单元高度缩放，居中放置。
                                    let extent = (cell.height * 0.55).clamp(6.0, 14.0);
                                    SceneRect {
                                        x: cell.x + (cell.width - extent) / 2.0,
                                        y: cell.y + (cell.height - extent) / 2.0,
                                        width: extent,
                                        height: extent,
                                    }
                                })
                                .collect();
                            if !arrows.is_empty() {
                                self.insert_primitive(batch_primitive(
                                    &visual_context,
                                    60,
                                    arrows,
                                    |bounds| ScenePrimitiveKind::IconBatch {
                                        bounds,
                                        icon: Icon::ArrowRight,
                                        color: Some(*whitespace_color),
                                    },
                                ));
                            }
                        }
                        // wrap guide：按列的全高 1px 竖线批次（slot 17）。
                        // 与缩进参考线（slot 10，行内缩进深度）同为低对比
                        // 竖线，但贯穿整个内容区高度。
                        if !wrap_guides.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                17,
                                wrap_guides.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle::solid(wrap_guides[0].1),
                            ));
                        }
                        // minimap：面板（slot 70）、行条 + 1px 分隔线（slot
                        // 71，同一 faint 色）、视口指示器（slot 72，半透明
                        // accent）各一个批次。占用 70-72：位于行号/Tab 批次
                        // （40+/60）之上、补全弹层（90+）与 hover 浮窗（120+，
                        // 正文行可用到 131）之下，minimap 不得盖住浮层。
                        if let Some(minimap) = minimap {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                70,
                                scene_rect(minimap.panel),
                                VisualQuadStyle::solid(minimap.panel_color),
                            ));
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                71,
                                std::iter::once(scene_rect(minimap.separator))
                                    .chain(minimap.bars.iter().map(|bar| scene_rect(*bar))),
                                VisualQuadStyle::solid(minimap.bar_color),
                            ));
                            if let Some(indicator) = minimap.indicator {
                                self.insert_primitive(visual_quad(
                                    &visual_context,
                                    72,
                                    scene_rect(indicator),
                                    VisualQuadStyle::solid(minimap.indicator_color),
                                ));
                            }
                        }
                        // sticky scroll 钉住行：内容区顶部背景条（slot 80）+
                        // 底缘 1px 分割线（slot 81）+ 头行文本（slot 82），
                        // 位于 minimap（70-72）之上、补全弹层（90+）之下，
                        // 覆盖滚动内容之上。纯装饰只读：无命中框、不可交互，
                        // 复用正文同一 component_text_primitive 字形管线。
                        if let Some(sticky) = sticky_line {
                            // 面板条与分割线同形：纯色 Quad，仅 slot/矩形/色不同。
                            let mut sticky_band = |slot: u8, rect: SceneRect, color: [f32; 4]| {
                                self.insert_primitive(visual_quad(
                                    &visual_context,
                                    slot,
                                    rect,
                                    VisualQuadStyle::solid(color),
                                ));
                            };
                            sticky_band(80, scene_rect(sticky.panel), sticky.background);
                            sticky_band(81, scene_rect(sticky.divider), sticky.divider_color);
                            self.insert_primitive(component_text_primitive(
                                id,
                                82,
                                &sticky.text,
                                TextHorizontalAlignment::Start,
                                false,
                                &node,
                                transform,
                                text_input_clips.clone(),
                                opacity,
                                node_order,
                            ));
                        }
                        // 括号匹配：两端各一个 1px accent 描边框，绘制在文本
                        // 之上（描边不遮挡字形）。
                        if !bracket_markers.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                12,
                                bracket_markers.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(bracket_markers[0].1),
                                    border_width: 1.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 拖拽移动选中文本的落点指示线：目标位置的细竖线
                        // （slot 6，与选区/当前行条同层、正文之上）。
                        if let Some((rect, color)) = drop_indicator {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                6,
                                scene_rect(*rect),
                                VisualQuadStyle::solid(*color),
                            ));
                        }
                        if let Some(caret) = caret {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                4,
                                scene_rect(*caret),
                                VisualQuadStyle::solid(*caret_color),
                            ));
                        }
                        // 附加多光标：与主光标同形、半透明色的 quad 批次
                        // （slot 13，与主光标同层）。
                        if !additional_carets.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                13,
                                additional_carets.iter().map(|rect| scene_rect(*rect)),
                                VisualQuadStyle::solid(*additional_caret_color),
                            ));
                        }
                        if !preedit.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                5,
                                preedit.iter().map(|preedit| scene_rect(*preedit)),
                                VisualQuadStyle::solid(*preedit_color),
                            ));
                        }
                        if let Some(color) = focus_ring {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                7,
                                SceneRect {
                                    x: bounds.x - 3.0,
                                    y: bounds.y - 3.0,
                                    width: bounds.width + 6.0,
                                    height: bounds.height + 6.0,
                                },
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*color),
                                    border_width: 2.0,
                                    corner_radius: focus_ring_corner_radius(
                                        style,
                                        SceneRect {
                                            x: bounds.x - 3.0,
                                            y: bounds.y - 3.0,
                                            width: bounds.width + 6.0,
                                            height: bounds.height + 6.0,
                                        },
                                        3.0,
                                    ),
                                },
                            ));
                        }
                        // 补全弹层与 hover 浮窗：编辑器覆盖层的最上层
                        // （面板 slot 90 / 120，其余文本在各自段内递增；
                        // 补全 label/detail/kind/doc 各占 8 席：92+/100+/
                        // 108+/132+，doc 带越过 hover 的 120-131——两浮
                        // 窗独立共存，slot 带不得相交），高于行号（40+）
                        // 与折叠 gutter（14/15）。面板绘制
                        // 共用 `overlay_panel_primitive`；行文本不换行，
                        // 超宽省略号截断。使用与焦点环同级的外层裁剪。
                        let overlay_context = VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &parent_clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        };
                        let overlay_text =
                            |slot: u8,
                             region: &ComponentTextRegion,
                             alignment: TextHorizontalAlignment| {
                                overlay_text_primitive(
                                    id,
                                    slot,
                                    region,
                                    alignment,
                                    &node,
                                    transform,
                                    std::sync::Arc::clone(&parent_clips),
                                    opacity,
                                    node_order,
                                )
                            };
                        if let Some(popup) = completion_popup {
                            self.insert_primitive(overlay_panel_primitive(
                                &overlay_context,
                                90,
                                scene_rect(popup.panel),
                                popup.background,
                                popup.border,
                            ));
                            let selected_in_view = popup
                                .rows
                                .iter()
                                .enumerate()
                                .find(|(index, _)| popup.first_row + index == popup.selected)
                                .map(|(index, _)| index);
                            if let Some(index) = selected_in_view {
                                self.insert_primitive(visual_quad(
                                    &overlay_context,
                                    91,
                                    scene_rect(popup.rows[index].bounds),
                                    VisualQuadStyle::solid(popup.selected_background),
                                ));
                            }
                            for (index, row) in popup.rows.iter().enumerate() {
                                self.insert_primitive(overlay_text(
                                    92 + index as u8,
                                    &row.label,
                                    TextHorizontalAlignment::Start,
                                ));
                                if let Some(detail) = row.detail.as_ref() {
                                    self.insert_primitive(overlay_text(
                                        100 + index as u8,
                                        detail,
                                        TextHorizontalAlignment::Start,
                                    ));
                                }
                                if let Some(kind) = row.kind.as_ref() {
                                    self.insert_primitive(overlay_text(
                                        108 + index as u8,
                                        kind,
                                        TextHorizontalAlignment::End,
                                    ));
                                }
                                if let Some(doc) = row.doc.as_ref() {
                                    // 文档行带 132+:hover 浮窗正文可用到
                                    // 131,两浮窗独立共存,带不得相交
                                    // (insert_primitive 按 (node,slot)
                                    // 覆盖,相交即静默丢图元)。
                                    self.insert_primitive(overlay_text(
                                        132 + index as u8,
                                        doc,
                                        TextHorizontalAlignment::Start,
                                    ));
                                }
                            }
                        }
                        if let Some(popup) = hover_popup {
                            self.insert_primitive(overlay_panel_primitive(
                                &overlay_context,
                                120,
                                scene_rect(popup.panel),
                                popup.background,
                                popup.border,
                            ));
                            self.insert_primitive(overlay_text(
                                121,
                                &popup.title,
                                TextHorizontalAlignment::Start,
                            ));
                            for (index, row) in popup.body_rows.iter().enumerate() {
                                self.insert_primitive(overlay_text(
                                    122 + index as u8,
                                    row,
                                    TextHorizontalAlignment::Start,
                                ));
                            }
                        }
                        if let Some(popup) = signature_popup {
                            // 签名帮助 slot 140+:面板/活动参数底/前缀/
                            // 活动参数/后缀/文档行。越过补全 doc 132-139
                            // 与 hover 120-131，带不得相交。
                            self.insert_primitive(overlay_panel_primitive(
                                &overlay_context,
                                140,
                                scene_rect(popup.panel),
                                popup.background,
                                popup.border,
                            ));
                            if let Some(bounds) = popup.active_bounds {
                                self.insert_primitive(visual_quad(
                                    &overlay_context,
                                    141,
                                    scene_rect(bounds),
                                    VisualQuadStyle::solid(popup.active_background),
                                ));
                            }
                            self.insert_primitive(overlay_text(
                                142,
                                &popup.prefix,
                                TextHorizontalAlignment::Start,
                            ));
                            if let Some(active) = popup.active.as_ref() {
                                self.insert_primitive(overlay_text(
                                    143,
                                    active,
                                    TextHorizontalAlignment::Start,
                                ));
                            }
                            self.insert_primitive(overlay_text(
                                144,
                                &popup.suffix,
                                TextHorizontalAlignment::Start,
                            ));
                            if let Some(doc) = popup.doc.as_ref() {
                                self.insert_primitive(overlay_text(
                                    145,
                                    doc,
                                    TextHorizontalAlignment::Start,
                                ));
                            }
                        }
                    }
                }
                Some(StandardVisual::Checkbox {
                    checked,
                    indeterminate,
                    size,
                }) => {
                    let extent = size.indicator_size().min(bounds.height);
                    let indicator = SceneRect {
                        x: bounds.x,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        indicator,
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: corner_radii(4.0),
                        },
                    ));
                    if indeterminate {
                        let dash_height = (extent / 8.0).max(1.5);
                        let dash_inset = extent / 4.0;
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            4,
                            SceneRect {
                                x: indicator.x + dash_inset,
                                y: indicator.y + (extent - dash_height) / 2.0,
                                width: (extent - dash_inset * 2.0).max(0.0),
                                height: dash_height,
                            },
                            VisualQuadStyle {
                                background: node.standard_visual_foreground,
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(dash_height / 2.0),
                            },
                        ));
                    } else if checked {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 4 },
                            node: id,
                            bounds: indicator,
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Text {
                                content: "✓".into(),
                                color: node.standard_visual_foreground,
                                size: extent * 0.75,
                                weight: Some(700),
                                family: None,
                                line_height: None,
                                letter_spacing: 0.0,
                                wrap: false,
                                ellipsis: false,
                                max_lines: None,
                                shaping: TextShaping::Auto,
                                horizontal_alignment: TextHorizontalAlignment::Center,
                                vertical_alignment: TextVerticalAlignment::Center,
                                spans: Vec::new(),
                                text_shadow: None,
                                underline: false,
                                line_through: false,
                                font_features: Vec::new(),
                                italic: false,
                                wrap_break: nana_ui_core::TextWrapBreak::Word,
                                opentype: SceneTextOpenType::default(),
                            },
                        });
                    }
                }
                Some(StandardVisual::Icon { icon, size, .. }) => {
                    let extent = size.max(0.0).min(bounds.width).min(bounds.height);
                    let x = bounds.x + (bounds.width - extent) / 2.0;
                    let y = self.icon_y_aligned_to_adjacent_text(&node, bounds, extent);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 3 },
                        node: id,
                        bounds: SceneRect {
                            x,
                            y,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Icon {
                            icon,
                            color: node.standard_visual_foreground.or(node.style.color),
                        },
                    });
                }
                Some(StandardVisual::Switch {
                    thumb_progress,
                    loading,
                    loading_phase,
                    ..
                }) => {
                    let (track, track_background, track_border, thumb_background) =
                        match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Switch {
                                control,
                                track_background,
                                track_border,
                                thumb_background,
                                ..
                            }) => (
                                scene_rect(*control),
                                Some(*track_background),
                                Some(*track_border),
                                Some(*thumb_background),
                            ),
                            _ => (
                                SceneRect {
                                    x: bounds.x,
                                    y: bounds.y + (bounds.height - 16.0) / 2.0,
                                    width: 30.0,
                                    height: 16.0,
                                },
                                node.style.background,
                                node.style.border_color,
                                node.standard_visual_foreground,
                            ),
                        };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        track,
                        VisualQuadStyle {
                            background: track_background,
                            border_color: track_border,
                            border_width: 1.0,
                            corner_radius: corner_radii(8.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        SceneRect {
                            x: track.x + 3.0 + 14.0 * thumb_progress.clamp(0.0, 1.0),
                            y: track.y + 3.0,
                            width: 10.0,
                            height: 10.0,
                        },
                        VisualQuadStyle {
                            background: thumb_background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(5.0),
                        },
                    ));
                    if loading {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 6 },
                            node: id,
                            bounds: SceneRect {
                                x: track.x + 1.0,
                                y: track.y + 1.0,
                                width: 14.0,
                                height: 14.0,
                            },
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Spinner {
                                phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                color: node.standard_visual_foreground.or(node.style.color),
                            },
                        });
                    }
                    if node.focused {
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            7,
                            SceneRect {
                                x: track.x - 4.0,
                                y: track.y - 4.0,
                                width: track.width + 8.0,
                                height: track.height + 8.0,
                            },
                            VisualQuadStyle {
                                background: None,
                                border_color: node.style.border_color,
                                border_width: 2.0,
                                corner_radius: corner_radii(12.0),
                            },
                        ));
                    }
                }
                Some(StandardVisual::Range { ratio, size, .. }) => {
                    let ratio = ratio.clamp(0.0, 1.0);
                    let track_band = match node.component_geometry.as_ref() {
                        Some(ComponentGeometry::Range { track, .. }) => scene_rect(*track),
                        _ => SceneRect {
                            x: bounds.x + 7.0,
                            y: bounds.y + (bounds.height - 14.0) / 2.0,
                            width: (bounds.width - 14.0).max(0.0),
                            height: 14.0,
                        },
                    };
                    let thumb_extent = match size {
                        ControlSize::Small => 12.0_f32,
                        ControlSize::Medium => 14.0_f32,
                        ControlSize::Large => 16.0_f32,
                    }
                    .min(bounds.width)
                    .min(track_band.height.max(0.0));
                    let rail = SceneRect {
                        x: track_band.x,
                        y: track_band.y + (track_band.height - 4.0) / 2.0,
                        width: track_band.width,
                        height: 4.0,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        rail,
                        VisualQuadStyle {
                            background: node.style.border_color,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(2.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        SceneRect {
                            width: rail.width * ratio,
                            ..rail
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(2.0),
                        },
                    ));
                    let thumb_rect = SceneRect {
                        x: track_band.x + track_band.width * ratio - thumb_extent / 2.0,
                        y: track_band.y + (track_band.height - thumb_extent) / 2.0,
                        width: thumb_extent,
                        height: thumb_extent,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        thumb_rect,
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: corner_radii(thumb_extent / 2.0),
                        },
                    ));
                    if node.focused {
                        // Focus marks the thumb (LiliaUI focus-visible outline),
                        // never the rail: the rail colour stays interaction-free.
                        let ring = SceneRect {
                            x: thumb_rect.x - 3.0,
                            y: thumb_rect.y - 3.0,
                            width: thumb_rect.width + 6.0,
                            height: thumb_rect.height + 6.0,
                        };
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            6,
                            ring,
                            VisualQuadStyle {
                                background: None,
                                border_color: node.standard_visual_foreground,
                                border_width: 2.0,
                                corner_radius: corner_radii(ring.width.max(ring.height) / 2.0),
                            },
                        ));
                    }
                }
                Some(StandardVisual::Scrollbar { .. }) => {
                    if let Some(ComponentGeometry::Scrollbar {
                        horizontal,
                        vertical,
                    }) = node.component_geometry.as_ref()
                    {
                        for (slot, bar) in [(3, vertical), (5, horizontal)] {
                            let Some(bar) = bar else {
                                continue;
                            };
                            if let Some(background) = bar.track_background {
                                self.insert_primitive(visual_quad(
                                    &visual_context,
                                    slot,
                                    scene_rect(bar.track),
                                    VisualQuadStyle::solid(background),
                                ));
                            }
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                slot + 1,
                                scene_rect(bar.thumb),
                                VisualQuadStyle {
                                    background: Some(bar.thumb_background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(bar.thumb_radius),
                                },
                            ));
                        }
                    }
                }
                Some(StandardVisual::Card {
                    loading,
                    loading_phase,
                    ..
                }) => {
                    if loading {
                        let spinner_bounds = match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Card {
                                spinner: Some(spinner),
                                ..
                            }) => scene_rect(*spinner),
                            _ => {
                                let extent = 20.0_f32.min(bounds.width).min(bounds.height);
                                SceneRect {
                                    x: bounds.x + (bounds.width - extent) / 2.0,
                                    y: bounds.y + (bounds.height - extent) / 2.0,
                                    width: extent,
                                    height: extent,
                                }
                            }
                        };
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: spinner_bounds,
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Spinner {
                                phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                color: node.standard_visual_foreground.or(node.style.color),
                            },
                        });
                    }
                }
                Some(StandardVisual::Spinner { size, phase, .. }) => {
                    let extent = size.max(0.0).min(bounds.width).min(bounds.height);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 3 },
                        node: id,
                        bounds: SceneRect {
                            x: bounds.x,
                            y: bounds.y + (bounds.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Spinner {
                            phase: (phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                            color: node.standard_visual_foreground.or(node.style.color),
                        },
                    });
                }
                Some(
                    StandardVisual::ListItem { .. }
                    | StandardVisual::StatusBadge { .. }
                    | StandardVisual::ValidationMessage { .. }
                    | StandardVisual::EmptyState { .. }
                    | StandardVisual::LabeledValue { .. }
                    | StandardVisual::SelectionOption { .. }
                    | StandardVisual::ModalFrame { .. }
                    | StandardVisual::Progress { .. }
                    | StandardVisual::LevelMeter { .. }
                    | StandardVisual::FormField { .. }
                    | StandardVisual::Toast { .. }
                    | StandardVisual::XYPad { .. }
                    | StandardVisual::QrCode { .. }
                    | StandardVisual::Select { .. }
                    | StandardVisual::MenuSurface { .. }
                    | StandardVisual::ActionMenuItem { .. }
                    | StandardVisual::TreeView { .. }
                    | StandardVisual::CommandPalette { .. }
                    | StandardVisual::KeyCaptureLayer { .. }
                    | StandardVisual::KeymapLayer,
                ) => {
                    // The row surface and fallback label are emitted above;
                    // typed slots remain ordinary retained child nodes.
                }
                #[cfg(feature = "calendar")]
                Some(StandardVisual::CalendarHeatmap { .. }) => {}
                #[cfg(feature = "charts")]
                Some(StandardVisual::TimeSeriesChart { .. }) => {}
                #[cfg(feature = "controls")]
                Some(StandardVisual::ReorderList { .. }) => {}
                #[cfg(feature = "rich-text")]
                Some(StandardVisual::NativeMarkdown { .. }) => {}
                #[cfg(feature = "rich-text")]
                Some(StandardVisual::SelectableRichText { .. }) => {}
                #[cfg(feature = "graph-canvas")]
                Some(StandardVisual::GraphCanvas { .. }) => {}
                #[cfg(feature = "graph-canvas")]
                Some(StandardVisual::GraphMinimap { .. }) => {}
                #[cfg(feature = "image-viewer")]
                Some(StandardVisual::ImageViewer { .. }) => {}
                None => {}
            }
        }
        self.primitives.len() - before
    }
}
