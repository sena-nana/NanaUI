#![recursion_limit = "256"]

//! Real-window input acceptance probe for IME composition (#271) and pen
//! input (#276).
//!
//! The window holds one focused `TextInput` ("IME editor") above an empty
//! stroke area. Every canonical input event the window's router delivers is
//! printed to stdout as one JSON line (`"event":"input"`), followed by the
//! editor's value, focus and caret whenever a presented frame changes them
//! (`"event":"editor"`). `scripts/validate-input-acceptance.ps1` drives it
//! with the system IMEs and Windows pen injection; run it by hand with a real
//! pen or IME and keep the stdout as the event trace.
//!
//! Stdin commands: `focus` (focus the editor), `clear` (empty it), `quit`.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use nana_ui::runtime::{FrameworkError, MutationQueue, NodeStyle, StableNodeId, Stack, TextInput};
use nana_ui::{
    ApplicationState, ApplicationWindow, RoutedInput, RuntimeApplication, RuntimeProgramContext,
    RuntimeProgramUpdate, WindowDescriptor, run_runtime,
};
use nana_ui_core::{LayoutStyle, LengthSpec};
use nana_ui_platform::{CompositionInput, InputPayload, WindowId};
use serde_json::{Value, json};

enum Message {
    Focus,
    Clear,
    Quit,
}

struct Probe {
    editor: Option<StableNodeId>,
    last: Option<Value>,
    focus_requested: bool,
}

fn emit(value: Value) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn payload_json(payload: &InputPayload) -> Value {
    match payload {
        InputPayload::Pointer(p) => json!({
            "kind": "pointer",
            "phase": format!("{:?}", p.phase),
            "pointer_id": p.pointer_id.0,
            "pointer_type": format!("{:?}", p.pointer_type),
            "x": p.x, "y": p.y,
            "button": p.button, "buttons": p.buttons,
            "pressure": p.pressure,
            "tangential_pressure": p.tangential_pressure,
            "tilt_x": p.tilt_x, "tilt_y": p.tilt_y, "twist": p.twist,
            "primary": p.is_primary,
        }),
        InputPayload::PointerEnter { pointer_id, x, y } => {
            json!({"kind": "pointer_enter", "pointer_id": pointer_id.0, "x": x, "y": y})
        }
        InputPayload::PointerLeave { pointer_id } => {
            json!({"kind": "pointer_leave", "pointer_id": pointer_id.0})
        }
        InputPayload::Wheel(w) => json!({"kind": "wheel", "pointer_id": w.pointer_id.0}),
        InputPayload::Key(k) => json!({
            "kind": "key",
            "physical": k.physical.0, "logical": k.logical.0,
            "state": format!("{:?}", k.state), "repeat": k.repeat,
        }),
        InputPayload::Text(text) => json!({"kind": "text", "text": format!("{text:?}")}),
        InputPayload::Composition(c) => match c {
            CompositionInput::Update { text, selection } => json!({
                "kind": "composition", "op": "update", "text": text,
                "selection": selection.map(|(a, b)| [a, b]),
            }),
            CompositionInput::Commit(text) => {
                json!({"kind": "composition", "op": "commit", "text": text})
            }
            other => json!({"kind": "composition", "op": format!("{other:?}")}),
        },
        InputPayload::Focus { focused } => json!({"kind": "focus", "focused": focused}),
        other => json!({"kind": format!("{other:?}")}),
    }
}

impl ApplicationState for Probe {
    type Message = Message;
    type Error = FrameworkError;

    fn initialize(context: &RuntimeProgramContext<Message>) -> Result<Self, Self::Error> {
        emit(json!({
            "event": "adapter",
            "backend": format!("{:?}", context.gpu().capabilities().backend()),
            "pid": std::process::id(),
        }));
        let context = context.clone();
        std::thread::spawn(move || {
            for line in io::stdin().lock().lines().map_while(Result::ok) {
                let message = match line.trim_start_matches('\u{feff}').trim() {
                    "focus" => Message::Focus,
                    "clear" => Message::Clear,
                    "quit" => Message::Quit,
                    _ => continue,
                };
                context.dispatch(message);
            }
        });
        Ok(Self {
            editor: None,
            last: None,
            focus_requested: false,
        })
    }

    fn build(
        &mut self,
        window: &mut ApplicationWindow,
        _context: &RuntimeProgramContext<Message>,
    ) -> Result<(), Self::Error> {
        let document = window.document.document();
        let cx = window.document.context_mut();
        let root = cx.create_component(document, Stack::column(12.0))?;
        let editor = cx.create_component(document, TextInput::new("").label("IME editor"))?;
        // The stroke area: an empty node the pen draws over, so pointer
        // events have a target that is not the editor.
        let pad_layout = LayoutStyle {
            width: Some(LengthSpec::Px(480.0)),
            height: Some(LengthSpec::Px(260.0)),
            background: Some([0.16, 0.17, 0.2, 1.0]),
            ..Default::default()
        };
        let pad = cx.create_component(
            document,
            Stack::column(0.0).style(NodeStyle {
                layout: std::sync::Arc::new(pad_layout),
                ..Default::default()
            }),
        )?;
        let mut queue = MutationQueue::new();
        queue.insert(root.stable_id(), editor.stable_id(), None);
        queue.insert(root.stable_id(), pad.stable_id(), None);
        queue.request_focus(document, Some(editor.stable_id()));
        cx.commit_mutations(queue)?;
        self.editor = Some(editor.stable_id());
        Ok(())
    }

    fn update(
        &mut self,
        message: Message,
        windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        let Some(window) = windows.get_mut(&WindowId::PRIMARY) else {
            return RuntimeProgramUpdate::default();
        };
        let Some(editor) = self.editor else {
            return RuntimeProgramUpdate::default();
        };
        let document = window.document.document();
        let cx = window.document.context_mut();
        match message {
            Message::Quit => return RuntimeProgramUpdate::exit(),
            Message::Focus => {
                let mut queue = MutationQueue::new();
                queue.request_focus(document, Some(editor));
                let _ = cx.commit_mutations(queue);
                self.focus_requested = true;
            }
            Message::Clear => {
                let _ = cx.update_component(
                    nana_ui::runtime::Entity::<TextInput>::from_stable_id(editor),
                    |input, _| input.state = nana_ui::runtime::TextInputState::new(""),
                );
            }
        }
        RuntimeProgramUpdate::redraw(WindowId::PRIMARY)
    }

    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        _windows: &mut HashMap<WindowId, ApplicationWindow>,
        _context: &RuntimeProgramContext<Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let meta = input.event.metadata;
        emit(json!({
            "event": "input",
            "window": id.0,
            "device": meta.device.0,
            "sequence": meta.sequence.0,
            "timestamp_ns": meta.timestamp.0,
            "hit": input.pointer_hit.map(|node| format!("{node:?}")),
            "payload": payload_json(&input.event.payload),
        }));
        Ok(RuntimeProgramUpdate::redraw(id))
    }

    fn presented(
        &mut self,
        window: &mut ApplicationWindow,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        let Some(editor) = self.editor else {
            return RuntimeProgramUpdate::default();
        };
        let document = window.document.document();
        let world = window.document.context().world();
        let state = world.text_input(editor).map(|input| {
            json!({
                "value": input.value.to_owned(),
                "focused": world.focused(document) == Some(editor),
            })
        });
        let current = json!({"event": "editor", "state": state});
        if self.last.as_ref() != Some(&current) {
            emit(current.clone());
            self.last = Some(current);
        }
        RuntimeProgramUpdate::default()
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<RuntimeApplication<Probe>>(
        WindowDescriptor::new("NanaUI Input Acceptance Probe")
            .initial_size(560.0, 400.0)
            .minimum_size(560.0, 400.0),
    )
}
