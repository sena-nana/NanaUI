//! The page the mobile prototype measures (`docs/reference/mobile-host.md`):
//! a search field for the native text proxy, a switch and a slider, a
//! camera preview area, and a grid of 200 performance buttons in a vertical
//! scroll view. The iOS and Android hosts mount the same page so their
//! numbers compare.
//!
//! The preview area has no background: a native camera layer under a
//! transparent surface shows through it. With CPU preview on, the host
//! instead uploads frames into [`PREVIEW_SLOT`] and the page samples them;
//! the texture view inside the area shows only while that slot is
//! registered, since a frame that samples a missing host texture fails.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use nana_mobile_host::MobileRuntime;
use nana_ui::runtime::view::{El, Signal, entity_ref, signal, widget, with_refs};
use nana_ui::runtime::{
    Activate, Button, Entity, FrameworkError, GpuTextureView, List, RangeChanged, RangeField,
    ScrollAxes, ScrollView, Switch, Text, TextChanged, TextInput, ToggleChanged,
};
use nana_ui::{
    GpuContext, GpuError, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages, HostTexture, HostTextureAlphaMode, HostTextureRegistry,
};
use nana_ui_core::{
    ButtonKind, ContentFit, FlexDirection, FlexWrap, LayoutStyle, LengthSpec, LogicalRect,
};

/// Host texture slot the CPU preview uploads into.
pub const PREVIEW_SLOT: &str = "probe-camera";
/// Buttons in the grid.
pub const GRID_LEN: usize = 200;
/// Logical height of the preview area.
const PREVIEW_HEIGHT: f32 = 220.0;
const PAGE_PADDING: f32 = 16.0;
const GAP: f32 = 12.0;

/// What the host reads back from the page.
#[derive(Clone, Default)]
pub struct ProbeState {
    presses: Arc<AtomicU32>,
    cpu_preview: Arc<AtomicBool>,
    pressed: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ProbeState {
    /// `pressed` runs on every button activation: the host's haptic.
    pub fn with_press_feedback(pressed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pressed: Some(Arc::new(pressed)),
            ..Self::default()
        }
    }

    pub fn presses(&self) -> u32 {
        self.presses.load(Ordering::Relaxed)
    }

    /// Whether the page asks for the CPU-uploaded preview instead of the
    /// native layer.
    pub fn cpu_preview(&self) -> bool {
        self.cpu_preview.load(Ordering::Relaxed)
    }
}

/// The mounted page.
pub struct Probe {
    pub state: ProbeState,
    preview: Entity<List>,
    preview_ready: Signal<bool>,
    #[cfg_attr(not(test), allow(dead_code))]
    first_button: Entity<Button>,
    #[cfg_attr(not(test), allow(dead_code))]
    grid_scroll: Entity<ScrollView>,
}

impl Probe {
    pub fn mount(runtime: &mut MobileRuntime, state: ProbeState) -> Result<Self, FrameworkError> {
        let page_state = state.clone();
        let ready_out = Rc::new(Cell::new(None));
        let ready_in = ready_out.clone();
        let (_, (preview, first_button, grid_scroll)) = runtime.mount(move || {
            let (preview, first_button, grid_scroll) = (entity_ref(), entity_ref(), entity_ref());
            let ready = signal(false);
            ready_in.set(Some(ready));
            let toggles = page_state.cpu_preview.clone();
            let buttons: Vec<_> = (0..GRID_LEN)
                .map(|index| {
                    let button = grid_button(index, &page_state);
                    if index == 0 {
                        button.entity_ref(first_button)
                    } else {
                        button
                    }
                })
                .collect();
            with_refs(
                widget(container(|layout| {
                    layout.direction = Some(FlexDirection::Column);
                    layout.width = Some(LengthSpec::Fill);
                    layout.height = Some(LengthSpec::Fill);
                    layout.gap = Some(LengthSpec::Px(GAP));
                    layout.padding = Some(LengthSpec::Px(PAGE_PADDING));
                }))
                .key("probe-page")
                .children((
                    widget(Text::new("NanaUI 移动端原型")).key("title"),
                    widget(TextInput::new("")).key("search").on_cx(
                        |field: &mut TextInput, event: &TextChanged, _| {
                            field.state.replace_value(event.value.clone());
                        },
                    ),
                    widget(container(|layout| {
                        layout.direction = Some(FlexDirection::Row);
                        layout.width = Some(LengthSpec::Fill);
                        layout.gap = Some(LengthSpec::Px(GAP));
                    }))
                    .key("controls")
                    .children((
                        widget(Switch::new("CPU 预览", false))
                            .key("cpu-preview")
                            .on_cx(move |switch: &mut Switch, event: &ToggleChanged, _| {
                                switch.checked = event.checked;
                                toggles.store(event.checked, Ordering::Relaxed);
                            }),
                        widget(RangeField::new(50.0, 0.0, 100.0, 1.0))
                            .key("strength")
                            .on_cx(|range: &mut RangeField, event: &RangeChanged, _| {
                                range.value = event.value;
                            }),
                    )),
                    widget(container(|layout| {
                        layout.width = Some(LengthSpec::Fill);
                        layout.height = Some(LengthSpec::Px(PREVIEW_HEIGHT));
                    }))
                    .key("preview")
                    .entity_ref(preview)
                    .children(
                        widget(preview_view())
                            .key("preview-texture")
                            .visible(move || ready.get()),
                    ),
                    widget(ScrollView::new(ScrollAxes::Vertical).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Fill);
                        layout.flex_grow = Some(1.0);
                        layout.min_height = Some(LengthSpec::Px(0.0));
                    }))
                    .key("grid-scroll")
                    .entity_ref(grid_scroll)
                    .children(
                        widget(container(|layout| {
                            layout.direction = Some(FlexDirection::Row);
                            layout.flex_wrap = FlexWrap::Wrap;
                            layout.width = Some(LengthSpec::Fill);
                            layout.gap = Some(LengthSpec::Px(GAP));
                        }))
                        .key("grid")
                        .children(buttons),
                    ),
                )),
                (preview, first_button, grid_scroll),
            )
        })?;
        Ok(Self {
            state,
            preview,
            preview_ready: ready_out.take().expect("mount builds the page"),
            first_button,
            grid_scroll,
        })
    }

    /// Reconcile the CPU preview before a frame is prepared: with the
    /// switch off the slot is released, and the texture view shows exactly
    /// while the slot is registered.
    pub fn sync_preview(&self, uploader: &mut PreviewUploader, textures: &HostTextureRegistry) {
        if !self.state.cpu_preview() {
            uploader.clear(textures);
        }
        self.preview_ready
            .set_if_changed(textures.get(PREVIEW_SLOT).is_some());
    }

    /// Where the preview area is, in logical px of the surface: the shell
    /// lays its native camera layer out to it.
    pub fn preview_rect(&self, runtime: &MobileRuntime) -> Option<LogicalRect> {
        let layout = runtime
            .document()
            .context()
            .world()
            .layout_box(self.preview.stable_id())?;
        Some(LogicalRect::new(
            layout.x,
            layout.y,
            layout.width,
            layout.height,
        ))
    }
}

/// The CPU preview path: camera frames copied into [`PREVIEW_SLOT`] each
/// frame, for comparing against a native preview layer.
#[derive(Default)]
pub struct PreviewUploader {
    texture: Option<(GpuTexture, HostTexture, u32, u32)>,
}

impl PreviewUploader {
    /// Upload one BGRA8 frame and mark the slot changed. The first frame,
    /// and any size change, registers a new texture.
    pub fn push_bgra(
        &mut self,
        gpu: &GpuContext,
        textures: &HostTextureRegistry,
        bytes: &[u8],
        width: u32,
        height: u32,
        bytes_per_row: u32,
    ) -> Result<(), GpuError> {
        let stale = self
            .texture
            .as_ref()
            .is_none_or(|(_, _, w, h)| (*w, *h) != (width, height));
        if stale {
            let texture = gpu.create_texture(&GpuTextureDescriptor {
                label: Some("nana-mobile-probe preview"),
                width,
                height,
                format: GpuTextureFormat::BGRA8_UNORM_SRGB,
                usage: GpuTextureUsages::COPY_DST | GpuTextureUsages::SAMPLED,
            })?;
            let host = HostTexture::new(&texture);
            textures.register(
                PREVIEW_SLOT,
                host.clone(),
                width,
                height,
                HostTextureAlphaMode::Opaque,
            );
            self.texture = Some((texture, host, width, height));
        }
        let (texture, ..) = self.texture.as_ref().expect("preview texture");
        gpu.write_texture(
            texture,
            GpuTextureRegion::full(width, height),
            bytes,
            bytes_per_row,
        )?;
        textures.invalidate(PREVIEW_SLOT);
        Ok(())
    }

    /// Back to the native layer: the slot samples nothing again.
    pub fn clear(&mut self, textures: &HostTextureRegistry) {
        if self.texture.take().is_some() {
            textures.remove(PREVIEW_SLOT);
        }
    }
}

fn grid_button(index: usize, state: &ProbeState) -> El<Button> {
    let presses = state.presses.clone();
    let pressed = state.pressed.clone();
    // Subtle, not the default Ghost: touch has no hover to reveal chrome.
    widget(Button::new(format!("表演 {}", index + 1)).kind(ButtonKind::Subtle))
        .key(format!("perform-{index}"))
        .on(move |_: &Activate| {
            presses.fetch_add(1, Ordering::Relaxed);
            if let Some(pressed) = &pressed {
                pressed();
            }
        })
}

fn preview_view() -> GpuTextureView {
    let mut view = GpuTextureView::new(PREVIEW_SLOT);
    view.label = Some("摄像头预览".into());
    view.fit = ContentFit::Cover;
    view.corner_radius = 12.0;
    let layout = Arc::make_mut(&mut view.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    view
}

fn container(f: impl FnOnce(&mut LayoutStyle)) -> List {
    let mut list = List::new();
    f(Arc::make_mut(&mut list.style.layout));
    list
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nana_mobile_host::{
        FrameDemand, MobileAppearance, MobilePlatform, MobileViewport, SafeArea, ScrollPhysics,
        TouchPhase,
    };
    use nana_ui_platform::{
        SharedClipboardHost, TextInputContext, UnsupportedClipboard, shared_clipboard,
    };

    use super::*;

    struct Inert;

    impl MobilePlatform for Inert {
        fn set_text_input(&mut self, _state: Option<&TextInputContext>) {}

        fn clipboard(&self) -> SharedClipboardHost {
            shared_clipboard(UnsupportedClipboard)
        }
    }

    fn mounted() -> (MobileRuntime, Probe) {
        let viewport = MobileViewport::new((1179, 2556), 3.0, SafeArea::default());
        let mut runtime =
            MobileRuntime::new(viewport, MobileAppearance::default(), Box::new(Inert)).unwrap();
        let probe = Probe::mount(&mut runtime, ProbeState::default()).unwrap();
        (runtime, probe)
    }

    /// A GPU device with no surface, or `None` where the machine has none.
    /// It is held to the iOS Simulator's 15 inter-stage variables (Metal's
    /// Apple2 family), so a shader that outgrows them fails here as well.
    fn headless_gpu() -> Option<GpuContext> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_inter_stage_shader_variables: 15,
                ..adapter.limits()
            },
            ..wgpu::DeviceDescriptor::default()
        }))
        .ok()?;
        Some(GpuContext::from_wgpu(adapter, device, queue))
    }

    /// Paint the page's current scene the way `MobileRenderer` does, into
    /// an offscreen target.
    fn paint(
        gpu: &GpuContext,
        painter: &mut nana_ui::SceneWgpuPainter,
        runtime: &MobileRuntime,
        textures: &HostTextureRegistry,
    ) -> Result<(), String> {
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let (width, height) = (390, 844);
        let texture = gpu
            .wgpu()
            .device()
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("probe test target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let target = nana_ui::GpuRenderTarget::from_wgpu(gpu, view, format, [width, height]);
        let mut recording = gpu.begin_frame("probe test frame");
        painter
            .paint(
                runtime.document().scene(),
                &mut recording,
                &target,
                nana_ui::ScenePaintViewport {
                    logical_size: [width as f32, height as f32],
                    physical_size: [width, height],
                    scale_factor: 1.0,
                    scene_origin: [0.0, 0.0],
                    target_origin: [0.0, 0.0],
                    clear_color: [0.0, 0.0, 0.0, 0.0],
                    clear: true,
                },
                Some(textures),
                None,
            )
            .map_err(|error| error.to_string())?;
        recording.submit();
        Ok(())
    }

    /// The native path registers no texture, so the page must not sample
    /// one; the CPU path shows it once uploaded and drops it again.
    #[test]
    fn every_preview_path_paints() {
        let Some(gpu) = headless_gpu() else {
            eprintln!("no GPU adapter; skipped");
            return;
        };
        let viewport = MobileViewport::new((390, 844), 1.0, SafeArea::default());
        let mut runtime =
            MobileRuntime::new(viewport, MobileAppearance::default(), Box::new(Inert)).unwrap();
        let probe = Probe::mount(&mut runtime, ProbeState::default()).unwrap();
        let textures = HostTextureRegistry::new();
        let mut uploader = PreviewUploader::default();
        let mut painter = nana_ui::SceneWgpuPainter::new(&gpu, GpuTextureFormat::BGRA8_UNORM_SRGB);
        let mut frame = |runtime: &mut MobileRuntime, uploader: &mut PreviewUploader| {
            probe.sync_preview(uploader, &textures);
            runtime.prepare_frame(Instant::now()).unwrap();
            paint(&gpu, &mut painter, runtime, &textures)
        };

        frame(&mut runtime, &mut uploader).expect("native path");

        probe.state.cpu_preview.store(true, Ordering::Relaxed);
        frame(&mut runtime, &mut uploader).expect("CPU path before the first frame");
        let pixels = vec![128; 64 * 48 * 4];
        uploader
            .push_bgra(&gpu, &textures, &pixels, 64, 48, 64 * 4)
            .unwrap();
        frame(&mut runtime, &mut uploader).expect("CPU path with a frame");

        probe.state.cpu_preview.store(false, Ordering::Relaxed);
        frame(&mut runtime, &mut uploader).expect("back to the native path");
        assert!(textures.get(PREVIEW_SLOT).is_none());
    }

    #[test]
    fn the_preview_area_lays_out_inside_the_surface() {
        let (runtime, probe) = mounted();
        let rect = probe.preview_rect(&runtime).expect("preview laid out");
        assert_eq!(rect.height, PREVIEW_HEIGHT);
        assert!(rect.width > 300.0 && rect.width <= 393.0, "{rect:?}");
    }

    #[test]
    fn a_grid_button_counts_a_press() {
        let (mut runtime, probe) = mounted();
        let first = runtime
            .document()
            .context()
            .world()
            .layout_box(probe.first_button.stable_id())
            .expect("first grid button");
        let (x, y) = ((first.x + 10.0) * 3.0, (first.y + 10.0) * 3.0);
        runtime.push_touch(TouchPhase::Down, x, y, 1).unwrap();
        runtime.push_touch(TouchPhase::Up, x, y, 1).unwrap();
        assert_eq!(probe.state.presses(), 1);
    }

    fn grid_offset(runtime: &MobileRuntime, probe: &Probe) -> f32 {
        runtime
            .document()
            .context()
            .world()
            .scroll_offset(probe.grid_scroll.stable_id())
            .map_or(0.0, |offset| offset.y)
    }

    /// A real-time upward swipe over the grid, in physical px.
    fn swipe(runtime: &mut MobileRuntime, probe: &Probe, distance: f32) {
        let start = runtime
            .document()
            .context()
            .world()
            .layout_box(probe.first_button.stable_id())
            .expect("first grid button");
        let (x, y) = ((start.x + 10.0) * 3.0, (start.y + 10.0) * 3.0 + distance);
        runtime.push_touch(TouchPhase::Down, x, y, 1).unwrap();
        for step in 1..=8 {
            std::thread::sleep(Duration::from_millis(8));
            let y = y - distance * step as f32 / 8.0;
            runtime.push_touch(TouchPhase::Move, x, y, 1).unwrap();
        }
        runtime
            .push_touch(TouchPhase::Up, x, y - distance, 1)
            .unwrap();
    }

    /// Drive the display link until the document goes idle; how many ticks.
    fn run_until_idle(runtime: &mut MobileRuntime) -> usize {
        let mut ticks = 0;
        loop {
            match runtime.frame_demand() {
                FrameDemand::Idle => return ticks,
                FrameDemand::At(deadline) => {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()))
                }
                FrameDemand::Now => {}
            }
            runtime.prepare_frame(Instant::now()).unwrap();
            ticks += 1;
            assert!(ticks < 2000, "the fling never settled");
        }
    }

    #[test]
    fn a_swipe_scrolls_the_grid_flings_and_settles_idle() {
        for physics in [ScrollPhysics::Ios, ScrollPhysics::Android] {
            let (mut runtime, probe) = mounted();
            runtime.set_scroll_physics(physics);
            run_until_idle(&mut runtime);
            swipe(&mut runtime, &probe, 600.0);
            let released = grid_offset(&runtime, &probe);
            assert!(released > 100.0, "{physics:?}: dragged to {released}");
            assert_eq!(
                probe.state.presses(),
                0,
                "{physics:?}: a scroll is not a tap"
            );
            let ticks = run_until_idle(&mut runtime);
            let settled = grid_offset(&runtime, &probe);
            assert!(ticks > 1, "{physics:?}: no fling frames");
            assert!(
                settled > released,
                "{physics:?}: fling {released} -> {settled}"
            );
            assert!(!runtime.prepare_frame(Instant::now()).unwrap());
        }
    }
}
