//! Committed node projection into Scene input.

use super::*;

impl UiWorld {
    pub(super) fn stacking_z_index_memo(&self, id: StableNodeId, memo: &mut AncestorMemo) -> i32 {
        if self.z_index_nodes == 0 {
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
            layout,
            scroll_offset,
            text,
            text_metrics,
            document,
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
                node.layout,
                node.scroll_offset,
                has_text.then(|| node.text.clone()),
                has_text.then_some(node.text_metrics),
                node.document,
            )
        };
        if resolved_epoch != self.palette_epoch {
            let inherited_color = parent.and_then(|parent| {
                memo.color
                    .get(&parent)
                    .copied()
                    .or_else(|| self.inherited_palette_color(Some(parent)))
            });
            let (foreground, color, background, border_color) =
                self.palette_paint_colors(id, inherited_color);
            if let Some(color) = color {
                memo.color.insert(id, color);
            }
            if style.foreground != foreground
                || style.color != color
                || style.background != background
                || style.border_color != border_color
            {
                let style = Arc::make_mut(&mut style);
                style.foreground = foreground;
                style.color = color;
                style.background = background;
                style.border_color = border_color;
            }
        }
        let mut standard_visual = self.nodes.visual(id).cloned();
        if let Some((busy, danger, is_confirm)) = self.confirm_action_effect(id) {
            if busy && !is_confirm {
                Arc::make_mut(&mut style).color =
                    Some(self.style_model.palette.muted.as_rgba_array());
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
        let component_geometry = standard_visual
            .as_ref()
            .and_then(|visual| self.derive_component_geometry(id, visual, style.as_ref()))
            .map(Box::new);
        let standard_visual_foreground = standard_visual
            .as_ref()
            .map(|visual| self.standard_visual_foreground(visual, style.color));
        let mut source_style = source_style;
        // The node's design intent is already resolved into `resolved_layout`,
        // on write. Resolving it here instead meant an `Arc::make_mut` copy of
        // a 4.8 KB `LayoutStyle` per control per frame.
        source_style.layout = self.motion_layout(id, &resolved_layout);
        // Scene receives the layout-resolved padding; it must not resolve %
        // against the painted node's own width. Authored world style stays intact.
        let padding = self.used_layout_padding(id);
        if source_style
            .layout
            .resolved_padding_against(Some(layout.width))
            != padding
        {
            let layout = Arc::make_mut(&mut source_style.layout);
            layout.padding = None;
            layout.logical_padding = Default::default();
            layout.padding_logical = Default::default();
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
        if let (Some(selection), Some(color)) = (document_selection, style.selection_color) {
            let (start, end) = selection.ordered();
            text_spans = merge_inlay_glyph_spans(text_spans, &[(start, end)], color);
        }
        let document_text_selection = document_selection
            .map(|selection| selection.lines.clone())
            .unwrap_or_default();
        let document_text_selection_color = style
            .selection_background
            .unwrap_or_else(|| self.style_model.palette.accent_soft.as_rgba_array());
        Some(ExtractedNode {
            id,
            kind,
            parent: hierarchy_parent,
            children: hierarchy_children,
            layout,
            scroll_offset,
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
            text_preserve_lines: self.text_preserves_lines(id),
            // Paint's question, so paint's answer: a control put into focus
            // by a click is focused and does not draw a ring about it.
            focused: self.focus_visible(document) == Some(id),
            ime: self.nodes.ime(id).cloned(),
            text_input: self.nodes.text_input(id).cloned(),
            text_spans,
            standard_visual,
            component_geometry,
            standard_visual_foreground,
            chrome_radii: self.style_model.metrics.into(),
            custom_render: self.nodes.custom_render(id).cloned(),
            drop_hover: (self.drop_hover.map(|(hover, _)| hover) == Some(id)).then(|| {
                crate::DropHoverOverlay {
                    fill: self.style_model.palette.accent_soft.as_rgba_array(),
                    border: self.style_model.palette.accent.as_rgba_array(),
                }
            }),
            document_text_selection,
            document_text_selection_color,
            compositor: self.extracted_compositor(id),
        })
    }
}

impl UiWorld {
    pub(super) fn extracted_text_spans(&self, id: StableNodeId) -> Vec<ExtractedTextSpan> {
        if self.nodes.ime(id).is_some() {
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
                                color: self.style_model.color(span.color).as_rgba_array(),
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
        let palette = &self.style_model.palette;
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
            self.style_model
                .color(SemanticColorRole::Muted)
                .as_rgba_array(),
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
        visual: &StandardVisual,
        authored: Option<[f32; 4]>,
    ) -> [f32; 4] {
        use nana_ui_core::ComponentRecipeId;

        let recipes = self.theme.recipes();
        let role = |family: ComponentRecipeId, checked: bool| recipes.foreground(family, checked);
        let paint = |role| self.style_model.color(role).as_rgba_array();

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
            StandardVisual::StatusBadge { .. }
            | StandardVisual::ValidationMessage { .. }
            | StandardVisual::EmptyState { .. }
            | StandardVisual::LabeledValue { .. }
            | StandardVisual::Progress { .. }
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
            StandardVisual::TimeSeriesChart { .. }
            | StandardVisual::TimestampSeriesChart { .. }
            | StandardVisual::DonutChart { .. }
            | StandardVisual::StackedTimeSeriesChart { .. } => {
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
