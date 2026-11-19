//! Component geometry derived from committed Runtime data.

use nana_ui_core::sort::CompactSort as _;

use super::*;

/// Block extent of a progress bar.
pub(crate) const PROGRESS_GIRTH: f32 = nana_ui_core::space::SM;

impl UiWorld {
    /// Derives public [`crate::TextInputGeometry`] in page coordinates (including
    /// cursor blinking state untouched for IME/hit callers; paint hides blink).
    pub fn text_input_geometry(&self, id: StableNodeId) -> Option<crate::TextInputGeometry> {
        let visual = self.widget_visual_as::<crate::TextInputVisual>(id)?;
        let style = self.painted_style(id)?;
        let previous = self.geometry_frame.replace(None);
        if previous.is_some() {
            self.geometry_read_page.set(true);
        }
        let geometry = visual.geometry_for(self, id, &style);
        self.geometry_frame.set(previous);
        geometry
    }
}

impl crate::TextInputVisual {
    /// Derives [`crate::TextInputGeometry`] for `id` in the active geometry
    /// frame of `world` (`local` during extraction, `page` from public queries).
    pub fn geometry_for(
        &self,
        world: &UiWorld,
        id: StableNodeId,
        style: &ComputedStyle,
    ) -> Option<crate::TextInputGeometry> {
        world.derive_text_input_geometry(id, self, style)
    }

    /// Emits node-local [`crate::WidgetPaintOutput`] for a precomputed [`crate::TextInputGeometry`].
    pub fn paint_with_geometry(
        geometry: &crate::TextInputGeometry,
        bounds: LayoutBox,
        padding: nana_ui_core::PaddingSpec,
        border: f32,
    ) -> crate::WidgetPaintOutput {
        use crate::{
            ComponentTextRegion, TEXT_ATOM_ICONS_NS, TEXT_ATOM_LABELS_NS,
            TEXT_DIAGNOSTIC_LABELS_NS, TEXT_DIAGNOSTIC_MARKERS_NS, TEXT_LINE_LABELS_NS, TextValue,
            TextVerticalAlignment, TextWhitespaceKind, WidgetBatchColors, WidgetGlyph, WidgetIcon,
            WidgetPaintOp, WidgetPaintOutput, WidgetQuad, WidgetQuadBatch, WidgetRootSurface,
            WidgetTaggedOp, WidgetText, widget_collection_slot,
        };

        let rects_bounds = |rects: &[LayoutBox]| -> LayoutBox {
            let Some((first, rest)) = rects.split_first() else {
                return LayoutBox::default();
            };
            rest.iter().fold(*first, |acc, r| {
                let x = acc.x.min(r.x);
                let y = acc.y.min(r.y);
                let right = (acc.x + acc.width).max(r.x + r.width);
                let bottom = (acc.y + acc.height).max(r.y + r.height);
                LayoutBox {
                    x,
                    y,
                    width: (right - x).max(0.0),
                    height: (bottom - y).max(0.0),
                }
            })
        };

        let mut out = WidgetPaintOutput::new();
        out.owns_node_text = true;
        out.suppress_root_surface = false;
        out.root_surface = Some(WidgetRootSurface {
            background: Some(geometry.background),
            border_color: Some(geometry.border),
            border_width: Some(geometry.border_width),
            fallback_elevation: None,
        });
        let editor_bounds = geometry
            .character_counter
            .as_ref()
            .map(|c| c.editor_bounds)
            .or_else(|| geometry.suffix.as_ref().map(|s| s.editor_bounds));
        out.content_clip = Some(editor_bounds.unwrap_or(LayoutBox {
            x: bounds.x + border + padding.left,
            y: bounds.y + border + padding.top,
            width: (bounds.width - border * 2.0 - padding.left - padding.right).max(0.0),
            height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
        }));

        let multiline = geometry.multiline;
        let vertical_alignment = if multiline {
            TextVerticalAlignment::Top
        } else {
            TextVerticalAlignment::Center
        };
        let scroll_x = geometry.scroll.map(|s| s.offset_x);
        let scroll_rect = |mut rect: LayoutBox| {
            if let Some(offset) = scroll_x {
                rect.x -= offset;
            }
            rect
        };
        let tag_scrolled = |op: WidgetTaggedOp| {
            if let Some(offset) = scroll_x {
                op.with_local_translation(offset, 0.0)
            } else {
                op
            }
        };
        let tag_scrolled_content = |op: WidgetTaggedOp| tag_scrolled(op).with_content_clip();

        let editor_text = |region: &ComponentTextRegion,
                           horizontal_alignment: crate::TextHorizontalAlignment,
                           ellipsis: bool| WidgetText {
            bounds: region.bounds,
            content: region.content.clone(),
            color: region.color,
            font_size: region.font_size,
            font_weight: region.font_weight,
            horizontal_alignment,
            vertical_alignment,
            wrap: multiline,
            ellipsis,
            ..WidgetText::default()
        };

        if let Some(suffix) = &geometry.suffix {
            out.push_tagged(
                WidgetTaggedOp::new(
                    27,
                    WidgetPaintOp::Text(editor_text(
                        &suffix.text,
                        crate::TextHorizontalAlignment::End,
                        false,
                    )),
                )
                .with_extra_clip(suffix.text.bounds),
            );
        }
        if let Some(counter) = &geometry.character_counter {
            let mut text = editor_text(&counter.text, crate::TextHorizontalAlignment::End, false);
            text.force_horizontal_writing = true;
            out.push_tagged(
                WidgetTaggedOp::new(26, WidgetPaintOp::Text(text))
                    .with_extra_clip(counter.text.bounds),
            );
        }
        if let Some(grip) = geometry.resize_grip {
            let rects: Arc<[LayoutBox]> = [4.0, 8.0]
                .into_iter()
                .map(|offset| LayoutBox {
                    x: grip.x + grip.width - offset,
                    y: grip.y + grip.height - 3.0,
                    width: 2.0,
                    height: 2.0,
                })
                .collect();
            out.push(
                250,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(
                        geometry.text.color.unwrap_or([0.5, 0.5, 0.5, 1.0]),
                    ),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            );
        }
        if let Some(steppers) = &geometry.steppers {
            for (slot, icon, box_rect, color) in [
                (
                    8,
                    nana_ui_core::Icon::ChevronUp,
                    steppers.increment,
                    steppers.increment_color,
                ),
                (
                    9,
                    nana_ui_core::Icon::ChevronDown,
                    steppers.decrement,
                    steppers.decrement_color,
                ),
            ] {
                let extent = steppers
                    .glyph_size
                    .min(box_rect.width)
                    .min(box_rect.height)
                    .max(0.0);
                out.push(
                    slot,
                    WidgetPaintOp::Icon(WidgetIcon {
                        bounds: LayoutBox {
                            x: box_rect.x + (box_rect.width - extent) / 2.0,
                            y: box_rect.y + (box_rect.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        },
                        glyph: WidgetGlyph::Icon(icon),
                        color: Some(color),
                        paint_color: None,
                        align_to_adjacent_text: false,
                    }),
                );
            }
        }
        if !geometry.selection.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .selection
                .iter()
                .copied()
                .map(scroll_rect)
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                1,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Selection(geometry.selection_color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }
        {
            let mut text =
                editor_text(&geometry.text, crate::TextHorizontalAlignment::Start, false);
            text.adopt_node_text_spans = true;
            if let Some(scroll) = geometry.scroll {
                text.bounds.x = scroll.text_x;
            }
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                2,
                WidgetPaintOp::Text(text),
            )));
        }

        if !geometry.git_marks.added.is_empty()
            || !geometry.git_marks.modified.is_empty()
            || !geometry.git_marks.deleted.is_empty()
        {
            for (slot, rects, color) in [
                (
                    18,
                    &geometry.git_marks.added,
                    geometry.git_marks.added_color,
                ),
                (
                    19,
                    &geometry.git_marks.modified,
                    geometry.git_marks.modified_color,
                ),
                (
                    8,
                    &geometry.git_marks.deleted,
                    geometry.git_marks.deleted_color,
                ),
            ] {
                if rects.is_empty() {
                    continue;
                }
                let rects: Arc<[LayoutBox]> = rects.iter().copied().collect();
                out.push(
                    slot,
                    WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                        bounds: rects_bounds(&rects),
                        rects,
                        colors: WidgetBatchColors::Uniform(color),
                        fill_paint: None,
                        border: None,
                        corner_radius: [0.0; 4],
                    }),
                );
            }
        }

        if !geometry.folds.gutters.is_empty() {
            let collapsed: Vec<_> = geometry
                .folds
                .gutters
                .iter()
                .filter(|gutter| gutter.collapsed)
                .collect();
            let expanded: Vec<_> = geometry
                .folds
                .gutters
                .iter()
                .filter(|gutter| !gutter.collapsed)
                .collect();
            if !collapsed.is_empty() {
                let rects: Arc<[LayoutBox]> = collapsed.iter().map(|g| g.bounds).collect();
                out.push(
                    14,
                    WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                        bounds: rects_bounds(&rects),
                        rects,
                        colors: WidgetBatchColors::Uniform(collapsed[0].color),
                        fill_paint: None,
                        border: None,
                        corner_radius: [0.0; 4],
                    }),
                );
            }
            if !expanded.is_empty() {
                let rects: Arc<[LayoutBox]> = expanded.iter().map(|g| g.bounds).collect();
                out.push(
                    15,
                    WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                        bounds: rects_bounds(&rects),
                        rects,
                        colors: WidgetBatchColors::None,
                        fill_paint: None,
                        border: Some((expanded[0].color, 1.0)),
                        corner_radius: [0.0; 4],
                    }),
                );
            }
        }

        if !geometry.line_labels.is_empty() {
            let gutter_width = padding.left;
            if gutter_width > 4.0 {
                for (label_index, label) in geometry.line_labels.iter().enumerate() {
                    let region = ComponentTextRegion {
                        bounds: LayoutBox {
                            x: bounds.x + border + 2.0,
                            y: label.y,
                            width: (gutter_width - 4.0).max(0.0),
                            height: label.height,
                        },
                        content: TextValue::from(label.number.to_string().as_str()),
                        color: Some(geometry.line_labels_color),
                        font_size: geometry.line_labels_font_size,
                        font_weight: None,
                    };
                    out.push(
                        widget_collection_slot(TEXT_LINE_LABELS_NS, label_index),
                        WidgetPaintOp::Text(editor_text(
                            &region,
                            crate::TextHorizontalAlignment::End,
                            false,
                        )),
                    );
                }
            }
        }

        for (marker_index, (rect, color)) in geometry.diagnostic_markers.iter().enumerate() {
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                widget_collection_slot(TEXT_DIAGNOSTIC_MARKERS_NS, marker_index),
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: scroll_rect(*rect),
                    fill: Some(*color),
                    ..WidgetQuad::default()
                }),
            )));
        }
        for (label_index, region) in geometry.diagnostic_labels.iter().enumerate() {
            let mut text = editor_text(region, crate::TextHorizontalAlignment::Start, true);
            text.bounds = scroll_rect(text.bounds);
            out.push_tagged(tag_scrolled(WidgetTaggedOp::new(
                widget_collection_slot(TEXT_DIAGNOSTIC_LABELS_NS, label_index),
                WidgetPaintOp::Text(text),
            )));
        }

        let (normal_matches, current_matches): (Vec<_>, Vec<_>) =
            geometry.match_markers.iter().partition(|m| !m.current);
        if !normal_matches.is_empty() {
            let rects: Arc<[LayoutBox]> =
                normal_matches.iter().map(|m| scroll_rect(m.rect)).collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                3,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(normal_matches[0].color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }
        if !current_matches.is_empty() {
            let rects: Arc<[LayoutBox]> = current_matches
                .iter()
                .map(|m| scroll_rect(m.rect))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                6,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(current_matches[0].color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if !geometry.swatch_markers.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .swatch_markers
                .iter()
                .map(|(r, _)| scroll_rect(*r))
                .collect();
            let colors: Arc<[[f32; 4]]> = geometry.swatch_markers.iter().map(|(_, c)| *c).collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                23,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::PerQuad(colors),
                    fill_paint: None,
                    border: Some((geometry.swatch_border_color, 1.0)),
                    corner_radius: [2.0; 4],
                }),
            )));
        }

        if !geometry.atom_chips.is_empty() {
            let chip_fg = geometry
                .atom_chips
                .iter()
                .find_map(|chip| chip.label.color)
                .unwrap_or([0.12, 0.12, 0.14, 1.0]);
            let chip_rects: Arc<[LayoutBox]> = geometry
                .atom_chips
                .iter()
                .map(|chip| scroll_rect(chip.bounds))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                22,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&chip_rects),
                    rects: chip_rects,
                    colors: WidgetBatchColors::Uniform(geometry.atom_chips[0].background),
                    fill_paint: None,
                    border: Some((geometry.atom_chips[0].border, 1.0)),
                    corner_radius: [4.0; 4],
                }),
            )));
            let close_rects: Arc<[LayoutBox]> = geometry
                .atom_chips
                .iter()
                .map(|chip| scroll_rect(chip.close))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                25,
                WidgetPaintOp::IconBatch {
                    bounds: rects_bounds(&close_rects),
                    rects: close_rects,
                    icon: nana_ui_core::Icon::Close,
                    color: Some(chip_fg),
                },
            )));
            for (index, chip) in geometry.atom_chips.iter().enumerate() {
                let icon_rect = scroll_rect(chip.icon_bounds);
                out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                    widget_collection_slot(TEXT_ATOM_ICONS_NS, index),
                    WidgetPaintOp::IconBatch {
                        bounds: icon_rect,
                        rects: Arc::from([icon_rect]),
                        icon: chip.icon,
                        color: Some(chip_fg),
                    },
                )));
                let mut label =
                    editor_text(&chip.label, crate::TextHorizontalAlignment::Start, true);
                label.bounds = scroll_rect(label.bounds);
                out.push_tagged(tag_scrolled(WidgetTaggedOp::new(
                    widget_collection_slot(TEXT_ATOM_LABELS_NS, index),
                    WidgetPaintOp::Text(label),
                )));
            }
        }

        if let Some((rect, color)) = geometry.caret_line {
            out.push_tagged(
                WidgetTaggedOp::new(
                    1,
                    WidgetPaintOp::Quad(WidgetQuad {
                        rect,
                        fill: Some(color),
                        ..WidgetQuad::default()
                    }),
                )
                .with_content_clip(),
            );
        }

        if !geometry.indent_guides.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .indent_guides
                .iter()
                .map(|(r, _)| scroll_rect(*r))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                10,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(geometry.indent_guides[0].1),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if !geometry.occurrence_markers.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .occurrence_markers
                .iter()
                .map(|(r, _)| scroll_rect(*r))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                11,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(geometry.occurrence_markers[0].1),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if !geometry.whitespace_marks.is_empty() {
            let dots: Arc<[LayoutBox]> = {
                let extent = (geometry.line_labels_font_size * 0.2).clamp(2.0, 3.0);
                geometry
                    .whitespace_marks
                    .iter()
                    .filter(|(_, kind)| *kind == TextWhitespaceKind::Space)
                    .map(|(rect, _)| {
                        let cell = scroll_rect(*rect);
                        LayoutBox {
                            x: cell.x + (cell.width - extent) / 2.0,
                            y: cell.y + (cell.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        }
                    })
                    .collect()
            };
            if !dots.is_empty() {
                out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                    16,
                    WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                        bounds: rects_bounds(&dots),
                        rects: dots,
                        colors: WidgetBatchColors::Uniform(geometry.whitespace_color),
                        fill_paint: None,
                        border: None,
                        corner_radius: [999.0; 4],
                    }),
                )));
            }
            let arrows: Arc<[LayoutBox]> = geometry
                .whitespace_marks
                .iter()
                .filter(|(_, kind)| *kind == TextWhitespaceKind::Tab)
                .map(|(rect, _)| {
                    let cell = scroll_rect(*rect);
                    let extent = (cell.height * 0.55).clamp(6.0, 14.0);
                    LayoutBox {
                        x: cell.x + (cell.width - extent) / 2.0,
                        y: cell.y + (cell.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    }
                })
                .collect();
            if !arrows.is_empty() {
                out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                    60,
                    WidgetPaintOp::IconBatch {
                        bounds: rects_bounds(&arrows),
                        rects: arrows,
                        icon: nana_ui_core::Icon::ArrowRight,
                        color: Some(geometry.whitespace_color),
                    },
                )));
            }
        }

        if !geometry.wrap_guides.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .wrap_guides
                .iter()
                .map(|(r, _)| scroll_rect(*r))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                17,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(geometry.wrap_guides[0].1),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if let Some(minimap) = &geometry.minimap {
            out.push_tagged(
                WidgetTaggedOp::new(
                    70,
                    WidgetPaintOp::Quad(WidgetQuad {
                        rect: minimap.panel,
                        fill: Some(minimap.panel_color),
                        ..WidgetQuad::default()
                    }),
                )
                .with_content_clip(),
            );
            let rects: Arc<[LayoutBox]> = std::iter::once(minimap.separator)
                .chain(minimap.bars.iter().copied())
                .collect();
            out.push_tagged(
                WidgetTaggedOp::new(
                    71,
                    WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                        bounds: rects_bounds(&rects),
                        rects,
                        colors: WidgetBatchColors::Uniform(minimap.bar_color),
                        fill_paint: None,
                        border: None,
                        corner_radius: [0.0; 4],
                    }),
                )
                .with_content_clip(),
            );
            if let Some(indicator) = minimap.indicator {
                out.push_tagged(
                    WidgetTaggedOp::new(
                        72,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: indicator,
                            fill: Some(minimap.indicator_color),
                            ..WidgetQuad::default()
                        }),
                    )
                    .with_content_clip(),
                );
            }
        }

        if let Some(sticky) = &geometry.sticky_line {
            for (slot, rect, color) in [
                (80, sticky.panel, sticky.background),
                (81, sticky.divider, sticky.divider_color),
            ] {
                out.push_tagged(
                    WidgetTaggedOp::new(
                        slot,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect,
                            fill: Some(color),
                            ..WidgetQuad::default()
                        }),
                    )
                    .with_content_clip(),
                );
            }
            out.push_tagged(
                WidgetTaggedOp::new(
                    82,
                    WidgetPaintOp::Text(editor_text(
                        &sticky.text,
                        crate::TextHorizontalAlignment::Start,
                        false,
                    )),
                )
                .with_content_clip(),
            );
        }

        if !geometry.bracket_markers.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .bracket_markers
                .iter()
                .map(|(r, _)| scroll_rect(*r))
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                12,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::None,
                    fill_paint: None,
                    border: Some((geometry.bracket_markers[0].1, 1.0)),
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if let Some((rect, color)) = geometry.drop_indicator {
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                6,
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: scroll_rect(rect),
                    fill: Some(color),
                    ..WidgetQuad::default()
                }),
            )));
        }

        if let Some(caret) = geometry.caret {
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                4,
                WidgetPaintOp::Quad(WidgetQuad {
                    rect: scroll_rect(caret),
                    fill: Some(geometry.caret_color),
                    ..WidgetQuad::default()
                }),
            )));
        }

        if !geometry.additional_carets.is_empty() {
            let rects: Arc<[LayoutBox]> = geometry
                .additional_carets
                .iter()
                .copied()
                .map(scroll_rect)
                .collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                13,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(geometry.additional_caret_color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if !geometry.preedit.is_empty() {
            let rects: Arc<[LayoutBox]> =
                geometry.preedit.iter().copied().map(scroll_rect).collect();
            out.push_tagged(tag_scrolled_content(WidgetTaggedOp::new(
                5,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: rects_bounds(&rects),
                    rects,
                    colors: WidgetBatchColors::Uniform(geometry.preedit_color),
                    fill_paint: None,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            )));
        }

        if let Some(color) = geometry.focus_ring {
            out.push_tagged(
                WidgetTaggedOp::new(
                    7,
                    WidgetPaintOp::FocusRing {
                        bounds,
                        color,
                        extra_outset: 0.0,
                    },
                )
                .with_parent_clips(),
            );
        }

        let push_overlay_panel = |out: &mut WidgetPaintOutput,
                                  slot: u64,
                                  rect: LayoutBox,
                                  background: [f32; 4],
                                  border_color: [f32; 4]| {
            out.push_tagged(
                WidgetTaggedOp::new(
                    slot,
                    WidgetPaintOp::Quad(WidgetQuad {
                        rect,
                        fill: Some(background),
                        border: Some((border_color, 1.0)),
                        corner_radius: [6.0; 4],
                        ..WidgetQuad::default()
                    }),
                )
                .with_parent_clips(),
            );
        };
        let push_overlay_text =
            |out: &mut WidgetPaintOutput,
             slot: u64,
             region: &ComponentTextRegion,
             horizontal_alignment: crate::TextHorizontalAlignment| {
                out.push_tagged(
                    WidgetTaggedOp::new(
                        slot,
                        WidgetPaintOp::Text(
                            WidgetText {
                                bounds: region.bounds,
                                content: region.content.clone(),
                                color: region.color,
                                font_size: region.font_size,
                                font_weight: region.font_weight,
                                horizontal_alignment,
                                vertical_alignment: TextVerticalAlignment::Center,
                                wrap: false,
                                ellipsis: true,
                                ..WidgetText::default()
                            }
                            .with_overlay_chrome(),
                        ),
                    )
                    .with_parent_clips(),
                );
            };

        if let Some(popup) = &geometry.completion_popup {
            push_overlay_panel(&mut out, 90, popup.panel, popup.background, popup.border);
            if let Some(index) = popup
                .rows
                .iter()
                .enumerate()
                .find(|(index, _)| popup.first_row + index == popup.selected)
                .map(|(index, _)| index)
            {
                out.push_tagged(
                    WidgetTaggedOp::new(
                        91,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: popup.rows[index].bounds,
                            fill: Some(popup.selected_background),
                            ..WidgetQuad::default()
                        }),
                    )
                    .with_parent_clips(),
                );
            }
            for (index, row) in popup.rows.iter().enumerate() {
                push_overlay_text(
                    &mut out,
                    92 + index as u64,
                    &row.label,
                    crate::TextHorizontalAlignment::Start,
                );
                if let Some(detail) = row.detail.as_ref() {
                    push_overlay_text(
                        &mut out,
                        100 + index as u64,
                        detail,
                        crate::TextHorizontalAlignment::Start,
                    );
                }
                if let Some(kind) = row.kind.as_ref() {
                    push_overlay_text(
                        &mut out,
                        108 + index as u64,
                        kind,
                        crate::TextHorizontalAlignment::End,
                    );
                }
                if let Some(doc) = row.doc.as_ref() {
                    push_overlay_text(
                        &mut out,
                        132 + index as u64,
                        doc,
                        crate::TextHorizontalAlignment::Start,
                    );
                }
            }
        }
        if let Some(popup) = &geometry.hover_popup {
            push_overlay_panel(&mut out, 120, popup.panel, popup.background, popup.border);
            push_overlay_text(
                &mut out,
                121,
                &popup.title,
                crate::TextHorizontalAlignment::Start,
            );
            for (index, row) in popup.body_rows.iter().enumerate() {
                push_overlay_text(
                    &mut out,
                    122 + index as u64,
                    row,
                    crate::TextHorizontalAlignment::Start,
                );
            }
        }
        if let Some(popup) = &geometry.signature_popup {
            push_overlay_panel(&mut out, 140, popup.panel, popup.background, popup.border);
            if let Some(active) = popup.active.as_ref() {
                out.push_tagged(
                    WidgetTaggedOp::new(
                        141,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: active.bounds,
                            fill: Some(popup.active_background),
                            ..WidgetQuad::default()
                        }),
                    )
                    .with_parent_clips(),
                );
            }
            push_overlay_text(
                &mut out,
                142,
                &popup.prefix,
                crate::TextHorizontalAlignment::Start,
            );
            if let Some(active) = popup.active.as_ref() {
                push_overlay_text(&mut out, 143, active, crate::TextHorizontalAlignment::Start);
            }
            push_overlay_text(
                &mut out,
                144,
                &popup.suffix,
                crate::TextHorizontalAlignment::Start,
            );
            if let Some(doc) = popup.doc.as_ref() {
                push_overlay_text(&mut out, 145, doc, crate::TextHorizontalAlignment::Start);
            }
        }

        out
    }
}

impl crate::WidgetVisual for crate::TextInputVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_INPUT)
    }

    fn measures_text(&self) -> bool {
        true
    }

    fn text_visual_key(&self) -> Option<(std::any::TypeId, u64)> {
        Some((std::any::TypeId::of::<Self>(), 0))
    }

    fn is_document_select_target(&self) -> bool {
        true
    }

    fn is_text_input(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let Some(mut geometry) = self.geometry_for(ctx.world, ctx.id, ctx.style) else {
            return crate::WidgetPaintOutput::default();
        };
        if ctx.world.caret_hidden(ctx.id) {
            // The hidden half of the blink. The caret's geometry stays
            // where the IME and scrolling read it.
            geometry.caret = None;
            geometry.additional_carets.clear();
        }
        let padding = ctx.world.used_layout_padding(ctx.id);
        let border = ctx.source_style.layout.resolved_border_width();
        Self::paint_with_geometry(&geometry, ctx.bounds, padding, border)
    }
}

impl UiWorld {
    pub(crate) fn derive_text_input_geometry(
        &self,
        id: StableNodeId,
        visual: &crate::TextInputVisual,
        style: &ComputedStyle,
    ) -> Option<crate::TextInputGeometry> {
        let crate::TextInputVisual {
            size,
            invalid,
            steppers,
            folds,
            editor_options,
            ..
        } = visual;
        let style_model = self.node_style_model_with(id, style.color_scope.as_deref());
        let bounds = self.component_layout_box(id)?;
        let mut source = NodeStyle::clone(&self.nodes.get(id)?.style);
        source.layout = Arc::clone(&self.record(id).resolved_layout);
        let padding = self.used_layout_padding(id);
        let border = source.layout.resolved_border_width();
        let content = self.component_content_box(id)?;
        let presentation = self.nodes.text_input_presentation(id)?;
        // Fold gutters and sticky headers share the displayed logical-line
        // index. Counting every prefix separately makes many folds quadratic.
        let display_newlines: Vec<usize> = if !folds.is_empty() {
            presentation
                .display_value
                .bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
                .collect()
        } else {
            Vec::new()
        };
        let display_line_of =
            |offset: usize| display_newlines.partition_point(|newline| *newline < offset);
        let accessibility = self
            .nodes
            .contains(id)
            .then(|| self.nodes.accessibility(id));
        let disabled = accessibility.as_ref().is_some_and(|state| state.disabled);
        // A read-only field refuses every step too, so its spinner
        // halves are inert: drawn and hit as they behave.
        let inert = disabled || accessibility.as_ref().is_some_and(|state| !state.editable);
        let steppers = steppers
            .then(|| {
                let band = (size.height_in(style_model.metrics) / 2.0).min(content.height / 2.0);
                let width = size.indicator_size();
                if band <= 0.0 || width <= 0.0 || content.width <= width {
                    return None;
                }
                let x = content.x + content.width - width;
                let numeric = accessibility.as_ref().map(|state| {
                    (
                        state.numeric_value.unwrap_or_default(),
                        state.numeric_minimum,
                        state.numeric_maximum,
                    )
                });
                let can_increment = !inert
                    && numeric.is_none_or(|(value, _, maximum)| {
                        maximum.is_none_or(|maximum| value < maximum)
                    });
                let can_decrement = !inert
                    && numeric.is_none_or(|(value, minimum, _)| {
                        minimum.is_none_or(|minimum| value > minimum)
                    });
                let active = style_model.color(SemanticColorRole::Muted).as_rgba_array();
                let inert = style_model.color(SemanticColorRole::Faint).as_rgba_array();
                Some(crate::NumberSteppers {
                    increment: LayoutBox {
                        x,
                        y: content.y + (content.height - band * 2.0) / 2.0,
                        width,
                        height: band,
                    },
                    decrement: LayoutBox {
                        x,
                        y: content.y + (content.height - band * 2.0) / 2.0 + band,
                        width,
                        height: band,
                    },
                    increment_color: if can_increment { active } else { inert },
                    decrement_color: if can_decrement { active } else { inert },
                    increment_enabled: can_increment,
                    decrement_enabled: can_decrement,
                    glyph_size: (width - 2.0).max(1.0),
                })
            })
            .flatten();
        // Where the text sits in the field — the content box less the
        // steppers, the scroll, a single-line field's centring and its
        // inline-start edge — is the editor frame's, the same one a
        // pointer hit is resolved through.
        let frame = self.editor_frame(id)?;
        let content = frame.content;
        let focused = self.input.focused.get(&self.record(id).document) == Some(&id);
        let multiline = accessibility.as_ref().is_some_and(|state| state.multiline);
        let line_height = frame.line;
        // Horizontal text space: `x` along the line, `y` across. A
        // single-line field has no block scroll of its own; its
        // centring is where its line top lands.
        let scroll_x = frame.inline_scroll;
        let scroll_y = if multiline { frame.block_scroll } else { 0.0 };
        let line_y = content.y - frame.block_scroll;
        let field_x = |offset: f32| content.x + offset - scroll_x;
        let (selection, preedit) = text_input_decorations(
            presentation,
            multiline,
            content,
            line_y,
            line_height,
            scroll_x,
            scroll_y,
        );
        let caret = focused.then(|| {
            caret_rule(
                field_x(presentation.caret_x),
                line_y + presentation.caret_y,
                line_height,
            )
        });
        // 附加多光标：与主光标同形，用主光标色的半透明变体区分；
        // 只随焦点出现（多行编辑器才有附加光标）。
        let node_theme = self.node_theme(id);
        let input_recipe = node_theme.recipes().input();
        let caret_color = style
            .color
            .unwrap_or_else(|| style_model.color(input_recipe.foreground).as_rgba_array());
        let additional_caret_color = {
            let mut color = caret_color;
            color[3] *= input_recipe.additional_caret_alpha;
            color
        };
        let additional_carets = if focused {
            presentation
                .additional_carets
                .iter()
                .map(|(x, y)| caret_rule(field_x(*x), line_y + *y, line_height))
                .collect()
        } else {
            Vec::new()
        };
        let marker_color = |severity| match severity {
            crate::TextDiagnosticSeverity::Error => {
                style_model.color(SemanticColorRole::Danger).as_rgba_array()
            }
            crate::TextDiagnosticSeverity::Warning => style_model
                .color(SemanticColorRole::Warning)
                .as_rgba_array(),
            crate::TextDiagnosticSeverity::Information | crate::TextDiagnosticSeverity::Hint => {
                style_model.color(SemanticColorRole::Muted).as_rgba_array()
            }
        };
        let diagnostic_markers = presentation
            .diagnostic_marks
            .iter()
            .map(|mark| {
                (
                    LayoutBox {
                        x: field_x(mark.rect.x),
                        y: content.y + mark.rect.y - scroll_y,
                        width: mark.rect.width,
                        height: mark.rect.height,
                    },
                    marker_color(mark.severity),
                )
            })
            .collect();
        let diagnostic_labels = presentation
            .diagnostic_labels
            .iter()
            .map(|label| crate::ComponentTextRegion {
                bounds: LayoutBox {
                    x: field_x(label.rect.x),
                    y: content.y + label.rect.y - scroll_y,
                    width: label.rect.width,
                    height: label.rect.height,
                },
                content: crate::TextValue::from(label.text.as_str()),
                color: Some(marker_color(label.severity)),
                font_size: size.text_size_in(self.node_theme(id).typography()),
                font_weight: None,
            })
            .collect();
        // 查找匹配：普通匹配用 accent 软色令牌，当前匹配用同一 accent
        // 色相加深（本地强调系数，与诊断条带的 2px 常量同级的局部约定）。
        let match_color = |current: bool| -> [f32; 4] {
            if current {
                let accent = style_model.color(SemanticColorRole::Accent).as_rgba_array();
                [accent[0], accent[1], accent[2], 0.45]
            } else {
                style_model
                    .color(SemanticColorRole::AccentSoftHover)
                    .as_rgba_array()
            }
        };
        let map_chip_box = |rect: LayoutBox| LayoutBox {
            x: field_x(rect.x),
            y: content.y + rect.y - scroll_y,
            width: rect.width,
            height: rect.height,
        };
        let atom_chips = presentation
            .atom_chips
            .iter()
            .map(|chip| {
                let mut mapped = chip.clone();
                mapped.bounds = map_chip_box(chip.bounds);
                mapped.close = map_chip_box(chip.close);
                mapped.icon_bounds = map_chip_box(chip.icon_bounds);
                mapped.label.bounds = map_chip_box(chip.label.bounds);
                if mapped.label.color.is_none() {
                    mapped.label.color =
                        Some(style_model.color(SemanticColorRole::Text).as_rgba_array());
                }
                mapped.background = style_model.color(SemanticColorRole::Subtle).as_rgba_array();
                mapped.border = style_model.color(SemanticColorRole::Border).as_rgba_array();
                mapped
            })
            .collect();
        let match_markers = presentation
            .match_marks
            .iter()
            .map(|mark| TextMatchMarker {
                rect: LayoutBox {
                    x: field_x(mark.rect.x),
                    y: content.y + mark.rect.y - scroll_y,
                    width: mark.rect.width,
                    height: mark.rect.height,
                },
                color: match_color(mark.current),
                current: mark.current,
            })
            .collect();
        // 颜色装饰 swatch：随滚动平移并按内容区纵向裁剪——只服务
        // 可见行，视口外的 swatch 不产生图元。颜色按宿主给定值直传
        // （半透明由绘制层常规 alpha 合成）。
        let swatch_markers = presentation
            .swatch_marks
            .iter()
            .filter_map(|mark| {
                let y = content.y + mark.rect.y - scroll_y;
                (y + mark.rect.height >= content.y && y <= content.y + content.height).then_some((
                    LayoutBox {
                        x: field_x(mark.rect.x),
                        y,
                        width: mark.rect.width,
                        height: mark.rect.height,
                    },
                    mark.color,
                ))
            })
            .collect();
        // 当前行条：聚焦多行且选区收起时，光标所在视觉行画低对比
        // 背景条（与选区层互斥，同用 slot 1 绘制层级）。
        let caret_line = if multiline && focused && presentation.selection.is_none() {
            Some((
                LayoutBox {
                    x: content.x,
                    y: line_y + presentation.caret_y - scroll_y,
                    width: content.width,
                    height: line_height,
                },
                style_model.color(SemanticColorRole::Hover).as_rgba_array(),
            ))
        } else {
            None
        };
        // 括号匹配描边框：跟随聚焦光标；两端共用 accent 色。
        let bracket_markers = if focused {
            presentation
                .bracket_marks
                .iter()
                .map(|rect| {
                    (
                        LayoutBox {
                            x: field_x(rect.x),
                            y: content.y + rect.y - scroll_y,
                            width: rect.width,
                            height: rect.height,
                        },
                        style_model.color(SemanticColorRole::Accent).as_rgba_array(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        // 出现高亮：淡底色填充（accent_soft，弱于查找匹配的
        // accent_soft_hover / accent@0.45），跟随聚焦光标。
        let occurrence_markers = if focused {
            presentation
                .occurrence_marks
                .iter()
                .map(|rect| {
                    (
                        LayoutBox {
                            x: field_x(rect.x),
                            y: content.y + rect.y - scroll_y,
                            width: rect.width,
                            height: rect.height,
                        },
                        style_model
                            .color(SemanticColorRole::AccentSoft)
                            .as_rgba_array(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        // 空白字符标记：静态结构标记（不随焦点变化），按视口裁剪
        // ——标记只服务可见行，视口外的空白不产生图元。
        let whitespace_color = style_model.color(SemanticColorRole::Faint).as_rgba_array();
        let whitespace_marks = presentation
            .whitespace_marks
            .iter()
            .filter_map(|mark| {
                let y = content.y + mark.rect.y - scroll_y;
                (y + mark.rect.height >= content.y && y <= content.y + content.height).then_some((
                    LayoutBox {
                        x: field_x(mark.rect.x),
                        y,
                        width: mark.rect.width,
                        height: mark.rect.height,
                    },
                    mark.kind,
                ))
            })
            .collect();
        // wrap guide：全高竖线（贯穿内容区，随横向滚动平移）。
        // 与缩进参考线（slot 10，仅行内缩进深度）区分：列位置由
        // 选项给定且贯穿全高。
        let wrap_guides = presentation
            .wrap_guides
            .iter()
            .map(|&x| {
                (
                    LayoutBox {
                        x: field_x(x),
                        y: content.y,
                        width: 1.0,
                        height: content.height,
                    },
                    style_model.color(SemanticColorRole::Faint).as_rgba_array(),
                )
            })
            .collect();
        // 缩进参考线：静态结构标记，不随焦点变化。
        let indent_guides = presentation
            .indent_guides
            .iter()
            .map(|rect| {
                (
                    LayoutBox {
                        x: field_x(rect.x),
                        y: content.y + rect.y - scroll_y,
                        width: rect.width,
                        height: rect.height,
                    },
                    style_model.color(SemanticColorRole::Border).as_rgba_array(),
                )
            })
            .collect();
        let line_labels = if multiline {
            presentation
                .line_tops
                .iter()
                .enumerate()
                .map(|(index, top)| crate::LineLabel {
                    y: content.y + top - scroll_y,
                    height: line_height,
                    // 折叠隐藏行后由 presentation 携带原始行号；
                    // 无折叠时行号就是显示索引 + 1。
                    number: presentation
                        .line_numbers
                        .get(index)
                        .copied()
                        .unwrap_or(index as u32 + 1),
                })
                .collect()
        } else {
            Vec::new()
        };
        // git gutter 竖条：gutter 最左侧 2px（宿主预留 padding-left
        // 的区域），随滚动平移并按内容区纵向裁剪。颜色取语义令牌：
        // 新增 = success、修改 = warning、删除 = danger（调色板的
        // 绿/黄/红三档，与诊断条带同源的语义配色，对应 git 惯例）。
        let git_color = |kind| match kind {
            crate::TextGitMarkKind::Added => style_model
                .color(SemanticColorRole::Success)
                .as_rgba_array(),
            crate::TextGitMarkKind::Modified => style_model
                .color(SemanticColorRole::Warning)
                .as_rgba_array(),
            crate::TextGitMarkKind::Deleted => {
                style_model.color(SemanticColorRole::Danger).as_rgba_array()
            }
        };
        let mut git_geometry = crate::TextGitGutterGeometry {
            added_color: git_color(crate::TextGitMarkKind::Added),
            modified_color: git_color(crate::TextGitMarkKind::Modified),
            deleted_color: git_color(crate::TextGitMarkKind::Deleted),
            ..crate::TextGitGutterGeometry::default()
        };
        for mark in &presentation.git_marks {
            let y = content.y + mark.y - scroll_y;
            // 视口外（上/下越界）的标记不产生图元。
            if y + mark.height < content.y || y > content.y + content.height {
                continue;
            }
            let rect = LayoutBox {
                x: bounds.x + border,
                y,
                width: 2.0,
                height: mark.height,
            };
            match mark.kind {
                crate::TextGitMarkKind::Added => git_geometry.added.push(rect),
                crate::TextGitMarkKind::Modified => git_geometry.modified.push(rect),
                crate::TextGitMarkKind::Deleted => git_geometry.deleted.push(rect),
            }
        }
        // 折叠几何：宿主喂入的每个折叠区间在起始行 gutter 画一个
        // 可点击箭头（折叠态右箭头/展开态下箭头）；折叠态区间在
        // 起始行行尾还有摘要标记命中框。gutter 空间不足（padding
        // 左侧小于 18px）时不画箭头，摘要标记仍可点击。
        let mut fold_geometry = crate::TextFoldGeometry::default();
        if multiline
            && let Some(input) = self.nodes.text_input(id)
            && let Some(entry) = self.nodes.text_fold_view(id)
        {
            let offered = Arc::clone(folds);
            if !offered.is_empty() {
                let view = self.text_display_view(id);
                let display_of = |offset: usize| match &view {
                    Some(view) => view
                        .display_of(offset)
                        .min(presentation.display_value.len()),
                    None => offset.min(presentation.display_value.len()),
                };
                let gutter_width = if padding.left >= 18.0 {
                    (padding.left - 4.0).min(14.0)
                } else {
                    0.0
                };
                for fold in offered.iter() {
                    let collapsed = entry.collapsed.contains(fold);
                    let fold_start = fold.start.min(input.value.len());
                    let display_offset = display_of(fold_start);
                    // 嵌套折叠：起始行被父折叠隐藏时（显示映射钳到
                    // 别处）不画箭头，避免在父折叠起始行叠加幽灵箭头。
                    if !collapsed && display_offset != fold_start {
                        continue;
                    }
                    let display_line = display_line_of(display_offset);
                    if let Some(&top) = presentation.line_tops.get(display_line)
                        && gutter_width > 0.0
                    {
                        let extent = gutter_width.min(line_height);
                        fold_geometry.gutters.push(crate::TextFoldGutter {
                            bounds: LayoutBox {
                                x: bounds.x + border + 2.0,
                                y: content.y + top - scroll_y
                                    + (line_height - extent).max(0.0) / 2.0,
                                width: extent,
                                height: extent,
                            },
                            fold: *fold,
                            collapsed,
                            color: style_model.color(SemanticColorRole::Faint).as_rgba_array(),
                        });
                    }
                }
                for mark in &presentation.fold_marks {
                    fold_geometry
                        .markers
                        .push(crate::components::TextFoldMarker {
                            bounds: LayoutBox {
                                x: field_x(mark.rect.x),
                                y: content.y + mark.rect.y - scroll_y,
                                width: mark.rect.width + 2.0,
                                height: mark.rect.height,
                            },
                            fold: mark.fold,
                        });
                }
            }
        }
        // sticky scroll：滚动视口顶部落在宿主喂入的折叠区间内部时，
        // 在内容区顶部钉住显示该区间头行（首视觉行，软换行只钉第一
        // 行）。嵌套区间钉最内层（头行顶最大、最靠近视口顶的候选）；
        // 头行仍自然可见（head_top >= scroll_y）或区间末行已完全滚
        // 过视口顶（end_top + line_height <= scroll_y）时不钉——头行
        // 滚回自然位置钉住行瞬时消失。折叠语义不改变派生：折叠态区
        // 间头照常可钉，头行落在折叠隐藏区间内部的跳过（没有可见头
        // 行可钉）。派生每帧
        // 线性扫区间表（区间数小，不加缓存）；钉住行不做内容偏移，
        // 视口下缘被覆盖的内容照常滚动（非 VSCode 推开式布局）。
        let sticky_line = if multiline
            && editor_options.sticky_scroll
            && let Some(input) = self.nodes.text_input(id)
        {
            let offered = folds;
            let view = self.text_display_view(id);
            let display_of = |offset: usize| match &view {
                Some(view) => view
                    .display_of(offset)
                    .min(presentation.display_value.len()),
                None => offset.min(presentation.display_value.len()),
            };
            let value = presentation.display_value.as_str();
            let mut candidate: Option<(f32, usize)> = None;
            for fold in offered.iter() {
                let head = fold.start.min(input.value.len());
                // 头行落在某个折叠态区间的隐藏范围内（值空间被钳制
                // 映射到替代文本）：无可见头行可钉。不能用显示偏移
                // 不等判定——头行在其前折叠区间之后时显示偏移同样
                // 平移，但头行本身可见。
                let head_hidden = view.as_ref().is_some_and(|view| {
                    view.spans
                        .iter()
                        .any(|span| head > span.value_start && head < span.value_end)
                });
                if head_hidden {
                    continue;
                }
                let head_display = display_of(head);
                let end_display = display_of(fold.end.min(input.value.len()));
                let (Some(&head_top), Some(&end_top)) = (
                    presentation.line_tops.get(display_line_of(head_display)),
                    presentation.line_tops.get(display_line_of(end_display)),
                ) else {
                    continue;
                };
                // 头行未滚出视口顶，或区间末行已完全滚过视口顶（按
                // 末行底缘计：末行仍跨视口顶时视口顶行仍在区间内）：
                // 不钉。
                if head_top >= scroll_y || end_top + line_height <= scroll_y {
                    continue;
                }
                if candidate.is_none_or(|(top, _)| head_top > top) {
                    candidate = Some((head_top, head_display));
                }
            }
            candidate.map(|(_, head_display)| {
                let head_line_start = value[..head_display]
                    .rfind('\n')
                    .map_or(0, |index| index + 1);
                let head_line_end = value[head_line_start..]
                    .find('\n')
                    .map_or(value.len(), |index| head_line_start + index);
                crate::TextStickyLineGeometry {
                    panel: LayoutBox {
                        x: content.x,
                        y: content.y,
                        width: content.width,
                        height: line_height,
                    },
                    divider: LayoutBox {
                        x: content.x,
                        y: content.y + line_height - 1.0,
                        width: content.width,
                        height: 1.0,
                    },
                    text: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: content.x,
                            y: content.y,
                            width: content.width,
                            height: line_height,
                        },
                        content: crate::TextValue::from(&value[head_line_start..head_line_end]),
                        color: Some(caret_color),
                        font_size: size.text_size_in(self.node_theme(id).typography()),
                        font_weight: style.font_weight,
                    },
                    background: style
                        .paint_colors
                        .background
                        .map(nana_ui_core::PaintColor::to_srgb)
                        .or(style.background)
                        .unwrap_or_else(|| {
                            style_model
                                .color(SemanticColorRole::Surface)
                                .as_rgba_array()
                        }),
                    divider_color: style_model.color(SemanticColorRole::Border).as_rgba_array(),
                }
            })
        } else {
            None
        };
        // minimap 竖条：内容区右缘 64px 覆盖条 + 1px 分隔线 + 行条
        // 与视口指示器。文本行宽计算不变（极长行会被条遮挡——声明
        // 取舍）；折叠只影响主视图，行条按原始逻辑行全长计算。禁用
        // 态与窄内容不产生条。
        let minimap = if multiline
            && !disabled
            && content.width > crate::components::TEXT_MINIMAP_STRIP_WIDTH
            && !presentation.minimap_line_lengths.is_empty()
        {
            Some(text_minimap_geometry(
                &presentation.minimap_line_lengths,
                content,
                scroll_y,
                line_height,
                &style_model.palette,
            ))
        } else {
            None
        };
        // 拖拽移动选中文本的落点指示线：框架侧拖拽态写入的文本
        // 空间细竖线（2px），随滚动平移；纯视觉指示，无命中框。
        let drop_indicator = self.text_drop_indicator(id).map(|rect| {
            (
                LayoutBox {
                    x: field_x(rect.x),
                    y: content.y + rect.y - scroll_y,
                    width: rect.width,
                    height: rect.height,
                },
                style_model.color(SemanticColorRole::Accent).as_rgba_array(),
            )
        });
        let geometry = crate::TextInputGeometry {
            character_counter: self.text_input_character_counter(id),
            suffix: self.text_input_suffix(id),
            resize_grip: editor_options.resize_vertical.then_some(LayoutBox {
                x: bounds.x + (bounds.width - 14.0).max(0.0),
                y: bounds.y + (bounds.height - 14.0).max(0.0),
                width: bounds.width.min(14.0),
                height: bounds.height.min(14.0),
            }),
            diagnostic_markers,
            diagnostic_labels,
            match_markers,
            swatch_markers,
            swatch_border_color: style_model.color(SemanticColorRole::Border).as_rgba_array(),
            atom_chips,
            caret_line,
            bracket_markers,
            occurrence_markers,
            drop_indicator,
            whitespace_marks,
            whitespace_color,
            wrap_guides,
            indent_guides,
            line_labels,
            folds: fold_geometry,
            git_marks: git_geometry,
            line_labels_color: style_model.color(SemanticColorRole::Faint).as_rgba_array(),
            line_labels_font_size: size.caption_size_in(self.node_theme(id).typography()),
            text: crate::ComponentTextRegion {
                bounds: frame.text_bounds(
                    presentation.content_size.width,
                    presentation.content_size.height,
                    multiline,
                ),
                content: presentation.display_value.clone(),
                color: Some(if presentation.placeholder {
                    text_input_placeholder_color(
                        &source.layout,
                        style_model.color(input_recipe.placeholder).as_rgba_array(),
                    )
                } else {
                    style.color.unwrap_or_else(|| {
                        style_model.color(input_recipe.foreground).as_rgba_array()
                    })
                }),
                font_size: size.text_size_in(self.node_theme(id).typography()),
                font_weight: style.font_weight,
            },
            multiline,
            scroll: multiline.then(|| {
                frame.scroll_x(
                    presentation.content_size.width,
                    presentation.content_size.height,
                    multiline,
                )
            }),
            selection,
            caret,
            additional_carets,
            preedit,
            completion_popup: {
                // 补全弹层：聚焦多行编辑器 + 未关闭的非空候选会话；
                // 锚定主光标行，与其他编辑器覆盖层共用一套定位翻
                // 转策略（见 `anchored_overlay_panel`）。
                if multiline && focused {
                    self.nodes
                        .text_completion_view(id)
                        .filter(|state| !state.dismissed)
                        .zip(presentation.overlay_metrics.completion.as_ref())
                        .and_then(|(state, metrics)| {
                            completion_popup_geometry(
                                state,
                                metrics,
                                OverlayAnchor {
                                    x: field_x(presentation.caret_x),
                                    line_top: line_y + presentation.caret_y,
                                    line_height,
                                },
                                bounds,
                                size.text_size_in(self.node_theme(id).typography()),
                                &style_model.palette,
                            )
                        })
                } else {
                    None
                }
            },
            hover_popup: {
                // hover 浮窗：宿主喂入即显示（不要求焦点），纯展示。
                presentation
                    .overlay_metrics
                    .hover_anchor
                    .zip(self.nodes.text_hover_view(id))
                    .and_then(|((hover_x, hover_y), state)| {
                        hover_popup_geometry(
                            state,
                            OverlayAnchor {
                                x: field_x(hover_x),
                                line_top: line_y + hover_y,
                                line_height,
                            },
                            bounds,
                            size.text_size_in(self.node_theme(id).typography()),
                            &style_model.palette,
                        )
                    })
            },
            signature_popup: {
                // 签名帮助：宿主喂入即显示，锚定 caret；占位符与
                // IME 组合期不弹出（与 hover 同一打字态门）。
                if multiline && !presentation.placeholder && presentation.preedit.is_none() {
                    self.nodes.text_signature(id).and_then(|help| {
                        signature_popup_geometry(
                            help,
                            OverlayAnchor {
                                x: field_x(presentation.caret_x),
                                line_top: line_y + presentation.caret_y,
                                line_height,
                            },
                            bounds,
                            size.text_size_in(self.node_theme(id).typography()),
                            &style_model.palette,
                            self.chrome_text_measure(id),
                        )
                    })
                } else {
                    None
                }
            },
            minimap,
            sticky_line,
            background: style
                .paint_colors
                .background
                .map(nana_ui_core::PaintColor::to_srgb)
                .or(style.background),
            border: style
                .paint_colors
                .border
                .map(nana_ui_core::PaintColor::to_srgb)
                .or(style.border_color),
            border_width: {
                let width = if style.border_color.is_some() {
                    source.layout.resolved_border_width()
                } else {
                    0.0
                };
                if focused && *invalid {
                    width.max(input_recipe.invalid_focus_border_width)
                } else {
                    width
                }
            },
            focus_ring: None,
            selection_color: style
                .paint_colors
                .selection_background
                .map(nana_ui_core::PaintColor::to_srgb)
                .or(style.selection_background)
                .unwrap_or_else(|| {
                    style_model
                        .color(input_recipe.selection_background)
                        .as_rgba_array()
                }),
            caret_color,
            additional_caret_color,
            preedit_color: style_model
                .color(input_recipe.preedit_underline)
                .as_rgba_array(),
            steppers,
        };
        Some(if frame.writing.is_vertical() {
            vertical_text_input_geometry(geometry, &frame, presentation, multiline, focused)
        } else {
            geometry
        })
    }

    /// An open menu's room in its field's own coordinates: the room the
    /// field projected (or the window, before it has one) moved by the
    /// field's page origin. `None` for a closed field, and for an open one
    /// with neither.
    ///
    /// This reads where the field stands, as fitting a menu to what can be
    /// seen must; what it decides is only where the menu's surface goes
    /// ([`crate::select::select_menu_surface`]), and a field that only moved
    /// keeps its record while that box is the same
    /// ([`Self::local_select_menu_surface`]).
    pub(crate) fn local_select_menu_room(
        &self,
        id: StableNodeId,
        opened: bool,
        room: Option<crate::SelectMenuRoom>,
    ) -> Option<crate::SelectMenuRoom> {
        if !opened {
            return None;
        }
        let room = room.or_else(|| {
            let document = self.node(id)?.document;
            crate::select::window_room(self.document_viewports.get(&document).copied())
        })?;
        let previous = self.geometry_frame.replace(None);
        let origin = self.layout_origin(id);
        self.geometry_frame.set(previous);
        let [x, y] = origin?;
        Some(crate::SelectMenuRoom {
            top: room.top - y,
            bottom: room.bottom - y,
            left: room.left - x,
            right: room.right - x,
        })
    }

    /// Where `id`'s open menu surface stands in the field's own coordinates
    /// now, as [`Self::local_widget_paint`] would place it: the one
    /// part of an open field's record that depends on where the field
    /// stands. `None` for anything but an open Select, Dropdown or
    /// SearchDropdown.
    pub(crate) fn local_select_menu_surface(&self, id: StableNodeId) -> Option<LayoutBox> {
        let Some(crate::SelectVisual {
            opened: true,
            size,
            options,
            menu_room,
            checkable,
            fit_options,
            ..
        }) = self.widget_visual_as::<crate::SelectVisual>(id)
        else {
            return None;
        };
        let previous = self.geometry_frame.replace(Some(id));
        let bounds = self.component_layout_box(id);
        let room = self.local_select_menu_room(id, true, *menu_room);
        let fitted = self.select_fitted_options_width(id, *fit_options);
        self.geometry_frame.set(previous);
        let node_theme = self.node_theme(id);
        let style_model = self.node_style_model(id);
        Some(crate::select::select_menu_surface(
            bounds?,
            *size,
            options,
            style_model,
            node_theme,
            room,
            *checkable,
            fitted,
        ))
    }

    /// The open Select, Dropdown and SearchDropdown menus of `document`
    /// whose room is no longer the one [`Self::select_menu_room`] gives
    /// now, each with its visual carrying the room it has now.
    ///
    /// A field projects its menu's room from the boxes the last layout gave
    /// it; a layout pass that moves those boxes or the viewport, but not the
    /// field's own state, leaves that room behind. Fed back after every
    /// layout pass, the menu that is painted, hit and read is placed against
    /// the layout just published.
    pub(crate) fn stale_select_menu_rooms(
        &self,
        document: DocumentId,
    ) -> Vec<(StableNodeId, crate::SelectVisual)> {
        let mut stale = self
            .nodes
            .open_select_menus()
            .filter(|id| self.document_of(*id) == Some(document) && self.is_mounted(*id))
            .filter_map(|id| {
                let visual = self.widget_visual_as::<crate::SelectVisual>(id)?;
                let room = self.select_menu_room(id);
                (visual.menu_room != room).then(|| {
                    let mut updated = visual.clone();
                    updated.menu_room = room;
                    (id, updated)
                })
            })
            .collect::<Vec<_>>();
        stale.compact_sort_by_key(|(id, _)| *id);
        stale
    }

    /// Where a menu hanging off `id` can be seen, in `id`'s own layout
    /// coordinates: the window and every clip on `id`'s clip chain, each moved
    /// through the scroll offsets between that box and `id`. The chain is the
    /// one paint and hit testing follow ([`Self::clip_ancestors`], #201): an
    /// escaped box passes over the containers it escapes, and a `fixed` box,
    /// the field or one above it, ends it. Transforms are left out: they bend
    /// where the menu is placed, not whether what it draws matches what it
    /// hits.
    ///
    /// `None` until the document is laid out, or when nothing is left to see.
    pub(crate) fn select_menu_room(&self, id: StableNodeId) -> Option<crate::SelectMenuRoom> {
        let viewport = self.document_viewport_of(id)?;
        let mut room = crate::SelectMenuRoom {
            top: f32::NEG_INFINITY,
            bottom: f32::INFINITY,
            left: f32::NEG_INFINITY,
            right: f32::INFINITY,
        };
        let mut clip = |bounds: LayoutBox, shift: crate::ScrollOffset| {
            room.top = room.top.max(bounds.y + shift.y);
            room.bottom = room.bottom.min(bounds.y + bounds.height + shift.y);
            room.left = room.left.max(bounds.x + shift.x);
            room.right = room.right.min(bounds.x + bounds.width + shift.x);
        };
        // Scroll offsets between the box being looked at and `id`.
        let mut shift = crate::ScrollOffset::default();
        // The link of the chain below `ancestor`.
        let mut below = id;
        for ancestor in self.clip_ancestors(id) {
            let offset = self.scroll_offset(ancestor).unwrap_or_default();
            shift.x += offset.x;
            shift.y += offset.y;
            let style = self.hit_motion_layout(ancestor);
            let layout = self.component_layout_box(ancestor)?;
            if let Some((x, y, width, height)) =
                style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                clip(
                    LayoutBox {
                        x,
                        y,
                        width,
                        height,
                    },
                    shift,
                );
            }
            if self.clip_visuals != 0 {
                if self
                    .widget_visual_ref(ancestor)
                    .is_some_and(crate::WidgetVisualHandle::clips_descendants)
                {
                    clip(layout, shift);
                }
                if let Some(modal) = self.widget_visual_as::<crate::ModalFrameVisual>(ancestor)
                    && let Some(crate::ModalFrameGeometry { surface, body, .. }) =
                        self.modal_frame_geometry(ancestor)
                {
                    clip(surface, shift);
                    if modal.slots.body == Some(below) {
                        clip(body, shift);
                    }
                }
            }
            below = ancestor;
        }
        clip(
            LayoutBox {
                width: viewport.width,
                height: viewport.height,
                ..LayoutBox::default()
            },
            shift,
        );
        (room.top < room.bottom && room.left < room.right).then_some(room)
    }
}

/// A vertical editor's component geometry (#59): the value, caret, selection
/// and preedit turned from text space onto the page through `frame`, and the
/// code-editor chrome dropped.
///
/// Gutters, guides, minimaps, inline diagnostics and anchored popups are laid
/// out against horizontal lines — a line-number column, a label after the
/// line's end, a popup under the caret's line — and there is no honest column
/// version of them to fall back to, so a vertical editor has none rather than
/// ones drawn across its columns.
fn vertical_text_input_geometry(
    geometry: crate::TextInputGeometry,
    frame: &EditorFrame,
    presentation: &TextInputPresentation,
    multiline: bool,
    focused: bool,
) -> crate::TextInputGeometry {
    let crate::TextInputGeometry {
        resize_grip,
        character_counter,
        suffix,
        mut text,
        additional_caret_color,
        swatch_border_color,
        whitespace_color,
        line_labels_color,
        line_labels_font_size,
        background,
        border,
        border_width,
        focus_ring,
        selection_color,
        caret_color,
        preedit_color,
        steppers,
        ..
    } = geometry;
    let line = presentation.line_height.max(1.0);
    let caret_at = |(inline, block): (f32, f32)| frame.field_rect(caret_rule(inline, block, line));
    // Along the columns is the page's height, across them its width.
    let (inline_extent, block_extent) = (
        presentation.content_size.height,
        presentation.content_size.width,
    );
    text.bounds = frame.text_bounds(inline_extent, block_extent, multiline);
    // The preedit underline runs beside the column, on its block-start side:
    // the right of a `vertical-rl` column.
    let preedit = presentation
        .preedit_lines
        .iter()
        .map(|rect| frame.field_rect(preedit_rule(*rect, false)))
        .collect();
    crate::TextInputGeometry {
        resize_grip,
        character_counter,
        suffix,
        text,
        multiline,
        scroll: multiline.then(|| frame.scroll_x(inline_extent, block_extent, multiline)),
        selection: presentation
            .selection_lines
            .iter()
            .map(|rect| frame.field_rect(*rect))
            .collect(),
        caret: focused.then(|| caret_at((presentation.caret_x, presentation.caret_y))),
        additional_carets: if focused {
            presentation
                .additional_carets
                .iter()
                .copied()
                .map(caret_at)
                .collect()
        } else {
            Vec::new()
        },
        additional_caret_color,
        preedit,
        diagnostic_markers: Vec::new(),
        diagnostic_labels: Vec::new(),
        match_markers: Vec::new(),
        swatch_markers: Vec::new(),
        swatch_border_color,
        atom_chips: Vec::new(),
        caret_line: None,
        bracket_markers: Vec::new(),
        occurrence_markers: Vec::new(),
        drop_indicator: None,
        whitespace_marks: Vec::new(),
        whitespace_color,
        wrap_guides: Vec::new(),
        indent_guides: Vec::new(),
        line_labels: Vec::new(),
        folds: crate::components::TextFoldGeometry::default(),
        git_marks: crate::components::TextGitGutterGeometry::default(),
        line_labels_color,
        line_labels_font_size,
        completion_popup: None,
        hover_popup: None,
        signature_popup: None,
        minimap: None,
        sticky_line: None,
        background,
        border,
        border_width,
        focus_ring,
        selection_color,
        caret_color,
        preedit_color,
        steppers,
    }
}

/// A value thumb's height: the control's text line with a small step of
/// room above and below it, a little under the control itself.
pub(crate) fn range_value_thumb_height(
    size: nana_ui_core::ControlSize,
    typography: nana_ui_core::TypographyTokens,
    spacing: nana_ui_core::SpacingTokens,
) -> f32 {
    let line = match size {
        nana_ui_core::ControlSize::Large => typography.line_tall,
        _ => typography.line,
    };
    line + spacing.sm
}
