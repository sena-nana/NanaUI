//! 普通文本框保留双击全选、多行选词、三击全选及部分拖选的输入契约。

use super::*;
use crate::{MutationQueue, StableNodeId, TextArea, TextInput, TextSelection};

#[test]
fn text_editor_pointer_selection_supports_field_select_all_word_and_area_drag() {
    let mut context = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let input = context
        .create_component(document, TextInput::new("abcdef"))
        .unwrap();
    let area = context
        .create_component(document, TextArea::new("first second\nthird"))
        .unwrap();
    let input_node = input.stable_id();
    let area_node = area.stable_id();

    let mut mutations = MutationQueue::new();
    for node in [input_node, area_node] {
        mutations.write_layout(
            node,
            crate::LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 240.0,
                height: 100.0,
            },
        );
    }
    context.commit_mutations(mutations).unwrap();
    let work = context.take_system_work();
    context.resolve_styles(&work.style).unwrap();
    context
        .shape_text(&work.text, &mut crate::MeasureTextShaper)
        .unwrap();

    let press = |context: &mut AppContext, node: StableNodeId, x: f32, at: u64| -> TextSelection {
        context
            .text_editor_pointer_press(
                document,
                node,
                1,
                x,
                16.0,
                false,
                false,
                std::time::Duration::from_millis(at),
                &mut crate::MeasureTextShaper,
            )
            .unwrap();
        context.world().text_input(node).unwrap().selection
    };

    assert!(context.focus_node(document, input_node).unwrap());
    press(&mut context, input_node, 0.0, 0);
    let selection = press(&mut context, input_node, 0.0, 100);
    assert_eq!(selection, TextSelection::new(0, "abcdef".len()));
    context.text_editor_pointer_release(1);

    assert!(context.focus_node(document, area_node).unwrap());
    press(&mut context, area_node, 0.0, 1000);
    let selection = press(&mut context, area_node, 0.0, 1100);
    assert_eq!(selection, TextSelection::new(0, "first".len()));
    let selection = press(&mut context, area_node, 0.0, 1200);
    assert_eq!(
        selection,
        TextSelection::new(0, "first second\nthird".len())
    );
    context.text_editor_pointer_release(1);

    // 单击后拖动保留部分选区。
    context.select_focused_text_range(document, 0, 0).unwrap();
    let selection = press(&mut context, area_node, 0.0, 2000);
    assert!(selection.is_collapsed());
    assert!(
        context
            .text_editor_pointer_drag(document, 1, 24.0, 16.0, &mut crate::MeasureTextShaper)
            .unwrap()
    );
    let selection = context.world().text_input(area_node).unwrap().selection;
    let range = selection.ordered();
    assert_eq!(range.start, 0);
    assert!(range.end > 0 && range.end < "first second\nthird".len());
    context.text_editor_pointer_release(1);

    // 连击的时间和位移阈值使相互独立的按下保持单击行为。
    assert!(context.focus_node(document, input_node).unwrap());
    let selection = press(&mut context, input_node, 0.0, 3000);
    assert!(selection.is_collapsed());
    let selection = press(&mut context, input_node, 0.0, 3600);
    assert!(selection.is_collapsed());
    let selection = press(&mut context, input_node, 0.0, 4200);
    assert!(selection.is_collapsed());
    let selection = press(&mut context, input_node, 20.0, 4800);
    assert!(selection.is_collapsed());
    let selection = press(&mut context, input_node, 0.0, 5000);
    assert!(selection.is_collapsed());
    let selection = press(&mut context, input_node, 20.0, 5100);
    assert!(selection.is_collapsed());
}
