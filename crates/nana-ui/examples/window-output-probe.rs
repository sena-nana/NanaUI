#![recursion_limit = "256"]

//! Real-window check of `WindowOutput`: a transparent composition window whose
//! content is painted a second time into a 1280×720 output, exported as DX12
//! shared textures on Windows and read by a D3D11 device of the same adapter,
//! the way an in-process Spout sender does.
//!
//! Phases (about 2.5 s each): visible and animated, hidden and animated,
//! hidden and static, visible and static. Prints one JSON line per phase and a
//! summary, then exits non-zero when a phase did not behave:
//! animated phases must keep producing frames (hidden included), static
//! phases must produce none, and every native token must reach the consumer.
//!
//! ```powershell
//! cargo run -p nana-ui --example window-output-probe --features hosted,bundled-fonts,native-export
//! ```

use std::convert::Infallible;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use nana_ui::runtime::view::{entity_ref, widget, with_refs};
use nana_ui::runtime::{DocumentId, Entity, RuntimeDocument, Stack, Text};
use nana_ui::{
    DocumentAccessError, FrameDemand, GpuBackendPolicy, MaterialEffect, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, WindowDescriptor, WindowHandle,
    WindowOutputConfig, WindowOutputExport, WindowOutputExtent, WindowOutputFrame,
    WindowOutputStatus, run_runtime,
};
use nana_ui_core::LengthSpec;
use nana_ui_platform::{WindowEvent, WindowId};

const PHASE: Duration = Duration::from_millis(2500);
const PHASES: [(&str, bool, bool); 4] = [
    ("visible-animated", true, true),
    ("hidden-animated", false, true),
    ("hidden-static", false, false),
    ("visible-static", true, false),
];

#[derive(Default, Debug, Clone, serde::Serialize)]
struct Counts {
    frames: u64,
    native_tokens: u64,
    consumed: u64,
    statuses: Vec<String>,
}

struct Probe {
    document: RuntimeDocument,
    label: Entity<Text>,
    window: Option<WindowHandle>,
    started: Instant,
    phase: usize,
    counts: Vec<Counts>,
    tick: u64,
    #[cfg(all(windows, feature = "native-export"))]
    consumer: Option<consumer::Consumer>,
    readback_ok: Option<bool>,
}

impl Probe {
    fn phase_at(&self, now: Instant) -> usize {
        ((now - self.started).as_millis() / PHASE.as_millis()) as usize
    }

    fn animated(&self) -> bool {
        PHASES.get(self.phase).is_some_and(|phase| phase.2)
    }

    fn advance(&mut self) -> RuntimeProgramUpdate {
        let phase = self.phase_at(Instant::now());
        if phase == self.phase {
            return RuntimeProgramUpdate::default();
        }
        let (name, _, _) = PHASES[self.phase];
        println!(
            "{}",
            serde_json::json!({"phase": name, "counts": self.counts[self.phase]})
        );
        self.phase = phase;
        let Some(&(_, visible, _)) = PHASES.get(phase) else {
            return self.finish();
        };
        if let Some(window) = &self.window {
            let _request = window.set_visible(visible);
        }
        RuntimeProgramUpdate::redraw(WindowId::PRIMARY)
    }

    fn finish(&mut self) -> RuntimeProgramUpdate {
        let [visible, hidden, hidden_static, visible_static] =
            [0, 1, 2, 3].map(|i| &self.counts[i]);
        let native = cfg!(all(windows, feature = "native-export"));
        let checks = serde_json::json!({
            "visible_animated_frames": visible.frames >= 30,
            "hidden_animated_frames": hidden.frames >= 30,
            "hidden_static_no_frames": hidden_static.frames == 0,
            "visible_static_at_most_one_frame": visible_static.frames <= 1,
            "native_tokens_consumed": !native || (visible.native_tokens > 0
                && self.counts.iter().all(|counts| counts.native_tokens == counts.consumed)),
            "readback_matches": !native || self.readback_ok == Some(true),
        });
        let passed = checks
            .as_object()
            .is_some_and(|checks| checks.values().all(|value| value == true));
        println!(
            "{}",
            serde_json::json!({"summary": checks, "passed": passed, "counts": self.counts})
        );
        std::process::exit(if passed { 0 } else { 1 });
    }
}

impl RuntimeProgram for Probe {
    type Message = ();
    type Error = Infallible;

    fn gpu_backend_policy() -> GpuBackendPolicy {
        GpuBackendPolicy::CompositionCapable
    }

    fn startup_window_material_mode() -> MaterialEffect {
        MaterialEffect::Transparent
    }

    fn initialize(context: &RuntimeProgramContext<()>) -> Result<(Self, Vec<()>), Infallible> {
        let id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(id);
        let (_, label) = document
            .context_mut()
            .mount_view_root(id, || {
                let label = entity_ref::<Text>();
                let root = widget(Stack::fill_column(0.0).align(nana_ui_core::AlignSpec::Start))
                    .children(
                        widget(
                            Stack::column(8.0)
                                .width(LengthSpec::Px(220.0))
                                .height(LengthSpec::Px(90.0))
                                .with_layout(|layout| {
                                    layout.margin_left = Some(LengthSpec::Px(30.0));
                                    layout.margin_top = Some(LengthSpec::Px(30.0));
                                    layout.background = Some([0.9, 0.3, 0.2, 1.0]);
                                }),
                        )
                        .children(widget(Text::new("frame 0")).entity_ref(label)),
                    );
                with_refs(root, label)
            })
            .unwrap();
        let _ = context;
        Ok((
            Self {
                document,
                label,
                window: None,
                started: Instant::now(),
                phase: 0,
                counts: vec![Counts::default(); PHASES.len()],
                tick: 0,
                #[cfg(all(windows, feature = "native-export"))]
                consumer: None,
                readback_ok: None,
            },
            Vec::new(),
        ))
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

    fn update(&mut self, _: (), _: &RuntimeProgramContext<()>) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn window_material_mode(&self) -> MaterialEffect {
        MaterialEffect::Transparent
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        context: &RuntimeProgramContext<()>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { .. } => {
                self.window = Some(context.window());
                self.started = Instant::now();
                println!(
                    "{}",
                    serde_json::json!({
                        "event": "ready",
                        "target": format!("{:?}", context.presentation().surface_target()),
                        "backend": format!("{:?}", context.gpu().capabilities().backend()),
                    })
                );
                RuntimeProgramUpdate::default()
            }
            WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
            _ => RuntimeProgramUpdate::default(),
        }
    }

    fn frame_demand(&self, _: WindowId) -> FrameDemand {
        // Drive the phase clock even when static.
        FrameDemand::Continuous(NonZeroU32::new(30).unwrap())
    }

    fn prepare_window_frame(&mut self, _: WindowId, _: &RuntimeProgramContext<()>) {
        if self.animated() {
            self.tick += 1;
            let text = format!("frame {}", self.tick);
            let _ = self
                .document
                .context_mut()
                .update_component(self.label, |label, _| label.value = text);
        }
    }

    fn window_frame_presented(
        &mut self,
        _: WindowId,
        _: &RuntimeProgramContext<()>,
    ) -> RuntimeProgramUpdate {
        self.advance()
    }

    fn next_wakeup(&self) -> Option<Instant> {
        Some(self.started + PHASE * (self.phase as u32 + 1))
    }

    fn wake(&mut self, _: Instant, _: &RuntimeProgramContext<()>) -> RuntimeProgramUpdate {
        self.advance()
    }

    fn window_output(&self, _: WindowId) -> Option<WindowOutputConfig> {
        Some(
            WindowOutputConfig::default()
                .with_extent(WindowOutputExtent::Fixed {
                    width: 1280,
                    height: 720,
                })
                .with_export(WindowOutputExport::Native),
        )
    }

    fn window_output_frame(
        &mut self,
        _: WindowId,
        frame: &WindowOutputFrame,
        context: &RuntimeProgramContext<()>,
    ) {
        let Some(counts) = self.counts.get_mut(self.phase) else {
            return;
        };
        counts.frames += 1;
        let _ = (frame, context);
        #[cfg(all(windows, feature = "native-export"))]
        if let Some(mut token) = frame.take_native() {
            counts.native_tokens += 1;
            let consumer = self
                .consumer
                .get_or_insert_with(|| consumer::Consumer::on_luid(token.adapter_luid()));
            // Read one frame back (probe-only) to prove the bytes arrived.
            let read_back = self.readback_ok.is_none() && self.tick > 5;
            match consumer.consume(&mut token, read_back) {
                Ok(pixel) => {
                    counts.consumed += 1;
                    if let Some(pixel) = pixel {
                        self.readback_ok = Some(pixel[3] == 255 && pixel[2] > 128);
                        println!("{}", serde_json::json!({"readback_bgra": pixel}));
                    }
                }
                Err(error) => println!("{}", serde_json::json!({"consumer_error": error})),
            }
        }
    }

    fn window_output_status(
        &mut self,
        _: WindowId,
        status: WindowOutputStatus,
        _: &RuntimeProgramContext<()>,
    ) {
        println!("{}", serde_json::json!({"status": format!("{status:?}")}));
        if let Some(counts) = self.counts.get_mut(self.phase) {
            counts.statuses.push(format!("{status:?}"));
        }
    }
}

#[cfg(all(windows, feature = "native-export"))]
mod consumer {
    //! A D3D11 device on the exporting adapter: wait, copy, signal.
    use std::os::windows::io::AsRawHandle;

    use nana_ui::NativeFrameToken;
    use windows::Win32::Foundation::{HANDLE, HMODULE, LUID};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
        D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11Device5,
        ID3D11DeviceContext, ID3D11DeviceContext4, ID3D11Fence, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter, IDXGIFactory4,
    };
    use windows::core::Interface;

    pub struct Consumer {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        opened: Option<(u64, Vec<ID3D11Texture2D>, ID3D11Fence)>,
        /// Where each frame is copied, as a sender's own texture would be.
        copy: Option<ID3D11Texture2D>,
    }

    impl Consumer {
        pub fn on_luid(luid: i64) -> Self {
            unsafe {
                let factory: IDXGIFactory4 =
                    CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
                let adapter: IDXGIAdapter = factory
                    .EnumAdapterByLuid(LUID {
                        LowPart: luid as u32,
                        HighPart: (luid >> 32) as i32,
                    })
                    .unwrap();
                let (mut device, mut context) = (None, None);
                D3D11CreateDevice(
                    &adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .unwrap();
                Self {
                    device: device.unwrap(),
                    context: context.unwrap(),
                    opened: None,
                    copy: None,
                }
            }
        }

        /// Returns the centre texel when `read_back` asks for it.
        pub fn consume(
            &mut self,
            token: &mut NativeFrameToken,
            read_back: bool,
        ) -> Result<Option<[u8; 4]>, String> {
            unsafe {
                if self
                    .opened
                    .as_ref()
                    .is_none_or(|(pool, _, _)| *pool != token.pool_generation())
                {
                    let device1: ID3D11Device1 = self.device.cast().map_err(|e| e.to_string())?;
                    let device5: ID3D11Device5 = self.device.cast().map_err(|e| e.to_string())?;
                    let mut textures = Vec::new();
                    for slot in 0..nana_ui::NATIVE_EXPORT_SLOTS {
                        let handle = token.texture_handle(slot).ok_or("slot")?;
                        textures.push(
                            device1
                                .OpenSharedResource1::<ID3D11Texture2D>(HANDLE(
                                    handle.as_raw_handle(),
                                ))
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    let mut fence: Option<ID3D11Fence> = None;
                    device5
                        .OpenSharedFence(HANDLE(token.fence_handle().as_raw_handle()), &mut fence)
                        .map_err(|e| e.to_string())?;
                    self.opened = Some((token.pool_generation(), textures, fence.ok_or("fence")?));
                    self.copy = None;
                }
                let (_, textures, fence) = self.opened.as_ref().unwrap();
                let shared = &textures[token.slot()];
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                shared.GetDesc(&mut desc);
                if self.copy.is_none() {
                    let copy_desc = D3D11_TEXTURE2D_DESC {
                        Usage: D3D11_USAGE_DEFAULT,
                        MiscFlags: 0,
                        ..desc
                    };
                    let mut copy = None;
                    self.device
                        .CreateTexture2D(&copy_desc, None, Some(&mut copy))
                        .map_err(|e| e.to_string())?;
                    self.copy = copy;
                }
                let copy = self.copy.as_ref().unwrap();
                let context4: ID3D11DeviceContext4 =
                    self.context.cast().map_err(|e| e.to_string())?;
                token.accept_release();
                context4
                    .Wait(fence, token.ready_value())
                    .map_err(|e| e.to_string())?;
                self.context.CopyResource(copy, shared);
                context4
                    .Signal(fence, token.release_value())
                    .map_err(|e| e.to_string())?;
                self.context.Flush();
                if !read_back {
                    return Ok(None);
                }
                // Probe-only verification: map a staging copy of the frame.
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                    ..desc
                };
                let mut staging = None;
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                    .map_err(|e| e.to_string())?;
                let staging = staging.unwrap();
                self.context.CopyResource(&staging, copy);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .map_err(|e| e.to_string())?;
                // The opaque part of the frame is the red box; find it and
                // return its centre texel.
                let (mut opaque, mut min, mut max) = (0usize, [usize::MAX; 2], [0usize; 2]);
                let rows = |y: usize| {
                    std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                        desc.Width as usize * 4,
                    )
                };
                for y in 0..desc.Height as usize {
                    let row = rows(y);
                    for x in 0..desc.Width as usize {
                        if row[x * 4 + 3] == 255 {
                            opaque += 1;
                            min = [min[0].min(x), min[1].min(y)];
                            max = [max[0].max(x), max[1].max(y)];
                        }
                    }
                }
                let pixel = if opaque == 0 {
                    [0; 4]
                } else {
                    let (x, y) = ((min[0] + max[0]) / 2, (min[1] + max[1]) / 2);
                    let texel = &rows(y)[x * 4..x * 4 + 4];
                    [texel[0], texel[1], texel[2], texel[3]]
                };
                self.context.Unmap(&staging, 0);
                Ok(Some(pixel))
            }
        }
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    let mut settings = WindowDescriptor::new("NanaUI window output probe")
        .initial_size(480.0, 320.0)
        .surface(nana_ui::WindowSurfacePreference::Composition);
    settings.transparent = true;
    settings.shadow = nana_ui::WindowShadow::None;
    settings.always_on_top = true;
    settings.initial_position = Some((80.0, 80.0));
    run_runtime::<Probe>(settings)
}
