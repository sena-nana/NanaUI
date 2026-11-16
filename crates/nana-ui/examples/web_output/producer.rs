use nana_gpu::{
    GpuContext, GpuTextureFormat, NativeExportOutcome, NativeExportPool, NativeFrameToken,
};
use nana_ui::{
    AlphaEncoding, NanaTextShaper, ScenePaintViewport,
    output::{ExternalSurface, ExternalSurfaceConfig},
    runtime::{DocumentId, LayoutViewport, RuntimeDocument, Stack, Text},
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::{self, BufRead, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    sync::mpsc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const EXTENT: [u32; 2] = [640, 360];
const WARMUP_FRAMES: u64 = 30;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
    fn GetCurrentProcess() -> RawHandle;
    fn DuplicateHandle(
        source_process: RawHandle,
        source: RawHandle,
        destination_process: RawHandle,
        destination: *mut RawHandle,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
}

struct ElectronProcess(OwnedHandle);
impl ElectronProcess {
    fn open(pid: u32) -> Result<Self> {
        // PROCESS_DUP_HANDLE. No pixel, address-space or GPU device access.
        let handle = unsafe { OpenProcess(0x0040, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }
    fn duplicate(&self, handle: RawHandle) -> Result<u64> {
        let mut remote = std::ptr::null_mut();
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle,
                self.0.as_raw_handle(),
                &mut remote,
                0,
                0,
                2,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(remote as usize as u64)
    }
    fn close_remote(&self, handle: u64) -> Result<()> {
        let mut local = std::ptr::null_mut();
        // DUPLICATE_CLOSE_SOURCE | DUPLICATE_SAME_ACCESS. Electron's import
        // makes its own duplicate; retire our temporary remote NT handle.
        let ok = unsafe {
            DuplicateHandle(
                self.0.as_raw_handle(),
                handle as usize as RawHandle,
                GetCurrentProcess(),
                &mut local,
                0,
                0,
                3,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        drop(unsafe { OwnedHandle::from_raw_handle(local) });
        Ok(())
    }
}

struct Pending {
    token: Option<NativeFrameToken>,
    id: u64,
    remote: Option<u64>,
    sent: bool,
    copied: Instant,
}

// On transport failure no further writes are allowed to this pool. Prevent
// token Drop from claiming a consumer release that Chromium never confirmed.
impl Drop for Pending {
    fn drop(&mut self) {
        // A published remote handle may still be queued in Electron's stdin.
        // Without an import acknowledgement, closing it could let that numeric
        // HANDLE be recycled before Electron imports it. Leave this one bounded
        // handle to the receiving process; Windows closes it on process exit.
        if let Some(token) = self.token.as_mut() {
            token.accept_release();
        }
    }
}

impl Pending {
    fn token(&self) -> &NativeFrameToken {
        self.token
            .as_ref()
            .expect("pending frame retains its native token")
    }
}

fn emit(value: Value) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer(&mut stdout, &value)?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

fn gpu() -> Result<(GpuContext, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .map_err(|error| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("DX12 adapter unavailable: {error}"),
        )
    })?;
    let name = adapter.get_info().name;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("NanaUI Electron shared texture producer"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))?;
    Ok((
        GpuContext::from_wgpu_with_loss_tracking(adapter, device, queue),
        name,
    ))
}

fn quantiles(values: &mut [f64]) -> Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return Value::Null;
    }
    let at = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    json!({"p50":at(0.5),"p95":at(0.95),"p99":at(0.99)})
}

fn work_report(work: nana_ui::OutputWorkObservation) -> Value {
    json!({"ui_renders":work.ui_renders,"gpu_copies":work.gpu_copies,
        "gpu_copy_bytes":work.gpu_copy_bytes,"extra_passes":work.extra_passes,
        "submissions":work.submissions,"cpu_readbacks":work.cpu_readbacks,
        "target_recreates":work.target_recreates,"deferred":work.deferred})
}

pub fn run() -> Result<()> {
    if std::mem::size_of::<usize>() != 8 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only x64 Windows NT handles are supported",
        )
        .into());
    }
    let mut args = std::env::args().skip(1);
    let mut pid = None;
    let mut required = 240_u64;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--electron-pid" => pid = Some(args.next().ok_or("missing PID")?.parse::<u32>()?),
            "--frames" => required = args.next().ok_or("missing frame count")?.parse()?,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    if required == 0 || required > 10_000 {
        return Err("frames must be 1..=10000".into());
    }
    let total_required = WARMUP_FRAMES + required;
    let electron = ElectronProcess::open(
        pid.filter(|pid| *pid != 0)
            .ok_or("--electron-pid required")?,
    )?;
    let (gpu, adapter) = gpu()?;
    let mut surface = ExternalSurface::new(
        &gpu,
        ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::BGRA8_UNORM).with_copy_source(true),
    )?;
    surface.set_alpha_encoding(AlphaEncoding::Gamma);
    let mut pool = NativeExportPool::new(&gpu, EXTENT).map_err(|error| match error {
        nana_gpu::NativeExportError::Unsupported(reason) if !gpu.is_lost() => {
            io::Error::new(io::ErrorKind::Unsupported, reason)
        }
        error => io::Error::other(error),
    })?;
    let document_id = DocumentId::new(1).ok_or("invalid document identity")?;
    let mut document = RuntimeDocument::new(document_id);
    document
        .context_mut()
        .create_component(document_id, Stack::fill_column(12.0))?;
    document
        .context_mut()
        .create_component(document_id, Text::new("NanaUI → DX12 → Electron → WebGPU"))?;
    document.flush(
        LayoutViewport::new(640.0, 360.0),
        &mut NanaTextShaper::default(),
    )?;
    if document.scene().primitive_count() == 0 {
        return Err("Electron source workload has no drawable UI primitives".into());
    }
    let mut viewport = ScenePaintViewport {
        logical_size: [640.0, 360.0],
        physical_size: EXTENT,
        scale_factor: 1.0,
        scene_origin: [0.0; 2],
        target_origin: [0.0; 2],
        clear_color: [0.02, 0.03, 0.04, 1.0],
        clear: true,
    };
    let (commands_tx, commands) = mpsc::sync_channel::<Option<Value>>(32);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let Ok(value) = serde_json::from_str(&line) else {
                break;
            };
            if commands_tx.send(Some(value)).is_err() {
                return;
            }
        }
        let _ = commands_tx.send(None);
    });
    let mut pending: Option<Pending> = None;
    let mut imported = HashSet::new();
    let mut displayed = HashSet::new();
    let mut released = HashSet::new();
    let mut last_release_value: Option<u64> = None;
    let mut emitted = 0_u64;
    let mut source_revision = None;
    let mut revision = 0_u64;
    let setup_work = nana_ui::OutputWorkObservation {
        target_recreates: surface.config().slots + nana_gpu::NATIVE_EXPORT_SLOTS,
        ..Default::default()
    };
    let mut work = nana_ui::OutputWorkObservation::default();
    let mut warmup_work = Value::Null;
    let mut warmup_proof = Value::Null;
    let mut warmup_ready_observations = 0;
    let mut warmup_timing = Value::Null;
    let mut warmup_timing_samples = 0;
    let mut measurement_started = false;
    let mut timings = Vec::new();
    let mut ready_latency = Vec::new();
    let started = Instant::now();
    let mut next_tick = started;
    loop {
        gpu.poll();
        if gpu.is_lost() || pool.completed_value() == Some(u64::MAX) {
            return Err("DX12 device was lost; old handles were retired".into());
        }
        for command in commands.try_iter() {
            let command = command.ok_or(
                "Electron disconnected with unconfirmed GPU resources; pool retired, never reused",
            )?;
            let id = command["id"]
                .as_u64()
                .ok_or("command has no frame identity")?;
            if id == 0
                || id > emitted
                || command["pool"].as_u64() != pool.pool_generation()
                || command["device_generation"].as_u64() != Some(gpu.generation().get())
                || command["resource_generation"].as_u64() != Some(surface.resource_generation())
            {
                return Err("stale or wrong-generation Electron receipt".into());
            }
            match command["type"].as_str() {
                Some("imported") => {
                    if pending.as_ref().map(|frame| frame.id) != Some(id) {
                        return Err("import receipt has no pending frame".into());
                    }
                    if !imported.insert(id) {
                        return Err("duplicate import receipt".into());
                    }
                    if let Some(handle) = pending.as_mut().and_then(|frame| frame.remote.take()) {
                        electron.close_remote(handle)?;
                    }
                }
                Some("displayed") => {
                    if !imported.contains(&id) || !displayed.insert(id) {
                        return Err("invalid or duplicate display receipt".into());
                    }
                }
                Some("released") => {
                    if pending.as_ref().map(|frame| frame.id) != Some(id) {
                        return Err("release receipt has no pending frame".into());
                    }
                    if !imported.contains(&id) || !displayed.contains(&id) || !released.insert(id) {
                        return Err("release is missing its import or was duplicated".into());
                    }
                    let mut frame = pending.take().ok_or("missing release target")?;
                    if let Some(handle) = frame.remote.take() {
                        electron.close_remote(handle)?;
                    }
                    // Only Electron's main-process allReferencesReleased can
                    // send this receipt. The foreign GPU is finished now.
                    let token = frame.token.take().ok_or("missing native release token")?;
                    last_release_value = Some(
                        last_release_value.map_or(token.release_value(), |previous| {
                            previous.max(token.release_value())
                        }),
                    );
                    drop(token); // producer queue signals release after ready
                }
                _ => return Err("unknown Electron receipt".into()),
            }
        }
        if let Some(frame) = pending.as_mut()
            && !frame.sent
            && pool
                .completed_value()
                .is_some_and(|value| value >= frame.token().ready_value())
        {
            let remote = electron.duplicate(
                frame
                    .token()
                    .texture_handle(frame.token().slot())
                    .ok_or("invalid native slot")?
                    .as_raw_handle(),
            )?;
            frame.remote = Some(remote);
            frame.sent = true;
            ready_latency.push(frame.copied.elapsed().as_secs_f64() * 1000.0);
            emit(
                json!({"type":"texture","id":frame.id,"pool":frame.token().pool_generation(),"resource_generation":surface.resource_generation(),"device_generation":gpu.generation().get(),"source_revision":source_revision,"nt_handle":format!("{remote:016x}"),"adapter_luid":frame.token().adapter_luid().to_string(),"width":EXTENT[0],"height":EXTENT[1],"pixel_format":"bgra","alpha":"opaque","color_space":"srgb","ready_observed":true}),
            )?;
        }
        let release_completed = pool
            .completed_value()
            .zip(last_release_value)
            .is_some_and(|(completed, released)| completed >= released);
        if !measurement_started
            && released.len() as u64 == WARMUP_FRAMES
            && pending.is_none()
            && release_completed
        {
            // The real foreign GPU and the producer release signal have both
            // completed the warmup. Keep receipt/revision identities monotonic.
            warmup_work = work_report(work);
            warmup_ready_observations = ready_latency.len();
            warmup_proof = json!({"native_handle_imports":imported.len(),"ready_observations":ready_latency.len(),
                "foreign_releases":released.len(),"displayed_frames":displayed.len(),"has_pending_gpu_reads":false});
            warmup_timing_samples = timings.len();
            warmup_timing = quantiles(&mut timings);
            work = Default::default();
            timings.clear();
            ready_latency.clear();
            measurement_started = true;
        }
        if released.len() as u64 == total_required
            && displayed.len() as u64 == total_required
            && pending.is_none()
            && release_completed
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(180) {
            return Err(
                "Electron consumer timed out; outstanding pool is quarantined, never reused".into(),
            );
        }
        if Instant::now() >= next_tick && emitted < total_required {
            next_tick = Instant::now() + Duration::from_millis(16);
            let tick = Instant::now();
            revision += 1;
            // Actual output pixels move; a synthetic host revision must not
            // manufacture activity for an otherwise static workload.
            viewport.clear_color = [
                0.03,
                0.08 + (revision % 120) as f32 / 120.0 * 0.18,
                0.15,
                1.0,
            ];
            surface.render(document.scene(), 0, viewport, None, None)?;
            let observed = surface.last_work();
            work.accumulate(observed);
            // Do not cross the phase boundary until warmup release is really
            // visible on the producer fence. Source UI ticks remain independent.
            if pending.is_none() && (emitted < WARMUP_FRAMES || measurement_started) {
                if let Ok(source) = surface.sample()
                    && source_revision != Some(source.content_revision())
                {
                    if let Some(mut frame) = gpu.try_begin_frame("NanaUI Electron export") {
                        match pool.stage(&mut frame, source.texture()?)? {
                            NativeExportOutcome::Staged(staged) => {
                                let content = source.content_revision();
                                frame.keep_alive_until_complete(source);
                                let _submission = frame.submit();
                                let token = pool.finish(staged)?;
                                emitted += 1;
                                source_revision = Some(content);
                                pending = Some(Pending {
                                    token: Some(token),
                                    id: emitted,
                                    remote: None,
                                    sent: false,
                                    copied: Instant::now(),
                                });
                                work.gpu_copies += 1;
                                work.gpu_copy_bytes += EXTENT[0] as usize * EXTENT[1] as usize * 4;
                                work.submissions += 1;
                            }
                            NativeExportOutcome::Deferred(_) => {
                                work.deferred += 1;
                            }
                        }
                    } else {
                        work.deferred += 1;
                    }
                }
            } else {
                work.deferred += 1;
            }
            timings.push(tick.elapsed().as_secs_f64() * 1000.0);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let timing_samples = timings.len();
    let measured_count = |ids: &HashSet<u64>| ids.iter().filter(|id| **id > WARMUP_FRAMES).count();
    emit(json!({
        "type":"summary", "status":"ok", "evidence":"actual-electron-webgpu",
        "fixture":"electron-webgpu-output", "fixture_version":1,
        "adapter":adapter, "device_generation":gpu.generation().get(), "extent":EXTENT,
        "frames":measured_count(&displayed), "warmup_frames":WARMUP_FRAMES,
        "total_frames":displayed.len(), "output_work":work_report(work),
        "setup_work":work_report(setup_work), "warmup_output_work":warmup_work,
        "proof":{"native_handle_imports":measured_count(&imported),"ready_observations":ready_latency.len(),
            "foreign_releases":measured_count(&released),"displayed_frames":measured_count(&displayed),"has_pending_gpu_reads":false},
        "warmup_proof":warmup_proof,
        "total_proof":{"native_handle_imports":imported.len(),"ready_observations":warmup_ready_observations + ready_latency.len(),
            "foreign_releases":released.len(),"displayed_frames":displayed.len(),"has_pending_gpu_reads":false},
        "timing_ms":quantiles(&mut timings), "timing_samples":timing_samples,
        "warmup_timing_ms":warmup_timing,"warmup_timing_samples":warmup_timing_samples,
        "ready_poll_latency_ms":quantiles(&mut ready_latency),
        "gpu_time_ms":null,"driver_vram_bytes":null,
        "scope":"30 actual foreign GPU frames prewarm persistent pools; following measured source calls exclude GPU/IPC waits; consumer encoding/submission CPU measured separately; no historical time baseline"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateEventW(
            attributes: *const std::ffi::c_void,
            manual: i32,
            initial: i32,
            name: *const u16,
        ) -> RawHandle;
        fn SetEvent(event: RawHandle) -> i32;
        fn WaitForSingleObject(handle: RawHandle, milliseconds: u32) -> u32;
    }

    #[test]
    fn dropping_unacknowledged_delivery_keeps_the_real_remote_nt_handle_valid() {
        // The current process is the receiving process for this NT ownership
        // regression. No GPU, browser, or shadow ownership model is involved.
        let original = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!original.is_null());
        let original = unsafe { OwnedHandle::from_raw_handle(original) };
        let receiver = ElectronProcess::open(std::process::id()).unwrap();
        let remote = receiver.duplicate(original.as_raw_handle()).unwrap();
        drop(Pending {
            token: None,
            id: 1,
            remote: Some(remote),
            sent: true,
            copied: Instant::now(),
        });
        // Electron can still import the precise object named by the delivered
        // HANDLE even after producer teardown; it owns cleanup without an ACK.
        assert_ne!(unsafe { SetEvent(remote as usize as RawHandle) }, 0);
        assert_eq!(
            unsafe { WaitForSingleObject(original.as_raw_handle(), 0) },
            0
        );
        drop(unsafe { OwnedHandle::from_raw_handle(remote as usize as RawHandle) });
    }
}
