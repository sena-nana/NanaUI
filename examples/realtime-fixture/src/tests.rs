//! Issue #177 gates, run through [`HeadlessRuntime`]: the same program hooks,
//! in the same order, as the Scene host, on a simulated clock.
//!
//! The headless host presents every visible window on every turn and does
//! not read `frame_demand`, so one [`HeadlessRuntime::step`] stands for one
//! presented display frame. The native-window evidence is the fixture's
//! `--evidence` run.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use nana_ui::headless::{HeadlessOptions, HeadlessRuntime, headless_geometry};
use nana_ui::runtime::perf::WorkCounters;
use nana_ui::runtime::{AccessibilityAction, AccessibilityActionRequest};
use nana_ui::wgpu;
use nana_ui::{
    FrameDemand, GpuContext, GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages,
    RuntimeProgram, ScenePaintViewport, SceneWgpuPainter, WindowDescriptor,
};
use nana_ui_platform::{
    InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey, WindowEvent,
    WindowId,
};

use super::*;

const SIZE: (f32, f32) = (640.0, 400.0);

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

/// A started fixture whose first frames (layout, text, first bind) are done.
fn settled_host() -> HeadlessRuntime<Fixture> {
    let mut host = HeadlessRuntime::<Fixture>::new(
        gpu_context(),
        WindowDescriptor::new("realtime fixture"),
        headless_geometry(SIZE, 1.0),
        HeadlessOptions::default(),
    )
    .expect("initialize headless runtime");
    host.use_manual_clock();
    let frame = Duration::from_secs(1) / 120;
    for _ in 0..8 {
        host.step(frame);
    }
    assert_eq!(
        host.step(frame).work,
        WorkCounters::default(),
        "a static overlay settles after its first frames"
    );
    host
}

/// Present `frames` frames at `hz`, returning the UI work they did.
fn present(host: &mut HeadlessRuntime<Fixture>, hz: u32, frames: u32) -> WorkCounters {
    let mut work = WorkCounters::default();
    for _ in 0..frames {
        work.accumulate(host.step(Duration::from_secs(1) / hz).work);
    }
    work
}

fn key(host: &mut HeadlessRuntime<Fixture>, code: &'static str, state: KeyState) {
    host.input(
        WindowId::PRIMARY,
        InputPayload::Key(KeyInput {
            physical: PhysicalKey(Cow::Borrowed(code)),
            logical: LogicalKey(Cow::Borrowed(code)),
            state,
            repeat: false,
            modifiers: InputModifiers::default(),
        }),
        None,
    )
    .expect("route key");
}

fn demand(host: &HeadlessRuntime<Fixture>) -> FrameDemand {
    host.program().frame_demand(WindowId::PRIMARY)
}

/// UI work a frame with a still overlay must not do.
fn assert_no_ui_work(work: WorkCounters, context: &str) {
    assert_eq!(work.style_processed, 0, "{context}: {work:?}");
    assert_eq!(work.layout_nodes, 0, "{context}: {work:?}");
    assert_eq!(work.text_shaped, 0, "{context}: {work:?}");
    assert_eq!(work.entities_changed, 0, "{context}: {work:?}");
    assert_eq!(work.render_nodes_changed, 0, "{context}: {work:?}");
    assert_eq!(work, WorkCounters::default(), "{context}");
}

#[test]
fn fixed_update_holds_60_hz_at_any_presentation_rate() {
    for hz in [30, 60, 144, 240] {
        let mut host = settled_host();
        let before = host.program().stats();
        let work = present(&mut host, hz, hz * 2);
        let after = host.program().stats();
        let updates = after.app_fixed_updates - before.app_fixed_updates;
        assert!(
            (119..=121).contains(&updates),
            "{hz} Hz presentation: {updates} fixed updates in 2 s"
        );
        assert_eq!(after.dropped_fixed_steps, 0, "{hz} Hz");
        assert_eq!(
            after.prepare_ticks - before.prepare_ticks,
            u64::from(hz * 2)
        );
        // The program is woken for its own deadlines, never more often than
        // the fixed rate, and never per presented frame above it.
        let wakeups = after.runtime_wakeups - before.runtime_wakeups;
        assert!(
            wakeups <= u64::from(hz.min(60) * 2 + 1),
            "{hz} Hz: {wakeups}"
        );
        assert_no_ui_work(work, &format!("{hz} Hz presentation"));
        assert_eq!(
            demand(&host),
            FrameDemand::Continuous(NonZeroU32::new(120).unwrap())
        );
    }
}

#[test]
fn realtime_state_never_enters_the_ui_world() {
    let mut host = settled_host();
    let (generation, scene, nodes) = host
        .with_document(WindowId::PRIMARY, |document| {
            (
                document.context().world().generation(),
                document.shared_scene(),
                document.scene().node_count(),
            )
        })
        .unwrap()
        .unwrap();
    let revision = host.program().playfield_revision();
    let work = present(&mut host, 120, 240);
    assert_no_ui_work(work, "two seconds of simulation");
    assert!(host.program().playfield_revision() >= revision + 119);
    host.with_document(WindowId::PRIMARY, |document| {
        assert_eq!(document.context().world().generation(), generation);
        assert!(
            Arc::ptr_eq(&document.shared_scene(), &scene),
            "no fixed step rebuilt the UiScene"
        );
        // One anchor for the playfield, however many bodies move in it.
        assert!(nodes < Options::default().bodies, "{nodes} scene nodes");
        let playfield = PLAYFIELD_SLOT.to_string();
        let anchors = document
            .scene()
            .custom_resources()
            .filter(|resource| *resource == playfield)
            .count();
        assert_eq!(anchors, 1);
    })
    .unwrap();
    let stats = host.program().stats();
    assert_eq!(stats.ui_mutations, 0);
    // Only the frames before settling saw the UiWorld change.
    assert!(stats.ui_world_frames <= 2, "{stats:?}");
    assert!(stats.frames_presented > 240, "{stats:?}");
}

#[test]
fn continuous_input_is_mapped_by_the_application() {
    let mut host = settled_host();
    let start = host.program().world().player().position;
    key(&mut host, "KeyD", KeyState::Pressed);
    key(&mut host, "ArrowUp", KeyState::Pressed);
    assert_eq!(host.program().input().move_x(), 1.0);
    assert_eq!(host.program().input().move_y(), -1.0);
    let work = present(&mut host, 120, 60);
    let moved = host.program().world().player().position;
    // Half a second at the player's speed: no focus, no control, no action map.
    let expected = sim::PLAYER_SPEED * 0.5;
    assert!(
        (moved[0] - start[0] - expected).abs() < 0.03,
        "{start:?} -> {moved:?}"
    );
    assert!(
        (start[1] - moved[1] - expected).abs() < 0.03,
        "{start:?} -> {moved:?}"
    );
    key(&mut host, "KeyD", KeyState::Released);
    key(&mut host, "ArrowUp", KeyState::Released);
    present(&mut host, 120, 12);
    let stopped = host.program().world().player().position;
    present(&mut host, 120, 60);
    assert_eq!(host.program().world().player().position, stopped);
    assert_eq!(host.program().stats().mapped_key_events, 4);
    assert_no_ui_work(work, "held keys");
}

#[test]
fn pause_from_the_overlay_stops_wakeups_and_resumes_without_catch_up() {
    let mut host = settled_host();
    present(&mut host, 120, 60);
    let pause = host.program().pause.stable_id();
    let click = AccessibilityActionRequest {
        target: pause,
        action: AccessibilityAction::Click,
    };

    host.accessibility_action(WindowId::PRIMARY, click.clone())
        .unwrap();
    let toggled = host.step(Duration::from_millis(8));
    assert!(host.program().paused());
    assert!(toggled.work.text_shaped > 0, "{:?}", toggled.work);
    assert_eq!(host.next_wakeup(), None);
    assert_eq!(demand(&host), FrameDemand::OnDemand);

    let paused = host.program().stats();
    let work = present(&mut host, 120, 600);
    let idle = host.program().stats();
    assert_eq!(idle.app_fixed_updates, paused.app_fixed_updates);
    assert_eq!(idle.runtime_wakeups, paused.runtime_wakeups);
    assert_no_ui_work(work, "paused");

    host.accessibility_action(WindowId::PRIMARY, click).unwrap();
    present(&mut host, 120, 120);
    let resumed = host.program().stats();
    let updates = resumed.app_fixed_updates - idle.app_fixed_updates;
    assert!((59..=61).contains(&updates), "{updates} after resume");
    assert_eq!(
        resumed.dropped_fixed_steps, 0,
        "five idle seconds are not owed"
    );
    assert_eq!(resumed.ui_mutations, 4);

    // The same key edge pauses from the application's own mapping.
    key(&mut host, "KeyP", KeyState::Pressed);
    assert!(host.program().paused());
}

#[test]
fn hidden_and_zero_size_windows_neither_wake_nor_owe_steps() {
    let mut host = settled_host();
    let hide = |hidden| WindowEvent::VisibilityChanged {
        id: WindowId::PRIMARY,
        hidden,
    };
    key(&mut host, "KeyD", KeyState::Pressed);
    host.window_event(hide(true));
    assert_eq!(host.next_wakeup(), None);
    assert_eq!(demand(&host), FrameDemand::OnDemand);
    assert_eq!(host.program().input().move_x(), 0.0, "held keys release");
    let hidden = host.program().stats();
    present(&mut host, 60, 300);
    let after_hidden = host.program().stats();
    assert_eq!(after_hidden.app_fixed_updates, hidden.app_fixed_updates);
    assert_eq!(after_hidden.hidden_wakeups, 0);

    host.window_event(hide(false));
    present(&mut host, 120, 120);
    let shown = host.program().stats();
    let updates = shown.app_fixed_updates - after_hidden.app_fixed_updates;
    assert!((59..=61).contains(&updates), "{updates} after showing");
    assert_eq!(shown.dropped_fixed_steps, 0);

    // A minimised window on Windows reports a 0 x 0 size instead.
    host.resize(WindowId::PRIMARY, (0.0, 0.0), 1.0);
    assert_eq!(host.next_wakeup(), None);
    let zero = host.program().stats();
    present(&mut host, 60, 120);
    assert_eq!(
        host.program().stats().app_fixed_updates,
        zero.app_fixed_updates
    );
    host.resize(WindowId::PRIMARY, SIZE, 1.0);
    present(&mut host, 120, 120);
    assert!(host.program().stats().app_fixed_updates >= zero.app_fixed_updates + 59);
    assert_eq!(host.program().stats().dropped_fixed_steps, 0);
}

#[test]
fn resize_lays_the_overlay_out_once_then_settles() {
    let mut host = settled_host();
    host.resize(WindowId::PRIMARY, (520.0, 320.0), 1.0);
    let relayout = host.step(Duration::from_secs(1) / 120);
    // The incremental frontier re-places the overlay for the new size.
    assert!(
        relayout.work.layout_placement_nodes > 0 && relayout.work.render_nodes_changed > 0,
        "{:?}",
        relayout.work
    );
    // Settling may take a frame or two; after that the overlay is still.
    present(&mut host, 120, 4);
    let before = host.program().stats();
    let work = present(&mut host, 120, 120);
    assert_no_ui_work(work, "after resize");
    assert!(host.program().stats().app_fixed_updates >= before.app_fixed_updates + 59);
}

/// Paint the program's current UiScene on the host's own device and read the
/// pixels back.
struct Paint {
    painter: SceneWgpuPainter,
    target: nana_ui::GpuTexture,
    size: [u32; 2],
}

impl Paint {
    fn new(gpu: &GpuContext) -> Self {
        let size = [SIZE.0 as u32, SIZE.1 as u32];
        let format = GpuTextureFormat::BGRA8_UNORM_SRGB;
        let target = gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("realtime fixture target"),
                width: size[0],
                height: size[1],
                format,
                usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
            })
            .unwrap();
        Self {
            painter: SceneWgpuPainter::try_new(gpu, format).expect("painter"),
            target,
            size,
        }
    }

    fn frame(&mut self, host: &HeadlessRuntime<Fixture>) -> Vec<u8> {
        let gpu = host.gpu().clone();
        let scene = host
            .with_document(WindowId::PRIMARY, |document| document.shared_scene())
            .unwrap()
            .unwrap();
        let viewport = ScenePaintViewport {
            logical_size: [SIZE.0, SIZE.1],
            physical_size: self.size,
            scale_factor: 1.0,
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear: true,
        };
        let mut frame = gpu.begin_frame("realtime fixture paint");
        self.painter
            .paint(
                &scene,
                &mut frame,
                &self.target.render_target().unwrap(),
                viewport,
                Some(host.program().textures()),
                Some(host.program().renderers()),
            )
            .expect("paint");
        frame.submit();
        self.readback(&gpu)
    }

    fn readback(&self, gpu: &GpuContext) -> Vec<u8> {
        let [width, height] = self.size;
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let device = gpu.wgpu().device();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("realtime fixture readback"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut frame = gpu.begin_frame("realtime fixture readback");
        frame.wgpu_encoder().copy_texture_to_buffer(
            self.target.wgpu().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = frame.submit();
        let slice = buffer.slice(..);
        let (done, mapped) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = done.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission.wgpu_index().clone()),
                timeout: Some(Duration::from_secs(10)),
            })
            .expect("poll readback");
        mapped
            .recv_timeout(Duration::from_secs(10))
            .expect("map callback")
            .expect("map readback");
        let bytes = slice.get_mapped_range().expect("mapped readback");
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for line in bytes.chunks_exact(row as usize) {
            pixels.extend_from_slice(&line[..(width * 4) as usize]);
        }
        drop(bytes);
        buffer.unmap();
        pixels
    }
}

/// Pixels that differ between two frames, below the overlay's header.
fn changed_playfield_pixels(a: &[u8], b: &[u8]) -> usize {
    let width = SIZE.0 as usize;
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .enumerate()
        .filter(|(index, (left, right))| index / width > 80 && left != right)
        .count()
}

#[test]
fn playfield_presents_published_state_through_the_shared_device() {
    let mut host = settled_host();
    let mut paint = Paint::new(&host.gpu().clone());
    let scene = host
        .with_document(WindowId::PRIMARY, |document| document.shared_scene())
        .unwrap()
        .unwrap();
    let start = host.program().render_stats();
    assert_eq!(start.sprite_realizations, 1, "{start:?}");

    let first = paint.frame(&host);
    present(&mut host, 120, 60);
    let second = paint.frame(&host);
    let third = paint.frame(&host);

    let changed = changed_playfield_pixels(&first, &second);
    assert!(changed > 200, "only {changed} playfield pixels moved");
    assert_eq!(first.len(), second.len());
    assert_eq!(
        second, third,
        "an unchanged revision repaints the same pixels"
    );
    host.with_document(WindowId::PRIMARY, |document| {
        assert!(Arc::ptr_eq(&document.shared_scene(), &scene));
    })
    .unwrap();

    let stats = host.program().render_stats();
    // One pipeline and one sprite for the device, prepared once, drawn once
    // per frame, uploaded once per revision a frame painted.
    assert_eq!(stats.pipeline_builds, 1, "{stats:?}");
    assert_eq!(stats.sprite_realizations, 1, "{stats:?}");
    assert_eq!(stats.prepares, 1, "{stats:?}");
    assert_eq!(stats.draws, 3, "{stats:?}");
    assert_eq!(stats.instance_uploads, 2, "{stats:?}");
    assert_eq!(host.program().stats().resource_loads, 1);
}
