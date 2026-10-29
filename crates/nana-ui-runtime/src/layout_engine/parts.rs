//! Lightweight component content uses the same Flex solve as retained children.
//! Parts have no entity, lifecycle, text engine or independent geometry store.

use super::*;
use crate::{LayoutPart, LayoutPartKind, StandardVisual, TextHorizontalAlignment};

#[derive(Clone, Copy)]
pub(crate) struct FlexPart {
    pub kind: LayoutPartKind,
    pub preferred: Size,
    pub grow: f32,
    pub shrink: f32,
    pub align: AlignSpec,
    pub gap_after: f32,
}

impl FlexPart {
    pub fn new(kind: LayoutPartKind, preferred: Size) -> Self {
        Self {
            kind,
            preferred,
            grow: 0.0,
            shrink: 0.0,
            align: AlignSpec::Center,
            gap_after: 0.0,
        }
    }
}

/// How many semantic parts one component's content publishes at most: a
/// toast's mark, title, description and dismiss target.
pub(crate) const CONTENT_PART_SLOTS: usize = 4;

/// A fixed number of semantic slots, held inline; absent slots cost no
/// retained allocation, and only the parts present are published.
pub(crate) type ContentParts = [Option<LayoutPart>; CONTENT_PART_SLOTS];

/// No content parts.
pub(crate) const NO_CONTENT_PARTS: ContentParts = [None; CONTENT_PART_SLOTS];

/// `parts` in the first slots, the rest empty.
pub(crate) fn content_parts<const N: usize>(parts: [Option<LayoutPart>; N]) -> ContentParts {
    const { assert!(N <= CONTENT_PART_SLOTS) };
    std::array::from_fn(|index| parts.get(index).copied().flatten())
}

pub(crate) fn flex_parts<const N: usize>(
    bounds: LayoutBox,
    direction: FlexDirection,
    justify: JustifySpec,
    reverse: bool,
    inputs: [Option<FlexPart>; N],
) -> [Option<LayoutPart>; N] {
    let main = main_extent(Size::new(bounds.width, bounds.height), direction);
    let cross = cross_extent(Size::new(bounds.width, bounds.height), direction);
    let mut sizes =
        inputs.map(|part| part.map_or(0.0, |part| main_extent(part.preferred, direction)));
    let mins = [0.0; N];
    let margins = [0.0; N];
    let shrinks = inputs.map(|part| part.map_or(0.0, |part| part.shrink));
    let gap: f32 = inputs
        .iter()
        .flatten()
        .map(|part| part.gap_after.max(0.0))
        .sum();
    let grows = inputs.map(|part| part.map_or(0.0, |part| part.grow));
    if grows.iter().any(|grow| *grow > 0.0) {
        let fixed =
            std::array::from_fn::<_, N, _>(|index| (grows[index] == 0.0).then_some(sizes[index]));
        let solved =
            resolve_flex_fill_sizes(main, gap, &margins, &fixed, &mins, &[None; N], &grows);
        sizes.copy_from_slice(&solved);
    }
    apply_flex_shrink(main, gap, &margins, &mut sizes, &mins, &shrinks);
    let occupied = sizes.iter().sum::<f32>() + gap;
    let (mut cursor, _) = justify_offsets(justify, main, occupied, 0.0, 0);
    std::array::from_fn(|index| {
        let input = inputs[index]?;
        let extent = sizes[index].min((main - cursor).max(0.0));
        let cross_extent = if input.align == AlignSpec::Stretch {
            cross
        } else {
            cross_extent(input.preferred, direction).min(cross)
        };
        let cross_offset = match input.align {
            AlignSpec::Center => (cross - cross_extent).max(0.0) / 2.0,
            AlignSpec::End => (cross - cross_extent).max(0.0),
            _ => 0.0,
        };
        let start = if reverse {
            main - cursor.min(main) - extent
        } else {
            cursor.min(main)
        };
        cursor += sizes[index] + input.gap_after.max(0.0);
        let part_bounds = match direction {
            FlexDirection::Row => LayoutBox {
                x: bounds.x + start,
                y: bounds.y + cross_offset,
                width: extent,
                height: cross_extent,
            },
            FlexDirection::Column => LayoutBox {
                x: bounds.x + cross_offset,
                y: bounds.y + start,
                width: cross_extent,
                height: extent,
            },
        };
        Some(LayoutPart::new(input.kind, part_bounds))
    })
}

pub(crate) fn button_parts(visual: &StandardVisual, text: Size) -> Option<[Option<FlexPart>; 3]> {
    let StandardVisual::Button {
        label,
        loading,
        icon,
        trailing_icon,
        icon_size,
        icon_gap,
        ..
    } = visual
    else {
        return None;
    };
    let leading = *loading || icon.is_some();
    let trailing = trailing_icon.is_some();
    let glyph = Size::new(*icon_size, *icon_size);
    let mut label_part = FlexPart::new(LayoutPartKind::TextContent, text);
    label_part.shrink = 1.0;
    label_part.align = AlignSpec::Stretch;
    label_part.gap_after = if trailing && !label.is_empty() {
        *icon_gap
    } else {
        0.0
    };
    Some([
        leading.then(|| {
            let mut part = FlexPart::new(
                if *loading {
                    LayoutPartKind::Spinner
                } else {
                    LayoutPartKind::LeadingContent
                },
                glyph,
            );
            part.gap_after = if !label.is_empty() || trailing {
                *icon_gap
            } else {
                0.0
            };
            part
        }),
        Some(label_part),
        trailing.then(|| FlexPart::new(LayoutPartKind::TrailingContent, glyph)),
    ])
}

/// A checkbox's row: its `indicator` square, then its label, which takes
/// the rest of the row so the label's own alignment has the room the old
/// hand-placed label box gave it. The gap stands only before a label.
pub(crate) fn checkbox_parts(
    indicator: f32,
    gap: f32,
    labelled: bool,
    text: Size,
) -> [Option<FlexPart>; 3] {
    let mut mark = FlexPart::new(
        LayoutPartKind::LeadingContent,
        Size::new(indicator, indicator),
    );
    mark.gap_after = if labelled { gap } else { 0.0 };
    let mut label = FlexPart::new(LayoutPartKind::TextContent, text);
    label.grow = 1.0;
    label.shrink = 1.0;
    label.align = AlignSpec::Stretch;
    [Some(mark), Some(label), None]
}

/// A switch's content before placement: its track and, beside it, a column
/// of its label over its hint. Intrinsic measure ([`Self::extent`]) and
/// placement ([`switch_parts`]) both read this one declaration.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct SwitchInputs {
    pub track: Size,
    /// Between the track and the text column, when there is one.
    pub gap: f32,
    /// The track ends the row (`SwitchControlPosition::End`).
    pub end: bool,
    /// The switch says something beside its track: a label or a hint.
    pub labelled: bool,
    pub label_line: f32,
    /// The gap above the hint and the hint's line, when it has one.
    pub hint: Option<(f32, f32)>,
}

impl SwitchInputs {
    fn column_height(&self) -> f32 {
        if !self.labelled {
            return 0.0;
        }
        self.label_line + self.hint.map_or(0.0, |(gap, line)| gap + line)
    }

    /// What the switch holds beside its text: the track and its gap, and the
    /// taller of the track and the text column.
    pub fn extent(&self) -> Size {
        Size::new(
            self.track.width.max(0.0)
                + if self.labelled {
                    self.gap.max(0.0)
                } else {
                    0.0
                },
            self.track.height.max(0.0).max(self.column_height()),
        )
    }
}

/// A switch's track, label and hint in `bounds`. The row is the track and a
/// text column that takes the rest; a track at the end is the same row
/// reversed, so it keeps the end edge when the box is too narrow for both.
/// The column stacks the label line over the hint, or is all label.
pub(crate) fn switch_parts(bounds: LayoutBox, inputs: SwitchInputs) -> ContentParts {
    let mut track = FlexPart::new(
        if inputs.end {
            LayoutPartKind::TrailingContent
        } else {
            LayoutPartKind::LeadingContent
        },
        inputs.track,
    );
    track.gap_after = if inputs.labelled { inputs.gap } else { 0.0 };
    let column = inputs.labelled.then(|| {
        let mut part = FlexPart::new(
            LayoutPartKind::Label,
            Size::new(0.0, inputs.column_height()),
        );
        part.grow = 1.0;
        part.shrink = 1.0;
        part.align = AlignSpec::Stretch;
        part
    });
    let [track, column] = flex_parts(
        bounds,
        FlexDirection::Row,
        JustifySpec::Start,
        inputs.end,
        [Some(track), column],
    );
    let Some(column) = column else {
        return content_parts([track]);
    };
    let Some((gap, line)) = inputs.hint else {
        return content_parts([track, Some(column)]);
    };
    let mut label = FlexPart::new(LayoutPartKind::Label, Size::new(0.0, inputs.label_line));
    label.align = AlignSpec::Stretch;
    label.gap_after = gap;
    let mut hint = FlexPart::new(LayoutPartKind::Hint, Size::new(0.0, line));
    hint.align = AlignSpec::Stretch;
    let [label, hint] = flex_parts(
        column.bounds,
        FlexDirection::Column,
        JustifySpec::Start,
        false,
        [Some(label), Some(hint)],
    );
    content_parts([track, label, hint])
}

/// A status dot's row: the dot centred in its `slot`, then the label, which
/// takes the rest of the row. The space either side of the dot is a spacer
/// before it and part of its gap after it, so the published
/// `LeadingContent` is the dot itself.
pub(crate) fn status_dot_parts(
    slot: f32,
    diameter: f32,
    gap: f32,
    text: Size,
) -> [Option<FlexPart>; 3] {
    let inset = ((slot - diameter) / 2.0).max(0.0);
    let lead = FlexPart::new(LayoutPartKind::ComponentContent, Size::new(inset, 0.0));
    let mut dot = FlexPart::new(
        LayoutPartKind::LeadingContent,
        Size::new(diameter, diameter),
    );
    dot.gap_after = inset + gap;
    let mut label = FlexPart::new(LayoutPartKind::TextContent, text);
    label.grow = 1.0;
    label.shrink = 1.0;
    label.align = AlignSpec::Stretch;
    [Some(lead), Some(dot), Some(label)]
}

/// A selection option's row (a radio row, a segmented item, a tab): the
/// radio `ring` square and its gap, the `icon` square and its gap, then the
/// label, which takes the rest so its own alignment (start for a radio,
/// centred for a segmented item) has the whole lane. Each gap stands whether
/// or not there is a label, as the inset the row used to reserve did.
pub(crate) fn selection_option_parts(
    ring: Option<(f32, f32)>,
    icon: Option<(f32, f32)>,
    text: Size,
) -> [Option<FlexPart>; 3] {
    let glyph = |kind, (extent, gap): (f32, f32)| {
        let mut part = FlexPart::new(kind, Size::new(extent, extent));
        part.gap_after = gap;
        part
    };
    let mut label = FlexPart::new(LayoutPartKind::TextContent, text);
    label.grow = 1.0;
    label.shrink = 1.0;
    label.align = AlignSpec::Stretch;
    [
        ring.map(|ring| glyph(LayoutPartKind::Indicator, ring)),
        icon.map(|icon| glyph(LayoutPartKind::LeadingContent, icon)),
        Some(label),
    ]
}

/// [`selection_option_parts`] placed in `bounds`. An icon beside a label
/// that is not centred in the row sits on the centre of the label's first
/// line, `icon_line` tall, rather than on the row's.
pub(crate) fn selection_option_row(
    bounds: LayoutBox,
    inputs: [Option<FlexPart>; 3],
    icon_line: Option<f32>,
) -> ContentParts {
    let mut parts = start_row_parts(bounds, inputs);
    if let (Some(line), Some(icon)) = (icon_line, parts[1].as_mut()) {
        icon.bounds.y = bounds.y + line * 0.5 - icon.bounds.height * 0.5;
    }
    parts
}

/// An empty state's text block before placement: an optional icon over its
/// title over an optional message, in a band inset from the border box.
/// The band's width is the paragraph budget, the column's height (with the
/// action's lead-in when there is one) is what the block reserves above the
/// action, and placement solves the same column; all read this one
/// declaration.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct EmptyStateInputs {
    /// The band's inset from the border box on each side.
    pub inset: Size,
    /// The icon's square, when there is one.
    pub icon: Option<f32>,
    /// Between the icon, the title and the message.
    pub gap: f32,
    /// Each item centred in the band rather than at its start.
    pub centred: bool,
    /// Between the message and the action, when there is an action.
    pub action: Option<f32>,
}

impl EmptyStateInputs {
    /// The band the block and the chrome clip share, in a `bounds` border box.
    pub fn band(&self, bounds: LayoutBox) -> LayoutBox {
        LayoutBox {
            x: bounds.x + self.inset.width,
            y: bounds.y + self.inset.height,
            width: (bounds.width - self.inset.width * 2.0).max(0.0),
            height: (bounds.height - self.inset.height * 2.0).max(0.0),
        }
    }

    fn column(&self, title: Size, message: Option<Size>) -> [Option<FlexPart>; 3] {
        let item = |kind, size: Size, gap_after| {
            let mut part = FlexPart::new(kind, size);
            part.align = if self.centred {
                AlignSpec::Center
            } else {
                AlignSpec::Start
            };
            part.gap_after = gap_after;
            part
        };
        [
            self.icon.map(|icon| {
                item(
                    LayoutPartKind::LeadingContent,
                    Size::new(icon, icon),
                    self.gap,
                )
            }),
            Some(item(
                LayoutPartKind::Header,
                title,
                if message.is_some() { self.gap } else { 0.0 },
            )),
            message.map(|message| item(LayoutPartKind::Detail, message, 0.0)),
        ]
    }

    /// The block's height: the column and the action's lead-in.
    pub fn block_height(&self, title: Size, message: Option<Size>) -> f32 {
        let mut height = title.height;
        if let Some(icon) = self.icon {
            height += icon + self.gap;
        }
        if let Some(message) = message {
            height += self.gap + message.height;
        }
        if let Some(lead) = self.action {
            height += self.gap + lead;
        }
        height
    }
}

/// An empty state's icon, title and message in the band of `bounds`: one
/// Flex column at the band's top, each item centred or at its start, the
/// title and message as wide as their text up to the band.
pub(crate) fn empty_state_parts(
    bounds: LayoutBox,
    inputs: EmptyStateInputs,
    title: Size,
    message: Option<Size>,
) -> ContentParts {
    let band = inputs.band(bounds);
    let fit = |size: Size| Size::new(size.width.clamp(0.0, band.width), size.height);
    // The block reserved its own height above the action, so the band never
    // cuts an item short: the column runs as long as its items (and a pixel
    // more, so rounding in the running sum cannot clip the last one).
    let column = inputs.column(fit(title), message.map(fit));
    let height = column
        .iter()
        .flatten()
        .map(|part| part.preferred.height.max(0.0) + part.gap_after.max(0.0))
        .sum::<f32>();
    content_parts(flex_parts(
        LayoutBox {
            height: height + 1.0,
            ..band
        },
        FlexDirection::Column,
        JustifySpec::Start,
        false,
        column,
    ))
}

/// What a row of `parts` holds beside its label: every glyph's width and
/// every gap, and the tallest glyph. Intrinsic measure adds the label's own
/// size to it.
pub(crate) fn chrome_extent(parts: &[Option<FlexPart>; 3]) -> Size {
    let glyphs = || {
        parts
            .iter()
            .flatten()
            .filter(|part| part.kind != LayoutPartKind::TextContent)
    };
    Size::new(
        parts
            .iter()
            .flatten()
            .map(|part| part.gap_after.max(0.0))
            .sum::<f32>()
            + glyphs()
                .map(|part| part.preferred.width.max(0.0))
                .sum::<f32>(),
        glyphs()
            .map(|part| part.preferred.height.max(0.0))
            .fold(0.0, f32::max),
    )
}

/// The paragraph budget excludes fixed chrome, never its own measured advance.
pub(crate) fn text_budget(mut parts: [Option<FlexPart>; 3], available: Size) -> f32 {
    fit_button_glyphs(&mut parts, available);
    (available.width - chrome_extent(&parts).width).max(0.0)
}

pub(crate) fn button_text_budget(visual: &StandardVisual, available: Size) -> Option<f32> {
    Some(text_budget(
        button_parts(visual, Size::default())?,
        available,
    ))
}

/// `inputs` placed as a start-aligned row of `bounds`, each glyph fitted
/// square into it.
pub(crate) fn start_row_parts(
    bounds: LayoutBox,
    mut inputs: [Option<FlexPart>; 3],
) -> ContentParts {
    fit_button_glyphs(&mut inputs, Size::new(bounds.width, bounds.height));
    content_parts(flex_parts(
        bounds,
        FlexDirection::Row,
        JustifySpec::Start,
        false,
        inputs,
    ))
}

fn fit_button_glyphs(inputs: &mut [Option<FlexPart>; 3], available: Size) {
    // A glyph keeps its aspect ratio even in a constrained content box.
    for part in inputs
        .iter_mut()
        .flatten()
        .filter(|part| part.kind != LayoutPartKind::TextContent)
    {
        let extent = part
            .preferred
            .width
            .min(available.width)
            .min(available.height)
            .max(0.0);
        part.preferred = Size::new(extent, extent);
    }
}

pub(crate) fn button_content_parts(
    visual: &StandardVisual,
    bounds: LayoutBox,
    text: Size,
) -> Option<ContentParts> {
    let StandardVisual::Button { content_align, .. } = visual else {
        return None;
    };
    let mut inputs = button_parts(visual, text)?;
    fit_button_glyphs(&mut inputs, Size::new(bounds.width, bounds.height));
    let justify = match content_align {
        TextHorizontalAlignment::Start => JustifySpec::Start,
        TextHorizontalAlignment::Center => JustifySpec::Center,
        TextHorizontalAlignment::End => JustifySpec::End,
    };
    Some(content_parts(flex_parts(
        bounds,
        FlexDirection::Row,
        justify,
        false,
        inputs,
    )))
}

pub(crate) fn part_bounds(parts: &ContentParts, kind: LayoutPartKind) -> Option<LayoutBox> {
    parts
        .iter()
        .flatten()
        .find(|part| part.kind == kind)
        .map(|part| part.bounds)
}

pub(crate) fn primary_text_box(parts: &ContentParts, fallback: LayoutBox) -> LayoutBox {
    part_bounds(parts, LayoutPartKind::TextContent)
        .or_else(|| part_bounds(parts, LayoutPartKind::Header))
        .unwrap_or(fallback)
}

pub(crate) fn card_content_parts(
    header: LayoutBox,
    has_title: bool,
    loading: bool,
    text: Size,
    reverse: bool,
) -> ContentParts {
    if !has_title && !loading {
        return NO_CONTENT_PARTS;
    }
    let glyph = crate::view_components::CARD_TITLE_SIZE
        .min(header.width)
        .min(header.height)
        .max(0.0);
    let gap = if loading {
        (crate::view_components::CARD_LOADING_RESERVE - crate::view_components::CARD_TITLE_SIZE)
            .min((header.width - glyph).max(0.0))
            .max(0.0)
    } else {
        0.0
    };
    let budget = (header.width - if loading { glyph + gap } else { 0.0 }).max(0.0);
    let mut ink = FlexPart::new(
        LayoutPartKind::TextContent,
        Size::new(text.width.min(budget), header.height),
    );
    ink.align = AlignSpec::Stretch;
    ink.gap_after = gap;
    let spinner = loading.then(|| FlexPart::new(LayoutPartKind::Spinner, Size::new(glyph, glyph)));
    let mut spacer = FlexPart::new(LayoutPartKind::ComponentContent, Size::default());
    spacer.grow = 1.0;
    let solved = flex_parts(
        header,
        FlexDirection::Row,
        JustifySpec::Start,
        reverse,
        [Some(ink), spinner, Some(spacer)],
    );
    let text = LayoutBox {
        x: if reverse {
            header.x + header.width - budget
        } else {
            header.x
        },
        width: budget,
        ..header
    };
    content_parts([
        has_title.then(|| LayoutPart::new(LayoutPartKind::Header, text)),
        solved[1],
    ])
}

pub(crate) struct RowTextMetrics {
    pub label_width: f32,
    pub note: Option<Size>,
    pub detail: Option<Size>,
    pub note_gap: f32,
    pub detail_gap: f32,
    pub min_secondary_width: f32,
}

pub(crate) fn paired_text_parts(
    bounds: LayoutBox,
    label: Size,
    value: Size,
    gap: f32,
    min_value_width: f32,
    reverse: bool,
) -> ContentParts {
    let value_width =
        priority_secondary_width(bounds.width, label.width, value.width, gap, min_value_width);
    let label_width = label.width.min((bounds.width - value_width - gap).max(0.0));
    let mut spacer = FlexPart::new(LayoutPartKind::ComponentContent, Size::default());
    spacer.grow = 1.0;
    spacer.gap_after = gap.min((bounds.width - value_width - label_width).max(0.0));
    let solved = flex_parts(
        bounds,
        FlexDirection::Row,
        JustifySpec::Start,
        reverse,
        [
            Some(FlexPart::new(
                LayoutPartKind::Label,
                Size::new(label_width, label.height),
            )),
            Some(spacer),
            Some(FlexPart::new(
                LayoutPartKind::Value,
                Size::new(value_width, value.height),
            )),
        ],
    );
    content_parts([solved[0], solved[2]])
}

fn priority_secondary_width(
    available: f32,
    primary: f32,
    secondary: f32,
    gap: f32,
    minimum: f32,
) -> f32 {
    if primary + gap + secondary <= available {
        secondary
    } else {
        (available - primary - gap).max(minimum).min(available)
    }
}

/// A row gives the label priority, reserves readable detail, then admits a
/// note if it fits. The common Flex solve owns allocation and placement.
pub(crate) fn list_text_parts(
    bounds: LayoutBox,
    metrics: RowTextMetrics,
    reverse: bool,
) -> ContentParts {
    let detail = metrics.detail.map(|size| {
        Size::new(
            priority_secondary_width(
                bounds.width,
                metrics.label_width,
                size.width,
                metrics.detail_gap,
                metrics.min_secondary_width,
            ),
            size.height,
        )
    });
    let detail_gap = detail.map_or(0.0, |size| {
        metrics.detail_gap.min((bounds.width - size.width).max(0.0))
    });
    let label_room = (bounds.width - detail.map_or(0.0, |size| size.width) - detail_gap).max(0.0);
    let note = metrics.note.filter(|size| {
        label_room - metrics.label_width.min(label_room) - metrics.note_gap
            >= size.width.min(metrics.min_secondary_width)
    });
    let mut label = FlexPart::new(
        LayoutPartKind::TextContent,
        Size::new(
            if note.is_some() {
                metrics.label_width.min(label_room)
            } else {
                0.0
            },
            bounds.height,
        ),
    );
    label.align = AlignSpec::Stretch;
    label.grow = if note.is_some() { 0.0 } else { 1.0 };
    label.gap_after = if note.is_some() {
        metrics.note_gap
    } else {
        detail_gap
    };
    let note = note.map(|size| {
        let mut part = FlexPart::new(LayoutPartKind::Note, size);
        part.shrink = 1.0;
        part
    });
    let spacer = note.map(|_| {
        let mut part = FlexPart::new(LayoutPartKind::ComponentContent, Size::default());
        part.grow = 1.0;
        part.gap_after = detail_gap;
        part
    });
    let solved = flex_parts(
        bounds,
        FlexDirection::Row,
        JustifySpec::Start,
        reverse,
        [
            Some(label),
            note,
            spacer,
            detail.map(|size| FlexPart::new(LayoutPartKind::Detail, size)),
        ],
    );
    content_parts([solved[0], solved[1], solved[3]])
}

pub(crate) fn menu_content_parts(
    bounds: LayoutBox,
    padding: f32,
    gap: f32,
    icon_size: Option<f32>,
    hint_width: Option<f32>,
) -> ContentParts {
    let bounds = crate::layout_result::inset(bounds, 0.0, padding, 0.0, padding);
    let mut label = FlexPart::new(LayoutPartKind::TextContent, Size::default());
    label.grow = 1.0;
    label.align = AlignSpec::Stretch;
    label.gap_after = if hint_width.is_some() { gap } else { 0.0 };
    content_parts(flex_parts(
        bounds,
        FlexDirection::Row,
        JustifySpec::Start,
        false,
        [
            icon_size.map(|size| {
                let mut part = FlexPart::new(LayoutPartKind::LeadingContent, Size::new(size, size));
                part.gap_after = gap;
                part
            }),
            Some(label),
            hint_width.map(|width| {
                let mut part = FlexPart::new(LayoutPartKind::Hint, Size::new(width, bounds.height));
                part.align = AlignSpec::Stretch;
                part
            }),
        ],
    ))
}

/// The label and support bands belong to the field's own visual surface.  They
/// are retained parts rather than synthetic child nodes; the application-owned
/// control remains an ordinary child placement in the same result.
pub(crate) fn form_field_parts(
    bounds: LayoutBox,
    size: nana_ui_core::ControlSize,
    support: Option<&str>,
    error: bool,
    support_measure: Option<f32>,
    reverse: bool,
) -> ContentParts {
    let label_height = crate::form_surfaces::form_field_label_line(size).min(bounds.height);
    let label = LayoutPart::new(
        LayoutPartKind::Label,
        LayoutBox {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: label_height,
        },
    );
    if support.is_none() && !error {
        return content_parts([Some(label)]);
    }
    let support_height = nana_ui_core::type_scale::META.min(bounds.height);
    let support_y = (bounds.y + bounds.height - support_height).max(bounds.y);
    let row = LayoutBox {
        x: bounds.x,
        y: support_y,
        width: bounds.width,
        height: support_height,
    };
    let indicator = error.then(|| {
        let slot = nana_ui_core::space::XL;
        let mut part = FlexPart::new(
            LayoutPartKind::LeadingContent,
            Size::new(slot, support_height),
        );
        part.align = AlignSpec::Stretch;
        part.gap_after = nana_ui_core::space::XS;
        part
    });
    let mut hint = FlexPart::new(
        LayoutPartKind::Hint,
        Size::new(support_measure.unwrap_or(0.0), support_height),
    );
    hint.align = AlignSpec::Stretch;
    hint.shrink = 1.0;
    let solved = flex_parts(
        row,
        FlexDirection::Row,
        JustifySpec::Start,
        reverse,
        [indicator, Some(hint), None],
    );
    content_parts([Some(label), solved[1], solved[0]])
}

/// Trigger text, optional hint and the arrow handle share one retained row.
/// The spacer is solved with the same Flex primitive but is not published as
/// a semantic part, so a select still has one node and one ordinary text run.
pub(crate) fn select_trigger_parts(
    bounds: LayoutBox,
    size: nana_ui_core::ControlSize,
    label: &str,
    hint: Option<&str>,
    placeholder: bool,
    measure: impl Fn(&str, f32) -> f32,
    natural_label: Option<f32>,
    reverse: bool,
) -> ContentParts {
    let handle_width = crate::select::HANDLE_WIDTH.min(bounds.width);
    let label_area = (bounds.width - handle_width).max(0.0);
    let label_natural = natural_label.unwrap_or_else(|| measure(label, size.text_size()));
    let hint_size = size.caption_size();
    let hint_natural = hint
        .filter(|hint| !placeholder && !hint.is_empty())
        .map(|hint| measure(hint, hint_size));
    let hint_fits = hint_natural.is_some_and(|hint| {
        let room = (label_area - label_natural.min(label_area) - crate::select::HINT_GAP).max(0.0);
        room >= hint.min(nana_ui_core::space::XXXL)
    });
    let label_width = if hint_fits {
        label_natural.min(label_area)
    } else {
        label_area
    };
    let mut label_part = FlexPart::new(
        LayoutPartKind::TextContent,
        Size::new(label_width, bounds.height),
    );
    label_part.align = AlignSpec::Stretch;
    label_part.grow = if !hint_fits { 1.0 } else { 0.0 };
    label_part.gap_after = if hint_fits {
        crate::select::HINT_GAP
    } else {
        0.0
    };
    let hint_part = hint_fits.then(|| {
        let mut part = FlexPart::new(
            LayoutPartKind::Hint,
            Size::new(hint_natural.unwrap_or(0.0), bounds.height),
        );
        part.align = AlignSpec::Stretch;
        part.shrink = 1.0;
        part
    });
    let mut spacer = FlexPart::new(LayoutPartKind::ComponentContent, Size::default());
    spacer.grow = if hint_fits { 1.0 } else { 0.0 };
    let handle = FlexPart::new(
        LayoutPartKind::TrailingContent,
        Size::new(handle_width, bounds.height),
    );
    let solved = flex_parts(
        bounds,
        FlexDirection::Row,
        JustifySpec::Start,
        reverse,
        [Some(label_part), hint_part, Some(spacer), Some(handle)],
    );
    content_parts([solved[0], solved[1], solved[3]])
}

/// Splits an option row's already-reserved text lane into label and hint
/// boxes. The caller owns colors and text payloads; this helper owns fit and
/// end-edge arithmetic for every menu surface.
pub(crate) fn menu_option_text_boxes(
    mut label: LayoutBox,
    label_text: &str,
    hint: Option<&str>,
    label_size: f32,
    hint_size: f32,
    gap: f32,
    minimum: f32,
    measure: impl Fn(&str, f32) -> f32,
) -> (LayoutBox, Option<LayoutBox>) {
    let Some(hint) = hint.filter(|hint| !hint.is_empty()) else {
        return (label, None);
    };
    let label_natural = measure(label_text, label_size);
    let hint_natural = measure(hint, hint_size);
    let room = label.width - label_natural.min(label.width) - gap;
    if room < hint_natural.min(minimum) {
        return (label, None);
    }
    let width = hint_natural.min(room);
    let right = label.x + label.width;
    label.width = (label.width - width - gap).max(0.0);
    (
        label,
        Some(LayoutBox {
            x: right - width,
            y: label.y,
            width,
            height: label.height,
        }),
    )
}
