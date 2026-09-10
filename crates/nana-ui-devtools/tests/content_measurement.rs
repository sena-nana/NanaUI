#![cfg(feature = "agent")]

use nana_ui::runtime::{
    Button, ComponentGeometry, DocumentId, LengthSpec, NativeMarkdown, RuntimeDocument, Stack,
};
use nana_ui_devtools::agent::RuntimeAgentSession;
use std::sync::Arc;

#[test]
fn native_markdown_wrapped_bubble_keeps_copy_below_its_drawing() {
    for width in [960, 1440] {
        let id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(id);
        let cx = document.context_mut();
        let root = cx
            .create_component(
                id,
                Stack::fill_column(4.0).with_layout(|layout| {
                    layout.padding_left = Some(LengthSpec::Px(260.0));
                    layout.padding_right = Some(LengthSpec::Px(52.0));
                }),
            )
            .unwrap();
        let bubble = cx
            .create_detached_component(
                id,
                Stack::column(6.0)
                    .padding_xy(14.0, 10.0)
                    .width(LengthSpec::FitContent)
                    .with_layout(|layout| {
                        layout.max_width = Some(LengthSpec::Percent(76.0));
                        layout.margin_left = Some(LengthSpec::Auto);
                    }),
            )
            .unwrap();
        cx.append_child(root, bubble).unwrap();
        let mut view = NativeMarkdown::parse("NATIVE_PARITY_FORK_A_20260903");
        let layout = Arc::make_mut(&mut view.style.layout);
        layout.width = Some(LengthSpec::FitContent);
        layout.max_width = Some(LengthSpec::Percent(100.0));
        let markdown = cx.create_detached_component(id, view).unwrap();
        cx.assemble_markdown(markdown).unwrap();
        let copy = cx
            .create_detached_component(id, Button::new("复制"))
            .unwrap();
        cx.reconcile_children(
            bubble.stable_id(),
            &[markdown.stable_id(), copy.stable_id()],
        )
        .unwrap();
        let session = RuntimeAgentSession::new(document, width, 600).unwrap();
        let world = session.document().context().world();
        let bounds = world.layout_box(markdown.stable_id()).unwrap();
        let action = world.layout_box(copy.stable_id()).unwrap();
        let ComponentGeometry::NativeMarkdown { drawing, .. } =
            world.component_geometry(markdown.stable_id()).unwrap()
        else {
            panic!("expected markdown drawing")
        };
        assert!(
            drawing.height <= bounds.height + 0.01,
            "real font drawing exceeds layout at {width}: {bounds:?}, drawing height {}",
            drawing.height
        );
        assert!(
            action.y >= bounds.y + drawing.height,
            "copy intersects real font drawing at {width}: {action:?}, {bounds:?}, drawing height {}",
            drawing.height
        );
    }
}
