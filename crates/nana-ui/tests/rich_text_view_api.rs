//! The public rich text surface an application builds from: the examples in
//! `docs/components/rich-text-view.md`, compiled.

use nana_ui::runtime::rich::{
    PaintColor, RichSpanStyle, RichText, RichTextShadow, RichTextStroke, TextStrokePlacement,
};
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{AppContext, DocumentId, RichTextView};

#[test]
fn the_documented_rich_text_builds_and_mounts() {
    let line = RichText::builder()
        .plain("今天也")
        .push(
            "辛苦了",
            RichSpanStyle::new()
                .bold()
                .color(PaintColor::srgb([1.0, 0.4, 0.5, 1.0])),
        )
        .build();
    let black = PaintColor::srgb([0.0, 0.0, 0.0, 1.0]);
    let outlined = RichText::new("注意看").with_span(
        0..9,
        RichSpanStyle::new()
            .stroke(RichTextStroke::new(3.0, black).placement(TextStrokePlacement::Under))
            .shadow(RichTextShadow::new([2.0, 2.0], 6.0, black).spread(1.0))
            .underline(),
    );
    let _ = widget(RichTextView::new(outlined.clone()));
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let view = cx
        .create_component(document, RichTextView::new(line).font_size(20.0))
        .unwrap();
    assert_eq!(cx.world().text(view.stable_id()), Some("今天也辛苦了"));
    cx.set_component(view, RichTextView::new(outlined.clone()))
        .unwrap();
    assert_eq!(cx.world().rich_text(view.stable_id()), Some(&outlined));
}
