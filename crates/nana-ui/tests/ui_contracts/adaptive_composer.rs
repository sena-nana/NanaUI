use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_core::{Icon, SemanticPalette, ThemeAppearance, UI_METRICS};

struct Composer {
    cx: AppContext,
    document: DocumentId,
    area: Entity<TextArea>,
    action: Entity<IconButton>,
    shaper: NanaTextShaper,
}

impl Composer {
    fn new() -> Self {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let action = cx
            .create_detached_component(document, IconButton::new(Icon::Close, "Send"))
            .unwrap();
        let area = cx
            .create_component(
                document,
                TextArea::new("")
                    .auto_grow(1, 6)
                    .character_counter(40)
                    .trailing_action(action.stable_id()),
            )
            .unwrap();
        cx.assemble_text_area(area).unwrap();
        cx.focus_node(document, area.stable_id()).unwrap();
        Self {
            cx,
            document,
            area,
            action,
            shaper: NanaTextShaper::default(),
        }
    }

    fn settle(&mut self, width: f32) {
        for _ in 0..8 {
            let ids = self.cx.world().document_order(self.document);
            self.cx.resolve_styles(&ids).unwrap();
            self.cx.shape_text(&ids, &mut self.shaper).unwrap();
            self.cx
                .layout_document(self.document, LayoutViewport::new(width, 400.0))
                .unwrap();
            self.cx
                .shape_text_for_layout(self.document, &mut self.shaper)
                .unwrap();
        }
    }

    fn text(&mut self, value: &str) {
        self.cx
            .update_component(self.area, |area, _| {
                area.state.replace_value(value);
                area.state.selection = TextSelection::new(value.len(), value.len());
            })
            .unwrap();
    }

    fn frame(&self) -> LayoutBox {
        self.cx.world().layout_box(self.area.stable_id()).unwrap()
    }

    fn counter(&self) -> TextInputCharacterCounter {
        let ComponentGeometry::TextInput {
            character_counter: Some(counter),
            ..
        } = self
            .cx
            .world()
            .component_geometry(self.area.stable_id())
            .unwrap()
        else {
            panic!("missing counter")
        };
        counter
    }

    fn expanded_geometry(&self) {
        let editor = self.counter().editor_bounds;
        let count = self.counter().text.bounds;
        let action = self.cx.world().layout_box(self.action.stable_id()).unwrap();
        assert!(editor.width > self.frame().width - 24.0);
        assert!(editor.y + editor.height <= count.y);
        assert!(editor.y + editor.height <= action.y);
        assert!(editor.x + editor.width >= action.x + action.width);
    }

    fn exact_fit(&mut self) -> f32 {
        self.settle(260.0);
        let reserved = self.frame().width - self.counter().editor_bounds.width;
        self.text("直播弹幕ABC");
        self.settle(260.0);
        let measured = self
            .cx
            .world()
            .text_input_presentation(self.area.stable_id())
            .unwrap()
            .compact_measure
            .unwrap()
            .0;
        measured + reserved
    }
}

#[test]
fn shaped_compact_boundary_is_stable_through_expansion_resize_and_deletion() {
    let mut composer = Composer::new();
    composer.settle(260.0);
    assert_eq!(composer.frame().height, 32.0);
    let compact = composer.counter();
    let action = composer
        .cx
        .world()
        .layout_box(composer.action.stable_id())
        .unwrap();
    assert!(compact.editor_bounds.x + compact.editor_bounds.width < compact.text.bounds.x);
    assert!(compact.text.bounds.x + compact.text.bounds.width <= action.x);
    let exact_width = composer.exact_fit();
    composer.settle(exact_width);
    assert_eq!(
        composer.frame().height,
        32.0,
        "an exact shaped fit remains on one row"
    );
    composer.settle(exact_width - 0.125);
    assert!(composer.frame().height >= 64.0);
    composer.expanded_geometry();
    // The text fits one expanded full-width row, yet must remain expanded:
    // the decision always uses the smaller hypothetical compact viewport.
    for _ in 0..3 {
        composer.settle(exact_width - 0.125);
        assert!(composer.frame().height >= 64.0);
    }
    composer.settle(exact_width);
    assert_eq!(composer.frame().height, 32.0);
    composer.settle(exact_width - 0.125);
    composer
        .cx
        .delete_focused_text_backward(composer.document)
        .unwrap();
    composer.settle(exact_width - 0.125);
    assert_eq!(composer.frame().height, 32.0);
    for separator in [
        '\n', '\r', '\u{85}', '\u{2028}', '\u{2029}', '\u{b}', '\u{c}',
    ] {
        composer.text(&format!("a{separator}b"));
        composer.settle(260.0);
        assert!(
            composer.frame().height > 32.0,
            "hard separator {separator:?}"
        );
        composer.expanded_geometry();
        composer.text(&format!("a{separator}"));
        composer.settle(260.0);
        assert!(
            composer.frame().height > 32.0,
            "trailing hard separator {separator:?}"
        );
    }
    composer.text("");
    composer
        .cx
        .update_component(composer.area, |area, _| {
            area.placeholder = "very long placeholder ".repeat(30).into()
        })
        .unwrap();
    composer.settle(180.0);
    assert_eq!(
        composer.frame().height,
        32.0,
        "a placeholder never expands the editor"
    );
    // Compact and expanded capacity must use the same authored padding,
    // including an asymmetric style supplied by a consumer.
    composer.text("");
    composer
        .cx
        .update_component(composer.area, |area, _| {
            area.style.control_padding_x = None;
            let layout = std::sync::Arc::make_mut(&mut area.style.layout);
            layout.padding_left = Some(nana_ui_core::LengthSpec::Px(4.0));
            layout.padding_right = Some(nana_ui_core::LengthSpec::Px(18.0));
        })
        .unwrap();
    composer.settle(260.0);
    let reserved = composer.frame().width - composer.counter().editor_bounds.width;
    composer.text("直播弹幕ABC");
    composer.settle(260.0);
    let measured = composer
        .cx
        .world()
        .text_input_presentation(composer.area.stable_id())
        .unwrap()
        .compact_measure
        .unwrap()
        .0;
    let exact = measured + reserved;
    composer.settle(exact);
    assert_eq!(composer.frame().height, 32.0);
    composer.settle(exact - 0.125);
    assert!(composer.frame().height >= 64.0);
    assert_eq!(composer.counter().editor_bounds.x, composer.frame().x + 5.0);
    assert!(
        (composer.counter().editor_bounds.width - (composer.frame().width - 24.0)).abs() < 0.001
    );
    composer.settle(exact);
    assert_eq!(composer.frame().height, 32.0);
}

#[test]
fn ime_and_font_changes_remeasure_the_same_compact_capacity() {
    let mut composer = Composer::new();
    let width = composer.exact_fit();
    composer.settle(width);
    assert_eq!(composer.frame().height, 32.0);
    let committed_count = composer.counter().text.content;
    composer
        .cx
        .set_ime_preedit(composer.document, "字".into(), None)
        .unwrap();
    composer.settle(width);
    assert!(composer.frame().height >= 64.0);
    composer.expanded_geometry();
    assert_eq!(composer.counter().text.content, committed_count);
    composer.cx.clear_ime(composer.document).unwrap();
    composer.settle(width);
    assert_eq!(composer.frame().height, 32.0);
    composer
        .cx
        .set_ime_preedit(composer.document, "字".into(), None)
        .unwrap();
    composer.cx.commit_ime(composer.document, "字").unwrap();
    composer.settle(width);
    assert!(composer.frame().height >= 64.0);
    composer
        .cx
        .delete_focused_text_backward(composer.document)
        .unwrap();
    composer.settle(width);
    assert_eq!(composer.frame().height, 32.0);
    let size = composer
        .cx
        .world()
        .computed_style(composer.area.stable_id())
        .unwrap()
        .font_size;
    // A literal size replaces the control's typography binding, as a literal
    // CSS declaration does; restoring the binding returns to the theme size.
    let mut binding = None;
    composer
        .cx
        .update_component(composer.area, |area, _| {
            let layout = std::sync::Arc::make_mut(&mut area.style.layout);
            binding = layout.theme.font_size.take();
            layout.font_size = Some(size * 1.5);
        })
        .unwrap();
    assert!(binding.is_some());
    composer.settle(width);
    assert!(composer.frame().height > 32.0);
    composer
        .cx
        .update_component(composer.area, |area, _| {
            let layout = std::sync::Arc::make_mut(&mut area.style.layout);
            layout.theme.font_size = binding;
            layout.font_size = None;
        })
        .unwrap();
    composer.settle(width);
    assert_eq!(composer.frame().height, 32.0);
    let mut metrics = UI_METRICS;
    metrics.control_height += 4.0;
    metrics.compact_control_height += 4.0;
    composer
        .cx
        .set_style_tokens(
            ThemeAppearance::Dark,
            metrics,
            SemanticPalette::dark(),
            SemanticPalette::dark().surface,
        )
        .unwrap();
    composer.settle(width);
    assert!(composer.frame().height > metrics.control_height);
    composer
        .cx
        .set_style_tokens(
            ThemeAppearance::Light,
            UI_METRICS,
            SemanticPalette::light(),
            SemanticPalette::light().surface,
        )
        .unwrap();
    composer.settle(width);
    assert_eq!(composer.frame().height, 32.0);
}
