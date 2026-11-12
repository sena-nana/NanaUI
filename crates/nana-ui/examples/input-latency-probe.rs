//! Input-to-present latency on the hosted Windows path. Measurement only.
//!
//! ```text
//! cargo run --release -p nana-ui --example input-latency-probe \
//!     --features hosted,bundled-fonts -- --clicks 300 --output latency.json
//! ```
//!
//! An ordinary hosted window fills with one color; every left-button press
//! flips it between red and blue. A driver thread posts `WM_MOUSEMOVE` /
//! `WM_LBUTTONDOWN` to the window at random 120–250 ms intervals (posting
//! rather than `SendInput`, so the probe never moves the user's cursor or
//! clicks anything else; it skips the HID → raw-input → queue hop, which is
//! sub-millisecond on an idle desktop). A Desktop Duplication thread watches
//! one pixel of the window and stamps the first desktop frame showing the new
//! color with that frame's `LastPresentTime` — the QPC time DWM presented the
//! composed desktop containing it.
//!
//! Per click, all on the QPC clock:
//! - `post`: just before `PostMessageW(WM_LBUTTONDOWN)`;
//! - `input`: `RuntimeProgram::input_event` for the press, i.e. after the
//!   host lowered it into `nana-ui-input` and the Runtime routed it;
//! - `prepare`: `prepare_window_frame` of the frame that paints it;
//! - `presented`: `window_frame_presented` of that frame, after
//!   `SurfaceTexture::present` returned, with the host's per-stage gauges
//!   (`acquire` is where the DXGI frame-latency waitable is waited on);
//! - `screen`: the duplicated desktop frame's `LastPresentTime`.

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("input-latency-probe measures the Windows hosted path only");
    std::process::exit(2);
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), nana_ui::HostedRunError> {
    probe::main()
}

#[cfg(target_os = "windows")]
mod probe {
    use std::convert::Infallible;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nana_ui::runtime::view::{entity_ref, widget, with_refs};
    use nana_ui::runtime::{DocumentId, Entity, FrameworkError, RuntimeDocument, Stack};
    use nana_ui::{
        DocumentAccessError, RoutedInput, RuntimeProgram, RuntimeProgramContext,
        RuntimeProgramUpdate, WindowDescriptor, WindowSurfaceInfo, run_runtime,
    };
    use nana_ui_platform::{InputPayload, PointerPhase, WindowEvent, WindowId};
    use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIFactory1,
        IDXGIOutput1,
    };
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, MONITOR_DEFAULTTONEAREST, MonitorFromWindow,
    };
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetClientRect, PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    };
    use windows::core::{HSTRING, Interface};

    const TITLE: &str = "NanaUI input latency probe";
    const WARMUP_CLICKS: usize = 10;
    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn qpc() -> i64 {
        let mut value = 0;
        unsafe { QueryPerformanceCounter(&mut value) }.unwrap();
        value
    }

    #[derive(Clone, Default, serde::Serialize)]
    struct Click {
        post: i64,
        input: Option<i64>,
        prepare: Option<i64>,
        presented: Option<i64>,
        screen: Option<i64>,
        /// Desktop updates folded into the frame that showed the change; >1
        /// means `screen` may be late, and the sample is dropped.
        accumulated_frames: u32,
        stages_ns: Option<[u64; 11]>,
    }

    #[derive(Default)]
    struct Shared {
        clicks: Vec<Click>,
        /// The click the window and the duplication thread are working on.
        current: Option<usize>,
        /// Color the current click turns the window.
        expect_red: bool,
        surface: Option<WindowSurfaceInfo>,
        backend: String,
        /// Every presented frame once the measured clicks start: the QPC
        /// time `present` returned and that frame's acquire wait in ns.
        frames: Vec<(i64, u64)>,
    }

    type State = Arc<Mutex<Shared>>;

    struct Options {
        clicks: usize,
        output: Option<PathBuf>,
        /// Keep presenting at this rate (`FrameDemand::Continuous`), so a
        /// press lands behind frames already in flight; `None` is an idle,
        /// on-demand window.
        continuous: Option<std::num::NonZeroU32>,
    }

    fn options() -> Options {
        let mut clicks = 300;
        let mut output = None;
        let mut continuous = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--clicks" => clicks = args.next().unwrap().parse().unwrap(),
                "--output" => output = args.next().map(PathBuf::from),
                "--continuous" => {
                    continuous = std::num::NonZeroU32::new(args.next().unwrap().parse().unwrap())
                }
                _ => {}
            }
        }
        Options {
            clicks,
            output,
            continuous,
        }
    }

    fn stage_values() -> [u64; 11] {
        use nana_ui::diagnostics::framework::host::*;
        [
            &PRESENTATION_PREPARE_NS,
            &PRESENTATION_SETTLE_NS,
            &PRESENTATION_RESERVE_NS,
            &PRESENTATION_ACQUIRE_NS,
            &PRESENTATION_RESOURCES_NS,
            &PRESENTATION_PLAN_NS,
            &PRESENTATION_ENCODE_NS,
            &PRESENTATION_SUBMIT_NS,
            &PRESENTATION_POST_SUBMIT_NS,
            &PRESENTATION_PRESENT_NS,
            &PRESENTATION_FINISH_NS,
        ]
        .map(|metric| metric.value())
    }

    const STAGE_NAMES: [&str; 11] = [
        "prepare",
        "settle",
        "reserve",
        "acquire",
        "resources",
        "plan",
        "encode",
        "submit",
        "post_submit",
        "present",
        "finish",
    ];

    struct Probe {
        document: RuntimeDocument,
        root: Entity<Stack>,
        red: bool,
        state: State,
        surface_request: Option<nana_ui::WindowRequest<WindowSurfaceInfo>>,
        started: bool,
    }

    impl RuntimeProgram for Probe {
        type Message = ();
        type Error = Infallible;

        fn initialize(_: &RuntimeProgramContext<()>) -> Result<(Self, Vec<()>), Infallible> {
            let id = DocumentId::new(1).unwrap();
            let mut document = RuntimeDocument::new(id);
            let (_, root) = document
                .context_mut()
                .mount_view_root(id, || {
                    let root = entity_ref::<Stack>();
                    let view = widget(
                        Stack::fill_column(0.0).with_layout(|layout| layout.background = Some(RED)),
                    )
                    .entity_ref(root);
                    with_refs(view, root)
                })
                .unwrap();
            Ok((
                Self {
                    document,
                    root,
                    red: true,
                    state: Arc::new(Mutex::new(Shared {
                        expect_red: true,
                        ..Shared::default()
                    })),
                    surface_request: None,
                    started: false,
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

        fn frame_demand(&self, _: WindowId) -> nana_ui::FrameDemand {
            static CONTINUOUS: std::sync::OnceLock<Option<std::num::NonZeroU32>> =
                std::sync::OnceLock::new();
            CONTINUOUS.get_or_init(|| options().continuous).map_or(
                nana_ui::FrameDemand::OnDemand,
                nana_ui::FrameDemand::Continuous,
            )
        }

        fn window_event(
            &mut self,
            event: WindowEvent,
            context: &RuntimeProgramContext<()>,
        ) -> RuntimeProgramUpdate {
            if let WindowEvent::Ready { .. } = event {
                self.state.lock().unwrap().backend = format!(
                    "{:?}: {}",
                    context.gpu().capabilities().backend(),
                    context.gpu().capabilities().adapter_name()
                );
                self.surface_request = Some(context.window().surface_info());
                return RuntimeProgramUpdate::redraw(WindowId::PRIMARY);
            }
            if let WindowEvent::CloseRequested { .. } = event {
                std::process::exit(3);
            }
            RuntimeProgramUpdate::default()
        }

        fn input_event(
            &mut self,
            id: WindowId,
            input: RoutedInput<'_>,
            _: &RuntimeProgramContext<()>,
        ) -> Result<RuntimeProgramUpdate, FrameworkError> {
            let InputPayload::Pointer(pointer) = &input.event.payload else {
                return Ok(RuntimeProgramUpdate::default());
            };
            if pointer.phase != PointerPhase::Down {
                return Ok(RuntimeProgramUpdate::default());
            }
            let now = qpc();
            self.red = !self.red;
            let color = if self.red { RED } else { BLUE };
            self.document
                .context_mut()
                .update_component(self.root, |stack, _| {
                    stack.layout_mut().background = Some(color);
                })?;
            let mut state = self.state.lock().unwrap();
            if let Some(index) = state.current {
                let click = &mut state.clicks[index];
                if click.input.is_none() {
                    click.input = Some(now);
                }
            }
            Ok(RuntimeProgramUpdate::redraw(id))
        }

        fn prepare_window_frame(&mut self, _: WindowId, _: &RuntimeProgramContext<()>) {
            let now = qpc();
            if let Some(Ok(info)) = self.surface_request.as_mut().and_then(|r| r.try_take()) {
                self.surface_request = None;
                self.state.lock().unwrap().surface = Some(info);
            }
            let mut state = self.state.lock().unwrap();
            if let Some(index) = state.current {
                let click = &mut state.clicks[index];
                if click.input.is_some() && click.prepare.is_none() {
                    click.prepare = Some(now);
                }
            }
        }

        fn window_frame_presented(
            &mut self,
            _: WindowId,
            context: &RuntimeProgramContext<()>,
        ) -> RuntimeProgramUpdate {
            let now = qpc();
            let mut state = self.state.lock().unwrap();
            if state.current.is_some_and(|index| index >= WARMUP_CLICKS) {
                let acquire = stage_values()[3];
                state.frames.push((now, acquire));
            }
            if let Some(index) = state.current {
                let click = &mut state.clicks[index];
                if click.prepare.is_some() && click.presented.is_none() {
                    click.presented = Some(now);
                    click.stages_ns = Some(stage_values());
                }
            }
            if !self.started {
                self.started = true;
                // The steady configuration, after the first frame's present
                // policy; the one asked for at `Ready` predates it.
                self.surface_request = Some(context.window().surface_info());
                let state = self.state.clone();
                std::thread::spawn(move || drive(state));
            }
            RuntimeProgramUpdate::default()
        }
    }

    /// The window's HWND, client size and the screen position of the pixel
    /// the duplication thread watches.
    fn locate() -> (HWND, RECT, POINT) {
        let hwnd = unsafe { FindWindowW(None, &HSTRING::from(TITLE)) }.expect("probe window");
        let mut client = RECT::default();
        unsafe { GetClientRect(hwnd, &mut client) }.unwrap();
        let mut watched = POINT { x: 12, y: 12 };
        let _ = unsafe { ClientToScreen(hwnd, &mut watched) };
        (hwnd, client, watched)
    }

    struct Duplication {
        duplication: windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication,
        context: ID3D11DeviceContext,
        staging: ID3D11Texture2D,
        /// Watched pixel in output coordinates.
        pixel: (u32, u32),
    }

    fn duplication(hwnd: HWND, watched: POINT) -> Duplication {
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.unwrap();
        for adapter_index in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(adapter_index) }) else {
                break;
            };
            for output_index in 0.. {
                let Ok(output) = (unsafe { adapter.EnumOutputs(output_index) }) else {
                    break;
                };
                let desc = unsafe { output.GetDesc() }.unwrap();
                if desc.Monitor != monitor {
                    continue;
                }
                let mut device: Option<ID3D11Device> = None;
                let mut context: Option<ID3D11DeviceContext> = None;
                unsafe {
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
                }
                .unwrap();
                let device = device.unwrap();
                let output: IDXGIOutput1 = output.cast().unwrap();
                let duplication = unsafe { output.DuplicateOutput(&device) }.unwrap();
                let mut staging = None;
                unsafe {
                    device.CreateTexture2D(
                        &D3D11_TEXTURE2D_DESC {
                            Width: 1,
                            Height: 1,
                            MipLevels: 1,
                            ArraySize: 1,
                            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            SampleDesc: DXGI_SAMPLE_DESC {
                                Count: 1,
                                Quality: 0,
                            },
                            Usage: D3D11_USAGE_STAGING,
                            BindFlags: 0,
                            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                            MiscFlags: 0,
                        },
                        None,
                        Some(&mut staging),
                    )
                }
                .unwrap();
                let origin = desc.DesktopCoordinates;
                return Duplication {
                    duplication,
                    context: context.unwrap(),
                    staging: staging.unwrap(),
                    pixel: (
                        (watched.x - origin.left) as u32,
                        (watched.y - origin.top) as u32,
                    ),
                };
            }
        }
        panic!("no DXGI output shows the probe window");
    }

    /// Whether the watched pixel reads red (`Some(true)`), blue or neither.
    fn read_pixel(dup: &Duplication, info: &mut DXGI_OUTDUPL_FRAME_INFO) -> Option<Option<bool>> {
        let mut resource = None;
        match unsafe { dup.duplication.AcquireNextFrame(50, info, &mut resource) } {
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return None,
            result => result.expect("AcquireNextFrame"),
        }
        let texture: ID3D11Texture2D = resource.unwrap().cast().unwrap();
        let region = D3D11_BOX {
            left: dup.pixel.0,
            top: dup.pixel.1,
            front: 0,
            right: dup.pixel.0 + 1,
            bottom: dup.pixel.1 + 1,
            back: 1,
        };
        let color = unsafe {
            dup.context
                .CopySubresourceRegion(&dup.staging, 0, 0, 0, 0, &texture, 0, Some(&region));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            dup.context
                .Map(&dup.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let bgra = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), 4).to_vec();
            dup.context.Unmap(&dup.staging, 0);
            bgra
        };
        unsafe { dup.duplication.ReleaseFrame() }.unwrap();
        let (b, r) = (color[0], color[2]);
        Some(if r > 200 && b < 60 {
            Some(true)
        } else if b > 200 && r < 60 {
            Some(false)
        } else {
            None
        })
    }

    fn watch(state: State, hwnd: HWND, watched: POINT) {
        let dup = duplication(hwnd, watched);
        loop {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let Some(color) = read_pixel(&dup, &mut info) else {
                continue;
            };
            if info.LastPresentTime == 0 {
                continue;
            }
            let mut state = state.lock().unwrap();
            let expect = state.expect_red;
            if let Some(index) = state.current {
                let click = &mut state.clicks[index];
                if click.screen.is_none()
                    && color == Some(expect)
                    && info.LastPresentTime > click.post
                {
                    click.screen = Some(info.LastPresentTime);
                    click.accumulated_frames = info.AccumulatedFrames;
                }
            }
        }
    }

    fn lparam(x: i32, y: i32) -> LPARAM {
        LPARAM(((y as u32 as isize) << 16) | (x as u32 as isize & 0xffff))
    }

    fn drive(state: State) {
        let options = options();
        std::thread::sleep(Duration::from_millis(500));
        let (hwnd, client, watched) = locate();
        {
            let state = state.clone();
            let raw = hwnd.0 as isize;
            std::thread::spawn(move || watch(state, HWND(raw as *mut _), watched));
        }
        std::thread::sleep(Duration::from_millis(300));
        let (x, y) = (client.right / 2, client.bottom / 2);
        let post = |message, wparam| unsafe {
            PostMessageW(Some(hwnd), message, WPARAM(wparam), lparam(x, y)).unwrap();
        };
        post(WM_MOUSEMOVE, 0);
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let total = options.clicks + WARMUP_CLICKS;
        for index in 0..total {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            std::thread::sleep(Duration::from_millis(120 + seed % 130));
            {
                let mut state = state.lock().unwrap();
                state.expect_red = !state.expect_red;
                state.clicks.push(Click::default());
                state.current = Some(index);
                state.clicks[index].post = qpc();
            }
            post(WM_LBUTTONDOWN, 0x0001 /* MK_LBUTTON */);
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(5));
                if state.lock().unwrap().clicks[index].screen.is_some() {
                    break;
                }
            }
            post(WM_LBUTTONUP, 0);
        }
        std::thread::sleep(Duration::from_millis(100));
        report(&state.lock().unwrap(), &options);
        std::process::exit(0);
    }

    fn summarize(mut values: Vec<f64>) -> serde_json::Value {
        if values.is_empty() {
            return serde_json::Value::Null;
        }
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        serde_json::json!({
            "n": values.len(),
            "min": values[0],
            "p50": at(0.5),
            "p90": at(0.9),
            "p99": at(0.99),
            "max": values[values.len() - 1],
            "mean": values.iter().sum::<f64>() / values.len() as f64,
        })
    }

    fn report(state: &Shared, options: &Options) {
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.unwrap();
        let ms = |from: i64, to: i64| (to - from) as f64 * 1e3 / frequency as f64;
        let measured = state.clicks[WARMUP_CLICKS.min(state.clicks.len())..].to_vec();
        let complete = measured
            .iter()
            .filter(|c| {
                c.input.is_some()
                    && c.prepare.is_some()
                    && c.presented.is_some()
                    && c.screen.is_some()
                    && c.accumulated_frames <= 1
            })
            .collect::<Vec<_>>();
        let span = |f: &dyn Fn(&Click) -> f64| summarize(complete.iter().map(|c| f(c)).collect());
        let mut stages = serde_json::Map::new();
        for (index, name) in STAGE_NAMES.iter().enumerate() {
            stages.insert(
                (*name).into(),
                span(&|c| c.stages_ns.unwrap()[index] as f64 / 1e6),
            );
        }
        let surface = state.surface.as_ref().map(|info| {
            serde_json::json!({
                "present_mode": format!("{:?}", info.present_mode),
                "desired_maximum_frame_latency": info.desired_maximum_frame_latency,
                "extent": info.extent,
                "refresh_rate_millihertz": info.display.as_ref()
                    .and_then(|d| d.refresh_rate_millihertz).map(|r| r.get()),
            })
        });
        let report = serde_json::json!({
            "backend": state.backend,
            "frame_demand": options.continuous.map_or("on-demand".to_string(), |fps| format!("continuous {fps}")),
            "surface": surface,
            "clicks": measured.len(),
            "complete": complete.len(),
            "dropped_accumulated": measured.iter().filter(|c| c.accumulated_frames > 1).count(),
            "missing_screen": measured.iter().filter(|c| c.screen.is_none()).count(),
            "ms": {
                "post_to_screen": span(&|c| ms(c.post, c.screen.unwrap())),
                "post_to_input": span(&|c| ms(c.post, c.input.unwrap())),
                "input_to_prepare": span(&|c| ms(c.input.unwrap(), c.prepare.unwrap())),
                "prepare_to_presented": span(&|c| ms(c.prepare.unwrap(), c.presented.unwrap())),
                "presented_to_screen": span(&|c| ms(c.presented.unwrap(), c.screen.unwrap())),
                "post_to_presented": span(&|c| ms(c.post, c.presented.unwrap())),
            },
            "host_stage_ms": stages,
            "frames": frame_summary(&state.frames, options.continuous, frequency),
        });
        let text = serde_json::to_string_pretty(&report).unwrap();
        println!("{text}");
        if let Some(path) = &options.output {
            std::fs::write(path, text).unwrap();
            let raw = path.with_extension("clicks.json");
            std::fs::write(raw, serde_json::to_string(&measured).unwrap()).unwrap();
        }
    }

    /// Presented-frame cadence over the measured clicks: throughput, the
    /// interval between `present` returns, intervals longer than 1.5 target
    /// periods (a skipped refresh), and the per-frame acquire wait.
    fn frame_summary(
        frames: &[(i64, u64)],
        continuous: Option<std::num::NonZeroU32>,
        frequency: i64,
    ) -> serde_json::Value {
        if frames.len() < 2 {
            return serde_json::Value::Null;
        }
        let ms = |from: i64, to: i64| (to - from) as f64 * 1e3 / frequency as f64;
        let intervals: Vec<f64> = frames.windows(2).map(|w| ms(w[0].0, w[1].0)).collect();
        let span_ms = ms(frames[0].0, frames[frames.len() - 1].0);
        let long = continuous.map(|fps| {
            let limit = 1.5e3 / fps.get() as f64;
            intervals.iter().filter(|&&i| i > limit).count()
        });
        serde_json::json!({
            "count": frames.len(),
            "fps": (frames.len() - 1) as f64 * 1e3 / span_ms,
            "interval_ms": summarize(intervals),
            "long_intervals": long,
            "acquire_ms": summarize(frames.iter().map(|f| f.1 as f64 / 1e6).collect()),
        })
    }

    pub fn main() -> Result<(), nana_ui::HostedRunError> {
        let _diagnostics = {
            use nana_ui::diagnostics::*;
            install(
                DiagnosticsConfig {
                    metrics: true,
                    persist: PersistMode::Off,
                    panic_hook: false,
                    ..Default::default()
                },
                SessionMetadata::new("dev.nana.input-latency", "Input latency probe", "0"),
                DiagnosticsPaths::in_memory(),
            )
            .expect("latency probe owns diagnostics instance")
        };
        let mut window = WindowDescriptor::new(TITLE).initial_size(480.0, 320.0);
        window.initial_position = Some((120.0, 120.0));
        run_runtime::<Probe>(window)
    }
}
