//! 下拉开关事件覆盖同值提交、外部关闭与真实 Escape 输入。
use super::*;
use std::sync::Mutex;

/// 事件序列包含所有开关入口，同值提交与关闭后的重复请求保持明确语义。
#[test]
fn select_open_events_include_same_option_commits_and_dismissals() {
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    let select = cx
        .create_component(
            document,
            Select::new(Some("a")).options([
                crate::SelectOption::new("a", "Alpha"),
                crate::SelectOption::new("b", "Beta"),
            ]),
        )
        .unwrap();
    let states = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&states);
    cx.on(select, move |_, event: &crate::SelectOpenedChanged, _| {
        sink.lock().unwrap().push(event.opened);
    })
    .unwrap();
    cx.focus_node(document, select.stable_id()).unwrap();
    cx.toggle_select(select).unwrap();
    let work = cx.take_system_work();
    cx.resolve_styles(&work.style).unwrap();
    cx.shape_text(&work.text, &mut crate::MeasureTextShaper)
        .unwrap();
    cx.layout_document(document, crate::LayoutViewport::new(600.0, 400.0))
        .unwrap();
    let menu = cx.open_select_menu(select.stable_id()).unwrap();
    let bounds = menu.options[0].bounds;
    assert!(
        cx.activate_select_at(
            select,
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
        .unwrap()
    );
    assert_eq!(
        cx.read(select, |select| select.value.clone())
            .unwrap()
            .as_deref(),
        Some("a")
    );
    cx.toggle_select(select).unwrap();
    assert!(cx.commit_focused_select(document).unwrap());
    cx.toggle_select(select).unwrap();
    assert!(cx.dismiss_focused_field_options(document).unwrap());
    cx.toggle_select(select).unwrap();
    cx.dismiss_detached_menus(None).unwrap();

    let mut input = crate::HeadlessInput::bind(&mut cx, document);
    for key in ["ArrowDown", "Escape"] {
        input
            .press(
                &mut cx,
                nana_ui_input::KeyInput::named(
                    key,
                    key,
                    nana_ui_input::KeyState::Pressed,
                    Default::default(),
                ),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            cx.read(select, |select| select.opened).unwrap(),
            key == "ArrowDown"
        );
    }
    assert!(!cx.dismiss_focused_field_options(document).unwrap());
    assert_eq!(
        *states.lock().unwrap(),
        [
            true, false, true, false, true, false, true, false, true, false
        ]
    );
}
