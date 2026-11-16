//! Row tools: revealed by the pointer and by keyboard focus inside the row,
//! so Tab reaches them whether they yield or reserve their width at rest.

use super::*;
use crate::view::{
    AnyView, EntityRef, IntoView, button, column, entity_ref, icon_button, list_item, widget,
    with_refs,
};
use crate::{Button, Entity, IconButton, LayoutViewport, ListItem, RowToolsRest, SidebarRow};
use nana_ui_core::Icon;

struct Fixture {
    cx: AppContext,
    input: TestInput,
    doc: DocumentId,
    before: StableNodeId,
    row: StableNodeId,
    tool: StableNodeId,
    after: StableNodeId,
}

impl Fixture {
    fn new(row: impl Fn(EntityRef<IconButton>) -> AnyView) -> Self {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let root = cx.create_component(doc, crate::Stack::column(0.0)).unwrap();
        let (_, (before, tool, after)) = cx
            .mount_view(root.stable_id(), || {
                let tool_ref = entity_ref::<IconButton>();
                let before = entity_ref::<Button>();
                let after = entity_ref::<Button>();
                let view = column().children((
                    button("前").entity_ref(before),
                    row(tool_ref),
                    button("后").entity_ref(after),
                ));
                with_refs(view, (before, tool_ref, after))
            })
            .unwrap();
        let tool = tool.stable_id();
        let row = cx.world().parent_id(tool).unwrap();
        let mut fixture = Self {
            cx,
            input: TestInput::default(),
            doc,
            before: before.stable_id(),
            row,
            tool,
            after: after.stable_id(),
        };
        fixture.layout();
        fixture
    }

    fn layout(&mut self) {
        self.cx
            .layout_document(self.doc, LayoutViewport::new(320.0, 240.0))
            .unwrap();
        self.cx.rebuild_hit_test(self.doc);
    }

    fn tab(&mut self) -> Option<StableNodeId> {
        self.tab_with(false)
    }

    fn back_tab(&mut self) -> Option<StableNodeId> {
        self.tab_with(true)
    }

    fn tab_with(&mut self, shift: bool) -> Option<StableNodeId> {
        self.input
            .dispatch(
                &mut self.cx,
                self.doc,
                &key_fixture! {
                    pressed: true,
                    key: "Tab".into(),
                    code: "Tab".into(),
                    text: None,
                    repeat: false,
                    modifiers: InputModifiers {
                        shift,
                        ..InputModifiers::default()
                    },
                },
            )
            .unwrap();
        self.layout();
        self.cx.world().focused(self.doc)
    }

    fn hidden(&self) -> bool {
        self.cx.world().node_style(self.tool).unwrap().layout.hidden
    }

    fn opacity(&self) -> Option<f32> {
        self.cx
            .world()
            .node_style(self.tool)
            .unwrap()
            .layout
            .opacity
    }

    fn trailing(&self) -> Option<StableNodeId> {
        match self
            .cx
            .world()
            .widget_visual_as::<crate::ListItemVisual>(self.row)
        {
            Some(visual) => visual.trailing,
            None => panic!("row should draw as a list item"),
        }
    }

    fn hover(&mut self, target: Option<StableNodeId>) {
        self.cx.set_pointer_hover(self.doc, 1, target).unwrap();
        self.layout();
    }
}

fn pin(tool: EntityRef<IconButton>) -> impl IntoView {
    icon_button(Icon::Close, "固定").entity_ref(tool)
}

#[test]
fn yielded_list_item_tools_follow_focus_so_tab_reaches_them() {
    let mut f = Fixture::new(|tool| list_item("模型").tools(pin(tool)).into_any());
    assert!(f.hidden(), "tools yield their width at rest");
    assert_eq!(f.trailing(), None);

    assert_eq!(f.tab(), Some(f.before));
    assert_eq!(f.tab(), Some(f.row));
    assert!(!f.hidden(), "focus on the row reveals its tools");
    assert_eq!(f.trailing(), Some(f.tool));
    assert_eq!(f.tab(), Some(f.tool), "Tab reaches the revealed tool");
    assert!(!f.hidden(), "focus on the tool keeps it shown");
    assert_eq!(f.tab(), Some(f.after));
    assert!(f.hidden(), "focus leaving the row puts the tools away");
    assert_eq!(
        f.back_tab(),
        Some(f.row),
        "Shift+Tab from below skips the put-away tools"
    );
    assert_eq!(f.tab(), Some(f.tool), "and Tab from the row reaches them");
    f.tab();

    f.hover(Some(f.row));
    assert!(!f.hidden(), "the pointer reveals them too");
    f.hover(None);
    assert!(f.hidden());
}

#[test]
fn reserved_list_item_tools_keep_their_place_and_stay_in_tab_order() {
    let mut f = Fixture::new(|tool| {
        widget(ListItem::new("模型").tools_rest(RowToolsRest::Reserve))
            .tools(pin(tool))
            .into_any()
    });
    assert!(!f.hidden());
    assert_eq!(
        f.opacity(),
        Some(0.0),
        "reserved tools are not drawn at rest"
    );
    assert_eq!(f.trailing(), Some(f.tool), "and keep their width");

    f.hover(Some(f.tool));
    assert_eq!(f.opacity(), None);
    f.hover(None);
    assert_eq!(f.opacity(), Some(0.0));

    assert_eq!(f.tab(), Some(f.before));
    assert_eq!(f.tab(), Some(f.row));
    assert_eq!(f.opacity(), None);
    assert_eq!(f.tab(), Some(f.tool));
    assert_eq!(f.opacity(), None);
    assert_eq!(f.tab(), Some(f.after));
    assert_eq!(f.opacity(), Some(0.0));
    assert_eq!(
        f.back_tab(),
        Some(f.tool),
        "reserved tools stay in Tab order"
    );
    assert_eq!(f.opacity(), None);
    f.tab();

    let row = Entity::<ListItem>::from_stable_id(f.row);
    f.cx.update_component(row, |item, _| item.keep_tools = true)
        .unwrap();
    assert_eq!(f.opacity(), None, "kept tools show at rest");
}

#[test]
fn sidebar_row_tools_follow_focus_so_tab_reaches_them() {
    let mut f = Fixture::new(|tool| {
        widget(SidebarRow::new("着色器"))
            .tools(pin(tool))
            .into_any()
    });
    assert!(f.hidden());
    assert_eq!(f.tab(), Some(f.before));
    assert_eq!(f.tab(), Some(f.row));
    assert!(!f.hidden());
    assert_eq!(f.trailing(), Some(f.tool));
    assert_eq!(f.tab(), Some(f.tool));
    assert_eq!(f.tab(), Some(f.after));
    assert!(f.hidden());
}
