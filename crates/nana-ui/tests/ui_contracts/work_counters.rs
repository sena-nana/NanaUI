//! Headless `WorkCounters` contract tests (#432).
//!
//! Exercises [`HeadlessRuntime::turn`] and [`HeadlessRuntime::present`] through
//! public `nana_ui` APIs to ensure structural work counters are returned after
//! initial layout, become zero / `None` once settled, and record zero layout or
//! text shaping work on a paint/colour-only style mutation.

use std::sync::Arc;

use nana_ui::headless::{HeadlessOptions, HeadlessRuntime, headless_geometry};
use nana_ui::runtime::perf::WorkCounters;
use nana_ui::runtime::{DocumentId, Entity, RuntimeDocument, Stack, Text};
use nana_ui::{
    DocumentAccessError, GpuContext, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate,
    WindowDescriptor, wgpu,
};
use nana_ui_core::LengthSpec;
use nana_ui_platform::WindowId;

enum Message {}

struct WorkProgram {
    document: RuntimeDocument,
    label: Entity<Text>,
}

impl RuntimeProgram for WorkProgram {
    type Message = Message;
    type Error = String;

    fn initialize(
        _context: &RuntimeProgramContext<Message>,
    ) -> Result<(Self, Vec<Message>), String> {
        let doc_id = DocumentId::new(1).ok_or_else(|| String::from("invalid document id"))?;
        let mut document = RuntimeDocument::new(doc_id);
        let root = document
            .context_mut()
            .create_component(
                doc_id,
                Stack::column(8.0)
                    .width(LengthSpec::Px(320.0))
                    .height(LengthSpec::Px(240.0)),
            )
            .map_err(|error| error.to_string())?;
        let label = document
            .context_mut()
            .create_detached_component(doc_id, Text::new("WorkCounters contract"))
            .map_err(|error| error.to_string())?;
        document
            .context_mut()
            .append_child(root, label)
            .map_err(|error| error.to_string())?;
        Ok((Self { document, label }, Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
    }

    fn update(
        &mut self,
        message: Message,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        match message {}
    }
}

fn gpu_context() -> GpuContext {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("actual GPU adapter required");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("actual GPU required");
    GpuContext::from_wgpu(adapter, device, queue)
}

#[test]
fn headless_turn_and_present_report_structural_work_counters() {
    let mut host = HeadlessRuntime::<WorkProgram>::new(
        gpu_context(),
        WindowDescriptor::new("WorkCounters"),
        headless_geometry((320.0, 240.0), 1.0),
        HeadlessOptions::default(),
    )
    .expect("initialize headless runtime");

    // 1. Initial frame lays out and shapes text.
    let initial = host.turn();
    assert!(initial.did_work);
    assert!(
        initial.work.layout_nodes > 0,
        "initial turn must record layout_nodes, got {:?}",
        initial.work
    );
    assert!(
        initial.work.text_shaped > 0,
        "initial turn must record shaped text, got {:?}",
        initial.work
    );

    // 2. Settled frame does zero work (`present` returns `None`, `turn().work` is zero).
    assert_eq!(host.present(WindowId::PRIMARY), None);
    let settled = host.turn();
    assert_eq!(settled.work, WorkCounters::default());

    // 3. Paint/colour-only update performs zero layout and zero text shaping.
    let label = host.program().label;
    host.program_mut()
        .document
        .context_mut()
        .update_component(label, |text, _| {
            Arc::make_mut(&mut text.style.layout).color = Some([0.9, 0.2, 0.1, 1.0]);
        })
        .unwrap();
    let paint_work = host
        .present(WindowId::PRIMARY)
        .expect("colour-only update must return flush WorkCounters");
    assert_eq!(
        paint_work.layout_nodes, 0,
        "colour-only update must not lay out nodes: {paint_work:?}"
    );
    assert_eq!(
        paint_work.text_shaped, 0,
        "colour-only update must not reshape text: {paint_work:?}"
    );
    assert!(
        paint_work.render_nodes_changed > 0,
        "colour-only update must extract changed render nodes: {paint_work:?}"
    );

    // Settled again after the colour-only flush.
    assert_eq!(host.present(WindowId::PRIMARY), None);
    let post_paint = host.turn();
    assert_eq!(post_paint.work, WorkCounters::default());
}
