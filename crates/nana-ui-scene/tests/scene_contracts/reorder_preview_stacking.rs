//! A live reorder drag's preview stacks over the rows it passes and under an
//! open menu, as the scene orders their paint.

use std::time::Duration;

use nana_ui_core::LengthSpec;
use nana_ui_runtime::view::{button, entity_ref, widget, with_refs};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, MeasureTextShaper, Popover, ReorderItem, ReorderList, StableNodeId,
    Stack,
};
use nana_ui_scene::RuntimeDocument;

const SURFACE_SLOT: u64 = 0xFFFF_FFE0 << 32;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 400.0)
}

fn row() -> Stack {
    Stack::column(0.0).with_layout(|layout| {
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(40.0));
        layout.background = Some([0.2, 0.4, 0.8, 1.0]);
    })
}

#[test]
fn a_dragged_rows_preview_paints_over_its_neighbours_and_under_an_open_menu() {
    let id = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(id);
    let root = runtime
        .context_mut()
        .create_component(id, Stack::column(0.0))
        .unwrap();
    let (_, (popover, list, rows)) = runtime
        .context_mut()
        .mount_view(root.stable_id(), || {
            let popover = entity_ref::<Popover>();
            let list = entity_ref::<ReorderList>();
            let rows = [
                entity_ref::<Stack>(),
                entity_ref::<Stack>(),
                entity_ref::<Stack>(),
            ];
            let view = (
                widget(Popover::new().trigger("菜单").open(true))
                    .entity_ref(popover)
                    .children(button("打开")),
                widget(
                    ReorderList::new(["a", "b", "c"].map(|value| ReorderItem::new(value, value)))
                        .live_rows(true),
                )
                .entity_ref(list)
                .children((
                    widget(row()).entity_ref(rows[0]),
                    widget(row()).entity_ref(rows[1]),
                    widget(row()).entity_ref(rows[2]),
                )),
            );
            with_refs(view, (popover, list, rows))
        })
        .unwrap();
    runtime
        .context_mut()
        .advance_animations(Duration::from_secs(1));
    runtime.flush(viewport(), &mut MeasureTextShaper).unwrap();

    let [a, b, _] = rows.map(|row| row.stable_id());
    let start = runtime.context().world().layout_box(a).unwrap();
    let below = runtime.context().world().layout_box(b).unwrap();
    let (x, y) = (start.x + 20.0, start.y + 20.0);
    let context = runtime.context_mut();
    assert!(
        context
            .begin_reorder_list_pointer(id, 1, list.stable_id(), x, y)
            .unwrap()
    );
    // Over the lower half of `b`: `a` hangs over it and `b` makes room.
    context
        .update_reorder_list_pointer(id, 1, x, below.y + 30.0)
        .unwrap();
    runtime.flush(viewport(), &mut MeasureTextShaper).unwrap();

    let order: Vec<(StableNodeId, u64)> = runtime
        .scene()
        .primitives()
        .map(|primitive| (primitive.node, primitive.id.slot))
        .collect();
    let at = |wanted: &dyn Fn(&(StableNodeId, u64)) -> bool| -> Vec<usize> {
        order
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| wanted(entry).then_some(index))
            .collect()
    };
    let preview = at(&|(node, _)| *node == a);
    let neighbour = at(&|(node, _)| *node == b);
    let surface = at(&|(node, slot)| *node == popover.stable_id() && *slot == SURFACE_SLOT);
    assert!(!preview.is_empty() && !neighbour.is_empty() && !surface.is_empty());
    assert!(
        preview.iter().min() > neighbour.iter().max(),
        "the preview {preview:?} paints under the row it passes {neighbour:?}"
    );
    assert!(
        preview.iter().max() < surface.iter().min(),
        "the preview {preview:?} paints over the open menu {surface:?}"
    );
}
