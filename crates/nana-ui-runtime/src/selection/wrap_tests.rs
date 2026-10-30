use super::*;
use crate::{
    AppContext, DocumentId, Entity, LayoutBox, LayoutViewport, TextMetrics, TextShapeConstraints,
    TextShaper,
};

struct Shaper;
impl TextShaper for Shaper {
    fn shape(
        &mut self,
        _: StableNodeId,
        text: &TextContent,
        _: &crate::ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let natural = text.value.chars().count() as f32 * 7.0;
        let width = if constraints.wrap {
            constraints.max_width.unwrap_or(natural).max(7.0)
        } else {
            natural
        };
        TextMetrics {
            width: natural.min(width),
            height: 18.0 * (natural / width.max(7.0)).ceil().max(1.0),
            ascent: Some(13.0),
        }
    }
}

fn layout(cx: &mut AppContext, doc: DocumentId, width: f32) {
    for _ in 0..12 {
        cx.shape_text_for_layout(doc, &mut Shaper).unwrap();
        cx.layout_document_with_frontier(doc, LayoutViewport::new(width, 900.0), &[])
            .unwrap();
        if !cx.shape_text_for_layout(doc, &mut Shaper).unwrap() {
            return;
        }
    }
    panic!("segmented layout did not settle at {width}");
}

fn bounds(cx: &AppContext, option: Entity<SegmentedOption>) -> LayoutBox {
    cx.world().layout_box(option.stable_id()).unwrap()
}

fn assert_equal_lines(
    cx: &AppContext,
    control: Entity<SegmentedControl>,
    options: &[Entity<SegmentedOption>],
) {
    let track = cx.world().layout_box(control.stable_id()).unwrap();
    let mut start = 0;
    while start < options.len() {
        let first = bounds(cx, options[start]);
        let mut end = start + 1;
        while end < options.len() && (bounds(cx, options[end]).y - first.y).abs() < 0.5 {
            end += 1;
        }
        let last = bounds(cx, options[end - 1]);
        let inset = segmented_pill_inset(cx.world());
        assert!((first.x - track.x - inset).abs() < 0.5);
        assert!((last.x + last.width - track.x - track.width + inset).abs() < 0.5);
        for option in &options[start..end] {
            let rect = bounds(cx, *option);
            assert!(
                (rect.width - first.width).abs() < 0.5,
                "{rect:?} != {first:?}"
            );
            assert!(rect.y >= track.y && rect.y + rect.height <= track.y + track.height + 0.5);
            if let Some(crate::ComponentGeometry::SelectionOption { label, .. }) =
                cx.world().component_geometry(option.stable_id())
            {
                let text = cx.world().text_metrics(option.stable_id()).unwrap();
                assert!(
                    text.width <= label.bounds.width + 0.5,
                    "label clipped: {label:?}, {text:?}"
                );
                assert!(
                    text.height <= label.bounds.height + 0.5,
                    "label height clipped"
                );
            } else {
                panic!("missing option geometry");
            }
        }
        start = end;
    }
}

fn fixture(
    labels: &[&str],
    fill: bool,
) -> (
    AppContext,
    DocumentId,
    Entity<SegmentedControl>,
    Vec<Entity<SegmentedOption>>,
) {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let control = cx
        .create_component(
            doc,
            SegmentedControl::new()
                .size(ControlSize::Small)
                .fill(fill)
                .wrap(true),
        )
        .unwrap();
    let options: Vec<_> = labels
        .iter()
        .map(|label| {
            cx.create_detached_component(doc, SegmentedOption::new(*label))
                .unwrap()
        })
        .collect();
    cx.set_segmented_options(control, options.clone(), options.first().copied())
        .unwrap();
    (cx, doc, control, options)
}

#[test]
fn segmented_equal_wrap_fills_every_line_and_resizes_without_changing_selection_or_focus() {
    for labels in [
        vec!["未压缩", "GPU 压缩"],
        vec!["关闭", "SSAA 2×", "SSAA 4×"],
        vec!["面捕", "唇形同步", "鼠标", "手柄", "VBridger", "iPhone"],
    ] {
        let (mut cx, doc, control, options) = fixture(&labels, true);
        cx.focus_node(doc, options[0].stable_id()).unwrap();
        for width in [640.0, 280.0, 180.0, 400.0, 320.0, 640.0, 280.0] {
            layout(&mut cx, doc, width);
            assert_equal_lines(&cx, control, &options);
            assert_eq!(
                cx.read(control, SegmentedControl::selected).unwrap(),
                Some(options[0].stable_id())
            );
            assert_eq!(cx.world().focused(doc), Some(options[0].stable_id()));
        }
        layout(&mut cx, doc, 640.0);
        assert!(
            options
                .iter()
                .all(|o| (bounds(&cx, *o).y - bounds(&cx, options[0]).y).abs() < 0.5)
        );
    }
}

#[test]
fn segmented_equal_wrap_uses_widest_label_and_wraps_at_the_exact_threshold() {
    let (mut cx, doc, control, options) = fixture(&["A", "VeryLongLabel", "B"], true);
    let inset =
        ControlSize::Small.padding_x_in(cx.world().theme_metrics()) + nana_ui_core::space::XXS;
    let threshold = (13.0 * 7.0 + inset * 2.0) * 3.0
        + SEGMENTED_TRACK_PADDING * 2.0
        + SEGMENTED_TRACK_BORDER * 2.0
        + nana_ui_core::space::XXS * 2.0;
    layout(&mut cx, doc, threshold);
    assert_equal_lines(&cx, control, &options);
    assert_eq!(bounds(&cx, options[0]).y, bounds(&cx, options[2]).y);
    layout(&mut cx, doc, threshold - 1.0);
    assert_equal_lines(&cx, control, &options);
    assert!(bounds(&cx, options[2]).y > bounds(&cx, options[0]).y);
}

#[test]
fn segmented_equal_wrap_tracks_icons_hidden_disabled_labels_and_dynamic_options() {
    let (mut cx, doc, control, options) = fixture(&["A", "Code", "Disabled"], true);
    cx.update_component(options[1], |option, _| option.icon = Some(Icon::File))
        .unwrap();
    cx.set_segmented_option_disabled(control, options[2], true)
        .unwrap();
    layout(&mut cx, doc, 220.0);
    assert_equal_lines(&cx, control, &options);
    cx.update_component(options[0], |option, _| {
        Arc::make_mut(&mut option.style.layout).hidden = true;
    })
    .unwrap();
    layout(&mut cx, doc, 220.0);
    assert_equal_lines(&cx, control, &options[1..]);
    cx.update_component(options[1], |option, _| {
        option.label = "A much longer translated option".into()
    })
    .unwrap();
    layout(&mut cx, doc, 220.0);
    assert_equal_lines(&cx, control, &options[1..]);
    let added = cx
        .create_detached_component(doc, SegmentedOption::new("New"))
        .unwrap();
    cx.set_segmented_options(control, vec![options[1], added], Some(options[1]))
        .unwrap();
    layout(&mut cx, doc, 400.0);
    assert_equal_lines(&cx, control, &[options[1], added]);
    cx.set_segmented_options(control, Vec::new(), None).unwrap();
    layout(&mut cx, doc, 0.0);
}

#[test]
fn segmented_wrap_grows_for_a_single_label_wider_than_the_container() {
    for fill in [false, true] {
        let (mut cx, doc, control, options) = fixture(
            &["A very long translated label that cannot fit on one line"],
            fill,
        );
        layout(&mut cx, doc, 130.0);
        assert!(
            bounds(&cx, options[0]).height
                > ControlSize::Small.height_in(cx.world().theme_metrics())
        );
        if fill {
            assert_equal_lines(&cx, control, &options);
        }
        layout(&mut cx, doc, 640.0);
        assert!(
            bounds(&cx, options[0]).height
                < ControlSize::Small.height_in(cx.world().theme_metrics())
        );
    }
}

#[test]
fn segmented_content_wrap_keeps_intrinsic_widths_and_wrap_false_stays_unwrapped() {
    let (mut cx, doc, _, options) = fixture(&["A", "A longer label", "B"], false);
    layout(&mut cx, doc, 150.0);
    assert!(bounds(&cx, options[1]).width > bounds(&cx, options[0]).width);
    assert!(bounds(&cx, options[2]).y > bounds(&cx, options[0]).y);
    let plain = SegmentedControl::new().fill(true).wrap(false);
    assert!(!plain.wrap_value());
    assert!(!plain.style.layout.equal_wrap);
    assert_eq!(plain.style.layout.flex_wrap, nana_ui_core::FlexWrap::NoWrap);
    assert!(
        !SegmentedControl::radio_group()
            .fill(true)
            .wrap(true)
            .style
            .layout
            .equal_wrap
    );
}

/// The default wraps only when it must: with room it keeps one line at its
/// own width and its one-line height, in a narrow box it wraps instead of
/// overflowing.
#[test]
fn a_default_segmented_control_keeps_one_line_with_room_and_wraps_without() {
    assert!(SegmentedControl::new().wrap_value());
    let (mut cx, doc, control, options) = fixture(&["Light", "Dark", "Follow system"], false);
    let track = |cx: &AppContext| cx.world().layout_box(control.stable_id()).unwrap();
    layout(&mut cx, doc, 640.0);
    assert!(track(&cx).width < 640.0, "keeps its own width with room");
    assert_eq!(bounds(&cx, options[2]).y, bounds(&cx, options[0]).y);
    assert!(
        (track(&cx).height - ControlSize::Small.height_in(cx.world().theme_metrics())).abs() < 0.5,
        "one line is one control tall"
    );
    layout(&mut cx, doc, 120.0);
    assert!(
        track(&cx).width <= 120.5,
        "stays inside the width it is given"
    );
    assert!(bounds(&cx, options[2]).y > bounds(&cx, options[0]).y);
}
