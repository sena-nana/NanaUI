// 让位几何与稳定落点回归；测试和生产手势共用 ReorderList 模型。

fn gap_rows(heights: &[f32], spacing: f32) -> Vec<LayoutBox> {
    let mut y = 100.0;
    heights
        .iter()
        .map(|height| {
            let row = LayoutBox {
                x: 0.0,
                y,
                width: 180.0,
                height: *height,
            };
            y += height + spacing;
            row
        })
        .collect()
}

fn gap_list(count: usize) -> ReorderList {
    ReorderList::new((0..count).map(|i| ReorderItem::new(i.to_string(), "")))
        .live_rows(true)
        .drag_preview(true)
        .drag_gap(true)
        .spacing(10.0)
}

fn gap_drag(
    list: &mut ReorderList,
    rows: &[LayoutBox],
    source: usize,
    y: f32,
) -> ReorderDragVisual {
    list.apply_pointer_with_rows(
        ReorderListPointer::Down {
            x: 5.0,
            y: rows[source].y + 3.0,
        },
        rows,
        &[],
    );
    list.apply_pointer_with_rows(ReorderListPointer::Move { x: 15.0, y }, rows, &[]);
    list.drag_visual().unwrap()
}

#[test]
fn gap_moves_rows_down_when_source_moves_up_and_inserts_at_slot_top() {
    let rows = gap_rows(&[20.0, 40.0, 30.0], 10.0);
    let mut list = gap_list(3);
    let drag = gap_drag(&mut list, &rows, 2, 100.0);
    assert_eq!(drag.gap_offsets(&rows, 10.0), Some(vec![40.0, 40.0, 0.0]));
    let line = drag.insert_line(bounds(), &rows).unwrap();
    assert_eq!(line.y, 100.0 - INSERT_THICKNESS);
    assert_eq!(
        list.apply_pointer_with_rows(ReorderListPointer::Up { x: 15.0, y: 100.0 }, &rows, &[]),
        Some(ReorderListEvent::Reorder {
            source: "2".into(),
            before: Some("0".into())
        })
    );
}

#[test]
fn gap_moves_rows_up_when_source_moves_down_and_uses_its_own_height() {
    let rows = gap_rows(&[20.0, 40.0, 30.0], 10.0);
    let mut list = gap_list(3);
    let drag = gap_drag(&mut list, &rows, 0, 240.0);
    assert_eq!(drag.gap_offsets(&rows, 10.0), Some(vec![0.0, -30.0, -30.0]));
    assert_eq!(
        drag.insert_line(bounds(), &rows).unwrap().y,
        190.0 - INSERT_THICKNESS
    );
    assert_eq!(
        list.apply_pointer_with_rows(ReorderListPointer::Up { x: 15.0, y: 240.0 }, &rows, &[]),
        Some(ReorderListEvent::Reorder {
            source: "0".into(),
            before: None
        })
    );
}

#[test]
fn gap_adjacent_targets_use_stable_layout_midpoints_with_different_heights() {
    let rows = gap_rows(&[20.0, 40.0, 30.0], 10.0);
    let mut list = gap_list(3);
    let drag = gap_drag(&mut list, &rows, 0, 155.0);
    assert_eq!(drag.gap_offsets(&rows, 10.0), Some(vec![0.0, -30.0, 0.0]));
    assert_eq!(
        drag.insert_line(bounds(), &rows).unwrap().y,
        150.0 - INSERT_THICKNESS
    );
    list.cancel();
    let drag = gap_drag(&mut list, &rows, 2, 149.0);
    assert_eq!(drag.gap_offsets(&rows, 10.0), Some(vec![0.0, 40.0, 0.0]));
    assert_eq!(
        drag.insert_line(bounds(), &rows).unwrap().y,
        130.0 - INSERT_THICKNESS
    );
}

#[test]
fn gap_no_change_and_single_row_have_zero_offsets_and_no_insert_line() {
    let rows = gap_rows(&[20.0, 40.0, 30.0], 10.0);
    let mut list = gap_list(3);
    let drag = gap_drag(&mut list, &rows, 1, 185.0);
    assert_eq!(drag.gap_offsets(&rows, 10.0), Some(vec![0.0; 3]));
    assert_eq!(drag.insert_line(bounds(), &rows), None);
    let rows = gap_rows(&[20.0], 10.0);
    let drag = gap_drag(&mut gap_list(1), &rows, 0, 200.0);
    assert_eq!(drag.gap_offsets(&rows, 10.0), Some(vec![0.0]));
    assert_eq!(drag.insert_line(bounds(), &rows), None);
}

#[test]
fn gap_requires_explicit_opt_in_live_rows_preview_and_plain_reorder() {
    let rows = gap_rows(&[20.0, 40.0, 30.0], 10.0);
    assert!(!ReorderList::default().drag_gap);
    for mut list in [
        gap_list(3).drag_gap(false),
        gap_list(3).live_rows(false),
        gap_list(3).drag_preview(false),
        gap_list(3).tree_drop(true),
    ] {
        let drag = gap_drag(&mut list, &rows, 0, 240.0);
        assert_eq!(drag.gap_offsets(&rows, 10.0), None);
    }
}

#[test]
fn gap_skips_disabled_drop_targets_and_preserves_zero_spacing() {
    let rows = gap_rows(&[20.0, 40.0, 30.0], 0.0);
    let mut list = gap_list(3).spacing(0.0);
    list.items[1].drop_target = false;
    let drag = gap_drag(&mut list, &rows, 0, 125.0);
    assert_eq!(drag.gap_offsets(&rows, 0.0), Some(vec![0.0, -20.0, 0.0]));
    assert_eq!(
        drag.insert_line(bounds(), &rows).unwrap().y,
        140.0 - INSERT_THICKNESS
    );
}

#[test]
fn preview_geometry_keeps_the_insert_line_and_removes_the_dragged_frame() {
    let mut fixture = ButtonRows::new(false);
    let rows = fixture.rows.clone();
    fixture
        .context
        .update_component(fixture.list, |list, _| {
            list.drag_preview = true;
            list.drag_gap = true;
            gap_drag(list, &rows, 0, rows.last().unwrap().y + 60.0);
        })
        .unwrap();
    match fixture
        .context
        .world()
        .component_geometry(fixture.list.stable_id())
        .unwrap()
    {
        crate::ComponentGeometry::ReorderList {
            insert, dragged, ..
        } => {
            assert!(insert.is_some());
            assert!(dragged.is_none());
        }
        _ => panic!("列表几何"),
    }
}

#[test]
fn preview_is_opt_in_and_tracks_the_grab_offset_after_threshold() {
    let mut list = sample();
    apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
    apply(&mut list, ReorderListPointer::Move { x: 60.0, y: 80.0 });
    assert_eq!(list.drag_visual().unwrap().preview_offset, None);
    let mut list = sample().drag_preview(true);
    apply(&mut list, ReorderListPointer::Down { x: 40.0, y: 12.0 });
    apply(&mut list, ReorderListPointer::Move { x: 42.0, y: 13.0 });
    assert!(list.drag_visual().is_none());
    apply(&mut list, ReorderListPointer::Move { x: 60.0, y: 80.0 });
    assert_eq!(
        list.drag_visual().unwrap().preview_offset,
        Some([20.0, 68.0])
    );
    list.cancel();
    assert!(list.drag_visual().is_none());
}
