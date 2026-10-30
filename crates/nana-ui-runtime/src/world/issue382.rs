//! Issue #382: the precision left after the baseline policy of #202.
//!
//! - Gate A: a box with a declared `aspect-ratio` and no content of its own
//!   takes its height from the width min/max and both insets leave it (CSS
//!   Sizing 4 §5), the same as a flex item, a grid item, an atomic inline
//!   box and an absolutely positioned box. The ratio relates the boxes
//!   `box-sizing` names, and the height it gives meets the box's own
//!   `min-height` / `max-height`. An in-flow container's auto height reads
//!   that height too (#393). Each matches a cold layout.
//! - Gate C: a display change republishes how the box takes part in its
//!   parent's context, though its box stays put; children turned
//!   inline-level again leave the flow plan their block recorded while they
//!   were block-level, and lie on a line again.
#![cfg(test)]

use nana_ui_core::{
    BoxSizing, DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, PositionSpec,
};

use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use crate::{
    AppContext, DocumentId, LayoutViewport, MutationQueue, NanaTextEngineShaper, StableNodeId,
};

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

/// A shaper with the product UI face as `sans-serif`, and `Wide` in two
/// faces of other metrics: Noto Sans Arabic up to weight 500, Noto Emoji
/// above. With `late`, `Late` is Noto Sans Arabic as well.
fn strut_shaper(late: bool) -> NanaTextEngineShaper {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_file(ui_face(), &FaceDescriptor::default())
        .expect("the bundled UI face registers");
    for (file, weight) in [
        ("noto-sans-arabic.ttf", (100.0, 500.0)),
        ("noto-emoji.ttf", (600.0, 900.0)),
    ] {
        fonts
            .register_file(
                corpus_face(file),
                &FaceDescriptor {
                    weight: Some(weight),
                    ..FaceDescriptor::family("Wide")
                },
            )
            .expect("a corpus face registers");
    }
    let shaper = NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))));
    if late {
        register_late(&shaper);
    }
    shaper
}

fn ui_face() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
    )
    .into()
}

fn corpus_face(file: &str) -> String {
    format!("{}/../nana-text/fonts/{file}", env!("CARGO_MANIFEST_DIR"))
}

/// Registers `Late`: a new font set, as a host's `@font-face` arriving.
fn register_late(shaper: &NanaTextEngineShaper) {
    nana_text::lock_text_engine(shaper.engine())
        .fonts_mut()
        .register_file(
            corpus_face("noto-sans-arabic.ttf"),
            &FaceDescriptor::family("Late"),
        )
        .expect("a corpus face registers");
}

/// What the ordinary block in gate B is written in.
#[derive(Debug, Clone, Copy)]
struct Face {
    family: Option<&'static str>,
    weight: u16,
}

const FONT: f32 = 20.0;

/// A page of an ordinary block (20px, `line-height: 20px`) holding a
/// 40×30 and a 10×4 inline-block, a sibling block of two inline-blocks in
/// the default font, and a flex row. `block_atoms` is the first atom's
/// display. Returns the block, its first atom, the sibling and the row.
fn strut_page(
    face: Face,
    first_atom: DisplaySpec,
    late: bool,
) -> (AppContext, NanaTextEngineShaper, [StableNodeId; 4]) {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 800.0);
    let block = b.element(page, strut_block(face));
    let atom = b.element(block, strut_atom(first_atom, 40.0, 30.0));
    b.element(block, strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0));
    let sibling = b.element(
        page,
        LayoutStyle {
            width: Some(LengthSpec::Px(600.0)),
            ..LayoutStyle::default()
        },
    );
    b.element(sibling, strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0));
    b.element(sibling, strut_atom(DisplaySpec::InlineBlock, 20.0, 10.0));
    let row = b.element(
        page,
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            width: Some(LengthSpec::Px(200.0)),
            ..LayoutStyle::default()
        },
    );
    b.label(row, "Row");
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let mut shaper = strut_shaper(late);
    product_frame(&mut context, document, viewport(), &mut shaper);
    (context, shaper, [block, atom, sibling, row])
}

fn strut_block(face: Face) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Block),
        width: Some(LengthSpec::Px(600.0)),
        font_size: Some(FONT),
        line_height: Some(nana_ui_core::LineHeightSpec::Absolute(FONT)),
        font_family: face.family.map(str::to_owned),
        font_weight: Some(face.weight),
        ..LayoutStyle::default()
    }
}

fn strut_atom(display: DisplaySpec, width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        display: Some(display),
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    }
}

/// Gate C: an atom of an ordinary block turned block-level, and back,
/// keeps its box where its line put it, and is published as what it takes
/// part as: a flow item, then an atomic inline box. Found by gate B, whose
/// cold check reads the participation; it shows on this page and font set.
#[test]
fn issue382_c_a_display_change_republishes_the_participation() {
    let sans = Face {
        family: None,
        weight: 400,
    };
    let document = DocumentId::new(1).unwrap();
    let (mut warm, mut shaper, [_, atom, _, _]) = strut_page(sans, DisplaySpec::InlineBlock, false);
    for (display, part) in [
        (DisplaySpec::Block, crate::ParticipationKind::FlowItem),
        (
            DisplaySpec::InlineBlock,
            crate::ParticipationKind::AtomicInline,
        ),
    ] {
        let mut queue = MutationQueue::new();
        queue.set_style(atom, styled(strut_atom(display, 40.0, 30.0)));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        let published = warm.world().layout_result(atom).unwrap().participation;
        assert_eq!(published, Some(part), "{display:?}");
        let (mut cold, _, _) = strut_page(sans, display, false);
        assert_matches_cold(&mut warm, &mut cold, document);
    }
}

/// Gate C, second part: every child of an ordinary block turned
/// block-level and back in one commit. Block-level, the block stacks them
/// and records a placement plan; inline-level again, it lays them on a line,
/// which the plan does not model, and places them afresh.
#[test]
fn issue382_c_children_turned_inline_level_again_leave_the_flow_plan() {
    let child = |display, width, height| LayoutStyle {
        display: Some(display),
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    };
    let build = |display| {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let block = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            },
        );
        let ids = [
            b.element(block, child(display, 40.0, 30.0)),
            b.element(block, child(display, 10.0, 4.0)),
        ];
        let mut app = AppContext::new();
        app.commit_mutations(b.queue).unwrap();
        product_frame(&mut app, document, viewport(), &mut bundled_face_shaper());
        (app, document, ids)
    };
    let (mut warm, document, [tall, short]) = build(DisplaySpec::InlineBlock);
    for display in [DisplaySpec::Block, DisplaySpec::InlineBlock] {
        let mut queue = MutationQueue::new();
        queue.set_style(tall, styled(child(display, 40.0, 30.0)));
        queue.set_style(short, styled(child(display, 10.0, 4.0)));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut bundled_face_shaper());
        let (mut cold, _, _) = build(display);
        assert_matches_cold(&mut warm, &mut cold, document);
    }
    // On one line again: the short one beside the tall one.
    let (tall, short) = (
        warm.world().layout_box(tall).unwrap(),
        warm.world().layout_box(short).unwrap(),
    );
    assert_eq!(short.x, tall.x + tall.width, "{short:?} beside {tall:?}");
}
