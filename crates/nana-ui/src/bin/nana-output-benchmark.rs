//! Real GPU work observations for Issue #242. Readback-free; timing measures
//! producer calls, with non-blocking completion polling outside that sample.
//! Native Window/XR/Web adapters require their concrete platform probes and
//! are explicitly unavailable in this windowless executable.

#[path = "support/output_allocator.rs"]
mod output_allocator;
#[path = "support/output_transport.rs"]
mod output_transport;
use output_allocator::Counts as HeapCounts;

use nana_gpu::{
    __framework, GpuContext, GpuPolicyStats, GpuTextureDescriptor, GpuTextureFormat,
    GpuTextureUsages,
};
use nana_ui::output::{
    EmbeddedSurfaceNode, ExternalFrame, ExternalRenderOutcome, ExternalSurface,
    ExternalSurfaceConfig, OutputAlpha, OutputConsumer, OutputConsumerId, OutputConsumerKind,
    OutputOwnership, OutputTextAa, OutputWorkObservation, PresentationDriver, PresentationId,
    PresentationWork, PresenterCapabilities, RenderTargetRequirements,
};
use nana_ui::runtime::{DocumentId, GpuTextureView, LayoutViewport, RuntimeDocument, Text};
use nana_ui::{
    HostTextureAlphaMode, HostTextureRegistry, NanaTextShaper, ScenePaintViewport,
    ScenePresentationProfile,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const FRAMES: usize = 240;
const WARMUP: usize = 30;
const EXTENT: [u32; 2] = [128, 96];
const WORKLOADS: [&str; 8] = [
    "direct",
    "static",
    "multi",
    "nested",
    "conversion",
    "slow",
    "recovery",
    "incompatible",
];
type BenchResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> BenchResult<()> {
    let mut output = None;
    let mut transports_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => output = args.next().map(PathBuf::from),
            "--transport-only" => transports_only = true,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let (gpu, adapter) = match request_gpu() {
        Some(result) => result,
        None => {
            if transports_only {
                let cases = output_transport::WORKLOADS
                    .into_iter()
                    .map(|name| (name, unavailable(name, "no GPU adapter", "unavailable")))
                    .collect::<BTreeMap<_, _>>();
                write_report(
                    output,
                    &json!({"schema_version":1,"status":"unsupported","unsupported_reason":"no GPU adapter","transport_cases":cases}),
                )?;
                std::process::exit(2);
            }
            let cases = WORKLOADS
                .into_iter()
                .map(|name| (name, unavailable(name, "no GPU adapter", "unavailable")))
                .collect::<BTreeMap<_, _>>();
            write_report(
                output,
                &json!({"schema_version":1,"benchmark":"nana-output-benchmark","status":"unsupported","unsupported_reason":"no GPU adapter","output_cases":cases}),
            )?;
            std::process::exit(2);
        }
    };
    if transports_only {
        let report = output_transport::run(&gpu, &adapter);
        let failed = report["transport_cases"]
            .as_object()
            .is_none_or(|cases| cases.values().any(|case| case["status"] != "ok"));
        write_report(output, &report)?;
        if failed {
            std::process::exit(1);
        }
        return Ok(());
    }
    let mut cases = BTreeMap::new();
    cases.insert(
        "direct",
        unavailable(
            "direct",
            "real Window/Surface required; use output-direct-probe",
            "unavailable",
        ),
    );
    for workload in &WORKLOADS[1..] {
        let before = gpu.policy().stats();
        let result = run_case(&gpu, workload);
        let after = gpu.policy().stats();
        let mut case = match result {
            Ok(case) => case,
            Err(error) => {
                json!({"status":"error","evidence":"actual-gpu","params":{"workload":workload,"frames":FRAMES},"error":error.to_string()})
            }
        };
        case["adapter"] = json!(adapter);
        case["gpu_policy"] = json!({"frame_slot_waits":after.frame_slot_waits-before.frame_slot_waits,"frame_slot_stalls":after.frame_slot_stalls-before.frame_slot_stalls,"upload_ring_waits":after.upload_ring_waits-before.upload_ring_waits,
            "upload_bytes":after.upload_bytes-before.upload_bytes,"upload_writes":after.upload_writes-before.upload_writes,"upload_copies":after.upload_copies-before.upload_copies,
            "scope":"whole workload device-policy delta, including setup, 30 warmup frames, 240 measured frames and cleanup"});
        cases.insert(workload, case);
    }
    let failed = cases.values().any(|case| case["status"] == "error");
    write_report(
        output,
        &json!({"schema_version":1,"benchmark":"nana-output-benchmark","output_cases":cases,
        "instrumentation":{
            "cpu_call_clock":"std::time::Instant elapsed wall time; never GPU execution time",
            "gpu_timestamp_ms":null,"gpu_timestamp_status":"not instrumented",
            "process_heap_allocations":null,"process_heap_status":"foreign/process-wide malloc not instrumented; sampled Rust GlobalAlloc deltas reported per case",
            "driver_vram_bytes":null,"driver_vram_status":"driver VRAM not instrumented"
        },
        "adapter_matrix":{
            "headless":{"evidence":"actual-gpu"},"nested":{"evidence":"actual-gpu"},
            "native_host":{"evidence":"unavailable","reason":"requires native import probe"},
            "xr":{"evidence":"unavailable","reason":"concrete OpenVR transport is not implemented; generic transport and mock retirement fixtures do not prove an XR reference path"},
            "web":{"evidence":"unavailable","reason":"concrete Chromium/VideoFrame bridge is not implemented; the installed-bridge wrapper alone is not a browser reference path"},
            "cpu_fallback":{"evidence":"unavailable","reason":"no CPU fallback implementation; never silently selected"},
            "device_loss":{"evidence":"unavailable","reason":"resize recovery is measured; actual device loss needs platform probe"}
        }}),
    )?;
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn write_report(path: Option<PathBuf>, report: &Value) -> BenchResult<()> {
    let text = serde_json::to_string_pretty(report)?;
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    } else {
        println!("{text}");
    }
    Ok(())
}

fn unavailable(workload: &str, reason: &str, evidence: &str) -> Value {
    json!({"status":"unsupported","evidence":evidence,"params":{"workload":workload,"frames":FRAMES},"unsupported_reason":reason})
}

fn request_gpu() -> Option<(GpuContext, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .ok()?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("output benchmark"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .ok()?;
    Some((
        __framework::adopt(adapter, device, queue),
        format!("{} ({:?})", info.name, info.backend),
    ))
}

fn consumer(gpu: &GpuContext, id: u64) -> OutputConsumer {
    OutputConsumer {
        id: OutputConsumerId(id),
        kind: OutputConsumerKind::External,
        requirements: RenderTargetRequirements {
            presentation: PresentationId(1),
            viewport_revision: 1,
            extent: EXTENT,
            profile: ScenePresentationProfile::sdr(GpuTextureFormat::RGBA8_UNORM_SRGB),
            alpha: OutputAlpha::LinearPremultiplied,
            text_aa: OutputTextAa::Grayscale,
            device_generation: gpu.generation(),
            ownership: OutputOwnership::SameDevice,
            persistent: true,
        },
        capabilities: PresenterCapabilities {
            sampled: true,
            copy_destination: true,
            ..Default::default()
        },
    }
}

fn document(texture: bool) -> BenchResult<RuntimeDocument> {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    if texture {
        document
            .context_mut()
            .create_component(id, GpuTextureView::new("child"))?;
    } else {
        document
            .context_mut()
            .create_component(id, Text::new("Retained NanaUI presentation"))?;
    }
    document.flush(
        LayoutViewport::new(128.0, 96.0),
        &mut NanaTextShaper::default(),
    )?;
    if document.scene().primitive_count() == 0 {
        return Err("benchmark fixture extracted no drawable primitives".into());
    }
    Ok(document)
}

fn completed(
    driver: &mut PresentationDriver,
    consumer: OutputConsumerId,
) -> BenchResult<ExternalFrame> {
    completed_at(driver, consumer, None)
}
fn completed_at(
    driver: &mut PresentationDriver,
    consumer: OutputConsumerId,
    wanted: Option<u64>,
) -> BenchResult<ExternalFrame> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        driver.poll();
        match driver.sample(consumer) {
            Ok(frame) => {
                if wanted.is_none_or(|revision| revision == frame.content_revision()) {
                    return Ok(frame);
                }
            }
            Err(nana_ui::output::PresentationError::Surface(
                nana_ui::output::ExternalSurfaceError::NoCompletedFrame,
            )) => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            return Err("GPU completion timed out".into());
        }
        std::thread::yield_now();
    }
}

fn render(
    driver: &mut PresentationDriver,
    document: &RuntimeDocument,
    revision: u64,
    textures: Option<&HostTextureRegistry>,
    measured: bool,
) -> BenchResult<u64> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let outcomes = driver.render_scene(
            document.scene(),
            [128.0, 96.0],
            revision,
            [0.1, 0.2, 0.3, 1.0],
            textures,
            None,
        )?;
        if outcomes
            .iter()
            .all(|outcome| !matches!(outcome, ExternalRenderOutcome::Deferred))
        {
            return match outcomes[0] {
                ExternalRenderOutcome::Submitted {
                    content_revision, ..
                }
                | ExternalRenderOutcome::Reused {
                    content_revision, ..
                } => Ok(content_revision),
                ExternalRenderOutcome::Deferred => unreachable!(),
            };
        }
        if measured {
            return Err("unexpected Deferred in measured producer iteration; refusing partial-attempt work counts".into());
        }
        if Instant::now() >= deadline {
            return Err("producer remained deferred".into());
        }
        driver.poll();
        std::thread::yield_now();
    }
}

fn work(counters: &PresentationWork) -> BenchResult<BTreeMap<String, usize>> {
    let pairs = [
        ("extra_passes", counters.output_extra_passes),
        ("gpu_copies", counters.output_gpu_copies),
        ("gpu_copy_bytes", counters.output_gpu_copy_bytes),
        ("target_recreates", counters.output_target_recreates),
        ("content_revisions", counters.output_content_revisions),
        ("idle_reuse_frames", counters.output_idle_reuse_frames),
        ("resolve_count", counters.output_resolve_count),
        ("gpu_convert_passes", counters.output_gpu_convert_passes),
        ("plan_rebuilds", counters.output_plan_rebuilds),
        ("ui_renders", counters.output_ui_renders),
        ("submissions", counters.output_submissions),
        ("cpu_readbacks", counters.output_cpu_readbacks),
        ("deferred", counters.output_deferred),
    ];
    pairs
        .into_iter()
        .map(|(key, value)| {
            value
                .map(|value| (key.to_owned(), value))
                .ok_or_else(|| format!("missing observed output counter {key}").into())
        })
        .collect()
}
fn add(total: &mut BTreeMap<String, usize>, current: BTreeMap<String, usize>) {
    for (key, value) in current {
        *total.entry(key).or_default() += value;
    }
}
fn observation(work: OutputWorkObservation) -> BenchResult<BTreeMap<String, usize>> {
    let mut counters = PresentationWork::default();
    counters.record_output_work(work);
    self::work(&counters)
}
fn finish(workload: &str, work: BTreeMap<String, usize>, times: Vec<f64>, proof: Value) -> Value {
    let timing_samples = times.len();
    json!({"status":"ok","evidence":"actual-gpu","params":{"workload":workload,"frames":FRAMES},
        "output_work":work,"timing_ms":percentiles(times),"timing_samples":timing_samples,"warmup_frames":WARMUP,
        "measurement_scope":"output_work and timing exclude setup and warmup; gpu_policy explicitly includes them",
        "gpu_timestamp_ms":null,"process_heap_allocations":null,"driver_vram_bytes":null,
        "timing_scope":if workload == "recovery" { "producer render plus nonblocking GPU completion polling; no wall-clock gate" } else { "producer call only; completion polling outside sample; no wall-clock gate" }, "proof":proof})
}

fn percentiles(mut times: Vec<f64>) -> Value {
    times.sort_by(f64::total_cmp);
    let at = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    json!({"p50":at(0.5),"p95":at(0.95),"p99":at(0.99)})
}

#[derive(Default, serde::Serialize)]
struct MeasuredPolicy {
    samples: usize,
    upload_bytes: u64,
    upload_writes: u64,
    upload_copies: u64,
    buffer_reallocations: u64,
    frame_slot_waits: u64,
    frame_slot_stalls: u64,
    upload_ring_waits: u64,
}
impl MeasuredPolicy {
    fn observe(&mut self, before: GpuPolicyStats, after: GpuPolicyStats) {
        self.samples += 1;
        self.upload_bytes += after.upload_bytes - before.upload_bytes;
        self.upload_writes += after.upload_writes - before.upload_writes;
        self.upload_copies += after.upload_copies - before.upload_copies;
        self.buffer_reallocations += after.buffer_reallocations - before.buffer_reallocations;
        self.frame_slot_waits += after.frame_slot_waits - before.frame_slot_waits;
        self.frame_slot_stalls += after.frame_slot_stalls - before.frame_slot_stalls;
        self.upload_ring_waits += after.upload_ring_waits - before.upload_ring_waits;
    }
}

#[derive(Default, serde::Serialize)]
struct PainterCounts {
    draw_calls: usize,
    draw_batches: usize,
    batch_rebuilds: usize,
    gpu_upload_bytes: usize,
    gpu_buffer_reallocations: usize,
}
#[derive(Default)]
struct MeasuredPainter {
    encoded_frames: usize,
    skipped_frames: usize,
    counts: Option<PainterCounts>,
}
impl MeasuredPainter {
    fn observe(&mut self, work: &PresentationWork) -> BenchResult<()> {
        let renders = work
            .output_ui_renders
            .ok_or("missing producer render observation")?;
        if renders == 0 {
            self.skipped_frames += 1;
            return Ok(());
        }
        let counts = self.counts.get_or_insert_with(PainterCounts::default);
        // An encode with no metric is missing evidence, never an invented zero.
        counts.draw_calls += work
            .draw_calls
            .ok_or("encoded producer omitted draw_calls")?;
        counts.draw_batches += work
            .draw_batches
            .ok_or("encoded producer omitted draw_batches")?;
        counts.batch_rebuilds += work
            .batch_rebuilds
            .ok_or("encoded producer omitted batch_rebuilds")?;
        counts.gpu_upload_bytes += work
            .gpu_upload_bytes
            .ok_or("encoded producer omitted gpu_upload_bytes")?;
        counts.gpu_buffer_reallocations += work
            .gpu_buffer_reallocations
            .ok_or("encoded producer omitted gpu_buffer_reallocations")?;
        self.encoded_frames += renders;
        Ok(())
    }
    fn report(&self) -> Value {
        json!({"status":if self.encoded_frames==0 {"skipped"} else {"measured"},
            "encoded_frames":self.encoded_frames,"skipped_frames":self.skipped_frames,"work":self.counts,
            "scope":"measured producer Scene painter encodes only; skipped frames do not fabricate GPU counters; buffer reallocations are not process malloc"})
    }
}

fn attach_measurements(
    report: &mut Value,
    policy: &MeasuredPolicy,
    painter: Option<&MeasuredPainter>,
) {
    report["measurement_gpu_policy"] = json!(policy);
    report["measurement_gpu_policy"]["scope"] =
        json!("240 measured iterations; setup, warmup and cleanup excluded");
    report["producer_painter"] = painter.map_or_else(||json!({"status":"not instrumented","work":null,"encoded_frames":null,"skipped_frames":null,"reason":"ExternalSurface does not expose its internal painter metrics; actual measured device-policy deltas are separate"}), MeasuredPainter::report);
}

fn retain_warmup_slot(
    held: &mut Vec<ExternalFrame>,
    frame: ExternalFrame,
    slots: usize,
) -> BenchResult<()> {
    if held.iter().any(|old| {
        old.texture()
            .is_ok_and(|texture| frame.texture().is_ok_and(|next| texture.ptr_eq(next)))
    }) {
        return Err("warmup reused storage while an earlier revision remained leased".into());
    }
    held.push(frame);
    if held.len() == slots {
        held.clear();
    }
    Ok(())
}

fn run_case(gpu: &GpuContext, workload: &str) -> BenchResult<Value> {
    if workload == "slow" || workload == "recovery" {
        return surface_case(gpu, workload);
    }
    let document = document(false)?;
    let mut first = consumer(gpu, 1);
    if workload == "conversion" {
        first.requirements.profile = ScenePresentationProfile::sdr(GpuTextureFormat::BGRA8_UNORM);
        first.requirements.alpha = OutputAlpha::GammaPremultiplied;
    }
    let mut consumers = vec![first];
    if workload == "multi" {
        consumers.extend([consumer(gpu, 2), consumer(gpu, 3)]);
    }
    // Two groups the planner cannot merge: the second needs a float working
    // target. They render into one submission through one shared painter.
    let incompatible = workload == "incompatible";
    if incompatible {
        let mut second = consumer(gpu, 2);
        second.requirements.profile = first.requirements.profile.with_force_float_working();
        consumers.push(second);
    }
    if workload == "conversion" {
        let mut second = first;
        second.id = OutputConsumerId(2);
        second.kind = OutputConsumerKind::Window;
        second.requirements.profile =
            ScenePresentationProfile::sdr(GpuTextureFormat::BGRA8_UNORM_SRGB);
        second.capabilities.sampled = false;
        consumers.push(second);
    }
    let mut driver = PresentationDriver::new(gpu);
    driver.configure(&consumers)?;
    render(&mut driver, &document, 0, None, false)?;
    drop(completed(&mut driver, first.id)?);
    let mut total = BTreeMap::new();
    let mut times = Vec::new();
    let mut samples = 0;
    let mut measured_policy = MeasuredPolicy::default();
    let mut measured_heap = HeapCounts::default();
    let mut measured_painter = MeasuredPainter::default();
    let warmup_slots =
        ExternalSurfaceConfig::new(EXTENT, first.requirements.profile.target_format).slots;
    let mut warmup_leases = Vec::new();
    if workload == "nested" {
        let child =
            EmbeddedSurfaceNode::from_frame(completed(&mut driver, first.id)?, [128.0, 96.0])?;
        let textures = HostTextureRegistry::new();
        child.register_host_texture(&textures, "child", HostTextureAlphaMode::Premultiplied);
        let parent_document = self::document(true)?;
        let mut parent = PresentationDriver::new(gpu);
        parent.configure(&[consumer(gpu, 4)])?;
        let mut parent_times = Vec::new();
        let mut parent_policy = MeasuredPolicy::default();
        let mut parent_heap = HeapCounts::default();
        let mut parent_painter = MeasuredPainter::default();
        for frame in 0..WARMUP + FRAMES {
            let measured = frame >= WARMUP;
            let child_before = gpu.policy().stats();
            let heap_before = HeapCounts::snapshot();
            let at = Instant::now();
            render(&mut driver, &document, 0, None, measured)?;
            let heap_after = HeapCounts::snapshot();
            if measured {
                measured_heap.observe(heap_before, heap_after);
                times.push(at.elapsed().as_secs_f64() * 1000.0);
                add(&mut total, work(driver.work())?);
                measured_policy.observe(child_before, gpu.policy().stats());
                measured_painter.observe(driver.work())?;
            }
            let parent_before = gpu.policy().stats();
            let parent_heap_before = HeapCounts::snapshot();
            let parent_at = Instant::now();
            let wanted = render(
                &mut parent,
                &parent_document,
                frame as u64 + 1,
                Some(&textures),
                measured,
            )?;
            let parent_heap_after = HeapCounts::snapshot();
            if measured {
                parent_heap.observe(parent_heap_before, parent_heap_after);
                parent_times.push(parent_at.elapsed().as_secs_f64() * 1000.0);
                parent_policy.observe(parent_before, gpu.policy().stats());
                parent_painter.observe(parent.work())?;
            }
            let parent_frame = completed_at(&mut parent, OutputConsumerId(4), Some(wanted))?;
            if frame < warmup_slots {
                retain_warmup_slot(&mut warmup_leases, parent_frame, warmup_slots)?;
            }
            if measured {
                samples += parent
                    .work()
                    .output_submissions
                    .ok_or("parent submit observation missing")?;
            }
        }
        let mut report = finish(
            workload,
            total,
            times,
            json!({"consumer_samples":samples,"parent_submissions":samples,"scene_primitives":document.scene().primitive_count(),"child_content_revision":child.content_revision(),"warmup_retained_slots":warmup_slots}),
        );
        attach_measurements(&mut report, &measured_policy, Some(&measured_painter));
        report["parent_measurement_gpu_policy"] = json!(parent_policy);
        report["parent_painter"] = parent_painter.report();
        report["measurement_rust_heap"] = measured_heap.report(
            FRAMES,
            "static child producer call only; polling and report aggregation excluded",
        );
        report["parent_measurement_rust_heap"] = parent_heap.report(FRAMES, "parent producer render/submit call only; completion polling and report aggregation excluded");
        report["consumer_timing_samples"] = json!(parent_times.len());
        report["consumer_call_ms"] = percentiles(parent_times);
        report["timing_scope"] = json!(
            "timing_ms is static child producer reuse; consumer_call_ms is real parent render/submit; completion polling outside samples"
        );
        return Ok(report);
    }
    let destination = if workload == "conversion" {
        Some(gpu.create_texture(&GpuTextureDescriptor {
            label: Some("output conversion benchmark"),
            width: 128,
            height: 96,
            format: GpuTextureFormat::BGRA8_UNORM_SRGB,
            usage: GpuTextureUsages::RENDER_TARGET,
        })?)
    } else {
        None
    };
    for frame in 0..WARMUP + FRAMES {
        let measured = frame >= WARMUP;
        let policy_before = gpu.policy().stats();
        let heap_before = HeapCounts::snapshot();
        let at = Instant::now();
        let wanted = render(
            &mut driver,
            &document,
            if workload == "static" {
                0
            } else {
                frame as u64 + 1
            },
            None,
            measured,
        )?;
        let heap_after = HeapCounts::snapshot();
        if measured {
            measured_heap.observe(heap_before, heap_after);
            times.push(at.elapsed().as_secs_f64() * 1000.0);
        }
        let sampled = completed_at(&mut driver, first.id, Some(wanted))?;
        if workload != "static" && frame < warmup_slots {
            retain_warmup_slot(&mut warmup_leases, sampled.clone(), warmup_slots)?;
        }
        for endpoint in &consumers {
            let other = if incompatible {
                completed_at(&mut driver, endpoint.id, Some(wanted))?
            } else {
                driver.sample(endpoint.id)?
            };
            if sampled.texture()?.ptr_eq(other.texture()?)
                == (incompatible && endpoint.id != first.id)
            {
                return Err(if incompatible {
                    "incompatible groups shared one canonical texture"
                } else {
                    "compatible consumers did not share canonical texture"
                }
                .into());
            }
            if measured {
                samples += 1;
            }
        }
        if let Some(destination) = &destination {
            let mut frame = gpu.begin_frame("output final transfer");
            driver.present_to(
                OutputConsumerId(2),
                &mut frame,
                &destination.render_target()?,
            )?;
            frame.submit();
        }
        if measured {
            add(&mut total, work(driver.work())?);
            measured_policy.observe(policy_before, gpu.policy().stats());
            measured_painter.observe(driver.work())?;
        }
        if measured && destination.is_some() {
            // `present_to` only records; this benchmark owns the separate
            // consumer submission above and must count that actual submit too.
            *total.entry("submissions".to_owned()).or_default() += 1;
        }
    }
    let mut report = finish(
        workload,
        total,
        times,
        json!({"consumer_samples":samples,"canonical_targets":driver.plan().canonical_target_count(),"scene_primitives":document.scene().primitive_count(),"consumer_submissions":if workload=="conversion" { FRAMES } else {0},"warmup_retained_slots":if workload=="static" {1} else {warmup_slots}}),
    );
    attach_measurements(&mut report, &measured_policy, Some(&measured_painter));
    report["measurement_rust_heap"] = measured_heap.report(FRAMES, "producer render/submit call only; consumer final transfer, completion polling and report aggregation excluded");
    report["painter_resources"] = painter_resources(&driver);
    Ok(report)
}

/// What the one painter the groups share holds after the measured frames.
/// Texel bytes as the painter allocates them; driver padding and VRAM are
/// not observed.
fn painter_resources(driver: &PresentationDriver) -> Value {
    match (
        driver.text_glyph_counters(),
        driver.painter_resource_counters(),
    ) {
        (Some(text), Some(images)) => json!({"status":"measured",
            "glyph_atlas_pages":text.glyph_atlas_pages,"glyph_atlas_bytes":text.glyph_atlas_bytes,
            "glyph_raster_cache_bytes":text.glyph_raster_cache_bytes,"glyph_upload_bytes":text.glyph_upload_bytes,
            "icon_atlases":images.icon_atlases,"icon_atlas_bytes":images.icon_atlas_bytes,
            "url_textures":images.url_textures,"url_texture_gpu_bytes":images.url_texture_gpu_bytes,
            "url_texture_cpu_bytes":images.url_texture_cpu_bytes,"url_pending":images.url_pending,
            "scope":"cumulative over setup, warmup and measured frames; painter texel bytes, not driver VRAM"}),
        _ => json!({"status":"unavailable","reason":"no configured painter"}),
    }
}

fn viewport(extent: [u32; 2]) -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [extent[0] as f32, extent[1] as f32],
        physical_size: extent,
        scale_factor: 1.0,
        scene_origin: [0.0; 2],
        target_origin: [0.0; 2],
        clear_color: [0.1, 0.2, 0.3, 1.0],
        clear: true,
    }
}
fn surface_render(
    surface: &mut ExternalSurface,
    document: &RuntimeDocument,
    revision: u64,
    extent: [u32; 2],
    measured: bool,
) -> BenchResult<ExternalFrame> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut wanted = None;
    loop {
        surface.poll();
        if wanted.is_none() {
            match surface.render(document.scene(), revision, viewport(extent), None, None)? {
                ExternalRenderOutcome::Submitted {
                    content_revision, ..
                }
                | ExternalRenderOutcome::Reused {
                    content_revision, ..
                } => wanted = Some(content_revision),
                ExternalRenderOutcome::Deferred if measured => return Err("unexpected Deferred in measured recovery iteration; refusing hidden retry work".into()),
                ExternalRenderOutcome::Deferred => {}
            }
        }
        if let Ok(frame) = surface.sample()
            && Some(frame.content_revision()) == wanted
        {
            return Ok(frame);
        }
        if Instant::now() >= deadline {
            return Err("surface completion timed out".into());
        }
        std::thread::yield_now();
    }
}
fn surface_case(gpu: &GpuContext, workload: &str) -> BenchResult<Value> {
    let document = document(false)?;
    let config = ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM_SRGB);
    let mut surface = ExternalSurface::new(gpu, config)?;
    let mut held = Vec::new();
    let mut total = BTreeMap::new();
    let mut times = Vec::new();
    let mut measured_policy = MeasuredPolicy::default();
    let mut measured_heap = HeapCounts::default();
    if workload == "slow" {
        for revision in 0..config.slots {
            held.push(surface_render(
                &mut surface,
                &document,
                revision as u64,
                EXTENT,
                false,
            )?);
        }
        for frame in 0..WARMUP + FRAMES {
            let measured = frame >= WARMUP;
            let policy_before = gpu.policy().stats();
            let heap_before = HeapCounts::snapshot();
            let at = Instant::now();
            let outcome = surface.render(document.scene(), 99, viewport(EXTENT), None, None)?;
            let heap_after = HeapCounts::snapshot();
            if measured {
                measured_heap.observe(heap_before, heap_after);
                times.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            drop(surface.sample()?);
            if !matches!(outcome, ExternalRenderOutcome::Deferred) {
                return Err("held pool failed to apply backpressure".into());
            }
            if measured {
                add(&mut total, observation(surface.last_work())?);
                measured_policy.observe(policy_before, gpu.policy().stats());
            }
        }
        held.clear();
        drop(surface_render(&mut surface, &document, 100, EXTENT, false)?);
        let mut report = finish(
            workload,
            total,
            times,
            json!({"consumer_samples":FRAMES,"held_slots":config.slots,"recovered_after_release":1,"scene_primitives":document.scene().primitive_count()}),
        );
        attach_measurements(&mut report, &measured_policy, None);
        report["measurement_rust_heap"] = measured_heap.report(
            FRAMES,
            "backpressured producer render call only; sample and report aggregation excluded",
        );
        return Ok(report);
    }
    let initial = surface_render(&mut surface, &document, 0, EXTENT, false)?;
    let old_generation = initial.resource_generation();
    for revision in 1..=WARMUP {
        drop(surface_render(
            &mut surface,
            &document,
            revision as u64,
            EXTENT,
            false,
        )?);
    }
    let mut extent = EXTENT;
    let mut resizes = 0;
    let mut stale_rejections = 0;
    for frame in 0..FRAMES {
        let policy_before = gpu.policy().stats();
        if frame == FRAMES / 2 {
            extent = [160, 96];
            surface.resize(
                gpu,
                ExternalSurfaceConfig::new(extent, GpuTextureFormat::RGBA8_UNORM_SRGB),
            )?;
            add(&mut total, observation(surface.last_work())?);
            resizes += 1;
            if initial.validate().is_err() {
                stale_rejections += 1;
            }
        }
        let heap_before = HeapCounts::snapshot();
        let at = Instant::now();
        drop(surface_render(
            &mut surface,
            &document,
            (frame + WARMUP) as u64 + 1,
            extent,
            true,
        )?);
        let heap_after = HeapCounts::snapshot();
        times.push(at.elapsed().as_secs_f64() * 1000.0);
        measured_heap.observe(heap_before, heap_after);
        add(&mut total, observation(surface.last_work())?);
        measured_policy.observe(policy_before, gpu.policy().stats());
    }
    let mut report = finish(
        workload,
        total,
        times,
        json!({"consumer_samples":FRAMES,"resizes":resizes,"stale_generation_rejections":stale_rejections,
        "old_resource_generation":old_generation,"new_resource_generation":surface.resource_generation(),"scene_primitives":document.scene().primitive_count(),"device_loss_evidence":"unavailable"}),
    );
    attach_measurements(&mut report, &measured_policy, None);
    report["measurement_rust_heap"] = measured_heap.report(FRAMES, "producer render plus nonblocking completion polling; resize and report aggregation excluded");
    Ok(report)
}
