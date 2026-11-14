use nana_gpu::{GpuContext, GpuTextureFormat};
use nana_ui::ScenePresentationProfile;
use nana_ui::output::*;
use nana_ui_scene::UiScene;
use std::time::{Duration, Instant};

fn context() -> GpuContext {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("actual GPU adapter required");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("actual GPU required");
    GpuContext::from_wgpu(adapter, device, queue)
}

fn consumer(gpu: &GpuContext, id: u64, extent: [u32; 2]) -> OutputConsumer {
    OutputConsumer {
        id: OutputConsumerId(id),
        kind: OutputConsumerKind::External,
        requirements: RenderTargetRequirements {
            presentation: PresentationId(1),
            viewport_revision: 1,
            extent,
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

fn exact_filter(name: &str) -> String {
    match module_path!().split_once("::") {
        Some((_, prefix)) => format!("{prefix}::{name}"),
        None => name.to_owned(),
    }
}

fn run_child_test(name: &str, env_key: &str) {
    let filter = exact_filter(name);
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", &filter, "--nocapture", "--test-threads=1"])
        .env(env_key, "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || {
        use std::io::Read as _;
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        use std::io::Read as _;
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = stdout_reader.join().unwrap_or_default();
                let stderr = stderr_reader.join().unwrap_or_default();
                panic!(
                    "output_trace_owner child `{filter}` timed out after 15s:\n{}\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("failed waiting on output_trace_owner child `{filter}`: {err}");
            }
        }
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    assert!(
        status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn deferred_group_cannot_take_another_producers_submission_acquisition() {
    const CHILD: &str = "NANA_TRACE_MIXED_OWNER_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_child_test(
            "deferred_group_cannot_take_another_producers_submission_acquisition",
            CHILD,
        );
        return;
    }
    // This subprocess owns its context and intentionally saturates only one
    // retained pool. No global fixture device or frame-slot state is shared.
    let gpu = context();
    let scene = UiScene::new();
    let mut driver = PresentationDriver::new(&gpu);
    driver
        .configure(&[consumer(&gpu, 1, [32, 24]), consumer(&gpu, 2, [48, 24])])
        .unwrap();
    let mut held = Vec::new();
    for revision in 0..3 {
        let outcomes = driver
            .render_scene(&scene, [32.0, 24.0], revision, [0.0; 4], None, None)
            .unwrap();
        let ExternalRenderOutcome::Submitted {
            content_revision, ..
        } = outcomes[0]
        else {
            panic!("first pool must have a free slot")
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            driver.poll();
            if let Ok(frame) = driver.sample(OutputConsumerId(1))
                && frame.content_revision() == content_revision
            {
                held.push(frame);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "submitted output did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    driver.set_output_trace_enabled(true);
    let outcomes = driver
        .render_scene(&scene, [32.0, 24.0], 3, [0.0; 4], None, None)
        .unwrap();
    assert_eq!(outcomes[0], ExternalRenderOutcome::Deferred);
    assert!(matches!(
        outcomes[1],
        ExternalRenderOutcome::Submitted { .. }
    ));
    let producers = driver
        .output_traces()
        .iter()
        .filter(|trace| trace.role == OutputTraceRole::Producer)
        .collect::<Vec<_>>();
    assert_eq!(producers.len(), 2);
    assert_eq!(producers[0].work.submissions, 0);
    assert_eq!(producers[0].waits, None);
    assert_eq!(producers[0].state, OutputTraceState::Deferred);
    assert_eq!(producers[1].work.submissions, 1);
    assert_eq!(producers[1].waits, Some(OutputTraceWait::default()));
    assert_eq!(producers[1].state, OutputTraceState::Submitted);
}

#[test]
fn explicit_trace_capture_round_trips_nlog_without_per_frame_events() {
    const CHILD: &str = "NANA_OUTPUT_TRACE_NLOG_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_child_test(
            "explicit_trace_capture_round_trips_nlog_without_per_frame_events",
            CHILD,
        );
        return;
    }
    use nana_diagnostics::nlog::{self, Entry, Value};
    use nana_diagnostics::{
        DiagnosticsConfig, DiagnosticsPaths, PersistMode, SessionMetadata, Severity,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nana-output-trace-{}-{seq}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _cleanup = Cleanup(dir.clone());
    let guard = nana_diagnostics::install(
        DiagnosticsConfig {
            min_severity: Severity::Debug,
            persist: PersistMode::All,
            panic_hook: false,
            poll_interval: Duration::from_millis(1),
            ..Default::default()
        },
        SessionMetadata::new("dev.nana.output-trace-test", "Output trace round trip", "0"),
        DiagnosticsPaths::new(dir.join("logs"), dir.join("crash")),
    )
    .unwrap();
    let gpu = context();
    let mut snapshot = OutputTraceSnapshot::new(
        37,
        OutputConsumerId(91),
        OutputTraceRole::Consumer,
        OutputPath::GpuTransfer,
        GpuTextureFormat::BGRA8_UNORM,
        [128, 96],
        gpu.generation(),
    );
    snapshot.resource_generation = 3;
    snapshot.content_revision = 19;
    // Synthetic diagnostic data tests the actual codec/sink, not GPU work or
    // platform conformance. Ordinary observe/query calls must emit no event.
    for _ in 0..128 {
        snapshot.observe(
            nana_ui_core::OutputWorkObservation {
                idle_reuse_frames: 1,
                ..Default::default()
            },
            None,
        );
        assert_eq!(snapshot.consumer, OutputConsumerId(91));
    }
    snapshot.observe(
        nana_ui_core::OutputWorkObservation {
            extra_passes: 2,
            gpu_copies: 1,
            gpu_copy_bytes: 128 * 96 * 4,
            submissions: 1,
            target_recreates: 2,
            ..Default::default()
        },
        None,
    );
    snapshot.recreate_reason = OutputRecreateReason::Extent;
    snapshot.capture_diagnostics();
    assert!(guard.diagnostics().flush(false, Duration::from_secs(5)));
    let path = guard
        .diagnostics()
        .current_log()
        .expect("explicit capture must reach the real nlog sink");
    drop(guard);
    let log = nlog::read_file(path).unwrap();
    assert!(!log.truncated);
    let mut records = BTreeMap::new();
    let mut capture = None;
    for entry in &log.entries {
        let Entry::Event { key, values, .. } = entry else {
            continue;
        };
        let schema = &log.event_schemas[key];
        if !schema.name.starts_with("gpu.output.trace.") {
            continue;
        }
        assert_eq!(schema.severity, Severity::Debug);
        assert_eq!(values.len(), 4);
        let fields = schema
            .fields
            .iter()
            .zip(values)
            .map(|((name, _), value)| {
                let Value::U64(value) = value else {
                    panic!("trace metadata must remain typed u64")
                };
                (name.clone(), *value)
            })
            .collect::<BTreeMap<_, _>>();
        let trace = fields["trace"];
        assert_ne!(trace, 0);
        assert!(capture.is_none_or(|previous| previous == trace));
        capture = Some(trace);
        assert!(
            records.insert(schema.name.clone(), fields).is_none(),
            "observe/query emitted a per-frame trace event"
        );
    }
    assert_eq!(
        records.len(),
        9,
        "one explicit capture emits exactly nine correlated records"
    );
    let route = &records["gpu.output.trace.route"];
    assert_eq!(route["producer"], 37);
    assert_eq!(route["consumer"], 91);
    assert_eq!(route["path"], 4);
    let target = &records["gpu.output.trace.target"];
    assert_eq!(target["format"], 3);
    assert_eq!(target["width"], 128);
    assert_eq!(target["height"], 96);
    let version = &records["gpu.output.trace.version"];
    assert_eq!(version["device"], gpu.generation().get());
    assert_eq!(version["resource"], 3);
    assert_eq!(version["revision"], 19);
    let work = &records["gpu.output.trace.work"];
    assert_eq!(work["passes"], 2);
    assert_eq!(work["copies"], 1);
    assert_eq!(work["copy_bytes"], 128 * 96 * 4);
    let wait = &records["gpu.output.trace.wait"];
    assert_eq!(
        wait["observed"], 0,
        "unknown waits must not become observed zero"
    );
    let recreate = &records["gpu.output.trace.recreate"];
    assert_eq!(recreate["count"], 2);
    assert_eq!(recreate["reason"], OutputRecreateReason::Extent as u64);
    assert_eq!(recreate["role"], OutputTraceRole::Consumer as u64);
    let transport = &records["gpu.output.trace.transport"];
    assert_eq!(transport["submissions"], 1);
    assert_eq!(transport["readbacks"], 0);
    assert_eq!(records["gpu.output.trace.producer"]["reuses"], 128);
    assert_eq!(
        records["gpu.output.trace.state"]["state"],
        OutputTraceState::Submitted as u64
    );
}
