//! The host's material policy: a refused program paints natively and is
//! compiled again once admitted; every admitted draw is charged its area.
//! Its own binary, since the policy is process-wide.
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{DocumentId, RuntimeDocument, Stack, Text};
use nana_ui::{MaterialPolicy, set_material_policy};
use nana_ui_core::{LengthSpec, MaterialProgram, MaterialRef};
use nana_ui_devtools::agent::{AgentSession, RuntimeAgentSession};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const RED: &str =
    "fn material(i: MaterialInput) -> vec4<f32> { return vec4<f32>(1., 0., 0., 1.); }";
/// Green from its parameters, so text drawn with another slot's bindings
/// shows.
const GREEN: &str = "fn material(i: MaterialInput) -> vec4<f32> { return material_param(0u); }";

#[derive(Default)]
struct Policy {
    refused: AtomicBool,
    charged: Mutex<HashMap<u64, u64>>,
}
impl MaterialPolicy for Policy {
    fn admit(&self, _: Option<nana_gpu::DeviceGeneration>, _: u64) -> bool {
        !self.refused.load(Ordering::Relaxed)
    }
    fn charge(&self, _: Option<nana_gpu::DeviceGeneration>, program: u64, pixels: u64) {
        *self.charged.lock().unwrap().entry(program).or_default() += pixels;
    }
}

/// A 160×60 red-material shape over 28px green-material text, at scale 1.
fn session(materials: bool) -> RuntimeAgentSession {
    let id = DocumentId::new(1).unwrap();
    let mut doc = RuntimeDocument::new(id);
    doc.context_mut()
        .mount_view_root(id, move || {
            let material = |source| {
                materials.then(|| {
                    MaterialRef::new(MaterialProgram::new(source))
                        .with_parameters(Arc::from([[0., 1., 0., 1.]]))
                })
            };
            let shape = Stack::row(0.)
                .width(LengthSpec::Px(160.))
                .height(LengthSpec::Px(60.))
                .with_layout(|l| {
                    l.background = Some([0.2, 0.2, 0.2, 1.]);
                    l.paint.background_material = material(RED);
                });
            let mut text = nana_ui::runtime::NodeStyle::default();
            let layout = Arc::make_mut(&mut text.layout);
            layout.font_size = Some(28.);
            layout.color = Some([0.1, 0.1, 0.1, 1.]);
            layout.paint.text_material = material(GREEN);
            widget(Stack::column(8.).padding(10.))
                .children((widget(shape), widget(Text::new("ABC").style(text))))
        })
        .unwrap();
    let mut session = RuntimeAgentSession::new(doc, 200, 150).unwrap();
    session.set_clear(Some([0.; 4]));
    session
}
fn frame(session: &mut RuntimeAgentSession) -> Vec<u8> {
    session.screenshot_rgba().unwrap().1
}

#[test]
fn refused_programs_paint_natively_and_admitted_ones_are_charged() {
    let native = frame(&mut session(false));
    let policy = Arc::new(Policy::default());
    set_material_policy(Some(policy.clone()));
    let red = MaterialProgram::new(RED).key();
    let green = MaterialProgram::new(GREEN).key();

    policy.refused.store(true, Ordering::Relaxed);
    let mut session = session(true);
    assert_eq!(
        native,
        frame(&mut session),
        "a refused program still painted"
    );
    assert!(
        policy.charged.lock().unwrap().is_empty(),
        "refused draws were charged"
    );

    policy.refused.store(false, Ordering::Relaxed);
    // The same painter: nothing it cached while refused holds the program back.
    let painted = frame(&mut session);
    let fill = &painted[(40 * 200 + 80) * 4..][..4];
    assert!(
        fill[0] > 240 && fill[1] < 10,
        "admitted again but not compiled: {fill:?}"
    );
    assert!(
        painted
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[1] > 200 && p[0] < 40 && p[3] > 200)
            .count()
            > 30,
        "text material missing"
    );
    let charged = policy.charged.lock().unwrap().clone();
    let screen = 200 * 150;
    // The shape covers 160×60 device pixels; the text a little of the rest.
    assert!(
        (160 * 60..=screen).contains(&charged[&red]),
        "shape charged {}",
        charged[&red]
    );
    assert!(
        (1..=screen).contains(&charged[&green]),
        "text charged {}",
        charged[&green]
    );
    set_material_policy(None);
}
