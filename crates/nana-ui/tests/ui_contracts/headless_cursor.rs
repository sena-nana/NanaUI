//! `HeadlessRuntime::cursor` reports the cursor a window would show: the
//! one input routing last asked of the window's host services. A node's
//! cursor reaches it, a custom (raster) cursor included, so a headless test
//! can check what a player or user would see under the pointer.

use std::sync::Arc;

use nana_ui::headless::{HeadlessOptions, HeadlessRuntime, headless_geometry};
use nana_ui::runtime::{DocumentId, Entity, GpuView, RuntimeDocument};
use nana_ui::{
    DocumentAccessError, GpuContext, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate,
    WindowDescriptor, wgpu,
};
use nana_ui_core::{CursorSpec, CustomCursorSpec, CustomCursorVariant, SystemCursor};
use nana_ui_platform::{CursorIcon, InputPayload, PointerInput, PointerPhase, WindowId};

enum Message {}

struct CursorProgram {
    document: RuntimeDocument,
    view: Entity<GpuView>,
}

impl RuntimeProgram for CursorProgram {
    type Message = Message;
    type Error = String;

    fn initialize(
        _context: &RuntimeProgramContext<Message>,
    ) -> Result<(Self, Vec<Message>), String> {
        let doc_id = DocumentId::new(1).ok_or_else(|| String::from("invalid document id"))?;
        let mut document = RuntimeDocument::new(doc_id);
        let view = document
            .context_mut()
            .create_component(doc_id, GpuView::new(7))
            .map_err(|error| error.to_string())?;
        Ok((Self { document, view }, Vec::new()))
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

fn set_cursor(host: &mut HeadlessRuntime<CursorProgram>, cursor: CursorSpec) {
    let view = host.program().view;
    host.program_mut()
        .document
        .context_mut()
        .update_component(view, |view, _| {
            Arc::make_mut(&mut view.style.layout).cursor = Some(cursor);
        })
        .unwrap();
    host.turn();
}

fn hover(host: &mut HeadlessRuntime<CursorProgram>, x: f32) {
    host.input(
        WindowId::PRIMARY,
        InputPayload::Pointer(PointerInput::mouse(PointerPhase::Move, x, 120.0)),
        None,
    )
    .unwrap();
}

#[test]
fn headless_runtime_reports_the_cursor_under_the_pointer() {
    let mut host = HeadlessRuntime::<CursorProgram>::new(
        gpu_context(),
        WindowDescriptor::new("Cursor"),
        headless_geometry((320.0, 240.0), 1.0),
        HeadlessOptions::default(),
    )
    .expect("initialize headless runtime");
    host.turn();
    assert_eq!(host.cursor(WindowId::PRIMARY), None, "no input yet");

    set_cursor(&mut host, CursorSpec::Grab);
    hover(&mut host, 100.0);
    assert_eq!(host.cursor(WindowId::PRIMARY), Some(CursorIcon::Grab));

    // A custom raster cursor reaches the host as itself, not as its fallback.
    let pixels = vec![255u8; 8 * 8 * 4];
    let variant = CustomCursorVariant::new(1000, 8, 8, [1, 1], pixels).unwrap();
    let custom = CustomCursorSpec::raster(vec![variant], SystemCursor::Default).unwrap();
    set_cursor(&mut host, CursorSpec::Custom(custom.clone()));
    hover(&mut host, 110.0);
    assert_eq!(
        host.cursor(WindowId::PRIMARY),
        Some(CursorIcon::Custom(custom))
    );
}

/// The pointer rests; the program changes the node's cursor (a key picked up
/// a tool, a drag ended). The next frame shows it, as a native host's does,
/// without waiting for the pointer to move again.
#[test]
fn a_cursor_changed_under_a_resting_pointer_shows_on_the_next_frame() {
    let mut host = HeadlessRuntime::<CursorProgram>::new(
        gpu_context(),
        WindowDescriptor::new("Cursor"),
        headless_geometry((320.0, 240.0), 1.0),
        HeadlessOptions::default(),
    )
    .expect("initialize headless runtime");
    host.turn();
    set_cursor(&mut host, CursorSpec::Grab);
    hover(&mut host, 100.0);
    assert_eq!(host.cursor(WindowId::PRIMARY), Some(CursorIcon::Grab));

    // `set_cursor` ends with a turn: no pointer event in between.
    set_cursor(&mut host, CursorSpec::None);
    assert_eq!(host.cursor(WindowId::PRIMARY), Some(CursorIcon::Hidden));
    set_cursor(&mut host, CursorSpec::Grabbing);
    assert_eq!(host.cursor(WindowId::PRIMARY), Some(CursorIcon::Grabbing));
}
