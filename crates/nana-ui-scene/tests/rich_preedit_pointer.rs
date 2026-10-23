//! 显示预编辑的命中坐标回到已提交文档后的输入链回归。

#![cfg(feature = "rich-text")]

use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_ui_input::{CompositionInput, InputPayload, PointerPhase};
use nana_ui_runtime::{
    DocumentId, Entity, HeadlessInput, LayoutViewport, NanaTextEngineShaper, RichEditCommand,
    RichTextEditor, RichTextEditorEvent, rich_editor_caret_box,
};
use nana_ui_scene::RuntimeDocument;

/// 保留真实文本布局，以覆盖输入法显示字节与文档字节之间的差异。
struct Fixture {
    runtime: RuntimeDocument,
    shaper: NanaTextEngineShaper,
    input: HeadlessInput,
    editor: Entity<RichTextEditor>,
}

impl Fixture {
    fn new(text: &str) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let editor = runtime
            .context_mut()
            .create_component(
                document,
                RichTextEditor::new(text).width(nana_ui_core::LengthSpec::Px(400.0)),
            )
            .unwrap();
        let mut policy = FallbackPolicy::empty();
        policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
        let mut fonts = FontSystem::with_policy(policy);
        fonts
            .register_bytes(
                font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
                &FaceDescriptor::default(),
            )
            .unwrap();
        let shaper = NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))));
        let mut input = HeadlessInput::bind(runtime.context_mut(), document);
        input
            .route(runtime.context_mut(), InputPayload::Focus { focused: true })
            .unwrap();
        runtime
            .context_mut()
            .focus_node(document, editor.stable_id())
            .unwrap();
        let mut fixture = Self {
            runtime,
            shaper,
            input,
            editor,
        };
        fixture.settle();
        fixture
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.runtime
                .flush(LayoutViewport::new(400.0, 300.0), &mut self.shaper)
                .unwrap();
        }
    }

    fn preedit(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.runtime
            .context_mut()
            .rich_edit(self.editor, RichEditCommand::Select(range))
            .unwrap();
        for composition in [
            CompositionInput::Start,
            CompositionInput::Update {
                text: text.into(),
                selection: None,
            },
        ] {
            self.input
                .composition(self.runtime.context_mut(), composition)
                .unwrap();
        }
        self.settle();
    }

    fn position(&self, byte: usize) -> (f32, f32) {
        let world = self.runtime.context().world();
        let bounds = world.layout_box(self.editor.stable_id()).unwrap();
        let (_, layout) = world.text_layout(self.editor.stable_id()).unwrap();
        let (x, y, height) = rich_editor_caret_box(layout, byte).unwrap();
        (bounds.x + x + 0.1, bounds.y + y + height / 2.0)
    }

    fn click(&mut self, position: (f32, f32)) {
        for phase in [PointerPhase::Down, PointerPhase::Up] {
            self.input
                .pointer(self.runtime.context_mut(), phase, position.0, position.1)
                .unwrap();
            self.settle();
        }
    }

    fn selection(&self) -> (usize, usize) {
        self.runtime
            .context()
            .read(self.editor, RichTextEditor::selection)
            .unwrap()
    }
}

#[test]
fn empty_preedit_click_cancels_composition_and_allows_typing_and_undo() {
    let mut f = Fixture::new("");
    f.preedit(0..0, "你好");
    f.click(f.position(6));
    assert_eq!(f.selection(), (0, 0));
    assert!(
        f.runtime
            .context()
            .read(f.editor, |v| v.preedit().is_none())
            .unwrap()
    );
    f.input.text(f.runtime.context_mut(), "继续").unwrap();
    assert_eq!(
        f.runtime
            .context()
            .read(f.editor, |v| v.value.text().to_owned())
            .unwrap(),
        "继续"
    );
    f.runtime
        .context_mut()
        .rich_edit(f.editor, RichEditCommand::Undo)
        .unwrap();
    assert_eq!(f.selection(), (0, 0));
    assert_eq!(
        f.runtime
            .context()
            .read(f.editor, |v| v.value.text().to_owned())
            .unwrap(),
        ""
    );
}

#[test]
fn replacement_preedit_hits_map_before_inside_and_after_to_committed_characters() {
    // 文档为「前ab后尾」，显示为「前你好后尾」；组合内部回到替换起点。
    for (display_byte, document_byte) in [(0, 0), (3, 3), (6, 3), (9, 5), (12, 8), (15, 11)] {
        let mut f = Fixture::new("前ab后尾");
        f.preedit(3..5, "你好");
        f.click(f.position(display_byte));
        assert_eq!(
            f.selection(),
            (document_byte, document_byte),
            "display byte {display_byte}"
        );
        f.input.text(f.runtime.context_mut(), "字").unwrap();
        let mut expected = "前ab后尾".to_owned();
        expected.insert_str(document_byte, "字");
        assert_eq!(
            f.runtime
                .context()
                .read(f.editor, |v| v.value.text().to_owned())
                .unwrap(),
            expected
        );
    }
}

#[test]
fn external_clear_before_layout_refresh_keeps_pointer_selection_valid() {
    let mut f = Fixture::new("旧文档");
    let position = f.position(9);
    f.runtime
        .context_mut()
        .set_component(f.editor, RichTextEditor::new(""))
        .unwrap();
    f.click(position);
    assert_eq!(f.selection(), (0, 0));
    f.input.text(f.runtime.context_mut(), "新").unwrap();
    assert_eq!(
        f.runtime
            .context()
            .read(f.editor, |v| v.value.text().to_owned())
            .unwrap(),
        "新"
    );
}

#[test]
fn cancelled_preedit_layout_change_preserves_stationary_pointer_and_allows_dragging() {
    let mut f = Fixture::new("前ab后尾");
    f.preedit(3..5, "你好");
    let position = f.position(9);
    for phase in [PointerPhase::Down, PointerPhase::Move, PointerPhase::Up] {
        f.input
            .pointer(f.runtime.context_mut(), phase, position.0, position.1)
            .unwrap();
        f.settle();
        assert_eq!(f.selection(), (5, 5));
    }
    let start = f.position(11);
    let end = f.position(0);
    f.input
        .pointer(
            f.runtime.context_mut(),
            PointerPhase::Down,
            start.0,
            start.1,
        )
        .unwrap();
    f.input
        .pointer(f.runtime.context_mut(), PointerPhase::Move, end.0, end.1)
        .unwrap();
    f.input
        .pointer(f.runtime.context_mut(), PointerPhase::Up, end.0, end.1)
        .unwrap();
    assert_eq!(f.selection(), (11, 0));
}

#[test]
fn preedit_shifted_object_activation_keeps_offset_when_object_ids_repeat() {
    let mut f = Fixture::new("");
    let mut value = nana_ui_core::RichText::new("ab");
    value.insert_object(0, nana_ui_core::RichObject::chip(7, "wait", 0));
    value.insert_object(5, nana_ui_core::RichObject::chip(7, "wait", 0));
    f.runtime
        .context_mut()
        .set_component(
            f.editor,
            RichTextEditor::new(value).width(nana_ui_core::LengthSpec::Px(400.0)),
        )
        .unwrap();
    let activated = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::clone(&activated);
    f.runtime
        .context_mut()
        .on(f.editor, move |_, event: &RichTextEditorEvent, _| {
            if let RichTextEditorEvent::ObjectActivated { id, offset } = event {
                events.lock().unwrap().push((*id, *offset));
            }
        })
        .unwrap();
    f.preedit(3..4, "你好");
    let world = f.runtime.context().world();
    let bounds = world.layout_box(f.editor.stable_id()).unwrap();
    let (_, layout) = world.text_layout(f.editor.stable_id()).unwrap();
    let placed = layout
        .objects
        .iter()
        .find(|object| object.offset == 10)
        .unwrap();
    let line = &layout.lines[placed.line as usize];
    let position = (
        bounds.x + placed.rect.x + placed.rect.width * 0.5,
        bounds.y + line.metrics.top_y_px + line.metrics.height_px * 0.5,
    );
    f.click(position);
    assert_eq!(*activated.lock().unwrap(), [(7, 5)]);
}
