//! Committed node projection into Scene input.

use super::*;

impl UiWorld {
    pub(super) fn stacking_z_index_memo(&self, id: StableNodeId, memo: &mut AncestorMemo) -> i32 {
        #[cfg(feature = "controls")]
        let previews = !self.reorder_previews.is_empty();
        #[cfg(not(feature = "controls"))]
        let previews = false;
        if self.z_index_nodes == 0 && !previews {
            return 0;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut z_index = 0;
        while let Some(node) = current {
            if let Some(&known) = memo.stacking.get(&node) {
                z_index = known;
                break;
            }
            if !self.nodes.contains(node) {
                break;
            };
            let z = self
                .nodes
                .get(node)
                .and_then(|record| record.style.layout.z_index)
                .or_else(|| {
                    self.parent_triggered_overlay(node)
                        .map(|_| crate::popover::MENU_OVERLAY_Z_INDEX)
                });
            #[cfg(feature = "controls")]
            let z = z.or_else(|| self.reorder_preview_z_index(node));
            if let Some(z) = z {
                z_index = z;
                memo.stacking.insert(node, z);
                break;
            }
            memo.chain.push(node);
            current = self
                .nodes
                .get(node)
                .and_then(|record| record.hierarchy.parent);
        }
        for node in memo.chain.drain(..) {
            memo.stacking.insert(node, z_index);
        }
        z_index
    }
}

impl UiWorld {
    /// Write the installed palette's paint colours over `style`, `id`'s
    /// resolved style from before a palette-only theme switch, which
    /// re-resolves no style. Returns the text colour.
    fn repaint_style(
        &self,
        id: StableNodeId,
        style: &mut Arc<ComputedStyle>,
        inherited_color: Option<[f32; 4]>,
    ) -> Option<[f32; 4]> {
        let model = self.scoped_style_model(style.color_scope.as_deref());
        let (foreground, color, background, border_color) =
            self.palette_paint_colors(id, inherited_color, model);
        let ring_color = self.palette_ring_color(id, model);
        if style.foreground != foreground
            || style.color != color
            || style.background != background
            || style.border_color != border_color
            || style.ring_color != ring_color
        {
            let style = Arc::make_mut(style);
            style.foreground = foreground;
            style.color = color;
            style.background = background;
            style.border_color = border_color;
            style.ring_color = ring_color;
        }
        color
    }

    /// `id`'s resolved style in the installed palette: what extraction
    /// paints it with.
    pub(super) fn painted_style(&self, id: StableNodeId) -> Option<Arc<ComputedStyle>> {
        let node = self.nodes.get(id)?;
        let mut style = Arc::clone(&node.resolved.0);
        if node.resolved.1 != self.palette_epoch {
            let inherited_color = self.inherited_palette_color(node.hierarchy.parent);
            self.repaint_style(id, &mut style, inherited_color);
        }
        Some(style)
    }

    pub(super) fn extract_node_memo(
        &self,
        id: StableNodeId,
        memo: &mut AncestorMemo,
    ) -> Option<ExtractedNode> {
        if !self.presence_live_memo(id, memo) {
            return None;
        }
        let (
            mut style,
            resolved_epoch,
            parent,
            kind,
            has_text,
            source_style,
            resolved_layout,
            hierarchy_parent,
            hierarchy_children,
            scroll_offset,
            text,
            text_metrics,
            document,
            layout_padding,
            preserve_newlines,
            local_layout,
            layout_frame,
        ) = {
            let node = self.nodes.get(id)?;
            let kind = Arc::clone(&node.kind);
            let has_text = matches!(kind.as_ref(), NodeKind::Text) || !node.text.value.is_empty();
            (
                Arc::clone(&node.resolved.0),
                node.resolved.1,
                node.hierarchy.parent,
                kind,
                has_text,
                node.style.clone(),
                Arc::clone(&node.resolved_layout),
                node.hierarchy.parent,
                Arc::clone(&node.hierarchy.children),
                node.scroll_offset,
                has_text.then(|| node.text.clone()),
                has_text.then_some(node.text_metrics),
                node.document,
                node.layout_padding,
                node.style.layout.white_space.preserve_newlines(),
                node.local_layout,
                node.layout_frame,
            )
        };
        // Extraction also runs during retained writeback, before the next
        // immutable result is published. Keep the compatibility fallback for
        // this transitional projection; scene/hit geometry uses canonical
        // accessors once the batch is committed.
        let layout = self.component_layout_box(id)?;
        if resolved_epoch != self.palette_epoch {
            let inherited_color = parent.and_then(|parent| {
                memo.color
                    .get(&parent)
                    .copied()
                    .or_else(|| self.inherited_palette_color(Some(parent)))
            });
            if let Some(color) = self.repaint_style(id, &mut style, inherited_color) {
                memo.color.insert(id, color);
            }
        }
        // The colours this node paints in: the installed theme under its
        // colour scope.
        let model = self.scoped_style_model(style.color_scope.as_deref());
        if !self.inline_presentation_visible_memo(id, memo) {
            let style = Arc::make_mut(&mut style);
            style.visible = false;
            style.box_visible = false;
        }
        let mut standard_visual = if self.nodes.has_visuals() {
            self.nodes.visual(id).cloned()
        } else {
            None
        };
        if let Some((busy, danger, is_confirm)) = self.confirm_action_effect(id) {
            if busy && !is_confirm {
                Arc::make_mut(&mut style).color = Some(model.palette.muted.as_rgba_array());
            }
            if is_confirm
                && let Some(StandardVisual::Button { kind, loading, .. }) = standard_visual.as_mut()
            {
                *kind = if danger {
                    nana_ui_core::ButtonKind::Danger
                } else {
                    nana_ui_core::ButtonKind::Primary
                };
                *loading = busy;
            }
        }
        let mut component_geometry = standard_visual
            .as_ref()
            .and_then(|visual| self.local_component_geometry(id, visual, style.as_ref()))
            .map(Box::new);
        if self.caret_hidden(id)
            && let Some(crate::ComponentGeometry::TextInput {
                caret,
                additional_carets,
                ..
            }) = component_geometry.as_deref_mut()
        {
            // The hidden half of the blink. The caret's geometry stays
            // where the IME and scrolling read it.
            *caret = None;
            additional_carets.clear();
        }
        let standard_visual_foreground = standard_visual
            .as_ref()
            .map(|visual| self.standard_visual_foreground(model, visual, style.color));
        // The style's painter is already in hand; only a tree someone set a
        // painter on apart from the style pays a lookup.
        let painter = if self.painter_overrides.is_empty() {
            source_style.painter.as_ref()
        } else {
            self.painter_overrides
                .get(&id)
                .or(source_style.painter.as_ref())
        };
        let custom_paint = painter.map(|painter| self.custom_paint(id, painter, layout));
        let mut source_style = NodeStyle::clone(&source_style);
        // The node's design intent is already resolved into `resolved_layout`,
        // on write. Resolving it here instead meant an `Arc::make_mut` copy of
        // a 4.8 KB `LayoutStyle` per control per frame.
        source_style.layout = self.motion_layout(id, &resolved_layout);
        // Open menu content is viewport-fixed for layout and hit testing; the
        // Scene must paint it so too, or the trigger's clips and stacking
        // groups cut the card that was laid out above them.
        self.triggered_overlay_layout(id, &mut source_style.layout);
        // Scene receives the layout-resolved padding; it must not resolve %
        // against the painted node's own width. Authored world style stays intact.
        // With no length animation, used padding is the layout pass value or
        // the resolved style itself, so an unset pass cannot disagree.
        let padding = if self.layout_length_tracks.is_empty() {
            layout_padding
        } else {
            Some(self.used_layout_padding(id))
        };
        if let Some(padding) = padding
            && source_style
                .layout
                .resolved_padding_against(Some(layout.width))
                != padding
        {
            let layout = Arc::make_mut(&mut source_style.layout);
            layout.padding = None;
            layout.logical_padding = Default::default();
            layout.padding_top = Some(nana_ui_core::LengthSpec::Px(padding.top));
            layout.padding_right = Some(nana_ui_core::LengthSpec::Px(padding.right));
            layout.padding_bottom = Some(nana_ui_core::LengthSpec::Px(padding.bottom));
            layout.padding_left = Some(nana_ui_core::LengthSpec::Px(padding.left));
        }
        let document_selection = self
            .document_text_selections
            .get(&document)
            .filter(|selection| selection.node == id && !selection.is_empty());
        let mut text_spans = if has_text {
            self.extracted_text_spans(id)
        } else {
            Vec::new()
        };
        let selection_paint_color = style.paint_colors.selection_color;
        let selection_color = selection_paint_color
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(style.selection_color);
        if let (Some(selection), Some(color)) = (document_selection, selection_color) {
            let (start, end) = selection.ordered();
            text_spans = merge_inlay_glyph_spans_with_paint(
                text_spans,
                &[(start, end)],
                color,
                selection_paint_color,
            );
        }
        let document_text_selection = document_selection
            .map(|selection| selection.lines.clone())
            .unwrap_or_default();
        let document_text_selection_color = style
            .paint_colors
            .selection_background
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(style.selection_background)
            .unwrap_or_else(|| model.palette.accent_soft.as_rgba_array());
        // Only while the node still shows that value's text: spans over other
        // bytes would paint the wrong characters.
        let rich_text = has_text
            .then(|| self.nodes.rich_text(id))
            .flatten()
            .filter(|rich| {
                text.as_ref()
                    .is_some_and(|text| text.value.as_str() == rich.text())
            })
            .cloned();
        Some(ExtractedNode {
            layout_result: self.local_layout_result(id).cloned(),
            id,
            kind,
            parent: hierarchy_parent,
            children: hierarchy_children,
            layout,
            local_layout,
            layout_frame,
            scroll_offset: self.rich_editor_scroll_offset(id).unwrap_or(scroll_offset),
            z_index: self.stacking_z_index_memo(id, memo),
            source_style,
            style,
            text,
            text_metrics,
            text_layout: has_text
                .then(|| self.text_layout(id))
                .flatten()
                .map(|(id, layout)| crate::RetainedTextLayout {
                    id,
                    layout: Arc::clone(layout),
                }),
            // Not gated on `has_text`: an editor's displayed value is built
            // by its presentation, not held in `text`, and it is exactly the
            // node whose newlines must survive.
            text_preserve_lines: if self.nodes.has_visuals() {
                self.text_preserves_lines(id)
            } else {
                preserve_newlines
            },
            // Paint's question, so paint's answer: a control put into focus
            // by a click is focused and does not draw a ring about it.
            focused: self.focus_visible(document) == Some(id),
            editable: self.nodes.has_text_inputs() && self.nodes.text_input(id).is_some(),
            text_spans,
            rich_text,
            // The caret shows only on the focused editor, and not in the
            // hidden half of its blink; its selection stays visible either
            // way.
            glyph_presentation: self.nodes.glyph_presentation(id).cloned(),
            rich_editor: self.nodes.rich_editor_marks(id).map(|marks| {
                let mut marks = marks.clone();
                if self.focused(document) != Some(id) || self.caret_hidden(id) {
                    marks.caret = None;
                }
                marks
            }),
            standard_visual,
            component_geometry,
            standard_visual_foreground,
            chrome_radii: nana_ui_core::ChromeRadii::resolve(
                self.style_model.metrics,
                self.theme.recipes().shape(),
            ),
            custom_render: self
                .nodes
                .has_custom_render()
                .then(|| self.nodes.custom_render(id).cloned())
                .flatten(),
            drop_hover: (self.drop_hover.map(|(hover, _)| hover) == Some(id)).then(|| {
                crate::DropHoverOverlay {
                    fill: model.palette.accent_soft.as_rgba_array(),
                    border: model.palette.accent.as_rgba_array(),
                }
            }),
            document_text_selection,
            document_text_selection_color,
            compositor: self.extracted_compositor(id),
            custom_paint,
        })
    }

    /// The node's painter output for its current size, theme and state,
    /// recorded only when that key moved since the last extraction.
    pub(super) fn custom_paint(
        &self,
        id: StableNodeId,
        painter: &crate::NodePainter,
        layout: LayoutBox,
    ) -> Arc<crate::PaintRecording> {
        let size = [layout.width.max(0.0), layout.height.max(0.0)];
        let state = self.paint_state(id);
        let key = painter.cache_key(size, self.palette_epoch, state);
        let base = self
            .nodes
            .get(id)
            .map(|node| Arc::clone(&node.resolved.0))
            .unwrap_or_default();
        if let Some(held) = self.paint_recordings.borrow().get(&id)
            && held.key == key
            && held.measured_text.as_ref().is_none_or(|(style, backend)| {
                *backend == self.text_backend
                    && (Arc::ptr_eq(style, &base)
                        || !crate::text_node::classify_computed_style_change(style, &base)
                            .intersects(
                                crate::text_node::TextDirty::SHAPE_STYLE
                                    .union(crate::text_node::TextDirty::CONSTRAINT),
                            ))
            })
        {
            return Arc::clone(&held.recording);
        }
        let engine = self.paint_text_engine.clone();
        let measured = std::cell::Cell::new(false);
        let measure = |text: &crate::PaintText, size: f32, max_width: Option<f32>| {
            measured.set(true);
            measure_paint_text(id, engine.as_ref(), &base, text, size, max_width)
        };
        let recording = Arc::new(crate::custom_paint::record(
            painter.painter(),
            &self.theme,
            size,
            state,
            &measure,
        ));
        let mut recordings = self.paint_recordings.borrow_mut();
        // The same ops as last time keep the same `Arc`: the scene reuses
        // its triangles by identity, so a painter that ignores, say, hover is
        // not re-triangulated on it.
        let recording = match recordings.get(&id) {
            Some(held) if *held.recording == *recording => Arc::clone(&held.recording),
            _ => recording,
        };
        let latest = match recordings.get(&id) {
            Some(held) => {
                *held
                    .latest
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::clone(&recording);
                Arc::clone(&held.latest)
            }
            None => Arc::new(std::sync::RwLock::new(Arc::clone(&recording))),
        };
        recordings.insert(
            id,
            super::PaintCacheEntry {
                key,
                recording: Arc::clone(&recording),
                latest,
                measured_text: measured.get().then_some((base, self.text_backend)),
            },
        );
        recording
    }

    /// `id`'s card surface painter's output for a card `surface` divided
    /// into `sections`, recorded only when the painter's key, the card's size
    /// or sections, or the theme moved since the last geometry.
    pub(super) fn modal_surface_paint(
        &self,
        id: StableNodeId,
        painter: &crate::NodePainter,
        surface: LayoutBox,
        sections: crate::ModalSurfaceSections,
    ) -> Arc<crate::PaintRecording> {
        let size = [surface.width.max(0.0), surface.height.max(0.0)];
        let key = painter.surface_cache_key(size, self.palette_epoch, &sections);
        let base = self
            .nodes
            .get(id)
            .map(|node| Arc::clone(&node.resolved.0))
            .unwrap_or_default();
        if let Some(held) = self.modal_surface_recordings.borrow().get(&id)
            && held.key == key
            && held.measured_text.as_ref().is_none_or(|(style, backend)| {
                *backend == self.text_backend
                    && (Arc::ptr_eq(style, &base)
                        || !crate::text_node::classify_computed_style_change(style, &base)
                            .intersects(
                                crate::text_node::TextDirty::SHAPE_STYLE
                                    .union(crate::text_node::TextDirty::CONSTRAINT),
                            ))
            })
        {
            return Arc::clone(&held.recording);
        }
        let engine = self.paint_text_engine.clone();
        let measured = std::cell::Cell::new(false);
        let measure = |text: &crate::PaintText, size: f32, max_width: Option<f32>| {
            measured.set(true);
            measure_paint_text(id, engine.as_ref(), &base, text, size, max_width)
        };
        let recording = Arc::new(crate::custom_paint::record_surface(
            painter.painter(),
            &self.theme,
            size,
            sections,
            &measure,
        ));
        let mut recordings = self.modal_surface_recordings.borrow_mut();
        // The same ops keep the same `Arc`, so the scene reuses the card's
        // triangles by identity.
        let recording = match recordings.get(&id) {
            Some(held) if *held.recording == *recording => Arc::clone(&held.recording),
            _ => recording,
        };
        recordings.insert(
            id,
            super::PaintCacheEntry {
                key,
                recording: Arc::clone(&recording),
                latest: Arc::new(std::sync::RwLock::new(Arc::clone(&recording))),
                measured_text: measured.get().then_some((base, self.text_backend)),
            },
        );
        recording
    }

    /// The slot holding `id`'s latest recording, recording it first if it
    /// has none that fits.
    pub(super) fn shared_recording(
        &self,
        id: StableNodeId,
        painter: &crate::NodePainter,
        layout: LayoutBox,
    ) -> Option<super::SharedRecording> {
        self.custom_paint(id, painter, layout);
        self.paint_recordings
            .borrow()
            .get(&id)
            .map(|held| Arc::clone(&held.latest))
    }

    /// The painter set on `id` apart from its style
    /// ([`crate::UiMutation::SetPainter`]).
    pub fn painter_override(&self, id: StableNodeId) -> Option<&crate::NodePainter> {
        self.painter_overrides.get(&id)
    }

    /// The painter a node paints with: one set on the node, else its
    /// style's.
    pub(crate) fn node_painter(&self, id: StableNodeId) -> Option<&crate::NodePainter> {
        if !self.painter_overrides.is_empty()
            && let Some(painter) = self.painter_overrides.get(&id)
        {
            return Some(painter);
        }
        self.record(id).style.painter.as_ref()
    }

    /// The interaction state a painter sees, from the same facts the
    /// built-in interaction paint reads.
    pub(super) fn paint_state(&self, id: StableNodeId) -> crate::PaintState {
        let record = self.record(id);
        let accessibility = &record.accessibility;
        crate::PaintState {
            hovered: self
                .input
                .pointer_hover
                .values()
                .any(|target| *target == id),
            pressed: self
                .input
                .pointer_press
                .values()
                .any(|target| *target == id),
            focused: self.focus_visible(record.document) == Some(id),
            disabled: accessibility.disabled,
            selected: accessibility.checked == Some(true)
                || accessibility.mixed
                || accessibility.selected == Some(true),
        }
    }
}

/// What `PaintContext::measure_text` answers: the host's `nana-text` engine
/// when one has shaped this world, the em estimate otherwise.
fn measure_paint_text(
    id: StableNodeId,
    engine: Option<&nana_text::SharedTextEngine>,
    base: &crate::ComputedStyle,
    text: &crate::PaintText,
    size: f32,
    max_width: Option<f32>,
) -> crate::TextSize {
    use crate::TextShaper as _;
    let (style, constraints, content) =
        crate::custom_paint::paint_text_request(base, text, size, max_width);
    let metrics = match engine {
        Some(engine) => crate::NanaTextEngineShaper::new(engine.clone()).shape(
            id,
            &content,
            &style,
            constraints,
        ),
        None => crate::components::measure_em_text(&content, &style, constraints),
    };
    crate::custom_paint::paint_text_size(&metrics, size)
}

impl UiWorld {
    pub(super) fn extracted_text_spans(&self, id: StableNodeId) -> Vec<ExtractedTextSpan> {
        let model = self.node_style_model(id);
        // A preedit shifts every offset after it; an empty one shifts none.
        if self
            .nodes
            .editor(id)
            .is_some_and(|editor| editor.session.is_composing())
        {
            return Vec::new();
        }
        if self
            .nodes
            .text_input_presentation(id)
            .is_some_and(|presentation| presentation.placeholder)
        {
            return Vec::new();
        }
        if matches!(
            self.nodes.visual(id),
            Some(StandardVisual::TextInput { secure: true, .. })
        ) {
            return Vec::new();
        }
        // 括号配对着色：与语法高亮同一字形管线（ExtractedTextSpan →
        // 场景文本 span）。括号字符的覆盖色优先于语法 span（合并时切分
        // 重叠的语法 span），语义上括号配对色取代该字符的 punctuation 色。
        // 折叠态：值空间语法 span 先经显示视图重投到显示串（起点落在
        // 隐藏区间内部的钳到摘要之后；跨折叠区间的在边界切分；摘要文本
        // 保持中性色），与显示空间的括号 span 同空间合并。无 span 时零
        // 分配跳过视图构建。
        // 与 shape 路径同门（`text_input_presentation_source` 仅多行态构
        // 视图）：单行输入不构折叠/inlay 显示视图，两条路径对单行喂入
        // 的 inlay 一致不呈现。
        let display_view = if self
            .nodes
            .get(id)
            .is_some_and(|node| node.accessibility.multiline)
            && (self.nodes.text_inlays(id).is_some()
                || self
                    .nodes
                    .text_presentation(id)
                    .is_some_and(|presentation| !presentation.spans.is_empty()))
        {
            self.text_display_view(id)
        } else {
            None
        };
        let syntax_spans = self
            .nodes
            .text_presentation(id)
            .map(|presentation| {
                if presentation.spans.is_empty() {
                    return Vec::new();
                }
                presentation
                    .spans
                    .iter()
                    .flat_map(|span| {
                        let pieces = match display_view.as_ref() {
                            Some(view) => crate::world::text::remap_span_to_display(
                                (span.start, span.end),
                                view,
                            ),
                            None => vec![(span.start, span.end)],
                        };
                        pieces
                            .into_iter()
                            .map(move |(start, end)| ExtractedTextSpan {
                                start,
                                end,
                                color: model.color(span.color).as_rgba_array(),
                                paint_color: None,
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let bracket_spans = self
            .nodes
            .text_input_presentation(id)
            .map(|presentation| {
                presentation
                    .bracket_color_spans
                    .iter()
                    .filter(|&&(_, end, _)| end <= presentation.display_value.len())
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // merge_bracket_glyph_spans 对空括号表原样返回语法 span，无需单独
        // 短路。
        let palette = &model.palette;
        let merged = merge_bracket_glyph_spans(syntax_spans, &bracket_spans, |depth| {
            bracket_depth_color(palette, depth)
        });
        // 行内 inlay 着色：插入区间整段 Muted 灰字（同字号，单 Attrs 限制
        // 下的声明式取舍），inlay 区间优先——与 inlay 重叠的基础层被切分
        // 丢弃。无 inlay 时零分配原样返回；组合期显示视图已不含 inlay。
        let inlay_spans: Vec<(usize, usize)> = display_view
            .as_ref()
            .map(|view| {
                view.spans
                    .iter()
                    .filter(|span| matches!(span.kind, TextDisplaySpanKind::Inlay))
                    .map(|span| (span.display_start, span.display_start + span.display_len))
                    .collect()
            })
            .unwrap_or_default();
        merge_inlay_glyph_spans(
            merged,
            &inlay_spans,
            model.color(SemanticColorRole::Muted).as_rgba_array(),
        )
    }
}

impl UiWorld {
    /// Extract only dirty nodes. Hidden nodes stay present with `visible=false`
    /// so an incremental renderer can remove their previous primitives.
    pub fn extract_nodes(&self, ids: &[StableNodeId]) -> Vec<ExtractedNode> {
        let mut memo = AncestorMemo::default();
        let mut extracted = Vec::with_capacity(ids.len());
        extracted.extend(
            ids.iter()
                .filter_map(|&id| self.extract_node_memo(id, &mut memo)),
        );
        extracted
    }

    /// [`Self::extract_nodes`] for a renderer that keeps what it extracted.
    ///
    /// `previous` answers the record the renderer holds for a node: the one
    /// the last call gave it. A node that layout only moved since the last
    /// call -- its local box changed, its result did not, and no other
    /// render mark -- comes back as a move, where it is now measured from,
    /// and the renderer keeps the rest of its record: what it holds is in
    /// its own coordinates. What the node holds moved with it and is not
    /// marked at all: only the root of a moved subtree comes back. Anything else, or a
    /// node `previous` does not know, is extracted in full. The record is
    /// not read: it is cold, and reading it costs what extracting it does.
    pub fn extract_nodes_reusing<'a>(
        &self,
        ids: &[StableNodeId],
        previous: impl Fn(StableNodeId) -> Option<&'a ExtractedNode>,
    ) -> (Vec<ExtractedNode>, Vec<ExtractedMove>) {
        let epoch = self.render_epoch.get();
        // A subtree mounted deep in the page sums its depth once.
        let _origins = self.remember_page_origins();
        let mut memo = AncestorMemo::default();
        let mut extracted = Vec::with_capacity(ids.len());
        let mut moved = Vec::new();
        for &id in ids {
            // A component's geometry (a button's label region, a field's
            // caret, a Markdown document) is in its own coordinates and moves
            // with it. Only one that reads the page (an open menu surface's
            // items) is extracted again rather than kept, and an open select
            // whose menu no longer lands where its record has it.
            if self.only_moved_since(id, epoch)
                && let Some(held) = previous(id)
                && !held
                    .standard_visual
                    .as_ref()
                    .is_some_and(StandardVisual::geometry_reads_page)
                && self.keeps_select_menu(id, held)
            {
                moved.extend(self.extract_move_memo(id, held, &mut memo));
            } else if self.only_room_asked_since(id, epoch)
                && let Some(held) = previous(id)
                && self.keeps_select_menu(id, held)
            {
                // Moved with a box above it, and its menu is where it was in
                // its own coordinates: the record it holds is what a cold
                // extraction gives.
            } else {
                extracted.extend(self.extract_node_memo(id, &mut memo));
            }
        }
        #[cfg(any(test, feature = "layout-verify"))]
        self.check_records_kept_under_moves(ids, &previous);
        // Marks from here on say why the next call's nodes are dirty.
        self.render_epoch.set(epoch.wrapping_add(1));
        (extracted, moved)
    }

    /// The cold check of what a move keeps: under every node of `ids` whose
    /// box moved since the record the renderer holds, each record the
    /// renderer keeps -- a node this call does not extract -- is what a cold
    /// extraction gives, but for the page box it was extracted at (which a
    /// record in its own coordinates only carries for reference). A node
    /// below a move whose geometry reads the page and was not extracted
    /// again holds where it stood, and fails here.
    ///
    /// Linear in the moved subtrees, so it runs on worlds the size tests
    /// build ([`KEPT_RECORD_AUDIT_LIMIT`]).
    #[cfg(any(test, feature = "layout-verify"))]
    fn check_records_kept_under_moves<'a>(
        &self,
        ids: &[StableNodeId],
        previous: &impl Fn(StableNodeId) -> Option<&'a ExtractedNode>,
    ) {
        if self.nodes.len() > KEPT_RECORD_AUDIT_LIMIT {
            return;
        }
        let batch: NodeSet = ids.iter().copied().collect();
        let mut seen = NodeSet::default();
        for &root in ids {
            let (Some(held), Some(record)) = (previous(root), self.nodes.get(root)) else {
                continue;
            };
            if held.local_layout == record.local_layout && held.layout_frame == record.layout_frame
            {
                continue;
            }
            for id in self.subtree_ids(root) {
                if id == root || batch.contains(&id) || !seen.insert(id) {
                    continue;
                }
                let Some(held) = previous(id) else {
                    continue;
                };
                let Some(mut cold) = self.extract_node_memo(id, &mut AncestorMemo::default())
                else {
                    continue;
                };
                cold.layout.x = held.layout.x;
                cold.layout.y = held.layout.y;
                let differs = extracted_fields_differ(held, &cold);
                assert!(
                    differs.is_empty(),
                    "{id:?}: kept under {root:?}, which moved, its record differs from a \
                     cold extraction in {differs:?}"
                );
            }
        }
    }

    /// Whether the only mark on `id` since `epoch` began is a move above it
    /// asking whether its open menu still lands where its record has it
    /// (`settle_layout_moves`).
    fn only_room_asked_since(&self, id: StableNodeId, epoch: u64) -> bool {
        self.nodes.get(id).is_some_and(|node| {
            node.render_room_epoch == epoch
                && node.render_full_epoch != epoch
                && node.render_moved_epoch != epoch
        })
    }

    /// Whether `held`, the record the renderer keeps of `id`, still has the
    /// menu an open select field would be given now. Everything an open menu
    /// reads of where its field stands goes through where its surface lands
    /// in the field's coordinates ([`Self::local_select_menu_surface`]); the
    /// rest of the record is the field's own. Any other record keeps.
    fn keeps_select_menu(&self, id: StableNodeId, held: &ExtractedNode) -> bool {
        let Some(StandardVisual::Select { opened: true, .. }) = held.standard_visual.as_ref()
        else {
            return true;
        };
        let Some(crate::ComponentGeometry::Select {
            menu: Some(menu), ..
        }) = held.component_geometry.as_deref()
        else {
            return false;
        };
        self.local_select_menu_surface(id)
            .is_some_and(|surface| same_box(surface, menu.surface))
    }

    fn only_moved_since(&self, id: StableNodeId, epoch: u64) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| node.render_moved_epoch == epoch && node.render_full_epoch != epoch)
    }

    /// Where the node's box and result now stand. Whether the node is
    /// extracted at all is asked again: that is not the node's own.
    fn extract_move_memo(
        &self,
        id: StableNodeId,
        #[cfg_attr(not(any(test, feature = "layout-verify")), allow(unused_variables))]
        held: &ExtractedNode,
        memo: &mut AncestorMemo,
    ) -> Option<ExtractedMove> {
        if !self.presence_live_memo(id, memo) {
            return None;
        }
        let record = self.nodes.get(id)?;
        let moved = ExtractedMove {
            id,
            layout: self.record_layout_box(record),
            local_layout: record.local_layout,
            layout_frame: record.layout_frame,
            layout_result: self.local_layout_result(id).cloned(),
        };
        #[cfg(any(test, feature = "layout-verify"))]
        {
            let mut reused = held.clone();
            reused.local_layout = moved.local_layout;
            reused.layout_frame = moved.layout_frame;
            reused.layout_result = moved.layout_result.clone();
            let mut full = self
                .extract_node_memo(id, &mut AncestorMemo::default())
                .expect("a live node extracts");
            // The kept record keeps the page box it was extracted with, which
            // differs from a fresh one's by where the box moved, at the same
            // size. Everything else -- its component geometry among it -- is
            // in its own coordinates, and the same.
            full.layout.x = held.layout.x;
            full.layout.y = held.layout.y;
            let differs = extracted_fields_differ(&reused, &full);
            assert!(
                differs.is_empty(),
                "{id:?}: a moved node's reused record differs from a full \
                 extraction in {differs:?}"
            );
        }
        Some(moved)
    }
}

/// The largest world [`UiWorld::check_records_kept_under_moves`] checks.
#[cfg(any(test, feature = "layout-verify"))]
const KEPT_RECORD_AUDIT_LIMIT: usize = 1024;

/// The fields of two records that are not equal, by name. Layout boxes
/// compare to the bit.
#[cfg(any(test, feature = "layout-verify"))]
fn extracted_fields_differ(a: &ExtractedNode, b: &ExtractedNode) -> Vec<&'static str> {
    let ExtractedNode {
        layout_result,
        id,
        kind,
        parent,
        children,
        layout,
        local_layout,
        layout_frame,
        scroll_offset,
        source_style,
        style,
        text,
        text_metrics,
        text_layout,
        text_preserve_lines,
        z_index,
        focused,
        editable,
        text_spans,
        rich_text,
        rich_editor,
        glyph_presentation,
        standard_visual,
        component_geometry,
        standard_visual_foreground,
        chrome_radii,
        custom_render,
        drop_hover,
        document_text_selection,
        document_text_selection_color,
        compositor,
        custom_paint,
    } = a;
    let bits =
        |layout: &LayoutBox| [layout.x, layout.y, layout.width, layout.height].map(f32::to_bits);
    let mut differ = Vec::new();
    let mut check = |name: &'static str, equal: bool| {
        if !equal {
            differ.push(name);
        }
    };
    check("layout_result", *layout_result == b.layout_result);
    check("id", *id == b.id);
    check("kind", *kind == b.kind);
    check("parent", *parent == b.parent);
    check("children", *children == b.children);
    check("layout", bits(layout) == bits(&b.layout));
    check("local_layout", bits(local_layout) == bits(&b.local_layout));
    check("layout_frame", *layout_frame == b.layout_frame);
    check("scroll_offset", *scroll_offset == b.scroll_offset);
    check("source_style", *source_style == b.source_style);
    check("style", *style == b.style);
    check("text", *text == b.text);
    check("text_metrics", *text_metrics == b.text_metrics);
    check("text_layout", *text_layout == b.text_layout);
    check(
        "text_preserve_lines",
        *text_preserve_lines == b.text_preserve_lines,
    );
    check("z_index", *z_index == b.z_index);
    check("focused", *focused == b.focused);
    check("editable", *editable == b.editable);
    check("text_spans", *text_spans == b.text_spans);
    check("rich_text", *rich_text == b.rich_text);
    check("rich_editor", *rich_editor == b.rich_editor);
    check(
        "glyph_presentation",
        *glyph_presentation == b.glyph_presentation,
    );
    check("standard_visual", *standard_visual == b.standard_visual);
    check(
        "component_geometry",
        *component_geometry == b.component_geometry,
    );
    check(
        "standard_visual_foreground",
        *standard_visual_foreground == b.standard_visual_foreground,
    );
    check("chrome_radii", *chrome_radii == b.chrome_radii);
    check("custom_render", *custom_render == b.custom_render);
    check("drop_hover", *drop_hover == b.drop_hover);
    check(
        "document_text_selection",
        *document_text_selection == b.document_text_selection,
    );
    check(
        "document_text_selection_color",
        *document_text_selection_color == b.document_text_selection_color,
    );
    check("compositor", *compositor == b.compositor);
    check("custom_paint", *custom_paint == b.custom_paint);
    differ
}

impl UiWorld {
    /// Produce a renderer-neutral snapshot in retained document order.
    pub fn extract_document(&self, document: DocumentId) -> Vec<ExtractedNode> {
        let mut memo = AncestorMemo::default();
        self.document_order(document)
            .into_iter()
            .filter_map(|id| {
                self.extract_node_memo(id, &mut memo)
                    .filter(|node| node.style.visible)
            })
            .collect()
    }
}

impl UiWorld {
    /// Which colour a `StandardVisual` paints its own foreground with.
    ///
    /// This used to be a 25-arm `match` that named a palette field per visual.
    /// The arms are still here, but they now answer *which component family
    /// this is*; the family's role is the installed theme's
    /// [`ComponentRecipe`](nana_ui_core::ComponentRecipe). The decision moved,
    /// the dispatch did not — and because it resolves here, per extract, an
    /// installed recipe reaches live nodes without reprojecting anything.
    ///
    /// `authored` is the node's own resolved colour. Four families let it win:
    /// an icon, a button, a field and a selection row all take a caller's
    /// explicit colour ahead of the recipe. The rest paint chrome the caller
    /// does not address.
    fn standard_visual_foreground(
        &self,
        model: StyleModelRef,
        visual: &StandardVisual,
        authored: Option<[f32; 4]>,
    ) -> [f32; 4] {
        use nana_ui_core::ComponentRecipeId;

        let recipes = self.theme.recipes();
        let role = |family: ComponentRecipeId, checked: bool| recipes.foreground(family, checked);
        let paint = |role| model.color(role).as_rgba_array();

        match visual {
            // Families that defer to an authored colour when the caller set one.
            StandardVisual::Icon { .. } => {
                authored.unwrap_or_else(|| paint(role(ComponentRecipeId::Icon, false)))
            }
            StandardVisual::Button { .. } => {
                authored.unwrap_or_else(|| paint(role(ComponentRecipeId::Button, false)))
            }
            StandardVisual::TextInput { .. } => {
                authored.unwrap_or_else(|| paint(role(ComponentRecipeId::TextInput, false)))
            }
            StandardVisual::SelectionOption { .. } => {
                authored.unwrap_or_else(|| paint(role(ComponentRecipeId::Selection, false)))
            }

            // Indicator families: the recipe carries an on-state role.
            StandardVisual::Checkbox {
                checked,
                indeterminate,
                ..
            } => paint(role(
                ComponentRecipeId::Checkbox,
                *checked || *indeterminate,
            )),
            StandardVisual::Switch { checked, .. } => {
                paint(role(ComponentRecipeId::Switch, *checked))
            }

            // Status-driven surfaces read the tone table, not a family.
            StandardVisual::Toast { tone, .. } => paint(recipes.status().role(tone.status())),
            StandardVisual::LevelMeter { tone, .. } => paint(recipes.status().role(*tone)),

            StandardVisual::ModalFrame { .. } => paint(role(ComponentRecipeId::Overlay, false)),
            StandardVisual::Scrollbar { .. } => paint(role(ComponentRecipeId::Scrollbar, false)),
            StandardVisual::Range { .. } => paint(role(ComponentRecipeId::Range, false)),
            StandardVisual::Card { .. } => paint(role(ComponentRecipeId::Card, false)),
            StandardVisual::ListItem { .. } => paint(role(ComponentRecipeId::ListItem, false)),
            // A bar may name its own fill; a still indeterminate one keeps
            // only a trace of it, so the full track reads as no amount.
            StandardVisual::Progress { mode, fill, .. } => {
                let fill = fill.unwrap_or_else(|| role(ComponentRecipeId::Indicator, false));
                match mode {
                    crate::ProgressMode::Still => nana_ui_core::SemanticColorMix::alpha(
                        fill,
                        crate::feedback::PROGRESS_STILL_ALPHA,
                    )
                    .resolve(model)
                    .as_rgba_array(),
                    _ => paint(fill),
                }
            }
            StandardVisual::StatusBadge { .. }
            | StandardVisual::ValidationMessage { .. }
            | StandardVisual::EmptyState { .. }
            | StandardVisual::PathText { .. }
            | StandardVisual::LabeledValue { .. }
            | StandardVisual::Spinner { .. }
            | StandardVisual::FormField { .. } => paint(role(ComponentRecipeId::Indicator, false)),
            StandardVisual::Select { .. }
            | StandardVisual::MenuSurface { .. }
            | StandardVisual::ActionMenuItem { .. }
            | StandardVisual::TreeView { .. }
            | StandardVisual::CommandPalette { .. } => paint(role(ComponentRecipeId::Menu, false)),

            // A QR code's ink is not a theme colour. The quiet zone has to stay
            // scannable, so it is black on white whatever the theme says.
            StandardVisual::QrCode { .. } => [0.0, 0.0, 0.0, 1.0],

            StandardVisual::XYPad { .. } => paint(role(ComponentRecipeId::Content, false)),
            #[cfg(feature = "calendar")]
            StandardVisual::CalendarHeatmap { .. } => {
                paint(role(ComponentRecipeId::Content, false))
            }
            #[cfg(feature = "charts")]
            StandardVisual::Chart { .. } | StandardVisual::ChartTooltip { .. } => {
                paint(role(ComponentRecipeId::Content, false))
            }
            #[cfg(feature = "controls")]
            StandardVisual::ReorderList { .. } => paint(role(ComponentRecipeId::Content, false)),
            #[cfg(feature = "rich-text")]
            StandardVisual::NativeMarkdown { .. } => paint(role(ComponentRecipeId::Content, false)),
            #[cfg(feature = "rich-text")]
            StandardVisual::SelectableRichText { .. } => {
                paint(role(ComponentRecipeId::Content, false))
            }
            #[cfg(feature = "graph-canvas")]
            StandardVisual::GraphCanvas { .. } => paint(role(ComponentRecipeId::Content, false)),
            #[cfg(feature = "graph-canvas")]
            StandardVisual::GraphMinimap { .. } => paint(role(ComponentRecipeId::Content, false)),
            #[cfg(feature = "image-viewer")]
            StandardVisual::ImageViewer { .. } => paint(role(ComponentRecipeId::Content, false)),
            StandardVisual::KeyCaptureLayer { .. } | StandardVisual::KeymapLayer => {
                paint(role(ComponentRecipeId::Content, false))
            }
        }
    }
}

/// The same box to the bit: a menu's surface kept only when where it lands
/// is exactly what a cold extraction gives.
fn same_box(a: LayoutBox, b: LayoutBox) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.width.to_bits() == b.width.to_bits()
        && a.height.to_bits() == b.height.to_bits()
}
