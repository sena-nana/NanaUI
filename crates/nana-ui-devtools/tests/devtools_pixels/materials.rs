//! Real retained-tree pixels: the same material contract shades a shape and
//! glyphs, with native coverage, rich-span overrides and explicit fallback.
use nana_ui::runtime::view::{IntoView, widget};
use nana_ui::runtime::{DocumentId, RichTextView, RuntimeDocument, Stack, Text};
use nana_ui_core::{
    LengthSpec, MaterialProgram, MaterialRef, MaterialTexture, RichSpanStyle, RichText,
};
use nana_ui_devtools::agent::{AgentSession, RuntimeAgentSession};
use std::sync::Arc;

const TINT: &str = "struct nm_bg_MaterialInput { color: vec3<f32> } fn color(v:vec3<f32>)->vec3<f32>{return v;} fn material(i: MaterialInput) -> vec4<f32> { let sample=nm_bg_MaterialInput(i.color.rgb); return vec4<f32>(color(material_param(0u).rgb) + sample.color * 0., 1.); }";
const GRAIN: &str = "fn material(i: MaterialInput) -> vec4<f32> { let stripe = select(.4, 1., fract(i.position.x / 4.) < .5); return vec4<f32>(i.color.rgb, stripe); }";

fn render(material: Option<MaterialRef>, border: Option<MaterialRef>, rich: bool) -> Vec<u8> {
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    doc.context_mut()
        .mount_view_root(id, move || {
            let shape = Stack::row(0.)
                .width(LengthSpec::Px(160.))
                .height(LengthSpec::Px(60.))
                .with_layout(|l| {
                    l.background = Some([0.2, 0.2, 0.2, 1.]);
                    l.border_radius = Some(16.);
                    l.border_color = Some([1., 1., 1., 1.]);
                    l.border_top_width = Some(6.);
                    l.border_right_width = Some(6.);
                    l.border_bottom_width = Some(6.);
                    l.border_left_width = Some(6.);
                    l.border_style = Some(nana_ui_core::BorderStyle::Solid);
                    l.paint.background_material = material.clone();
                    l.paint.border_material = border;
                });
            let style = |l: &mut nana_ui_core::LayoutStyle| {
                l.font_size = Some(28.);
                l.color = Some([0.1, 0.1, 0.1, 1.]);
                l.paint.text_material = material.clone();
            };
            let mut text_style = nana_ui::runtime::NodeStyle::default();
            style(Arc::make_mut(&mut text_style.layout));
            let label = if rich {
                let green = MaterialRef::new(MaterialProgram::new(TINT))
                    .with_parameters(Arc::from([[0., 1., 0., 1.]]));
                let mut span = RichSpanStyle::new();
                span.paint.material = Some(green);
                let text = RichText::builder()
                    .push("彩铅 ", RichSpanStyle::new())
                    .push("AB", span)
                    .push(" 😀", RichSpanStyle::new())
                    .build();
                widget(RichTextView::new(text).style(text_style)).into_any()
            } else {
                widget(Text::new("彩铅 ABC 😀").style(text_style)).into_any()
            };
            widget(Stack::column(8.).padding(10.)).children((widget(shape), label))
        })
        .unwrap();
    let mut session = RuntimeAgentSession::new(doc, 200, 150).unwrap();
    session.set_clear(Some([0.; 4]));
    session.screenshot_rgba().unwrap().1
}
fn at(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4]
}

#[test]
fn shapes_and_text_share_materials_and_keep_their_coverage() {
    let red =
        MaterialRef::new(MaterialProgram::new(TINT)).with_parameters(Arc::from([[1., 0., 0., 1.]]));
    let blue =
        MaterialRef::new(MaterialProgram::new(TINT)).with_parameters(Arc::from([[0., 0., 1., 1.]]));
    let pixels = render(Some(red), Some(blue), true);
    assert!(
        at(&pixels, 70, 35)[0] > 240 && at(&pixels, 70, 35)[2] < 10,
        "shape fill: {:?}",
        at(&pixels, 70, 35)
    );
    assert!(
        at(&pixels, 70, 12)[2] > 240 && at(&pixels, 70, 12)[0] < 10,
        "border: {:?}",
        at(&pixels, 70, 12)
    );
    assert_eq!(
        at(&pixels, 10, 10)[3],
        0,
        "material escaped rounded coverage"
    );
    let text = &pixels[78 * 200 * 4..];
    assert!(
        text.chunks_exact(4)
            .filter(|p| p[0] > 200 && p[1] < 40 && p[3] > 200)
            .count()
            > 50,
        "base text material missing"
    );
    assert!(
        text.chunks_exact(4)
            .filter(|p| p[1] > 200 && p[0] < 40 && p[3] > 200)
            .count()
            > 30,
        "rich material override missing"
    );
    assert_eq!(
        at(&pixels, 195, 100)[3],
        0,
        "material shaded outside glyphs"
    );
}

#[test]
fn materials_are_optional_and_invalid_programs_fall_back_to_native_paint() {
    let native = render(None, None, false);
    let passthrough = MaterialRef::new(MaterialProgram::new(
        "fn material(i:MaterialInput)->vec4<f32>{return i.color;}",
    ));
    let unchanged = render(Some(passthrough), None, false);
    assert_eq!(
        native, unchanged,
        "identity material changed native coverage or color"
    );
    let invalid = MaterialRef::new(MaterialProgram::new("invalid shader"));
    assert_eq!(
        native,
        render(Some(invalid), None, false),
        "failed shader did not restore native paint"
    );
    let bad_texture = MaterialRef::new(MaterialProgram::new(
        "fn material(i:MaterialInput)->vec4<f32>{return i.color;}",
    ))
    .with_textures(Arc::from([MaterialTexture {
        size: [2, 2],
        rgba: Arc::from([0u8; 4]),
    }]));
    assert_eq!(
        native,
        render(Some(bad_texture), None, false),
        "invalid texture did not fall back"
    );
    let grain = render(
        Some(MaterialRef::new(MaterialProgram::new(GRAIN))),
        None,
        false,
    );
    assert_ne!(native, grain, "second material example was ignored");
    let colored = |p: &[u8]| {
        p[78 * 200 * 4..]
            .chunks_exact(4)
            .filter(|p| p[3] > 150 && p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 80)
            .count()
    };
    assert!(
        colored(&native) > 20,
        "native emoji fixture was not colored"
    );
    assert_eq!(
        colored(&native),
        colored(&grain),
        "material changed color emoji pixels"
    );
    assert_eq!(
        grain,
        render(
            Some(MaterialRef::new(MaterialProgram::new(GRAIN))),
            None,
            false
        ),
        "static material changed between captures"
    );
}

#[test]
fn material_texture_replacements_change_pixels_without_changing_the_contract() {
    let program = MaterialProgram::new(
        "fn material(i:MaterialInput)->vec4<f32>{return material_sample0(i.uv);}",
    );
    let red = MaterialRef::new(program).with_textures(Arc::from([MaterialTexture {
        size: [1, 1],
        rgba: Arc::from([255, 0, 0, 255]),
    }]));
    let blue = red.clone().with_textures(Arc::from([MaterialTexture {
        size: [1, 1],
        rgba: Arc::from([0, 0, 255, 255]),
    }]));
    assert_eq!(red.id(), blue.id());
    let a = render(Some(red), None, false);
    let b = render(Some(blue), None, false);
    assert!(at(&a, 70, 35)[0] > 240);
    assert!(at(&b, 70, 35)[2] > 240);
}

#[test]
fn presentation_masks_preserve_color_and_erase_the_whole_text_target() {
    use nana_ui::runtime::{MutationQueue, NodeStyle};
    use nana_ui_core::{
        GlyphPresentation, PresentationMask, PresentationMaskSpace, TextTransitions,
    };
    use std::time::Duration;
    let id = DocumentId::new(1).unwrap();
    let mask = MaterialRef::new(MaterialProgram::new(
        "fn material(i:MaterialInput)->vec4<f32>{return vec4<f32>(1.,0.,0.,select(select(select(1.,0.,i.uv.x<i.progress),1.,i.progress<=0.),0.,i.progress>=1.));}",
    ));
    let mut builder = RichText::builder();
    builder.push("彩铅 ABC 😀\n第二行 ABC", RichSpanStyle::new());
    builder.ruby("文字", "もじ");
    builder.object(nana_ui_core::RichObject::image(100,"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==",24.,24.));
    let value = builder.build();
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.font_size = Some(28.);
    layout.color = Some([0., 0.1, 0.8, 1.]);
    layout.paint.text_stroke = Some(nana_ui_core::TextStrokeSpec {
        width: 1.,
        color: Some([0., 0., 0., 1.]),
        paint_color: None,
    });
    let mut doc = RuntimeDocument::new(id);
    let mounted = doc
        .context_mut()
        .mount_view_root(id, move || {
            widget(RichTextView::new(value.clone()).style(style.clone()))
        })
        .unwrap();
    let node = mounted.roots()[0];
    let mut session = RuntimeAgentSession::new(doc, 240, 130).unwrap();
    session.set_clear(Some([0.; 4]));
    for _ in 0..3 {
        let _ = session.screenshot_rgba().unwrap();
    }
    std::fs::create_dir_all("target/material-mask").unwrap();
    session
        .screenshot_png("target/material-mask/native.png")
        .unwrap();
    let native = session.screenshot_rgba().unwrap().1;
    assert!(
        native
            .chunks_exact(4)
            .filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] > 200)
            .count()
            > 100,
        "the inline image fixture was not actually visible"
    );
    let mut presentation = GlyphPresentation::default();
    presentation.transitions = TextTransitions {
        exit: Some(PresentationMask {
            material: mask,
            space: PresentationMaskSpace::Target,
            start: Duration::ZERO,
            duration: Duration::from_secs(1),
        }),
        viewport: Some(nana_ui_core::LogicalRect::new(20., 0., 180., 130.)),
        ..Default::default()
    };
    let mut queue = MutationQueue::new();
    queue.set_glyph_presentation(node, Some(presentation));
    session
        .document_mut()
        .context_mut()
        .commit_mutations(queue)
        .unwrap();
    let first = session.screenshot_rgba().unwrap().1;
    assert_eq!(first, native, "neutral mask changed native RGB or coverage");
    session
        .document_mut()
        .sync_presentation_clock(Duration::from_millis(500));
    let halfway = session.screenshot_rgba().unwrap().1;
    let mut visible = 0;
    for y in 0..130 {
        for x in 0..240 {
            let p = (y * 240 + x) * 4;
            if x < 109 {
                assert_eq!(halfway[p + 3], 0, "left edge did not erase");
            } else if x > 111 && native[p + 3] > 0 {
                visible += 1;
                assert_eq!(
                    &halfway[p..p + 4],
                    &native[p..p + 4],
                    "mask recolored surviving text at {x},{y}"
                );
            }
        }
    }
    assert!(visible > 30, "right side vanished prematurely");
    session
        .document_mut()
        .sync_presentation_clock(Duration::from_millis(700));
    let late = session.screenshot_rgba().unwrap().1;
    for y in 0..130 {
        for x in 148..240 {
            let p = (y * 240 + x) * 4;
            if native[p + 3] > 0 {
                assert_eq!(
                    &late[p..p + 4],
                    &native[p..p + 4],
                    "inline object did not use the text's declared target at {x},{y}"
                );
            }
        }
    }
    session
        .document_mut()
        .sync_presentation_clock(Duration::from_secs(1));
    let erased = session.screenshot_rgba().unwrap().1;
    assert!(
        erased.chunks_exact(4).all(|p| p[3] == 0),
        "glyph, ruby, stroke or emoji survived final mask"
    );
}
