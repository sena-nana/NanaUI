//! Issue #382: the precision left after the baseline policy of #202.
//!
//! - Gate A: a box with a declared `aspect-ratio` and no content of its own
//!   takes its height from the width min/max and both insets leave it (CSS
//!   Sizing 4 §5), the same as a flex item, a grid item, an atomic inline
//!   box and an absolutely positioned box. The ratio relates the boxes
//!   `box-sizing` names, and the height it gives meets the box's own
//!   `min-height` / `max-height`. An in-flow container's auto height reads
//!   that height too (#393). Each matches a cold layout.
//! - Gate B: an ordinary block's line boxes start from its own face's strut,
//!   the ascent and descent `nana-text` reads for a native paragraph in the
//!   same face, so the two leave the same room below an empty atom. A new
//!   family, weight or font set moves the strut and the lines with it; a
//!   language, a font size and changes beside the block do not lay it out
//!   for its strut; an atom turned block-level keeps the strut while one
//!   inline-level child remains and drops it after. Each step matches a cold
//!   layout, and every pass checks every retained strut against a fresh
//!   read (`UiWorld::audit_block_struts`).
//! - Gate C: a display change republishes how the box takes part in its
//!   parent's context, though its box stays put; children turned
//!   inline-level again leave the flow plan their block recorded while they
//!   were block-level, and lie on a line again.
//! - Gate D: an item a flex row aligns by its first baseline, whose line
//!   boxes are its descendants', sits by its first in-flow line box: below
//!   a spacer in a fixed box, two blocks down, the first of two lines. A new
//!   font size and a taller spacer inside the fixed box align the row again;
//!   text that leaves the first line where it was, and text outside the row,
//!   move nothing. Each step matches a cold layout.
//! - Gate E: a block flow not packed to its start (`justify-content`
//!   center, end, space-between, space-evenly; a reversed flow) sits by
//!   where its first and last line boxes are, on a flex row and on a line.
//!   A new justification aligns them again; each step matches a cold layout.
//! - Gate F: a box whose lines run beside a float sits by where those lines
//!   are, packed in the room the float leaves them: its first line below
//!   the float on a flex row, its last wrapped below it on a line.
#![cfg(test)]

use nana_ui_core::{
    BoxSizing, DisplaySpec, FlexDirection, GridTrack, JustifySpec, LayoutStyle, LengthSpec,
    PositionSpec,
};

use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily};

use super::NodeKind;
use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, measured, product_frame,
    product_frame_seeded, styled,
};
use crate::{
    AppContext, ComputedStyle, DocumentId, FontStrut, InlineContent, InlineFlow, LayoutViewport,
    MutationQueue, NanaTextEngineShaper, StableNodeId, TextShaper,
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

/// The face's own ascent and descent at `font` px, read the way the text
/// pass reads them.
fn face_strut(
    shaper: &mut NanaTextEngineShaper,
    family: Option<&str>,
    weight: u16,
    font: f32,
) -> FontStrut {
    let style = ComputedStyle {
        font_family: family.map(Arc::from),
        font_weight: Some(weight),
        font_size: font,
        ..ComputedStyle::default()
    };
    shaper
        .strut_metrics(&style)
        .expect("the face reports its metrics")
}

/// The block's line in `face`: the tall atom's bottom is the baseline, the
/// strut's descent below it, so the block is 30 plus that descent tall
/// (the line height is the font size, so the half-leading is negative).
fn assert_line_in(context: &AppContext, block: StableNodeId, atom: StableNodeId, strut: FontStrut) {
    let world = context.world();
    let (line, tall) = (
        world.layout_box(block).unwrap(),
        world.layout_box(atom).unwrap(),
    );
    let half_leading = (FONT - (strut.ascent + strut.descent)) / 2.0;
    let below = strut.descent + half_leading;
    assert!(
        (line.height - (tall.height + below)).abs() < 0.01,
        "{line:?} with {tall:?}: {below} below the baseline"
    );
}

/// Gate B, first part: an ordinary block's line and a native paragraph's in
/// the same font leave the same room below an empty atom, the face's own
/// descent and half-leading, not the approximation's.
#[test]
fn issue382_b_a_block_line_and_a_paragraph_line_share_one_strut() {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 800.0);
    let block = b.element(
        page,
        LayoutStyle {
            width: Some(LengthSpec::Px(600.0)),
            ..LayoutStyle::default()
        },
    );
    let atom = b.element(block, strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0));
    let held = b.detached(strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0));
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let paragraph = context
        .create_component(
            document,
            InlineFlow::new(InlineContent::builder().child(held).build())
                .width(LengthSpec::Px(600.0)),
        )
        .unwrap()
        .stable_id();
    let mut shaper = strut_shaper(false);
    product_frame(&mut context, document, viewport(), &mut shaper);
    // The paragraph sets the theme's body text; the block takes the same
    // size on the same line height.
    let text = context.world().computed_style(paragraph).cloned().unwrap();
    let (font, line_height) = (
        text.font_size,
        nana_ui_core::text_line_box_height_px(text.font_size, text.line_height),
    );
    let mut queue = MutationQueue::new();
    queue.set_style(
        block,
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(600.0)),
            font_size: Some(font),
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(line_height)),
            ..LayoutStyle::default()
        }),
    );
    context.commit_mutations(queue).unwrap();
    product_frame(&mut context, document, viewport(), &mut shaper);
    let world = context.world();
    let gap = |line: StableNodeId, atom: StableNodeId| {
        let (line, atom) = (
            world.layout_box(line).unwrap(),
            world.layout_box(atom).unwrap(),
        );
        line.y + line.height - (atom.y + atom.height)
    };
    let (block_gap, paragraph_gap) = (gap(block, atom), gap(paragraph, held));
    assert!(
        (block_gap - paragraph_gap).abs() < 0.01,
        "block {block_gap} vs paragraph {paragraph_gap}"
    );
    // The face's descent and half-leading, not the approximation's.
    let strut = face_strut(&mut shaper, None, 400, font);
    let expected = strut.descent + (line_height - (strut.ascent + strut.descent)) / 2.0;
    assert!(
        (block_gap - expected).abs() < 0.01,
        "{block_gap} vs {expected}"
    );
    let approximation =
        font * (1.0 - nana_ui_core::TEXT_APPROX_ASCENT_EM) + (line_height - font) / 2.0;
    assert!(
        (block_gap - approximation).abs() > 1.0,
        "{block_gap} is the approximation"
    );
}

/// Gate B, second part: each input of the strut moves it, or leaves it,
/// the way a cold layout of the same tree does.
#[test]
fn issue382_b_the_strut_follows_the_blocks_face() {
    let sans = Face {
        family: None,
        weight: 400,
    };
    let wide = Face {
        family: Some("Wide"),
        weight: 400,
    };
    let wide_bold = Face {
        family: Some("Wide"),
        weight: 700,
    };
    let late = Face {
        family: Some("Late, sans-serif"),
        weight: 400,
    };
    let document = DocumentId::new(1).unwrap();
    let (mut warm, mut shaper, [block, atom, sibling, row]) =
        strut_page(sans, DisplaySpec::InlineBlock, false);
    let mut probe = strut_shaper(true);
    assert_line_in(&warm, block, atom, face_strut(&mut probe, None, 400, FONT));
    let check = |warm: &mut AppContext, face: Face, display: DisplaySpec, late: bool| {
        let (mut cold, _, _) = strut_page(face, display, late);
        assert_matches_cold(warm, &mut cold, document);
    };

    // Another family, then another weight of it: other faces, other struts.
    for face in [wide, wide_bold] {
        let mut queue = MutationQueue::new();
        queue.set_style(block, styled(strut_block(face)));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_line_in(
            &warm,
            block,
            atom,
            face_strut(&mut probe, face.family, face.weight, FONT),
        );
        check(&mut warm, face, DisplaySpec::InlineBlock, false);
    }
    let held = warm.world().block_strut(block);

    // A language picks no other primary face: the strut and every box stay.
    let before = warm.world().layout_box(block);
    let mut queue = MutationQueue::new();
    queue.set_language(block, nana_text::font::LanguageTag::new("ja"));
    warm.commit_mutations(queue).unwrap();
    let (_, seeded) = product_frame_seeded(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(warm.world().block_strut(block), held);
    assert_eq!(warm.world().layout_box(block), before);
    assert!(!seeded.contains(&block), "{seeded:?}");
    let mut queue = MutationQueue::new();
    queue.set_language(block, None);
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);

    // A family the font set lacks falls back; registering it is a new font
    // set, and the block's lines start from it.
    let mut queue = MutationQueue::new();
    queue.set_style(block, styled(strut_block(late)));
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_line_in(
        &warm,
        block,
        atom,
        face_strut(&mut strut_shaper(false), None, 400, FONT),
    );
    register_late(&shaper);
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_line_in(
        &warm,
        block,
        atom,
        face_strut(&mut probe, Some("Late"), 400, FONT),
    );
    check(&mut warm, late, DisplaySpec::InlineBlock, true);

    // An atom turned block-level leaves the block no inline-level child of
    // its own size: the line it shares with the other is still a line.
    // Both atoms block-level: no lines, no strut.
    for display in [DisplaySpec::Block, DisplaySpec::InlineBlock] {
        let mut queue = MutationQueue::new();
        queue.set_style(atom, styled(strut_atom(display, 40.0, 30.0)));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert!(warm.world().block_strut(block).is_some());
        check(&mut warm, late, display, true);
    }
    let short = warm.world().node(block).unwrap().children[1];
    let mut queue = MutationQueue::new();
    for id in [atom, short] {
        queue.set_style(id, styled(strut_atom(DisplaySpec::Block, 40.0, 30.0)));
    }
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(warm.world().block_strut(block), None);
    let mut queue = MutationQueue::new();
    queue.set_style(
        short,
        styled(strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0)),
    );
    queue.set_style(
        atom,
        styled(strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0)),
    );
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    check(&mut warm, late, DisplaySpec::InlineBlock, true);

    // Changes that do not touch a strut lay no block out for one: the
    // flex row's label, and a font size (the strut is per px of it).
    let mut queue = MutationQueue::new();
    let label = warm.world().node(row).unwrap().children[0];
    queue.set_text(label, crate::TextContent::new("A longer row label"));
    warm.commit_mutations(queue).unwrap();
    let (_, seeded) = product_frame_seeded(&mut warm, document, viewport(), &mut shaper);
    for id in [block, sibling] {
        assert!(!seeded.contains(&id), "{id:?} in {seeded:?}");
        assert!(!measured(&warm, id), "{id:?}");
    }
    let held = warm.world().block_strut(block);
    let mut queue = MutationQueue::new();
    queue.set_style(
        block,
        styled(LayoutStyle {
            font_size: Some(FONT * 1.5),
            ..strut_block(late)
        }),
    );
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(warm.world().block_strut(block), held);

    // The block's last inline-level child leaves: no lines, no strut; one
    // arrives: the strut is back. Child list edits alone, with no style of
    // the block's changing.
    let mut queue = MutationQueue::new();
    queue.set_style(atom, styled(strut_atom(DisplaySpec::Block, 40.0, 30.0)));
    queue.despawn_subtree(short);
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(warm.world().block_strut(block), None);
    let arrived = StableNodeId::new(900).unwrap();
    let mut queue = MutationQueue::new();
    queue.create(arrived, document, NodeKind::Element { tag: "div".into() });
    queue.insert(block, arrived, None);
    queue.set_style(
        arrived,
        styled(strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0)),
    );
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(warm.world().block_strut(block), held);
}

/// Gate B, third part: what the block inherits and what it holds. Its face
/// comes from a wrapper it inherits from, and its only inline-level child
/// moves out and back with no style changing on either.
#[test]
fn issue382_b_an_inherited_face_and_a_moved_child_reach_the_strut() {
    let wrapper = |family: Option<&str>| LayoutStyle {
        width: Some(LengthSpec::Px(600.0)),
        font_family: family.map(str::to_owned),
        ..LayoutStyle::default()
    };
    let inheriting = || LayoutStyle {
        font_family: None,
        font_weight: None,
        ..strut_block(Face {
            family: None,
            weight: 400,
        })
    };
    let build = |family: Option<&'static str>, held: bool| {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let outer = b.element(page, wrapper(family));
        let block = b.element(outer, inheriting());
        b.element(block, strut_atom(DisplaySpec::Block, 40.0, 30.0));
        let atom = b.element(
            if held { block } else { page },
            strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0),
        );
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = strut_shaper(false);
        product_frame(&mut context, document, viewport(), &mut shaper);
        (context, shaper, [page, outer, block, atom])
    };
    let document = DocumentId::new(1).unwrap();
    let (mut warm, mut shaper, [page, outer, block, atom]) = build(None, true);
    let mut probe = strut_shaper(false);
    let per_px = |strut: FontStrut| FontStrut {
        ascent: strut.ascent / FONT,
        descent: strut.descent / FONT,
    };
    assert_eq!(
        warm.world().block_strut(block),
        Some(per_px(face_strut(&mut probe, None, 400, FONT)))
    );

    // The wrapper's family reaches the block through inheritance alone.
    let mut queue = MutationQueue::new();
    queue.set_style(outer, styled(wrapper(Some("Wide"))));
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(
        warm.world().block_strut(block),
        Some(per_px(face_strut(&mut probe, Some("Wide"), 400, FONT)))
    );
    let (mut cold, _, _) = build(Some("Wide"), true);
    assert_matches_cold(&mut warm, &mut cold, document);

    // The only inline-level child moves out: the block lays no line out
    // and holds no strut. Back in, it does again.
    for held in [false, true] {
        let mut queue = MutationQueue::new();
        queue.insert(if held { block } else { page }, atom, None);
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(warm.world().block_strut(block).is_some(), held);
        let (mut cold, _, _) = build(Some("Wide"), held);
        assert_matches_cold(&mut warm, &mut cold, document);
    }
}

/// What gate D aligns: a label, a fixed box whose first line box is a
/// label below a spacer, a box whose first line box is two blocks down,
/// and a narrow box whose two empty atoms wrap to two lines, on a flex row
/// that aligns them by their first baselines. Returns the row, the labels,
/// the boxed text and spacer, the nested box's second label, a label
/// outside the row, the fixed box and the narrow box's two atoms.
fn first_baseline_page(
    boxed_font: f32,
    spacer: f32,
    second: &str,
    outside: &str,
) -> (AppContext, NanaTextEngineShaper, [StableNodeId; 11]) {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 800.0);
    let row = b.element(
        page,
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            align_items: nana_ui_core::AlignSpec::Baseline,
            width: Some(LengthSpec::Px(700.0)),
            ..LayoutStyle::default()
        },
    );
    let plain = b.label(row, "Plain");
    let boxed = b.element(
        row,
        LayoutStyle {
            width: Some(LengthSpec::Px(120.0)),
            height: Some(LengthSpec::Px(80.0)),
            padding_top: Some(LengthSpec::Px(7.0)),
            ..LayoutStyle::default()
        },
    );
    let boxed_spacer = b.element(
        boxed,
        LayoutStyle {
            height: Some(LengthSpec::Px(spacer)),
            ..LayoutStyle::default()
        },
    );
    let boxed_text = b.element(
        boxed,
        LayoutStyle {
            font_size: Some(boxed_font),
            ..LayoutStyle::default()
        },
    );
    let boxed_label = b.label(boxed_text, "Boxed");
    let nested = b.element(
        row,
        LayoutStyle {
            width: Some(LengthSpec::Px(120.0)),
            ..LayoutStyle::default()
        },
    );
    let inner = b.element(nested, LayoutStyle::default());
    let nested_label = b.label(inner, "Nested");
    let nested_second = b.label(nested, second);
    let narrow = b.element(
        row,
        LayoutStyle {
            width: Some(LengthSpec::Px(60.0)),
            ..LayoutStyle::default()
        },
    );
    let lines = [
        b.element(narrow, strut_atom(DisplaySpec::InlineBlock, 40.0, 20.0)),
        b.element(narrow, strut_atom(DisplaySpec::InlineBlock, 40.0, 20.0)),
    ];
    let outside = b.label(page, outside);
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let mut shaper = strut_shaper(false);
    product_frame(&mut context, document, viewport(), &mut shaper);
    (
        context,
        shaper,
        [
            row,
            plain,
            boxed_label,
            boxed_text,
            boxed_spacer,
            nested_label,
            nested_second,
            outside,
            boxed,
            lines[0],
            lines[1],
        ],
    )
}

/// Every first line on the row sits on one baseline: the plain label's,
/// the boxed label's below its spacer, the nested label's, and the narrow
/// box's first line, which its empty atom sits on by its bottom edge; its
/// second line is below.
fn assert_on_one_baseline(
    context: &AppContext,
    labels: [StableNodeId; 3],
    lines: [StableNodeId; 2],
) {
    let world = context.world();
    let baselines = labels.map(|label| {
        world
            .layout_result(label)
            .and_then(|result| result.first_baseline)
            .unwrap_or_else(|| panic!("{label:?} publishes no baseline"))
    });
    let bottom = |id| {
        let atom = world.layout_box(id).unwrap();
        atom.y + atom.height
    };
    for baseline in baselines.into_iter().chain([bottom(lines[0])]) {
        assert!(
            (baseline - baselines[0]).abs() < 0.01,
            "first baselines {baselines:?}, first atom's bottom {}",
            bottom(lines[0])
        );
    }
    assert!(
        bottom(lines[1]) > bottom(lines[0]) + 10.0,
        "the narrow box holds one line"
    );
}

/// Gate D: an item of a baseline-aligned flex row whose line boxes are its
/// descendants' sits by its first in-flow line box, wherever that lies
/// inside it: below a spacer in a fixed box, two blocks down. A new font
/// size in the fixed box and a taller spacer align the row again; text that
/// leaves the first line where it was, and text outside the row, move
/// nothing. Each step matches a cold layout.
#[test]
fn issue382_d_a_flex_item_sits_by_its_first_descendant_line() {
    let document = DocumentId::new(1).unwrap();
    let (mut warm, mut shaper, ids) = first_baseline_page(16.0, 10.0, "Second", "Outside");
    let [
        row,
        plain,
        boxed_label,
        boxed_text,
        boxed_spacer,
        nested_label,
        nested_second,
        outside,
        boxed,
        first_line,
        second_line,
    ] = ids;
    let lines = [first_line, second_line];
    let labels = [plain, boxed_label, nested_label];
    assert_on_one_baseline(&warm, labels, lines);
    let (mut cold, _, _) = first_baseline_page(16.0, 10.0, "Second", "Outside");
    assert_matches_cold(&mut warm, &mut cold, document);
    let fixed = warm.world().layout_box(boxed);

    // A bigger font in the fixed box: its first line moves down inside it,
    // the box does not grow, the row aligns again.
    let mut queue = MutationQueue::new();
    queue.set_style(
        boxed_text,
        styled(LayoutStyle {
            font_size: Some(28.0),
            ..LayoutStyle::default()
        }),
    );
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_eq!(
        warm.world().layout_box(boxed).map(|b| (b.width, b.height)),
        fixed.map(|b| (b.width, b.height))
    );
    assert_on_one_baseline(&warm, labels, lines);
    let (mut cold, _, _) = first_baseline_page(28.0, 10.0, "Second", "Outside");
    assert_matches_cold(&mut warm, &mut cold, document);

    // A taller spacer above the first line in the fixed box.
    let mut queue = MutationQueue::new();
    queue.set_style(
        boxed_spacer,
        styled(LayoutStyle {
            height: Some(LengthSpec::Px(24.0)),
            ..LayoutStyle::default()
        }),
    );
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_on_one_baseline(&warm, labels, lines);
    let (mut cold, _, _) = first_baseline_page(28.0, 24.0, "Second", "Outside");
    assert_matches_cold(&mut warm, &mut cold, document);

    // Text below the nested box's first line, and text outside the row:
    // no item of the row moves.
    let items = |context: &AppContext| {
        let world = context.world();
        world
            .node(row)
            .unwrap()
            .children
            .iter()
            .map(|id| world.layout_box(*id))
            .collect::<Vec<_>>()
    };
    let before = items(&warm);
    for (id, text) in [
        (nested_second, "Second, longer"),
        (outside, "Outside, longer"),
    ] {
        let mut queue = MutationQueue::new();
        queue.set_text(id, crate::TextContent::new(text));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(items(&warm), before, "{text}");
    }
    assert!(
        !measured(&warm, row),
        "the row measured for text outside it"
    );
    let (mut cold, _, _) = first_baseline_page(28.0, 24.0, "Second, longer", "Outside, longer");
    assert_matches_cold(&mut warm, &mut cold, document);
}

/// The block flows gate E lays out: two labels in a fixed box whose flow
/// puts its free space before, between or around them, or stacks them up
/// from the bottom.
const FLOWS: [(JustifySpec, bool); 6] = [
    (JustifySpec::Center, false),
    (JustifySpec::End, false),
    (JustifySpec::SpaceBetween, false),
    (JustifySpec::SpaceEvenly, false),
    (JustifySpec::Start, true),
    (JustifySpec::End, true),
];

/// A flex row aligning by first baselines and an ordinary block's line,
/// each holding a reference label and one fixed box per [`FLOWS`] entry,
/// the first entry's justification replaced by `first`. Returns the row's
/// and the line's labels, the reference first, then each box's two, and
/// the row's and the line's first box.
fn flow_baseline_page(
    first: JustifySpec,
) -> (
    AppContext,
    NanaTextEngineShaper,
    [Vec<StableNodeId>; 2],
    [StableNodeId; 2],
) {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 900.0);
    let row = b.element(
        page,
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            align_items: nana_ui_core::AlignSpec::Baseline,
            width: Some(LengthSpec::Px(800.0)),
            ..LayoutStyle::default()
        },
    );
    let line = b.element(
        page,
        LayoutStyle {
            width: Some(LengthSpec::Px(800.0)),
            ..LayoutStyle::default()
        },
    );
    let mut labels = [Vec::new(), Vec::new()];
    let mut first_boxes = Vec::new();
    for (slot, parent) in [row, line].into_iter().enumerate() {
        let reference = b.element(
            parent,
            LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                ..LayoutStyle::default()
            },
        );
        labels[slot].push(b.label(reference, "Ref"));
        for (index, (justify, reversed)) in FLOWS.into_iter().enumerate() {
            let flow = b.element(
                parent,
                LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    width: Some(LengthSpec::Px(90.0)),
                    height: Some(LengthSpec::Px(80.0)),
                    justify_content: if index == 0 { first } else { justify },
                    flex_reverse: reversed,
                    ..LayoutStyle::default()
                },
            );
            if index == 0 {
                first_boxes.push(flow);
            }
            for text in ["A", "B"] {
                let holder = b.element(flow, LayoutStyle::default());
                labels[slot].push(b.label(holder, text));
            }
        }
    }
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let mut shaper = strut_shaper(false);
    product_frame(&mut context, document, viewport(), &mut shaper);
    (context, shaper, labels, [first_boxes[0], first_boxes[1]])
}

/// The row sits each box by its first label in flow order, the line by its
/// last: each on the reference label's baseline, wherever its flow put it.
fn assert_flows_on_one_baseline(context: &AppContext, labels: &[Vec<StableNodeId>; 2]) {
    let world = context.world();
    let baseline = |id: StableNodeId| {
        world
            .layout_result(id)
            .and_then(|result| result.first_baseline)
            .unwrap_or_else(|| panic!("{id:?} publishes no baseline"))
    };
    for (slot, labels) in labels.iter().enumerate() {
        let reference = baseline(labels[0]);
        for (index, pair) in labels[1..].chunks(2).enumerate() {
            let sitting = pair[if slot == 0 { 0 } else { 1 }];
            let other = pair[if slot == 0 { 1 } else { 0 }];
            assert!(
                (baseline(sitting) - reference).abs() < 0.01,
                "{} {:?}: {} vs the reference {reference}",
                if slot == 0 { "row" } else { "line" },
                FLOWS[index],
                baseline(sitting)
            );
            assert!(
                (baseline(other) - reference).abs() > 1.0,
                "{:?}: both labels on the baseline",
                FLOWS[index]
            );
        }
    }
}

/// Gate E: a block flow that is not packed to its start sits by where its
/// first and last line boxes are: `justify-content` center, end,
/// space-between and space-evenly, and a reversed flow, alone and packed
/// to its end. On a flex row aligning first baselines and on an ordinary
/// block's line (last baselines) alike; a new justification aligns them
/// again. Each step matches a cold layout.
#[test]
fn issue382_e_a_justified_or_reversed_block_flow_sits_by_its_line_boxes() {
    let document = DocumentId::new(1).unwrap();
    let (mut warm, mut shaper, labels, first_boxes) = flow_baseline_page(JustifySpec::Center);
    assert_flows_on_one_baseline(&warm, &labels);
    let (mut cold, _, _, _) = flow_baseline_page(JustifySpec::Center);
    assert_matches_cold(&mut warm, &mut cold, document);
    let mut queue = MutationQueue::new();
    for first_box in first_boxes {
        queue.set_style(
            first_box,
            styled(LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(90.0)),
                height: Some(LengthSpec::Px(80.0)),
                justify_content: JustifySpec::End,
                ..LayoutStyle::default()
            }),
        );
    }
    warm.commit_mutations(queue).unwrap();
    product_frame(&mut warm, document, viewport(), &mut shaper);
    assert_flows_on_one_baseline(&warm, &labels);
    let (mut cold, _, _, _) = flow_baseline_page(JustifySpec::End);
    assert_matches_cold(&mut warm, &mut cold, document);
}

/// A flex row aligning first baselines and an ordinary block's line, each
/// with a reference label and a 100-wide box that floats an 80×40 box
/// left: on the row its one 40×20 atom does not fit beside the float and
/// drops below it; on the line three such atoms drop below it and wrap.
/// `float` floats the box (or leaves it in flow, 80 wide). Returns the
/// references and the atoms the box sits by: the row's only one, the
/// line's last.
fn float_baseline_page(float: bool) -> (AppContext, NanaTextEngineShaper, [StableNodeId; 4]) {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 900.0);
    let row = b.element(
        page,
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            align_items: nana_ui_core::AlignSpec::Baseline,
            width: Some(LengthSpec::Px(800.0)),
            ..LayoutStyle::default()
        },
    );
    let line = b.element(
        page,
        LayoutStyle {
            width: Some(LengthSpec::Px(800.0)),
            ..LayoutStyle::default()
        },
    );
    let mut ids = Vec::new();
    for (parent, atoms) in [(row, 1), (line, 3)] {
        let reference = b.element(
            parent,
            LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                ..LayoutStyle::default()
            },
        );
        ids.push(b.label(reference, "Ref"));
        let holder = b.element(
            parent,
            LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(100.0)),
                ..LayoutStyle::default()
            },
        );
        b.element(
            holder,
            LayoutStyle {
                width: Some(LengthSpec::Px(80.0)),
                height: Some(LengthSpec::Px(40.0)),
                float: if float {
                    nana_ui_core::FloatSpec::Left
                } else {
                    nana_ui_core::FloatSpec::None
                },
                ..LayoutStyle::default()
            },
        );
        let mut last = None;
        for _ in 0..atoms {
            last = Some(b.element(holder, strut_atom(DisplaySpec::InlineBlock, 40.0, 20.0)));
        }
        ids.push(last.unwrap());
    }
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let mut shaper = strut_shaper(false);
    product_frame(&mut context, document, viewport(), &mut shaper);
    (context, shaper, [ids[0], ids[1], ids[2], ids[3]])
}

/// Gate F: a box whose lines run beside a float sits by where those lines
/// are: on a flex row by its first line, which an atom too wide for the
/// room beside the float takes below it; on a line by its last, which
/// wraps below the float. Floating the box and sinking it back into the
/// flow each place them again, as a cold layout does.
#[test]
fn issue382_f_lines_beside_a_float_give_the_baselines() {
    let document = DocumentId::new(1).unwrap();
    let on_baseline =
        |context: &AppContext, [row_ref, row_atom, line_ref, line_atom]: [StableNodeId; 4]| {
            let world = context.world();
            for (reference, atom) in [(row_ref, row_atom), (line_ref, line_atom)] {
                let baseline = world
                    .layout_result(reference)
                    .and_then(|result| result.first_baseline)
                    .unwrap();
                let atom = world.layout_box(atom).unwrap();
                assert!(
                    (atom.y + atom.height - baseline).abs() < 0.01,
                    "{atom:?} sits off the baseline {baseline}"
                );
            }
        };
    let (mut warm, mut shaper, ids) = float_baseline_page(true);
    on_baseline(&warm, ids);
    // Below the float: the first line starts at its bottom.
    let holder = warm.world().parent_id(ids[1]).unwrap();
    let (atom, holder_box) = (
        warm.world().layout_box(ids[1]).unwrap(),
        warm.world().layout_box(holder).unwrap(),
    );
    assert!(
        atom.y >= holder_box.y + 40.0 - 0.01,
        "{atom:?} in {holder_box:?}"
    );
    let (mut cold, _, _) = float_baseline_page(true);
    assert_matches_cold(&mut warm, &mut cold, document);
    for float in [false, true] {
        let mut queue = MutationQueue::new();
        for atom in [ids[1], ids[3]] {
            let holder = warm.world().parent_id(atom).unwrap();
            let box_id = warm.world().node(holder).unwrap().children[0];
            queue.set_style(
                box_id,
                styled(LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    float: if float {
                        nana_ui_core::FloatSpec::Left
                    } else {
                        nana_ui_core::FloatSpec::None
                    },
                    ..LayoutStyle::default()
                }),
            );
        }
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        on_baseline(&warm, ids);
        let (mut cold, _, _) = float_baseline_page(float);
        assert_matches_cold(&mut warm, &mut cold, document);
    }
}
