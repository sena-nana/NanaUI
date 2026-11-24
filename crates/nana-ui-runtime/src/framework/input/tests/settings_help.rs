//! A settings row's help button: pressing it with a mouse, a touch or the
//! keyboard opens the hint in place under the row and pressing again closes
//! it, through the one input path.

use super::*;
use crate::view::{IntoView, column, settings_row, switch, widget};
use crate::{
    Entity, HintPlacement, IconButton, LayoutViewport, MeasureTextShaper, SettingsGroup,
    SettingsRow,
};

const HINT: &str = "按住快捷键时只显示这一层，松开后回到原来的画面";

struct Fixture {
    cx: AppContext,
    input: TestInput,
    doc: DocumentId,
    row: Entity<SettingsRow>,
}

impl Fixture {
    fn new() -> Self {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let view = cx
            .mount_view_root(doc, || {
                column()
                    .children(
                        widget(SettingsGroup::new().hint_placement(HintPlacement::Help)).children(
                            (
                                settings_row("只显示这一层")
                                    .hint(HINT)
                                    .divided(true)
                                    .control(switch("")),
                                settings_row("下一行").control(switch("")),
                            ),
                        ),
                    )
                    .into_any()
            })
            .unwrap();
        let group = cx.world().node(view.roots()[0]).unwrap().children[0];
        let row = cx.world().node(group).unwrap().children[0];
        let mut fixture = Self {
            cx,
            input: TestInput::default(),
            doc,
            row: Entity::from_stable_id(row),
        };
        fixture.layout();
        fixture
    }

    fn layout(&mut self) {
        let viewport = LayoutViewport::new(480.0, 320.0);
        self.cx.layout_document(self.doc, viewport).unwrap();
        self.cx
            .shape_text_for_layout(self.doc, &mut MeasureTextShaper)
            .unwrap();
        self.cx.layout_document(self.doc, viewport).unwrap();
        self.cx.rebuild_hit_test(self.doc);
    }

    fn button(&self) -> StableNodeId {
        self.cx
            .assembled_slot::<crate::settings::SettingsRowAssembled>(self.row.stable_id())
            .and_then(|s| s.help_slot)
            .expect("a row with a hint behind a help button has one")
    }

    fn open(&self) -> bool {
        self.cx.read(self.row, |row| row.help_open).unwrap()
    }

    fn opened_text_shown(&self) -> bool {
        let text = self
            .cx
            .assembled_slot::<crate::settings::SettingsRowAssembled>(self.row.stable_id())
            .and_then(|s| s.help_text_slot)
            .unwrap();
        !self.cx.world().node_style(text).unwrap().layout.hidden
    }

    fn tap(&mut self, pointer_type: PointerType) {
        let bounds = self.cx.world().canonical_layout_box(self.button()).unwrap();
        let (x, y) = (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        for phase in [PointerPhase::Down, PointerPhase::Up] {
            let gesture = Gesture::from(InputPayload::Pointer(PointerInput {
                pointer_type,
                pointer_id: PointerId(if pointer_type == PointerType::Touch {
                    7
                } else {
                    1
                }),
                ..PointerInput::mouse(phase, x, y)
            }));
            self.input
                .dispatch(&mut self.cx, self.doc, &gesture)
                .unwrap();
        }
        self.layout();
    }

    fn press(&mut self, key: &str, code: &str) {
        for pressed in [true, false] {
            self.input
                .dispatch(
                    &mut self.cx,
                    self.doc,
                    &key_fixture! {
                        pressed,
                        key: key.into(),
                        code: code.into(),
                        text: None,
                        repeat: false,
                        modifiers: InputModifiers::default(),
                    },
                )
                .unwrap();
        }
        self.layout();
    }

    /// Whether the button says it is open, and whether it still previews
    /// the hint on hover.
    fn button_state(&self) -> (Option<bool>, bool) {
        self.cx
            .read(
                Entity::<IconButton>::from_stable_id(self.button()),
                |button| (button.expanded, button.tooltip.is_some()),
            )
            .unwrap()
    }
}

#[test]
fn a_click_on_the_help_button_opens_the_hint_and_another_closes_it() {
    let mut f = Fixture::new();
    let bounds = f.cx.world().canonical_layout_box(f.button()).unwrap();
    assert_eq!(
        (bounds.width, bounds.height),
        (24.0, 24.0),
        "a heading tool's target to aim at"
    );
    assert!(!f.open());
    assert_eq!(f.button_state(), (Some(false), true));
    let tooltip = |f: &Fixture| {
        f.cx.component_tooltip(Entity::<IconButton>::from_stable_id(f.button()))
            .unwrap()
    };
    let preview = tooltip(&f).expect("hovering previews the hint");
    assert_eq!(&*f.cx.read(preview, |tip| tip.label.clone()).unwrap(), HINT);
    assert_eq!(
        f.cx.read(preview, |tip| tip.config.placement).unwrap(),
        nana_ui_core::TooltipPlacement::FollowCursor
    );

    f.tap(PointerType::Mouse);
    assert!(f.open());
    assert_eq!(tooltip(&f), None, "nothing to preview while it is open");
    assert!(f.opened_text_shown());
    assert_eq!(
        f.button_state(),
        (Some(true), false),
        "open, it says so and no longer previews the hint"
    );

    f.tap(PointerType::Mouse);
    assert!(!f.open());
    assert!(!f.opened_text_shown());
    assert_eq!(f.button_state(), (Some(false), true));
}

#[test]
fn a_touch_opens_the_hint_as_a_click_does() {
    let mut f = Fixture::new();
    f.tap(PointerType::Touch);
    assert!(f.open(), "a touch has no hover: the press is how it opens");
    assert!(f.opened_text_shown());
    f.tap(PointerType::Touch);
    assert!(!f.open());
}

#[test]
fn the_keyboard_reaches_the_help_button_and_toggles_the_hint() {
    let mut f = Fixture::new();
    let button = f.button();
    assert!(
        f.cx.world()
            .interaction(button)
            .is_some_and(|interaction| interaction.focusable),
        "the focus can stop on it"
    );
    f.cx.focus_node(f.doc, button).unwrap();
    f.press("Enter", "Enter");
    assert!(f.open());
    let spoken =
        f.cx.world()
            .project_accessibility(f.doc)
            .into_iter()
            .find(|node| node.id == button)
            .unwrap();
    assert_eq!(spoken.label.as_deref(), Some(HINT), "named by the hint");
    assert_eq!(spoken.expanded, Some(true));
    f.press(" ", "Space");
    assert!(!f.open());
}
