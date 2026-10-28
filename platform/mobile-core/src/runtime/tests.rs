use std::sync::{Arc, Mutex};
use std::time::Instant;

use nana_ui::runtime::view::{entity_ref, widget, with_refs};
use nana_ui::runtime::{Activate, Button, Entity, TextChanged, TextInput};
use nana_ui_platform::{
    CompositionInput, SharedClipboardHost, TextInputContext, UnsupportedClipboard, shared_clipboard,
};

use super::*;
use crate::viewport::SafeArea;

#[derive(Default)]
struct Recorded {
    text_input: Vec<bool>,
}

struct TestPlatform(Arc<Mutex<Recorded>>);

impl MobilePlatform for TestPlatform {
    fn set_text_input(&mut self, state: Option<&TextInputContext>) {
        self.0.lock().unwrap().text_input.push(state.is_some());
    }

    fn clipboard(&self) -> SharedClipboardHost {
        shared_clipboard(UnsupportedClipboard)
    }
}

struct Fixture {
    runtime: MobileRuntime,
    presses: Arc<Mutex<u32>>,
    typed: Arc<Mutex<String>>,
    recorded: Arc<Mutex<Recorded>>,
    button: Entity<Button>,
    field: Entity<TextInput>,
}

const SAFE: SafeArea = SafeArea {
    top: 177.0,
    right: 0.0,
    bottom: 102.0,
    left: 0.0,
};

fn fixture() -> Fixture {
    let presses = Arc::new(Mutex::new(0));
    let typed = Arc::new(Mutex::new(String::new()));
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let viewport = MobileViewport::new((1179, 2556), 3.0, SAFE);
    let mut runtime = MobileRuntime::new(
        viewport,
        MobileAppearance::default(),
        Box::new(TestPlatform(recorded.clone())),
    )
    .expect("mobile runtime");
    let (_, (button, field)) = {
        let (presses, typed) = (presses.clone(), typed.clone());
        runtime
            .mount(move || {
                let (button, field) = (entity_ref(), entity_ref());
                with_refs(
                    widget(List::new()).key("page").children((
                        widget(Button::new("Tap"))
                            .key("button")
                            .entity_ref(button)
                            .on(move |_: &Activate| {
                                *presses.lock().unwrap() += 1;
                            }),
                        widget(TextInput::new(""))
                            .key("field")
                            .entity_ref(field)
                            .on_cx(move |field: &mut TextInput, event: &TextChanged, _| {
                                field.state.replace_value(event.value.clone());
                                *typed.lock().unwrap() = event.value.to_string();
                            }),
                    )),
                    (button, field),
                )
            })
            .expect("mount")
    };
    Fixture {
        runtime,
        presses,
        typed,
        recorded,
        button,
        field,
    }
}

impl Fixture {
    /// The centre of `id`'s box in physical px.
    fn centre(&self, id: nana_ui::runtime::StableNodeId) -> (f32, f32) {
        let layout = self
            .runtime
            .document()
            .context()
            .world()
            .layout_box(id)
            .expect("layout");
        let scale = self.runtime.viewport().scale;
        (
            (layout.x + layout.width * 0.5) * scale,
            (layout.y + layout.height * 0.5) * scale,
        )
    }

    fn tap(&mut self, (x, y): (f32, f32)) {
        assert!(self.runtime.push_touch(TouchPhase::Down, x, y, 7).unwrap());
        assert!(self.runtime.push_touch(TouchPhase::Up, x, y, 7).unwrap());
    }

    /// Present whatever is due, as one display-link tick does.
    fn settle(&mut self) {
        while self.runtime.frame_demand() == FrameDemand::Now {
            self.runtime.prepare_frame(Instant::now()).unwrap();
        }
    }
}

#[test]
fn a_settled_document_asks_for_no_frames() {
    let mut fixture = fixture();
    assert_eq!(fixture.runtime.frame_demand(), FrameDemand::Now);
    assert!(fixture.runtime.prepare_frame(Instant::now()).unwrap());
    fixture.settle();
    assert_eq!(fixture.runtime.frame_demand(), FrameDemand::Idle);
    assert!(!fixture.runtime.prepare_frame(Instant::now()).unwrap());
}

#[test]
fn the_root_pads_the_application_into_the_safe_area() {
    let fixture = fixture();
    let layout = fixture
        .runtime
        .document()
        .context()
        .world()
        .layout_box(fixture.button.stable_id())
        .expect("button layout");
    assert!(layout.y >= SAFE.top / 3.0, "{layout:?}");
}

#[test]
fn rotating_moves_the_padding_to_the_sides() {
    let mut fixture = fixture();
    fixture.settle();
    let landscape = SafeArea {
        top: 0.0,
        right: 177.0,
        bottom: 63.0,
        left: 177.0,
    };
    fixture
        .runtime
        .set_viewport(MobileViewport::new((2556, 1179), 3.0, landscape))
        .unwrap();
    assert_eq!(fixture.runtime.frame_demand(), FrameDemand::Now);
    fixture.settle();
    let layout = fixture
        .runtime
        .document()
        .context()
        .world()
        .layout_box(fixture.button.stable_id())
        .expect("button layout");
    assert!(layout.x >= 59.0, "{layout:?}");
}

#[test]
fn a_tap_activates_and_the_frame_after_it_settles() {
    let mut fixture = fixture();
    fixture.settle();
    let centre = fixture.centre(fixture.button.stable_id());
    fixture.tap(centre);
    assert_eq!(*fixture.presses.lock().unwrap(), 1);
    fixture.settle();
    assert_ne!(fixture.runtime.frame_demand(), FrameDemand::Now);
}

#[test]
fn a_second_finger_is_not_followed() {
    let mut fixture = fixture();
    let (x, y) = fixture.centre(fixture.button.stable_id());
    assert!(
        fixture
            .runtime
            .push_touch(TouchPhase::Down, x, y, 1)
            .unwrap()
    );
    assert!(
        !fixture
            .runtime
            .push_touch(TouchPhase::Down, x, y, 2)
            .unwrap()
    );
    assert!(!fixture.runtime.push_touch(TouchPhase::Up, x, y, 2).unwrap());
    assert!(fixture.runtime.push_touch(TouchPhase::Up, x, y, 1).unwrap());
    assert_eq!(*fixture.presses.lock().unwrap(), 1);
}

#[test]
fn focusing_the_field_raises_the_native_text_input() {
    let mut fixture = fixture();
    let centre = fixture.centre(fixture.field.stable_id());
    fixture.tap(centre);
    assert!(fixture.runtime.text_input().is_some());
    assert_eq!(
        fixture.recorded.lock().unwrap().text_input.last(),
        Some(&true)
    );
}

#[test]
fn composition_from_the_proxy_commits_into_the_field() {
    let mut fixture = fixture();
    let centre = fixture.centre(fixture.field.stable_id());
    fixture.tap(centre);
    fixture
        .runtime
        .push_composition(CompositionInput::Update {
            text: "ni".into(),
            selection: None,
        })
        .unwrap();
    let marked = fixture.runtime.text_input().expect("editor");
    assert_eq!(marked.text, "ni");
    assert_eq!(marked.marked, Some((0, 2)));
    fixture
        .runtime
        .push_composition(CompositionInput::Commit("你".into()))
        .unwrap();
    assert_eq!(*fixture.typed.lock().unwrap(), "你");
    let committed = fixture.runtime.text_input().expect("editor");
    assert_eq!(committed.marked, None);
    assert_eq!(committed.selection, (3, 3));
}

#[test]
fn tapping_away_commits_the_preedit_and_hides_the_keyboard() {
    let mut fixture = fixture();
    let field = fixture.centre(fixture.field.stable_id());
    fixture.tap(field);
    fixture
        .runtime
        .push_composition(CompositionInput::Update {
            text: "ni".into(),
            selection: None,
        })
        .unwrap();
    let button = fixture.centre(fixture.button.stable_id());
    fixture.tap(button);
    assert_eq!(*fixture.typed.lock().unwrap(), "ni");
    assert!(fixture.runtime.text_input().is_none());
    assert_eq!(
        fixture.recorded.lock().unwrap().text_input.last(),
        Some(&false)
    );
}

#[test]
fn an_appearance_change_presents_once() {
    let mut fixture = fixture();
    fixture.settle();
    fixture.runtime.set_appearance(MobileAppearance {
        dark: true,
        text_scale: 1.0,
    });
    assert_eq!(fixture.runtime.frame_demand(), FrameDemand::Now);
    assert!(fixture.runtime.prepare_frame(Instant::now()).unwrap());
    fixture.settle();
    assert_eq!(fixture.runtime.frame_demand(), FrameDemand::Idle);
}
