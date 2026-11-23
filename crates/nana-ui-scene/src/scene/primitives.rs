//! Scene primitives projection.

use super::*;
use nana_ui_runtime::HOST_TEXTURE_RENDERER;

impl UiScene {
    fn resolve_overlay_mount_z(&self, node: &ExtractedNode, base_z: i32) -> i32 {
        match crate::scene::effective_overlay_visual_parent(&self.nodes, node) {
            nana_ui_runtime::VisualParent::TopLayer => {
                base_z.max(nana_ui_runtime::TOP_LAYER_Z_INDEX)
            }
            nana_ui_runtime::VisualParent::Modal(modal_id) => self
                .nodes
                .get(&modal_id)
                .map_or(0, |modal| modal.z_index)
                .saturating_add(base_z),
            _ => base_z,
        }
    }

    /// Rebuild one node's primitives, in place.
    ///
    /// The node's primitives are not dropped first: a rebuild almost always
    /// writes the same slots back at the same paint-order keys, and dropping
    /// them would mean taking every one of them back out of `ordered` and
    /// putting it in again. Each write stamps the slot with this rebuild, so
    /// the slots the node no longer has are the ones left unstamped.
    pub(super) fn rebuild_node_primitives(&mut self, id: StableNodeId) -> usize {
        self.build = next_primitive_revision();
        let build = self.build;
        self.build_node_primitives(id);
        self.retire_node_primitives(id, |held| held.build != build);
        self.node_primitive_count(id)
    }

    fn build_node_primitives(&mut self, id: StableNodeId) {
        self.unadjustable_projections.remove(&id);
        // Written again below when what it builds reads where it stands.
        self.origin_bound.remove(&id);
        self.offset_bound.remove(&id);
        // Written again below when the node still presents per glyph.
        self.glyph_live.remove(&id);
        self.gpu_payload_live.remove(&id);
        self.glyph_objects.remove(&id);
        let Some(node) = self.nodes.get(&id).cloned() else {
            if !self.widget_vector_paint.is_empty() {
                self.widget_vector_paint.remove(&id);
            }
            return;
        };
        if self.inside_ancestor_glyph(&node) {
            if node.custom_paint.is_none() && !self.custom_paint.is_empty() {
                self.custom_paint.remove(&id);
            }
            if !self.widget_vector_paint.is_empty() {
                self.widget_vector_paint.remove(&id);
            }
            return;
        }
        let (parent_transform, parent_opacity, parent_clips, parent_3d) =
            self.ancestor_state(&node);
        // Everything below is built in the node's own coordinates, its
        // border box at the origin, and drawn at the node's origin: a node
        // that only moves keeps it all ([`Self::record_box`]).
        let bounds = scene_rect(Self::record_box(&node));
        let local_transform =
            self.logical_local_transform(&node, parent_3d.blocks_3d, parent_3d.parent_perspective);
        let transform = parent_transform.then(local_transform);
        let page_layout = self.placed_layout(&node);
        let self_backface_hidden = parent_3d.backface_hidden
            || (node.source_style.layout.transforms.backface_visibility
                == nana_ui_core::BackfaceVisibility::Hidden
                && nana_ui_core::is_projective_backface(
                    transform.0,
                    transform.1,
                    page_layout.x + page_layout.width * 0.5,
                    page_layout.y + page_layout.height * 0.5,
                ));
        // The base is the logical projection. While the compositor presents
        // anything, a node may sit under a layer's transform: stamp the base
        // stale so the draw rebases it onto the current projection instead of
        // taking the retained fast path with the logical one.
        let epoch = if self.compositor.has_presentation() {
            self.attribute_epoch.wrapping_sub(1)
        } else {
            self.attribute_epoch
        };
        self.projections
            .insert(id, (epoch, transform, parent_clips.len()));
        if super::attributes::inverse(transform).is_none() {
            self.unadjustable_projections.insert(id);
        }
        self.draw_attributes
            .get_mut()
            .expect("scene attributes")
            .remove(&id);
        let local_opacity = local_opacity(&node);
        let opacity = if is_opacity_group(&self.nodes, &node) {
            parent_opacity
        } else {
            parent_opacity * local_opacity
        };
        let style = node.source_style.layout.as_ref();
        let overflow = overflow_clip_region(style, bounds, transform);
        let clip_path = clip_path_region(style, bounds, transform);
        // The clip-path closes both chains below, where the painter finds a
        // quad's own polygon, which it tests in the quad's shader instead.
        let clips: Arc<[ClipRegion]> = {
            let mut chain = parent_clips.to_vec();
            chain.extend(overflow.clone());
            chain.extend(clip_path.clone());
            chain.into()
        };
        // The overflow clip's rounding is for what the box contains. The box's
        // own fill and border already have that shape, and cutting them with
        // it again would fade their anti-aliased corners, so they keep the
        // plain border-box rectangle. So does its host texture, which the
        // painter rounds with the same radii.
        let surface_clips: Arc<[ClipRegion]> = match &overflow {
            Some(region) if region.corner_radius > 0.0 => {
                let mut chain = parent_clips.to_vec();
                chain.push(ClipRegion {
                    corner_radius: 0.0,
                    ..region.clone()
                });
                chain.extend(clip_path);
                chain.into()
            }
            _ => Arc::clone(&clips),
        };
        let text_input_clips: Arc<[ClipRegion]> =
            if let Some(clip) = node.widget_paint.as_deref().and_then(|w| w.content_clip) {
                let mut content_clips = clips.to_vec();
                content_clips.push(ClipRegion {
                    bounds: scene_rect(clip),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                content_clips.into()
            } else {
                Arc::clone(&clips)
            };
        let node_order = self.node_order.get(&id).copied().unwrap_or_default();
        let visible = node.style.visible && opacity > 0.0 && !self_backface_hidden;
        // A painted node's geometry must be registered before its first
        // primitive goes in: paint order asks whether the node is painted.
        // A hidden painted node keeps its built geometry: shown again with the
        // same recording, it is not triangulated again. Only losing the
        // painter (or the node) drops it.
        let custom_paint = match node.custom_paint.as_ref() {
            Some(recording) if visible => {
                let node_mask = super::custom_paint::node_mask(&node).map(Arc::new);
                Some(self.prepare_custom_paint(id, recording, node_mask))
            }
            Some(_) => None,
            None => {
                if !self.custom_paint.is_empty() {
                    self.custom_paint.remove(&id);
                }
                None
            }
        };
        if visible
            && node
                .custom_paint
                .as_deref()
                .is_none_or(|recording| recording.default_phase().is_some())
        {
            let surface_border_color = node.style.border_color;
            // Resolve the colour first, then ask for the widths *that colour*
            // strokes. Asking in the other order took the width from a
            // CSS-only view that cannot see a `SemanticColorRole`, so a
            // component that named its border got the colour and a width of
            // zero.
            let color = style.resolved_border_color().or(surface_border_color);
            let edges = style.paint_border_edges_with(color);
            let (surface_background, surface_border_color, surface_border_width) = (
                node.style.background,
                color,
                edges.top.max(edges.right).max(edges.bottom).max(edges.left),
            );
            let widget_root = node
                .widget_paint
                .as_deref()
                .and_then(|w| w.root_surface.as_ref());
            let (surface_background, surface_border_color, surface_border_width) =
                if let Some(rs) = widget_root {
                    (
                        rs.background.unwrap_or(surface_background),
                        rs.border_color.unwrap_or(surface_border_color),
                        rs.border_width.unwrap_or(surface_border_width),
                    )
                } else {
                    (
                        surface_background,
                        surface_border_color,
                        surface_border_width,
                    )
                };
            let bg_paint = if widget_root.is_some_and(|rs| rs.background.is_some()) {
                matching_paint_color(style.paint_colors.background, surface_background)
            } else {
                style.paint_colors.background
            };
            let border_paint = if widget_root.is_some_and(|rs| rs.border_color.is_some()) {
                matching_paint_color(style.paint_colors.border, surface_border_color)
            } else {
                style.paint_colors.border
            };
            if !node
                .widget_paint
                .as_deref()
                .is_some_and(|w| w.suppress_css_root_surface)
                && (style.has_surface_paint()
                    || style.paints_any_border()
                    || (!node
                        .widget_paint
                        .as_ref()
                        .is_some_and(|w| w.suppress_root_surface)
                        && (node.style.background.is_some()
                            || surface_background.is_some()
                            || node.style.border_color.is_some()
                            || surface_border_color.is_some()
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
                        background: super::slot_color(bg_paint, surface_background),
                        border_color: super::slot_color(border_paint, surface_border_color),
                        border_width: surface_border_width,
                        corner_radius: surface_corner_radii(style, bounds.width, bounds.height),
                        shadow: style
                            .paint
                            .primary_box_shadow()
                            .map(ComponentElevation::from_box_shadow)
                            .or_else(|| widget_root.and_then(|rs| rs.fallback_elevation)),
                        surface: {
                            let mut surface = quad_surface_from_style(
                                style,
                                node.style.ring_color,
                                bounds.width,
                                bounds.height,
                            );
                            if is_filter_group(&self.nodes, &node) {
                                surface.filter = None;
                            }
                            let component_owns_border = widget_root.is_some_and(|rs| {
                                rs.border_color.is_some() || rs.border_width.is_some()
                            });
                            if !component_owns_border {
                                // Same colour the uniform width above was
                                // taken for. Without it a border named by a
                                // `SemanticColorRole` has no colour on the
                                // layout, every side reads as zero, and the
                                // quad falls back to stroking all four at the
                                // uniform width — a one-sided border painted
                                // as a frame.
                                let edges = style.paint_border_edges_with(surface_border_color);
                                surface.border_widths =
                                    [edges.top, edges.right, edges.bottom, edges.left];
                                // A side's own authored colour, else its
                                // resolved colour where it strokes; a side
                                // with neither takes the quad's.
                                let resolved = style.paint_border_edge_colors();
                                for (side, resolved) in
                                    surface.border_colors.iter_mut().zip(resolved)
                                {
                                    *side = side.or((resolved[3] > 0.0)
                                        .then(|| nana_ui_core::PaintColor::srgb(resolved)));
                                }
                                surface.border_styles = style.paint_border_style_codes();
                            }
                            Box::new(surface)
                        },
                    },
                });
            }
            let viewer_owns_content = node
                .widget_paint
                .as_deref()
                .is_some_and(|p| p.owns_custom_render);
            if let Some(custom) = node.custom_render.clone()
                && !viewer_owns_content
            {
                if let Some(until) = custom.transition_until
                    && until > std::time::Duration::ZERO
                {
                    self.gpu_payload_live.insert(id, until);
                }
                let clips = if custom.renderer.as_ref() == HOST_TEXTURE_RENDERER {
                    &surface_clips
                } else {
                    &clips
                };
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 1 },
                    node: id,
                    bounds,
                    transform,
                    clips: Arc::clone(clips),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Custom {
                        node: custom,
                        mask: style.paint.mask.clone(),
                        corner_radius: surface_corner_radii(style, bounds.width, bounds.height),
                    },
                });
            }
            let component_owns_text = node.widget_paint.as_ref().is_some_and(|w| w.owns_node_text);
            if let Some(text) = node.text.as_ref().filter(|text| {
                // An empty rich editor has no glyphs but still its caret.
                (!text.value.is_empty() || node.rich_editor.is_some())
                    && !component_owns_text
                    && !self.parent_already_paints_text(&node)
            }) {
                let padding = style.resolved_padding_against(Some(bounds.width));
                let border = style.resolved_border_width();
                let mut text_bounds = SceneRect {
                    x: bounds.x + border + padding.left,
                    y: bounds.y + border + padding.top,
                    width: (bounds.width - border * 2.0 - padding.left - padding.right).max(0.0),
                    height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
                };
                // A checkbox's label sits where layout published its
                // `TextContent` part, beside the indicator part; the scene
                // keeps no copy of the indicator's room.
                if let Some(label_bounds) = node
                    .widget_paint
                    .as_deref()
                    .and_then(|w| w.node_text_bounds)
                {
                    text_bounds = scene_rect(label_bounds);
                }
                // A rich editor is a text node, so its own scroll translates
                // glyphs and marks together; the surrounding chrome stays put.
                if node.rich_editor.is_some() {
                    text_bounds.x -= node.scroll_offset.x;
                    text_bounds.y -= node.scroll_offset.y;
                }
                if let (Some(rich), Some(layout)) = (&node.rich_text, &node.text_layout)
                    && rich.text() == text.value.as_str()
                {
                    let backgrounds = crate::scene::rich_span_background_primitives(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        text_bounds,
                        node.source_style.text_vertical_alignment,
                        layout,
                        rich,
                    );
                    for primitive in backgrounds {
                        self.insert_primitive(primitive);
                    }
                }
                if !node.document_text_selection.is_empty() {
                    // Slot 1 sits under the glyph run at slot 2 (same as
                    // TextInput selection): CSS `::selection` background is
                    // behind text; span color still tints the glyphs.
                    let mut selection = visual_quad_batch(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        // Not slot 1: that is the custom-render primitive's,
                        // and `insert_primitive` would have one silently
                        // replace the other on any node that has both.
                        collection_slot(DOCUMENT_TEXT_SELECTION, DOCUMENT_SELECTION_INDEX),
                        node.document_text_selection.iter().map(|line| SceneRect {
                            x: text_bounds.x + line.x,
                            y: text_bounds.y + line.y,
                            width: line.width,
                            height: line.height,
                        }),
                        VisualQuadStyle::solid(node.document_text_selection_color),
                    );
                    if let ScenePrimitiveKind::QuadBatch { background, .. } = &mut selection.kind {
                        *background = node.style.paint_colors.selection_background.or(*background);
                    }
                    self.insert_primitive(selection);
                }
                // Presentation only: the painter samples it on the motion
                // clock; the scene keeps asking for frames while it moves.
                let mut glyph_presentation =
                    node.glyph_presentation.as_ref().and_then(|presentation| {
                        crate::scene::SceneGlyphPresentation::new(
                            text.value.as_str(),
                            node.rich_text.as_ref(),
                            presentation,
                            node.style.writing_mode,
                        )
                    });
                if let Some(presentation) = &mut glyph_presentation {
                    if (presentation.transitions.entrance.is_some()
                        || presentation.transitions.exit.is_some())
                        && presentation.transitions.viewport.is_none()
                    {
                        let mut viewport = text_bounds;
                        // The clips are on the page; the text is where its
                        // node stands, which a move changes.
                        let [ox, oy] = self.paint_origin(&node);
                        self.origin_bound.insert(id);
                        if let Some(inverse) = super::attributes::inverse(
                            transform
                                .then(AffineTransform::from_matrix([1.0, 0.0, 0.0, 1.0, ox, oy])),
                        ) {
                            for clip in clips.iter() {
                                if let Some(rect) = super::visibility::transform(
                                    clip.bounds,
                                    clip.transform.then(inverse),
                                ) {
                                    let x = viewport.x.max(rect.x);
                                    let y = viewport.y.max(rect.y);
                                    let right =
                                        (viewport.x + viewport.width).min(rect.x + rect.width);
                                    let bottom =
                                        (viewport.y + viewport.height).min(rect.y + rect.height);
                                    viewport = SceneRect {
                                        x,
                                        y,
                                        width: (right - x).max(0.),
                                        height: (bottom - y).max(0.),
                                    };
                                }
                            }
                        }
                        Arc::make_mut(presentation).transitions.viewport =
                            Some(nana_ui_core::LogicalRect::new(
                                viewport.x - text_bounds.x,
                                viewport.y - text_bounds.y,
                                viewport.width,
                                viewport.height,
                            ));
                    }
                    self.glyph_live.insert(id, presentation.live_until());
                }
                let editor_marks = node
                    .rich_editor
                    .as_ref()
                    .zip(node.text_layout.as_ref())
                    .map(|(marks, layout)| {
                        let caret_color = node
                            .style
                            .paint_colors
                            .color
                            .map(nana_ui_core::PaintColor::to_srgb)
                            .or(node.style.color)
                            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
                        crate::scene::rich_editor_mark_primitives(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            text_bounds,
                            node.source_style.text_vertical_alignment,
                            layout,
                            marks,
                            node.document_text_selection_color,
                            node.style.paint_colors.selection_background,
                            caret_color,
                            node.style.paint_colors.color,
                        )
                    });
                let (editor_selection, editor_caret) = editor_marks.unwrap_or_default();
                for primitive in editor_selection {
                    self.insert_primitive(primitive);
                }
                let glyphs = (!text.value.is_empty()).then(|| ScenePrimitive {
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
                        color: super::slot_color(
                            node.style.paint_colors.color,
                            node.style
                                .paint_colors
                                .color
                                .map(nana_ui_core::PaintColor::to_srgb)
                                .or(node.style.color),
                        ),
                        size: node.style.font_size,
                        weight: node.style.font_weight,
                        family: node.style.font_family.as_deref().map(str::to_owned),
                        line_height: node.style.line_height,
                        letter_spacing: node.style.letter_spacing,
                        wrap: style.text_wraps(),
                        ellipsis: style.uses_text_ellipsis(),
                        max_lines: style.resolved_line_clamp(),
                        shaping: if node.editable {
                            TextShaping::Advanced
                        } else {
                            TextShaping::Auto
                        },
                        horizontal_alignment: node.source_style.text_horizontal_alignment,
                        vertical_alignment: node.source_style.text_vertical_alignment,
                        spans: crate::scene::rich_fill_spans(
                            text.value.as_str(),
                            node.rich_text.as_ref(),
                            scene_text_spans(&node, None, &text.value),
                        ),
                        text_shadow: style.paint.text_shadow,
                        underline: style.text_ext.text_decoration.is_some_and(|d| d.underline),
                        line_through: style
                            .text_ext
                            .text_decoration
                            .is_some_and(|d| d.line_through),
                        font_features: style.text_ext.font_features.clone().unwrap_or_default(),
                        italic: node.style.italic,
                        wrap_break: style.text_wrap_break(),
                        opentype: SceneTextOpenType {
                            // What this node was *measured* with, carried by
                            // the Runtime rather than re-derived here: a
                            // renderer that lays the text out again has to
                            // reach the same answer, and neither `white-space`
                            // nor "is this a multiline editor" is visible from
                            // the scene.
                            preserve_lines: node.text_preserve_lines,
                            ..SceneTextOpenType::from_computed(&node.style)
                        },
                        layout: node.text_layout.clone(),
                        // Decorations, outline and shadows are drawn by the
                        // text painter from the paragraph's own lines and
                        // runs, not as strokes across the whole box.
                        rich: crate::scene::SceneRichPaint::from_base(
                            crate::scene::SceneTextEffects {
                                // Inherited, unlike the rest of the node's
                                // own CSS effects.
                                emphasis_color: node.style.text_emphasis.color.paint(),
                                ..crate::scene::css_text_effects(style)
                            },
                            node.rich_text.as_ref(),
                        ),
                        presentation: glyph_presentation.clone(),
                    },
                });
                if let Some(glyphs) = glyphs {
                    self.insert_primitive(glyphs);
                }
                if let (Some(rich), Some(layout)) = (&node.rich_text, &node.text_layout)
                    && !rich.objects().is_empty()
                {
                    let objects = inline_object_primitives(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        text_bounds,
                        node.source_style.text_vertical_alignment,
                        layout,
                        rich,
                        node.rich_editor
                            .as_ref()
                            .is_some_and(|marks| marks.show_editor_objects),
                    );
                    if let Some(presentation) = &glyph_presentation {
                        use unicode_segmentation::UnicodeSegmentation;
                        let content = text.value.as_str();
                        let held: Vec<crate::scene::GlyphObject> = objects
                            .iter()
                            .filter_map(|primitive| {
                                // A tag's pill and a label's picture move with
                                // their object too.
                                let index = (primitive.id.slot & 0xffff_ffff) as usize
                                    & (crate::scene::RICH_TAG_BASE - 1);
                                let placed = layout.layout.objects.get(index)?;
                                Some(crate::scene::GlyphObject {
                                    slot: primitive.id.slot,
                                    target: presentation.transitions.viewport.map_or(
                                        text_bounds,
                                        |r| SceneRect {
                                            x: text_bounds.x + r.x,
                                            y: text_bounds.y + r.y,
                                            width: r.width,
                                            height: r.height,
                                        },
                                    ),
                                    ordinal: content
                                        .get(..placed.offset)
                                        .map_or(0, |before| before.graphemes(true).count())
                                        as u32,
                                    center: [
                                        primitive.bounds.x + primitive.bounds.width * 0.5,
                                        primitive.bounds.y + primitive.bounds.height * 0.5,
                                    ],
                                    opacity: primitive.opacity,
                                    transform: primitive.transform,
                                })
                            })
                            .collect();
                        self.glyph_objects
                            .insert(id, (Arc::clone(presentation), held.into()));
                        // They turn about centres on the page.
                        self.origin_bound.insert(id);
                    }
                    for primitive in objects {
                        self.insert_primitive(primitive);
                    }
                }
                if let Some(caret) = editor_caret {
                    self.insert_primitive(caret);
                }
            }
            let page_origin = self.paint_origin(&node);
            let (overlay_visual_z, overlay_visual_opacity) = if let Some(overlay) = node
                .widget_paint
                .as_deref()
                .and_then(|w| w.floating_overlay.as_ref())
            {
                let base_z = match overlay.kind {
                    nana_ui_runtime::WidgetFloatingOverlayKind::AttachedPopup => {
                        node.z_index.max(nana_ui_runtime::MENU_OVERLAY_Z_INDEX)
                    }
                    nana_ui_runtime::WidgetFloatingOverlayKind::TriggeredSurface => {
                        nana_ui_runtime::MENU_OVERLAY_Z_INDEX
                    }
                };
                (
                    self.resolve_overlay_mount_z(&node, base_z),
                    self.visual_ancestor_opacity(crate::scene::VisualNode::Overlay(id), false),
                )
            } else {
                (0, 1.0)
            };
            let visual_context = VisualPrimitiveContext {
                node: id,
                transform,
                clips: if node
                    .widget_paint
                    .as_deref()
                    .is_some_and(|w| w.content_clip.is_some())
                {
                    &text_input_clips
                } else {
                    &clips
                },
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            if let Some(widget_paint) = node.widget_paint.as_deref() {
                self.emit_widget_paint(
                    &node,
                    widget_paint,
                    page_origin,
                    transform,
                    &clips,
                    &parent_clips,
                    &text_input_clips,
                    opacity,
                    overlay_visual_z,
                    overlay_visual_opacity,
                    node_order,
                );
            } else if !self.widget_vector_paint.is_empty() {
                self.widget_vector_paint.remove(&id);
            }
            if let Some(overlay) = node.drop_hover {
                let radius = corner_radii(style.border_radius.unwrap_or(0.0));
                self.insert_primitive(visual_quad(
                    &visual_context,
                    240,
                    bounds,
                    VisualQuadStyle {
                        background: Some(overlay.fill),
                        border_color: Some(overlay.border),
                        border_width: 2.0,
                        corner_radius: radius,
                    },
                ));
            }
        }
        if let Some(built) = custom_paint {
            // The node's own overflow clip cuts its content, not its own
            // appearance: a painted shadow must survive `overflow: hidden`.
            let paint_clips: Arc<[ClipRegion]> = match clip_path_region(style, bounds, transform) {
                Some(region) => {
                    let mut chain = parent_clips.to_vec();
                    chain.push(region);
                    chain.into()
                }
                None => Arc::clone(&parent_clips),
            };
            self.emit_custom_paint(&node, &built, transform, &paint_clips, opacity, node_order);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_widget_paint(
        &mut self,
        node: &ExtractedNode,
        paint: &nana_ui_runtime::WidgetPaintOutput,
        page_origin: [f32; 2],
        transform: AffineTransform,
        clips: &Arc<[ClipRegion]>,
        parent_clips: &Arc<[ClipRegion]>,
        content_clips: &Arc<[ClipRegion]>,
        opacity: f32,
        overlay_visual_z: i32,
        overlay_visual_opacity: f32,
        node_order: usize,
    ) {
        use nana_ui_runtime::{
            WidgetBatchColors, WidgetFloatingOverlayKind, WidgetGlyph, WidgetPaintOp,
            WidgetTaggedOp,
        };

        let id = node.id;
        let floating_kind = paint.floating_overlay.as_ref().map(|overlay| overlay.kind);
        let node_local_opacity = local_opacity(node);
        let resolve_placement = |tagged: &WidgetTaggedOp,
                                 force_parent_clips: bool|
         -> (AffineTransform, Arc<[ClipRegion]>, i32, f32) {
            let (clip_transform, base_clips, mut op_z, base_opacity) = if tagged.in_floating_overlay
            {
                match floating_kind {
                    Some(WidgetFloatingOverlayKind::TriggeredSurface) => (
                        AffineTransform::IDENTITY,
                        Arc::from([]),
                        overlay_visual_z,
                        overlay_visual_opacity.clamp(0.0, 1.0),
                    ),
                    Some(WidgetFloatingOverlayKind::AttachedPopup) => (
                        transform,
                        Arc::clone(parent_clips),
                        overlay_visual_z,
                        (node_local_opacity * overlay_visual_opacity).clamp(0.0, 1.0),
                    ),
                    None => (transform, Arc::clone(parent_clips), node.z_index, opacity),
                }
            } else {
                let base = if tagged.use_content_clip {
                    content_clips
                } else if force_parent_clips || tagged.use_parent_clips {
                    parent_clips
                } else {
                    clips
                };
                let mut z = node.z_index.saturating_add(tagged.z_offset);
                if let Some(min_z) = tagged.min_z_index {
                    z = z.max(min_z);
                }
                (transform, Arc::clone(base), z, opacity)
            };
            if tagged.in_floating_overlay && (tagged.z_offset != 0 || tagged.min_z_index.is_some())
            {
                op_z = op_z.saturating_add(tagged.z_offset);
                if let Some(min_z) = tagged.min_z_index {
                    op_z = op_z.max(min_z);
                }
            }
            let op_clips = if tagged.extra_clip.is_some() || tagged.rounded_extra_clip.is_some() {
                let mut chain = base_clips.to_vec();
                if let Some(extra) = tagged.extra_clip {
                    chain.push(ClipRegion::axis_aligned(scene_rect(extra), clip_transform));
                }
                if let Some((clip, radius)) = tagged.rounded_extra_clip {
                    chain.push(ClipRegion {
                        bounds: scene_rect(clip),
                        transform: clip_transform,
                        corner_radius: radius,
                        polygon_clip: None,
                    });
                }
                chain.into()
            } else {
                base_clips
            };
            let op_transform = if let Some([tx, ty]) = tagged.local_translation {
                clip_transform.then(AffineTransform::from_matrix([1.0, 0.0, 0.0, 1.0, tx, ty]))
            } else {
                clip_transform
            };
            let op_opacity = if (tagged.opacity_multiplier - 1.0).abs() > f32::EPSILON {
                (base_opacity * tagged.opacity_multiplier).clamp(0.0, 1.0)
            } else {
                base_opacity
            };
            (op_transform, op_clips, op_z, op_opacity)
        };
        let mut vectors: Vec<(
            u64,
            nana_ui_runtime::LayoutBox,
            Arc<nana_ui_runtime::PaintRecording>,
        )> = Vec::new();
        for tagged in &paint.ops {
            let slot = tagged.slot;
            match &tagged.op {
                WidgetPaintOp::Quad(quad) => {
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, quad.use_parent_clips);
                    let visual_context = VisualPrimitiveContext {
                        node: id,
                        transform: op_transform,
                        clips: &op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                    };
                    let mut primitive = visual_quad_with_paint(
                        &visual_context,
                        slot,
                        scene_rect(quad.rect),
                        VisualQuadStyle {
                            background: quad.fill,
                            border_color: quad.border.map(|(color, _)| color),
                            border_width: quad.border.map_or(0.0, |(_, width)| width),
                            corner_radius: quad.corner_radius,
                        },
                        quad.fill_paint,
                        quad.border_paint,
                    );
                    if let ScenePrimitiveKind::Quad {
                        shadow, surface, ..
                    } = &mut primitive.kind
                    {
                        if let Some(elevation) = quad.elevation {
                            *shadow = Some(elevation);
                        }
                        if let Some(image) = &quad.content_image {
                            surface.content_image = Some(image.clone());
                        }
                        if let Some(image) = &quad.background_image {
                            surface.background_image = Some(image.clone());
                        }
                        if quad.backdrop_blur > 0.0 {
                            let blur = nana_ui_core::BackdropFilter {
                                blur_radius: quad
                                    .backdrop_blur
                                    .clamp(0.0, nana_ui_core::BackdropFilter::MAX_BLUR_RADIUS),
                                saturate: 1.0,
                            };
                            if blur.is_active() {
                                surface.backdrop_filter = Some(blur);
                            }
                        }
                    }
                    self.insert_primitive(primitive);
                }
                WidgetPaintOp::QuadBatch(batch) => {
                    if batch.rects.is_empty() {
                        continue;
                    }
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    match &batch.colors {
                        WidgetBatchColors::Uniform(color) | WidgetBatchColors::Selection(color) => {
                            let visual_context = VisualPrimitiveContext {
                                node: id,
                                transform: op_transform,
                                clips: &op_clips,
                                opacity: op_opacity,
                                z_index,
                                document_order: node_order,
                            };
                            let mut primitive = visual_quad_batch(
                                &visual_context,
                                slot,
                                batch.rects.iter().copied().map(scene_rect),
                                VisualQuadStyle {
                                    background: Some(*color),
                                    border_color: batch.border.map(|(color, _)| color),
                                    border_width: batch.border.map_or(0.0, |(_, width)| width),
                                    corner_radius: batch.corner_radius,
                                },
                            );
                            if let ScenePrimitiveKind::QuadBatch { background, .. } =
                                &mut primitive.kind
                            {
                                if let Some(fill_paint) = batch.fill_paint {
                                    *background = Some(fill_paint);
                                } else if matches!(batch.colors, WidgetBatchColors::Selection(_))
                                    && let Some(selection_paint) =
                                        node.style.paint_colors.selection_background
                                {
                                    *background = Some(selection_paint);
                                }
                            }
                            self.insert_primitive(primitive);
                        }
                        WidgetBatchColors::None => {
                            let visual_context = VisualPrimitiveContext {
                                node: id,
                                transform: op_transform,
                                clips: &op_clips,
                                opacity: op_opacity,
                                z_index,
                                document_order: node_order,
                            };
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                slot,
                                batch.rects.iter().copied().map(scene_rect),
                                VisualQuadStyle {
                                    background: None,
                                    border_color: batch.border.map(|(color, _)| color),
                                    border_width: batch.border.map_or(0.0, |(_, width)| width),
                                    corner_radius: batch.corner_radius,
                                },
                            ));
                        }
                        WidgetBatchColors::PerQuad(colors) => {
                            let visual_context = VisualPrimitiveContext {
                                node: id,
                                transform: op_transform,
                                clips: &op_clips,
                                opacity: op_opacity,
                                z_index,
                                document_order: node_order,
                            };
                            self.insert_primitive(visual_quad_color_batch(
                                &visual_context,
                                slot,
                                batch
                                    .rects
                                    .iter()
                                    .copied()
                                    .map(scene_rect)
                                    .zip(colors.iter().copied()),
                                VisualQuadStyle {
                                    background: None,
                                    border_color: batch.border.map(|(color, _)| color),
                                    border_width: batch.border.map_or(0.0, |(_, width)| width),
                                    corner_radius: batch.corner_radius,
                                },
                            ));
                        }
                    }
                }
                WidgetPaintOp::IconBatch {
                    rects, icon, color, ..
                } => {
                    if rects.is_empty() {
                        continue;
                    }
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    let visual_context = VisualPrimitiveContext {
                        node: id,
                        transform: op_transform,
                        clips: &op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                    };
                    self.insert_primitive(batch_primitive(
                        &visual_context,
                        slot,
                        rects.iter().copied().map(scene_rect).collect(),
                        |bounds| ScenePrimitiveKind::IconBatch {
                            bounds,
                            icon: *icon,
                            color: *color,
                        },
                    ));
                }
                WidgetPaintOp::Text(text) => {
                    let (op_transform, mut op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    let paint_color = text.paint_color.or_else(|| {
                        node.style
                            .paint_colors
                            .color
                            .filter(|paint| text.color.is_none_or(|c| c == paint.to_srgb()))
                    });
                    let fallback_color = text
                        .color
                        .or_else(|| {
                            node.style
                                .paint_colors
                                .color
                                .map(nana_ui_core::PaintColor::to_srgb)
                        })
                        .or(node.style.color);
                    let spans = if text.suppress_spans || text.overlay_chrome {
                        Vec::new()
                    } else {
                        let base = super::scene_text_spans_with_adopt(
                            node,
                            None,
                            text.content.as_str(),
                            text.adopt_node_text_spans,
                        );
                        if let Some(rich) = text.rich_text.as_ref() {
                            crate::scene::rich_fill_spans(text.content.as_str(), Some(rich), base)
                        } else {
                            base
                        }
                    };
                    let unstyled = text.unstyled_glyph;
                    let unstyled_decor = unstyled || text.overlay_chrome;
                    let advanced_content = if text.overlay_chrome {
                        false
                    } else {
                        match text.retained_text {
                            nana_ui_runtime::WidgetRetainedText::None => false,
                            nana_ui_runtime::WidgetRetainedText::NodeTextIfRich => {
                                (node.rich_text.is_some() || !node.children.is_empty())
                                    && node
                                        .text
                                        .as_ref()
                                        .is_some_and(|n| n.value.as_str() == text.content.as_str())
                            }
                            nana_ui_runtime::WidgetRetainedText::AlwaysClipOverflow => node
                                .text
                                .as_ref()
                                .is_some_and(|n| n.value.as_str() == text.content.as_str()),
                        }
                    };
                    let retained = advanced_content
                        .then_some(node.text_layout.as_ref())
                        .flatten();
                    let mut text_bounds = scene_rect(text.bounds);
                    if let Some(retained) = retained
                        && !retained.layout.is_vertical()
                        && retained.layout.constraints.max_width_px != Some(text.bounds.width)
                    {
                        text_bounds.x -= retained.layout.bounds.x;
                        text_bounds.width = retained
                            .layout
                            .constraints
                            .max_width_px
                            .unwrap_or(text_bounds.width);
                    }
                    if text.retained_text == nana_ui_runtime::WidgetRetainedText::AlwaysClipOverflow
                        && retained.is_none_or(|r| r.layout.overflow != Default::default())
                    {
                        let mut clipped = op_clips.to_vec();
                        clipped.push(ClipRegion::axis_aligned(
                            scene_rect(text.bounds),
                            op_transform,
                        ));
                        op_clips = clipped.into();
                    }
                    let mut font_features = if unstyled_decor {
                        Vec::new()
                    } else {
                        node.source_style
                            .layout
                            .text_ext
                            .font_features
                            .clone()
                            .unwrap_or_default()
                    };
                    if text.tabular_nums
                        && !font_features.iter().any(|feature| feature.tag == *b"tnum")
                    {
                        font_features.push(nana_ui_core::FontFeatureSetting::new(*b"tnum", 1));
                    }
                    let mut opentype = if unstyled_decor {
                        SceneTextOpenType::default()
                    } else {
                        SceneTextOpenType {
                            preserve_lines: text.preserve_lines.unwrap_or(node.text_preserve_lines),
                            ..SceneTextOpenType::from_computed(&node.style)
                        }
                    };
                    if text.force_horizontal_writing {
                        opentype.writing_mode = nana_ui_core::WritingModeSpec::HorizontalTb;
                    }
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot },
                        node: id,
                        bounds: text_bounds,
                        transform: op_transform,
                        clips: op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Text {
                            content: text.content.clone(),
                            color: super::slot_color(paint_color, fallback_color),
                            size: text.font_size,
                            weight: text.font_weight,
                            family: (!unstyled)
                                .then(|| node.style.font_family.as_deref().map(str::to_owned))
                                .flatten(),
                            line_height: text.line_height.unwrap_or_else(|| {
                                (!unstyled).then_some(node.style.line_height).flatten()
                            }),
                            letter_spacing: if unstyled {
                                0.0
                            } else {
                                node.style.letter_spacing
                            },
                            wrap: text.wrap,
                            ellipsis: text.ellipsis,
                            max_lines: text.max_lines.and_then(|n| u16::try_from(n).ok()),
                            shaping: if text.advanced_shaping || node.editable {
                                TextShaping::Advanced
                            } else {
                                TextShaping::Auto
                            },
                            horizontal_alignment: text.horizontal_alignment,
                            vertical_alignment: text.vertical_alignment,
                            spans,
                            text_shadow: if unstyled_decor {
                                None
                            } else {
                                node.source_style.layout.paint.text_shadow
                            },
                            underline: if unstyled_decor {
                                false
                            } else {
                                text.underline.unwrap_or_else(|| {
                                    node.source_style
                                        .layout
                                        .text_ext
                                        .text_decoration
                                        .is_some_and(|d| d.underline)
                                })
                            },
                            line_through: if unstyled_decor {
                                false
                            } else {
                                text.line_through.unwrap_or_else(|| {
                                    node.source_style
                                        .layout
                                        .text_ext
                                        .text_decoration
                                        .is_some_and(|d| d.line_through)
                                })
                            },
                            font_features,
                            italic: if unstyled_decor {
                                false
                            } else {
                                text.italic.unwrap_or(node.style.italic)
                            },
                            wrap_break: if unstyled_decor {
                                nana_ui_core::TextWrapBreak::Word
                            } else {
                                node.source_style.layout.text_wrap_break()
                            },
                            opentype,
                            layout: retained.cloned(),
                            rich: advanced_content
                                .then(|| {
                                    crate::scene::SceneRichPaint::from_base(
                                        crate::scene::css_text_effects(&node.source_style.layout),
                                        node.rich_text.as_ref(),
                                    )
                                })
                                .flatten(),
                            presentation: None,
                        },
                    });
                }
                WidgetPaintOp::FocusRing {
                    bounds: ring_bounds,
                    color,
                    extra_outset,
                } => {
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, true);
                    let (ring_rect, ring_style) = focus_ring_quad(
                        &node.source_style.layout,
                        scene_rect(*ring_bounds),
                        self.focus_ring,
                        *extra_outset,
                        *color,
                    );
                    let visual_context = VisualPrimitiveContext {
                        node: id,
                        transform: op_transform,
                        clips: &op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                    };
                    self.insert_primitive(visual_quad_with_paint(
                        &visual_context,
                        slot,
                        ring_rect,
                        ring_style,
                        None,
                        matching_paint_color(node.style.paint_colors.color, Some(*color)).or_else(
                            || matching_paint_color(node.style.paint_colors.border, Some(*color)),
                        ),
                    ));
                }
                WidgetPaintOp::Icon(icon) => {
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    let mut icon_bounds = scene_rect(icon.bounds);
                    if icon.align_to_adjacent_text {
                        let base = scene_rect(Self::record_box(node));
                        let (y, read) =
                            self.icon_y_aligned_to_adjacent_text(node, base, icon_bounds.height);
                        match read {
                            super::PlacementRead::None => {}
                            super::PlacementRead::Offset => {
                                self.offset_bound.insert(id);
                            }
                            super::PlacementRead::Page => {
                                self.origin_bound.insert(id);
                            }
                        }
                        icon_bounds.y = y - base.y;
                    }
                    let paint_color = icon.paint_color.or_else(|| {
                        node.style
                            .paint_colors
                            .color
                            .filter(|paint| icon.color.is_none_or(|c| c == paint.to_srgb()))
                    });
                    let color = super::slot_color(paint_color, icon.color.or(node.style.color));
                    let kind = match icon.glyph {
                        WidgetGlyph::Icon(glyph) => ScenePrimitiveKind::Icon { icon: glyph, color },
                        WidgetGlyph::Spinner { phase } => {
                            ScenePrimitiveKind::Spinner { phase, color }
                        }
                    };
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot },
                        node: id,
                        bounds: icon_bounds,
                        transform: op_transform,
                        clips: op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                        kind,
                    });
                }
                WidgetPaintOp::Image(image) => {
                    if image.source.is_empty() {
                        continue;
                    }
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot },
                        node: id,
                        bounds: scene_rect(image.bounds),
                        transform: op_transform,
                        clips: op_clips,
                        opacity: op_opacity * image.opacity,
                        z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Custom {
                            node: CustomRenderNode::new(
                                HOST_TEXTURE_RENDERER,
                                Arc::clone(&image.source),
                                0,
                            )
                            .with_fit(image.fit)
                            .with_sampling(image.sampling),
                            mask: None,
                            corner_radius: image.corner_radius,
                        },
                    });
                }
                WidgetPaintOp::Stroke(stroke) => {
                    if stroke.points.len() < 2 {
                        continue;
                    }
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    let points = if stroke.snap_hairline && stroke.points.len() == 2 {
                        self.origin_bound.insert(id);
                        let [a, b] = [stroke.points[0], stroke.points[1]];
                        let crisp_x = |v: f32| (page_origin[0] + v).floor() + 0.5 - page_origin[0];
                        let crisp_y = |v: f32| (page_origin[1] + v).floor() + 0.5 - page_origin[1];
                        if (a[0] - b[0]).abs() < 0.01 {
                            vec![[crisp_x(a[0]), a[1]], [crisp_x(b[0]), b[1]]]
                        } else if (a[1] - b[1]).abs() < 0.01 {
                            vec![[a[0], crisp_y(a[1])], [b[0], crisp_y(b[1])]]
                        } else {
                            stroke.points.clone()
                        }
                    } else {
                        stroke.points.clone()
                    };
                    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
                        f32::INFINITY,
                        f32::INFINITY,
                        f32::NEG_INFINITY,
                        f32::NEG_INFINITY,
                    );
                    for &[x, y] in &points {
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                    let pad = stroke.width.max(1.0);
                    let stroke_bounds = SceneRect {
                        x: min_x - pad,
                        y: min_y - pad,
                        width: (max_x - min_x) + pad * 2.0,
                        height: (max_y - min_y) + pad * 2.0,
                    };
                    let visual_context = VisualPrimitiveContext {
                        node: id,
                        transform: op_transform,
                        clips: &op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                    };
                    let mut primitive = visual_stroke(
                        &visual_context,
                        slot,
                        stroke_bounds,
                        points,
                        stroke.width,
                        stroke.color,
                    );
                    if let ScenePrimitiveKind::Stroke { cap, pattern, .. } = &mut primitive.kind {
                        *cap = match stroke.cap {
                            nana_ui_runtime::LineCap::Butt => StrokeCap::Butt,
                            nana_ui_runtime::LineCap::Round => StrokeCap::Round,
                            nana_ui_runtime::LineCap::Square => StrokeCap::Square,
                        };
                        if let Some(dash) = &stroke.dash {
                            *pattern = Some(Box::new(StrokePattern {
                                dash: dash.clone(),
                                ..Default::default()
                            }));
                        }
                    }
                    self.insert_primitive(primitive);
                }
                WidgetPaintOp::Vector { bounds, recording } => {
                    vectors.push((tagged.slot, *bounds, Arc::clone(recording)));
                }
                WidgetPaintOp::CustomGpu {
                    bounds,
                    node: custom,
                } => {
                    let (op_transform, op_clips, z_index, op_opacity) =
                        resolve_placement(tagged, false);
                    if let Some(until) = custom.transition_until
                        && until > std::time::Duration::ZERO
                    {
                        self.gpu_payload_live.insert(id, until);
                    }
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot },
                        node: id,
                        bounds: scene_rect(*bounds),
                        transform: op_transform,
                        clips: op_clips,
                        opacity: op_opacity,
                        z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Custom {
                            node: custom.clone(),
                            mask: node.source_style.layout.paint.mask.clone(),
                            corner_radius: [0.0; 4],
                        },
                    });
                }
            }
        }
        if vectors.is_empty() {
            if !self.widget_vector_paint.is_empty() {
                self.widget_vector_paint.remove(&id);
            }
        } else {
            self.emit_widget_vector_paint(node, &vectors, transform, clips, opacity, node_order);
        }
    }
}
