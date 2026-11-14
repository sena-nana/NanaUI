use nana_ui::runtime::{DocumentId, LengthSpec, ListItem, RuntimeDocument, Stack};
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};
use std::sync::Arc;

#[test]
fn issue205_list_row_paint_matches_before_migration() {
    if !offscreen::pixels_available() {
        assert!(std::env::var_os("NANA_COMPONENT_REQUIRE_GPU").is_none());
        return;
    }
    let document = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(document);
    let cx = doc.context_mut();
    cx.set_reduced_motion(true);
    let root = cx
        .create_component(
            document,
            Stack::column(8.0)
                .padding(12.0)
                .width(LengthSpec::Px(320.0)),
        )
        .unwrap();
    for width in [280.0, 160.0, 48.0] {
        let mut row = ListItem::new("Document").note("note").detail("Ctrl+O");
        Arc::make_mut(&mut row.style.layout).width = Some(LengthSpec::Px(width));
        let row = cx.create_detached_component(document, row).unwrap();
        cx.append_child(root, row).unwrap();
    }
    let mut session = RuntimeAgentSession::new(doc, 340, 180).unwrap();
    let (size, initial) = session.screenshot_rgba().unwrap();
    assert_eq!(initial, session.screenshot_rgba().unwrap().1);
    if let Some(path) = std::env::var_os("NANA_ROW_RECORD_BASELINE") {
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path.with_extension("rgba"), &initial).unwrap();
        offscreen::write_png(&path.with_extension("png"), size, &initial).unwrap();
    }
    if let Some(path) = std::env::var_os("NANA_ROW_COMPARE_BASELINE") {
        let expected =
            std::fs::read(std::path::PathBuf::from(path).with_extension("rgba")).unwrap();
        assert!(
            initial == expected,
            "row pixels differ from preserved pre-migration fixture"
        );
    }
}
