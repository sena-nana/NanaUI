//! A confirm's message paints whole: the scene draws it at the line height
//! it was measured with, so every wrapped line fits the part sized for it
//! and none is elided. Copy taller than the card's cap stays in the card.

use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine};
use nana_ui_runtime::{
    ComponentGeometry, ComputedStyle, ConfirmDialog, DocumentId, LayoutBox, LayoutViewport,
    NanaTextEngineShaper, OverlayHost, StableNodeId, TextContent, TextShapeConstraints, TextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

fn engine() -> SharedTextEngine {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

/// A confirm saying `message`, open and flushed through the product loop.
fn open(message: &str) -> (RuntimeDocument, StableNodeId, NanaTextEngineShaper) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut shaper = NanaTextEngineShaper::new(engine());
    let cx = runtime.context_mut();
    let host = cx.create_component(document, OverlayHost::new()).unwrap();
    let dialog = cx
        .create_detached_component(document, ConfirmDialog::new("开启 VTS 兼容", message))
        .unwrap();
    cx.assemble_confirm_dialog(dialog).unwrap();
    cx.append_child(host, dialog).unwrap();
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(
        runtime
            .context_mut()
            .activate_overlay(host, dialog)
            .unwrap()
    );
    runtime.flush(viewport(), &mut shaper).unwrap();
    runtime.flush(viewport(), &mut shaper).unwrap();
    (runtime, dialog.stable_id(), shaper)
}

/// The card and the message's region.
fn regions(runtime: &RuntimeDocument, id: StableNodeId) -> (LayoutBox, LayoutBox) {
    match runtime.context().world().component_geometry(id).unwrap() {
        ComponentGeometry::ModalFrame {
            surface, body_text, ..
        } => (surface, body_text.expect("the message").bounds),
        other => panic!("{other:?}"),
    }
}

/// The height the message takes shaped as the scene paints it, and the
/// bounds it paints in.
fn painted_height(
    runtime: &RuntimeDocument,
    id: StableNodeId,
    message: &str,
    shaper: &mut NanaTextEngineShaper,
) -> (f32, LayoutBox) {
    let primitive = runtime
        .scene()
        .primitives()
        .find(|primitive| {
            primitive.node == id
                && matches!(&primitive.kind, ScenePrimitiveKind::Text { content, .. }
                    if content.as_ref() == message)
        })
        .expect("the message paints");
    let ScenePrimitiveKind::Text {
        size,
        line_height,
        letter_spacing,
        wrap,
        ..
    } = &primitive.kind
    else {
        unreachable!()
    };
    assert!(wrap, "the message wraps");
    let style = ComputedStyle {
        font_size: *size,
        line_height: *line_height,
        letter_spacing: *letter_spacing,
        ..ComputedStyle::default()
    };
    let bounds = LayoutBox {
        x: primitive.bounds.x,
        y: primitive.bounds.y,
        width: primitive.bounds.width,
        height: primitive.bounds.height,
    };
    let metrics = shaper.shape(
        id,
        &TextContent {
            value: message.to_owned().into(),
        },
        &style,
        TextShapeConstraints {
            max_width: Some(bounds.width),
            wrap: true,
            ..Default::default()
        },
    );
    (metrics.height, bounds)
}

/// Long Chinese without a space, long English, and a path: each message
/// paints in no more height than its part has, so no line is elided.
#[test]
fn a_long_message_paints_every_line_in_its_part() {
    for message in [
        "开启后，VBridger 等插件可以连接本机，每个插件第一次连接都需要你允许。插件能读取面捕数据并写入模型参数，只允许你信任的插件；不再使用时可以随时在这里撤销授权，已连接的插件会立即断开。",
        "缓存文件夹会搬到新位置，旧位置的缓存文件随即删除，搬动期间模型优化暂停，完成后自动继续",
        "Connected plugins disconnect and the values they injected stop applying. Face capture through VBridger stops as well, and plugins have to ask again before they can reconnect.",
        "缓存将移到 /Users/someone/Library/Application Support/NanaLive/cache/live2d/prewarm/collab/basis，旧位置的文件随即删除。",
    ] {
        let (runtime, id, mut shaper) = open(message);
        let (surface, part) = regions(&runtime, id);
        let (height, bounds) = painted_height(&runtime, id, message, &mut shaper);
        assert_eq!(bounds.height, part.height, "{message}");
        assert!(
            height <= part.height + 0.5,
            "{message}: paints {height} in a {} part",
            part.height
        );
        assert!(part.height > 20.0, "{message}: wraps to several lines");
        assert!(
            part.y + part.height <= surface.y + surface.height,
            "{message}"
        );
    }
}

/// Copy taller than the card's cap keeps its part inside the card instead
/// of running over the actions below it.
#[test]
fn copy_past_the_cap_stays_in_the_card() {
    let message = "很长的一句话需要很多行才能放下。".repeat(120);
    let (runtime, id, _) = open(&message);
    let (surface, part) = regions(&runtime, id);
    assert!(surface.height < viewport().height);
    assert!(part.y + part.height <= surface.y + surface.height);
}
