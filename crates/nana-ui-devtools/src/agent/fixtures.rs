//! Named Runtime documents the CLI can build without a consuming crate.
//!
//! A CLI cannot load an arbitrary Rust application from a path — the document
//! is built by a closure over the author's own component types, and a dlopen
//! plugin ABI would be a second instantiation ABI. These fixtures are the
//! honest substitute for framework-side work; a product drives its own document
//! through [`crate::agent::cli::runtime_main`] from its own dev binary.
//!
//! They are not demo code: the protocol tests drive these same fixtures, so
//! they cannot rot without a test going red.

use nana_ui::runtime::{
    Activate, Button, Card, Checkbox, Dialog, DocumentId, LengthSpec, List, NodeStyle, RangeField,
    RuntimeDocument, ScrollAxes, ScrollView, Stack, Switch, Text, TextInput,
};
use nana_ui_core::{LayoutStyle, PositionSpec};
use std::sync::Arc;

pub struct Fixture {
    pub name: &'static str,
    pub summary: &'static str,
    build: fn(&mut RuntimeDocument, DocumentId),
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "counter",
        summary: "one button and one label; the smallest click target",
        build: counter,
    },
    Fixture {
        name: "form-controls",
        summary: "switch, checkbox, text input and range; exercises a11y state",
        build: form_controls,
    },
    Fixture {
        name: "scroll-list",
        summary: "40 rows in a short scrollport; exercises scroll and clipping",
        build: scroll_list,
    },
    Fixture {
        name: "occlusion",
        summary: "a label fully covered by an opaque card; exercises probe and hit_test",
        build: occlusion,
    },
    Fixture {
        name: "dialog",
        summary: "a modal dialog over content; exercises modal projection",
        build: dialog,
    },
];

pub fn list() -> &'static [Fixture] {
    FIXTURES
}

pub fn build(name: &str) -> Option<RuntimeDocument> {
    let fixture = FIXTURES.iter().find(|fixture| fixture.name == name)?;
    let id = DocumentId::new(1).expect("document 1");
    let mut document = RuntimeDocument::new(id);
    (fixture.build)(&mut document, id);
    Some(document)
}

fn counter(document: &mut RuntimeDocument, id: DocumentId) {
    let mut total = 0u32;
    document
        .context_mut()
        .build(id, |ui| {
            ui.with("root", Stack::column(8.0), |ui| {
                ui.child("caption", Text::new("press the button"));
                let increment = ui.child("increment", Button::new("Increment 0"));
                // The fixture has to actually count: a button whose click
                // changes nothing would teach `diff` and `screenshot` the wrong
                // lesson. The count lives on the button's own label, so no
                // cross-entity event plumbing is invented here.
                ui.on(increment, move |button: &mut Button, _: &Activate, _| {
                    total += 1;
                    button.label = format!("Increment {total}");
                });
            })
        })
        .expect("counter fixture");
}

fn form_controls(document: &mut RuntimeDocument, id: DocumentId) {
    document
        .context_mut()
        .build(id, |ui| {
            ui.with("root", Stack::column(12.0), |ui| {
                ui.child("notifications", Switch::new("Notifications", true));
                ui.child("archive", Checkbox::new("Archive", false));
                ui.child("title", TextInput::new("draft"));
                ui.child(
                    "volume",
                    RangeField::new(4.0, 0.0, 10.0, 1.0).expect("range"),
                );
            })
        })
        .expect("form fixture");
}

fn scroll_list(document: &mut RuntimeDocument, id: DocumentId) {
    let mut viewport = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut viewport.layout);
        layout.width = Some(LengthSpec::Px(320.0));
        layout.height = Some(LengthSpec::Px(160.0));
    }
    document
        .context_mut()
        .build(id, |ui| {
            let scroll = ui.child(
                "scroll",
                ScrollView::new(ScrollAxes::Vertical).style(viewport),
            );
            ui.nest(scroll, |ui| {
                let list = ui.child("list", List::new());
                ui.nest(list, |ui| {
                    for row in 0..40 {
                        ui.child(format!("row-{row}"), Button::new(format!("row-{row}")));
                    }
                });
            });
        })
        .expect("scroll fixture");
}

fn occlusion(document: &mut RuntimeDocument, id: DocumentId) {
    let cover = || {
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_left = Some(LengthSpec::Px(0.0));
        layout.offset_top = Some(LengthSpec::Px(0.0));
        layout.width = Some(LengthSpec::Px(320.0));
        layout.height = Some(LengthSpec::Px(120.0));
        style
    };
    document
        .context_mut()
        .build(id, |ui| {
            ui.with("root", Stack::column(0.0), |ui| {
                let mut label = Text::new("covered");
                label.style.layout = Arc::new(LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(0.0)),
                    offset_top: Some(LengthSpec::Px(0.0)),
                    width: Some(LengthSpec::Px(320.0)),
                    height: Some(LengthSpec::Px(120.0)),
                    ..Default::default()
                });
                ui.child("covered", label);
                ui.child("cover", Card::new().style(cover()));
            })
        })
        .expect("occlusion fixture");
}

fn dialog(document: &mut RuntimeDocument, id: DocumentId) {
    document
        .context_mut()
        .build(id, |ui| {
            ui.with("root", Stack::column(8.0), |ui| {
                ui.child("background", Text::new("behind the dialog"));
                let dialog = ui.child("dialog", Dialog::new("Confirm"));
                ui.nest(dialog, |ui| {
                    ui.child("confirm", Button::new("Confirm"));
                });
            })
        })
        .expect("dialog fixture");
}
