//! Issue #382: the precision left after the baseline policy of #202.
//!
//! - Gate A: a box with a declared `aspect-ratio` and no content of its own
//!   takes its height from the width min/max and both insets leave it (CSS
//!   Sizing 4 §5), the same as a flex item, a grid item, an atomic inline
//!   box and an absolutely positioned box. The ratio relates the boxes
//!   `box-sizing` names, and the height it gives meets the box's own
//!   `min-height` / `max-height`. An in-flow container's auto height reads
//!   that height too (#393). Each matches a cold layout.
#![cfg(test)]

use nana_ui_core::{
    BoxSizing, DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, PositionSpec,
};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, StableNodeId};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

#[derive(Debug, Clone, Copy)]
enum Context {
    Flex,
    Grid,
    Inline,
    Absolute,
    /// Absolute, its width from both horizontal insets.
    AbsoluteInsets,
}

impl Context {
    fn container(self) -> LayoutStyle {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(600.0)),
            ..LayoutStyle::default()
        };
        match self {
            Self::Flex => {
                layout.display = Some(DisplaySpec::Flex);
                layout.direction = Some(FlexDirection::Row);
            }
            Self::Grid => {
                layout.display = Some(DisplaySpec::Grid);
                layout.grid_columns = Some(vec![GridTrack::Auto]);
            }
            Self::Inline => layout.display = Some(DisplaySpec::Block),
            Self::Absolute | Self::AbsoluteInsets => {
                layout.display = Some(DisplaySpec::Block);
                layout.position = PositionSpec::Relative;
                layout.height = Some(LengthSpec::Px(300.0));
            }
        }
        layout
    }

    fn item(self, mut layout: LayoutStyle) -> LayoutStyle {
        match self {
            Self::Flex | Self::Grid => {}
            Self::Inline => layout.display = Some(DisplaySpec::InlineBlock),
            Self::Absolute => {
                layout.position = PositionSpec::Absolute;
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
            }
            Self::AbsoluteInsets => {
                layout.position = PositionSpec::Absolute;
                layout.offset_top = Some(LengthSpec::Px(0.0));
                layout.offset_left = Some(LengthSpec::Px(0.0));
                // 600 wide: the insets leave the declared width, which a
                // content box declares inside its padding.
                let px = |spec: Option<LengthSpec>| {
                    spec.map_or(0.0, |spec| match spec {
                        LengthSpec::Px(px) => px,
                        other => panic!("{other:?}"),
                    })
                };
                let mut width = px(layout.width.take());
                if layout.box_sizing == BoxSizing::ContentBox {
                    width += 2.0 * px(layout.padding);
                }
                layout.offset_right = Some(LengthSpec::Px(600.0 - width));
            }
        }
        layout
    }
}

fn ratio_box(edit: fn(&mut LayoutStyle)) -> LayoutStyle {
    let mut layout = LayoutStyle {
        width: Some(LengthSpec::Px(120.0)),
        aspect_ratio: Some(1.5),
        ..LayoutStyle::default()
    };
    edit(&mut layout);
    layout
}

/// One box in `context`, laid out once. Returns the box.
fn page(context: Context, layout: LayoutStyle) -> (AppContext, DocumentId, StableNodeId) {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 800.0);
    let container = b.element(page, context.container());
    let id = b.element(container, context.item(layout));
    let mut app = AppContext::new();
    app.commit_mutations(b.queue).unwrap();
    product_frame(&mut app, document, viewport(), &mut bundled_face_shaper());
    (app, document, id)
}

/// Gate A.
#[test]
fn issue382_a_declared_ratio_follows_the_clamped_width_everywhere() {
    type Edit = fn(&mut LayoutStyle);
    let cases: [(&str, Edit, (f32, f32)); 7] = [
        ("width", |_| {}, (120.0, 80.0)),
        (
            "max-width",
            |l| l.max_width = Some(LengthSpec::Px(60.0)),
            (60.0, 40.0),
        ),
        (
            "min-width",
            |l| l.min_width = Some(LengthSpec::Px(240.0)),
            (240.0, 160.0),
        ),
        // Border boxes: 60 wide, 40 tall, the padding inside both.
        (
            "border-box padding",
            |l| {
                l.max_width = Some(LengthSpec::Px(60.0));
                l.padding = Some(LengthSpec::Px(6.0));
            },
            (60.0, 40.0),
        ),
        // Content boxes: 120×80 inside, the padding outside both.
        (
            "content-box padding",
            |l| {
                l.box_sizing = BoxSizing::ContentBox;
                l.padding = Some(LengthSpec::Px(6.0));
            },
            (132.0, 92.0),
        ),
        (
            "min-height",
            |l| {
                l.max_width = Some(LengthSpec::Px(60.0));
                l.min_height = Some(LengthSpec::Px(50.0));
            },
            (60.0, 50.0),
        ),
        (
            "max-height",
            |l| l.max_height = Some(LengthSpec::Px(30.0)),
            (120.0, 30.0),
        ),
    ];
    for context in [
        Context::Flex,
        Context::Grid,
        Context::Inline,
        Context::Absolute,
        Context::AbsoluteInsets,
    ] {
        // What an in-flow container adds below its one item: nothing, or on
        // a line the strut below the inline-block's baseline.
        let mut line_slack = None;
        for (label, edit, expected) in cases {
            // Warm: a plain box first, then the case as an edit.
            let (mut warm, document, id) = page(context, ratio_box(|l| l.aspect_ratio = None));
            let mut queue = MutationQueue::new();
            queue.set_style(id, styled(context.item(ratio_box(edit))));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut bundled_face_shaper());
            let bounds = warm.world().layout_box(id).unwrap();
            assert_eq!(
                (bounds.width, bounds.height),
                expected,
                "{context:?} {label}"
            );
            // The container's auto height reads the item's measured height,
            // which the ratio fills from the limited width too.
            if !matches!(context, Context::Absolute | Context::AbsoluteInsets) {
                let container = warm.world().parent_id(id).unwrap();
                let slack = warm.world().layout_box(container).unwrap().height - bounds.height;
                if !matches!(context, Context::Inline) {
                    assert_eq!(slack, 0.0, "{context:?} {label} container");
                }
                let first = *line_slack.get_or_insert(slack);
                assert!(
                    (slack - first).abs() < 0.01,
                    "{context:?} {label} container: {slack} below, {first} in the first case"
                );
            }
            let (mut cold, _, _) = page(context, ratio_box(edit));
            assert_matches_cold(&mut warm, &mut cold, document);
        }
    }
}
