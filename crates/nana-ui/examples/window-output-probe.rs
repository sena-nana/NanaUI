#![recursion_limit = "256"]

//! Real-window check of `WindowOutput`: a transparent composition window whose
//! content is painted a second time into a 1280×720 output, exported as DX12
//! shared textures on Windows and read by a D3D11 device of the same adapter,
//! the way an in-process Spout sender does.
//!
//! Phases: visible and animated, hidden and animated, hidden and static,
//! visible and static. Prints one JSON line per phase and a summary, then
//! exits non-zero when a phase did not behave: animated phases must keep
//! producing frames (hidden included), static phases must produce none, and
//! every native token must reach the consumer.
//!
//! Phases advance on progress, not on a clock: an animated phase ends after
//! its frames, a static one 2.5 s after the output went idle. A software
//! adapter (WARP on CI) spends seconds on its first frames and then manages
//! a few frames per second, so a fixed window would judge its throughput.
//!
//! ```powershell
//! cargo run -p nana-ui --example window-output-probe --features hosted,bundled-fonts,native-export
//! ```

use std::convert::Infallible;
use std::num::NonZeroU32;
use std::path::PathBuf;
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

/// Output frames an animated phase must produce.
const ANIMATED_FRAMES: u64 = 30;
/// How long a static phase watches an idle output.
const STATIC_PHASE: Duration = Duration::from_millis(2500);
/// A phase that has not finished by then failed.
const PHASE_DEADLINE: Duration = Duration::from_secs(60);
/// How often `wake` checks progress while the window is hidden.
const PHASE_POLL: Duration = Duration::from_millis(100);
/// The perf workload failed when no native token arrived for this long.
const NATIVE_STALL_LIMIT: Duration = Duration::from_secs(30);
const PHASES: [(&str, bool, bool); 4] = [
    ("visible-animated", true, true),
    ("hidden-animated", false, true),
    ("hidden-static", false, false),
    ("visible-static", true, false),
];

#[derive(Default, Debug, Clone, serde::Serialize)]
struct Counts {
    frames: u64,
    /// Frames of the previous phase's last content, before a static phase's
    /// output went idle.
    settle_frames: u64,
    native_tokens: u64,
    consumed: u64,
    statuses: Vec<String>,
}

struct Probe {
    document: RuntimeDocument,
    label: Entity<Text>,
    window: Option<WindowHandle>,
    /// When the current phase began.
    phase_entered: Instant,
    /// When a static phase's output went idle and its window opened.
    settled_at: Option<Instant>,
    poll_at: Instant,
    failure: Option<String>,
    phase: usize,
    counts: Vec<Counts>,
    tick: u64,
    #[cfg(all(windows, feature = "native-export"))]
    consumer: Option<consumer::Consumer>,
    readback_ok: Option<bool>,
    perf: Option<NativePerf>,
}

struct NativePerf {
    output: PathBuf,
    adapter: String,
    device_generation: u64,
    extent: Option<[u32; 2]>,
    warmup_tokens: usize,
    frames: usize,
    source_ui_renders: usize,
    source_draw_calls: usize,
    measurement_started: bool,
    source_submissions: usize,
    source_cpu_readbacks: usize,
    source_target_recreates: usize,
    native_copy_bytes: usize,
    source_native_copies: usize,
    consumer_times: Vec<f64>,
    last_progress: Instant,
    source_times: Vec<f64>,
    source_started: Instant,
    measured_source_frame: bool,
    #[cfg(all(windows, feature = "native-export"))]
    last_release: u64,
    consumer_baseline: Option<serde_json::Value>,
}

impl NativePerf {
    fn new(output: PathBuf, adapter: String, device_generation: u64) -> Self {
        Self {
            output,
            adapter,
            device_generation,
            extent: None,
            warmup_tokens: 0,
            frames: 0,
            source_ui_renders: 0,
            source_draw_calls: 0,
            measurement_started: false,
            source_submissions: 0,
            source_cpu_readbacks: 0,
            source_target_recreates: 0,
            native_copy_bytes: 0,
            source_native_copies: 0,
            consumer_times: Vec::with_capacity(240),
            last_progress: Instant::now(),
            source_times: Vec::new(),
            source_started: Instant::now(),
            measured_source_frame: false,
            #[cfg(all(windows, feature = "native-export"))]
            last_release: 0,
            consumer_baseline: None,
        }
    }
}

fn perf_output_path() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--perf-output" {
            return Some(args.next().expect("--perf-output needs a path").into());
        }
    }
    None
}

fn quantiles(mut values: Vec<f64>) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    values.sort_by(f64::total_cmp);
    let at = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    serde_json::json!({"samples":values.len(),"p50":at(0.5),"p95":at(0.95),"p99":at(0.99)})
}

impl Probe {
    fn animated(&self) -> bool {
        self.perf.is_some() || PHASES.get(self.phase).is_some_and(|phase| phase.2)
    }

    /// A static phase whose output still finishes the previous content.
    fn settling(&self) -> bool {
        self.perf.is_none() && !self.animated() && self.settled_at.is_none()
    }

    fn advance(&mut self) -> RuntimeProgramUpdate {
        if let Some(perf) = &self.perf {
            if perf.last_progress.elapsed() > NATIVE_STALL_LIMIT {
                self.finish_native(Some("native workload stalled"));
            }
            return RuntimeProgramUpdate::default();
        }
        let Some(&(name, _, animated)) = PHASES.get(self.phase) else {
            return RuntimeProgramUpdate::default();
        };
        let now = Instant::now();
        let done = if animated {
            self.counts[self.phase].frames >= ANIMATED_FRAMES
        } else {
            self.settled_at
                .is_some_and(|settled| now - settled >= STATIC_PHASE)
        };
        if !done {
            if now - self.phase_entered > PHASE_DEADLINE {
                self.failure = Some(format!("{name} did not finish within {PHASE_DEADLINE:?}"));
                return self.finish();
            }
            return RuntimeProgramUpdate::default();
        }
        println!(
            "{}",
            serde_json::json!({"phase": name, "counts": self.counts[self.phase]})
        );
        self.phase += 1;
        self.phase_entered = now;
        self.settled_at = None;
        let phase = self.phase;
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
            "phases_finished": self.failure.is_none(),
            "visible_animated_frames": visible.frames >= ANIMATED_FRAMES,
            "hidden_animated_frames": hidden.frames >= ANIMATED_FRAMES,
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
            serde_json::json!({"summary": checks, "passed": passed, "failure": self.failure, "counts": self.counts})
        );
        std::process::exit(if passed { 0 } else { 1 });
    }

    fn finish_native(&self, error: Option<&str>) -> ! {
        let perf = self.perf.as_ref().expect("native perf mode");
        #[cfg(all(windows, feature = "native-export"))]
        let consumer = self
            .consumer
            .as_ref()
            .map(|consumer| serde_json::json!(consumer.stats));
        #[cfg(not(all(windows, feature = "native-export")))]
        let consumer: Option<serde_json::Value> = None;
        let mut measured = serde_json::Map::new();
        if let (Some(after), Some(before)) = (&consumer, &perf.consumer_baseline) {
            for key in [
                "gpu_copies",
                "gpu_copy_bytes",
                "gpu_ready_waits",
                "gpu_release_signals",
                "target_recreates",
                "validation_readbacks",
            ] {
                if let (Some(after), Some(before)) = (after[key].as_u64(), before[key].as_u64()) {
                    measured.insert(key.into(), serde_json::json!(after - before));
                }
            }
        }
        let passed = error.is_none()
            && perf.frames == 240
            && measured.get("gpu_copies").and_then(|v| v.as_u64()) == Some(240);
        let report = serde_json::json!({"schema_version":1,"fixture":"native-host-output","fixture_version":1,"native_api":"DX12-NT-D3D11","platform":std::env::consts::OS,"device_generation":perf.device_generation,"extent":perf.extent,"status":if passed {"ok"} else {"error"},"error":error,
            "evidence":"actual-native-host-gpu","adapter":perf.adapter,"frames":perf.frames,"warmup_frames":perf.warmup_tokens,
            "source_calls":perf.source_times.len(),"producer_work":{"draw_calls":perf.source_draw_calls,"ui_renders":perf.source_ui_renders,"submissions":perf.source_submissions,"cpu_readbacks":perf.source_cpu_readbacks,"target_recreates":perf.source_target_recreates,"gpu_copies":perf.source_native_copies,"gpu_copy_bytes":perf.native_copy_bytes},
            "consumer_work":measured,"proof":{"consumer_completed":passed,"native_handle_imports":consumer.as_ref().and_then(|v|v["native_handle_imports"].as_u64()),"explicit_cpu_fence_waits":0,"production_cpu_readbacks":0},
            "producer_window_cpu_ms":quantiles(perf.source_times.clone()),"consumer_record_cpu_ms":quantiles(perf.consumer_times.clone()),
            "scope":"30 native tokens warmup then 240 actual native copies/foreign GPU consumer updates; source Window may redraw more while native consumer is busy; consumer CPU call excludes GPU completion; source timing includes full Window interval",
            "gpu_execution_ms":null,"driver_vram_bytes":null});
        if let Some(parent) = perf.output.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&perf.output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{report}");
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
                phase_entered: Instant::now(),
                settled_at: None,
                poll_at: Instant::now(),
                failure: None,
                phase: 0,
                counts: vec![Counts::default(); PHASES.len()],
                tick: 0,
                #[cfg(all(windows, feature = "native-export"))]
                consumer: None,
                readback_ok: None,
                perf: perf_output_path().map(|path| {
                    NativePerf::new(
                        path,
                        format!(
                            "{} ({:?}) -> same-LUID D3D11",
                            context.gpu().capabilities().adapter_name(),
                            context.gpu().capabilities().backend()
                        ),
                        context.gpu().generation().get(),
                    )
                }),
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
                self.phase_entered = Instant::now();
                self.poll_at = self.phase_entered + PHASE_POLL;
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

    fn prepare_window_frame(&mut self, _: WindowId, context: &RuntimeProgramContext<()>) {
        if let Some(perf) = &mut self.perf {
            #[cfg(all(windows, feature = "native-export"))]
            if !perf.measurement_started
                && perf.warmup_tokens == 30
                && self
                    .consumer
                    .as_ref()
                    .is_some_and(|consumer| consumer.release_completed(perf.last_release))
            {
                // The 30th foreign GPU read has really completed. No blocking
                // CPU wait: the Window keeps ticking until this observation.
                perf.consumer_baseline = self
                    .consumer
                    .as_ref()
                    .map(|consumer| serde_json::json!(consumer.stats));
                perf.measurement_started = true;
            }
            perf.source_started = Instant::now();
            perf.measured_source_frame = perf.measurement_started;
        }
        #[cfg(all(windows, feature = "native-export"))]
        if let Some(consumer) = &mut self.consumer {
            match consumer.poll_readback() {
                Ok(Some(pixel)) => {
                    self.readback_ok = Some(pixel[3] == 255 && pixel[2] > 128);
                    println!("{}", serde_json::json!({"readback_bgra": pixel}));
                }
                Ok(None) => {}
                Err(error) => println!("{}", serde_json::json!({"consumer_error": error})),
            }
        }
        // The last frame reused a completed output of unchanged content.
        if self.settling()
            && context
                .presentation_work()
                .output_idle_reuse_frames
                .is_some_and(|frames| frames > 0)
        {
            self.settled_at = Some(Instant::now());
        }
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
        context: &RuntimeProgramContext<()>,
    ) -> RuntimeProgramUpdate {
        if let Some(perf) = self.perf.as_mut() {
            let work = context.presentation_work();
            if perf.measured_source_frame {
                let (
                    Some(renders),
                    Some(draws),
                    Some(submits),
                    Some(readbacks),
                    Some(recreates),
                    Some(copies),
                    Some(bytes),
                ) = (
                    work.output_ui_renders,
                    work.draw_calls,
                    work.output_submissions,
                    work.output_cpu_readbacks,
                    work.output_target_recreates,
                    work.output_gpu_copies,
                    work.output_gpu_copy_bytes,
                )
                else {
                    self.finish_native(Some("source output counters were not observed"))
                };
                perf.source_ui_renders += renders;
                perf.source_draw_calls += draws;
                perf.source_submissions += submits;
                perf.source_cpu_readbacks += readbacks;
                perf.source_target_recreates += recreates;
                perf.source_native_copies += copies;
                perf.native_copy_bytes += bytes;
                perf.source_times
                    .push(perf.source_started.elapsed().as_secs_f64() * 1000.0);
            }
            #[cfg(all(windows, feature = "native-export"))]
            if perf.frames == 240
                && self
                    .consumer
                    .as_ref()
                    .is_some_and(|consumer| consumer.release_completed(perf.last_release))
            {
                self.finish_native(None);
            }
        }
        self.advance()
    }

    fn next_wakeup(&self) -> Option<Instant> {
        if let Some(perf) = &self.perf {
            return Some(perf.last_progress + NATIVE_STALL_LIMIT);
        }
        // A hidden window presents nothing, so progress is checked here.
        Some(self.poll_at)
    }

    fn wake(&mut self, now: Instant, _: &RuntimeProgramContext<()>) -> RuntimeProgramUpdate {
        self.poll_at = now + PHASE_POLL;
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
        #[cfg(all(windows, feature = "native-export"))]
        if let Some(perf) = &mut self.perf {
            if perf.frames == 240 || perf.warmup_tokens == 30 && !perf.measurement_started {
                return;
            }
            let Some(mut token) = frame.take_native() else {
                return;
            };
            perf.extent = Some(token.extent());
            perf.device_generation = context.gpu().generation().get();
            let consumer = self
                .consumer
                .get_or_insert_with(|| consumer::Consumer::on_luid(token.adapter_luid()));
            let at = Instant::now();
            if consumer.consume(&mut token, false).is_err() {
                self.finish_native(Some("native GPU import/copy failed"));
            }
            perf.last_progress = at;
            if perf.warmup_tokens < 30 {
                perf.warmup_tokens += 1;
            } else {
                perf.frames += 1;
                perf.consumer_times
                    .push(at.elapsed().as_secs_f64() * 1000.0);
            }
            perf.last_release = token.release_value();
            return;
        }
        let settling = self.settling();
        let Some(counts) = self.counts.get_mut(self.phase) else {
            return;
        };
        if settling {
            counts.settle_frames += 1;
        } else {
            counts.frames += 1;
        }
        let _ = (frame, context);
        #[cfg(all(windows, feature = "native-export"))]
        if let Some(mut token) = frame.take_native() {
            counts.native_tokens += 1;
            let consumer = self
                .consumer
                .get_or_insert_with(|| consumer::Consumer::on_luid(token.adapter_luid()));
            // Read one frame back (probe-only) to prove the bytes arrived.
            let read_back =
                self.readback_ok.is_none() && self.tick > 5 && !consumer.readback_pending();
            match consumer.consume(&mut token, read_back) {
                Ok(()) => counts.consumed += 1,
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
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_FLAG_DO_NOT_WAIT,
        D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11Device1,
        ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4, ID3D11Fence, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_WAS_STILL_DRAWING, IDXGIAdapter,
        IDXGIFactory4,
    };
    use windows::core::Interface;

    pub struct Consumer {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        opened: Option<(u64, Vec<ID3D11Texture2D>, ID3D11Fence)>,
        /// Where each frame is copied, as a sender's own texture would be.
        copy: Option<ID3D11Texture2D>,
        /// A validation copy not mapped yet.
        readback: Option<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC)>,
        pub stats: Stats,
    }

    #[derive(Default, serde::Serialize)]
    pub struct Stats {
        gpu_copies: usize,
        gpu_copy_bytes: usize,
        gpu_ready_waits: usize,
        gpu_release_signals: usize,
        target_recreates: usize,
        validation_readbacks: usize,
        native_handle_imports: usize,
    }

    impl Consumer {
        pub fn release_completed(&self, value: u64) -> bool {
            self.opened.as_ref().is_some_and(|(_, _, fence)| {
                let completed = unsafe { fence.GetCompletedValue() };
                completed != u64::MAX && completed >= value
            })
        }
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
                    readback: None,
                    stats: Stats::default(),
                }
            }
        }

        pub fn readback_pending(&self) -> bool {
            self.readback.is_some()
        }

        /// Copies the frame for validation too when `read_back` asks for it;
        /// [`Self::poll_readback`] reads that copy.
        pub fn consume(
            &mut self,
            token: &mut NativeFrameToken,
            read_back: bool,
        ) -> Result<(), String> {
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
                        self.stats.native_handle_imports += 1;
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
                    self.stats.target_recreates += 1;
                }
                let copy = self.copy.as_ref().unwrap();
                let context4: ID3D11DeviceContext4 =
                    self.context.cast().map_err(|e| e.to_string())?;
                token.accept_release();
                context4
                    .Wait(fence, token.ready_value())
                    .map_err(|e| e.to_string())?;
                self.stats.gpu_ready_waits += 1;
                self.context.CopyResource(copy, shared);
                self.stats.gpu_copies += 1;
                self.stats.gpu_copy_bytes += desc.Width as usize * desc.Height as usize * 4;
                context4
                    .Signal(fence, token.release_value())
                    .map_err(|e| e.to_string())?;
                self.stats.gpu_release_signals += 1;
                self.context.Flush();
                if !read_back {
                    return Ok(());
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
                self.context.Flush();
                self.readback = Some((staging, desc));
                Ok(())
            }
        }

        /// The centre texel of the validation copy once this context finished
        /// it. Never waits: that copy waits for the producer's ready signal,
        /// which can sit behind a present the blocked event loop would never
        /// let complete.
        pub fn poll_readback(&mut self) -> Result<Option<[u8; 4]>, String> {
            let Some((staging, desc)) = self.readback.take() else {
                return Ok(None);
            };
            unsafe {
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                match self.context.Map(
                    &staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                ) {
                    Err(error) if error.code() == DXGI_ERROR_WAS_STILL_DRAWING => {
                        self.readback = Some((staging, desc));
                        return Ok(None);
                    }
                    result => result.map_err(|e| e.to_string())?,
                }
                self.stats.validation_readbacks += 1;
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
    #[cfg(not(all(windows, feature = "native-export")))]
    if let Some(path) = perf_output_path() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"schema_version":1,"fixture":"native-host-output","status":"unsupported","evidence":"unavailable","unsupported_reason":"actual DX12/D3D11 native-host import requires Windows"})).unwrap()).unwrap();
        std::process::exit(2);
    }
    let mut settings = WindowDescriptor::new("NanaUI window output probe")
        .initial_size(480.0, 320.0)
        .surface(nana_ui::WindowSurfacePreference::Composition);
    settings.transparent = true;
    settings.shadow = nana_ui::WindowShadow::None;
    settings.always_on_top = true;
    settings.initial_position = Some((80.0, 80.0));
    run_runtime::<Probe>(settings)
}
