//! A device loss is on disk (fault and snapshot) by the time anyone can see
//! it, recorded on the thread that noticed it rather than by the diagnostics
//! worker. Own test binary: it installs the process-wide diagnostics.
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use nana_diagnostics::nlog::{self, Entry, Value};
use nana_diagnostics::{DiagnosticsConfig, DiagnosticsPaths, PersistMode, SessionMetadata};
use nana_gpu::GpuContext;

fn snapshots(dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|read| {
            read.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with("-device-lost.nlog"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_lost_device_is_recorded_synchronously_once() {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nana-gpu-device-lost-{nanos}"));
    let crash = dir.join("crash");
    let _guard = nana_diagnostics::install(
        DiagnosticsConfig {
            persist: PersistMode::Off,
            // Clamped to 60 s: the worker never polls during this test.
            poll_interval: Duration::from_secs(3600),
            native_crash_handler: false,
            ..DiagnosticsConfig::default()
        },
        SessionMetadata::new("dev.nana.gpu-loss", "GPU loss", "1"),
        DiagnosticsPaths::new(dir.join("logs"), &crash),
    )
    .unwrap();

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("device loss test requires a real WGPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("test device");
    let gpu = GpuContext::from_wgpu_with_loss_tracking(adapter, device, queue);
    let observer = gpu.clone();
    gpu.wgpu().device().destroy();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !observer.is_lost() {
        assert!(Instant::now() < deadline, "loss callback did not arrive");
        gpu.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
    // Already written when the loss became visible.
    let found = snapshots(&crash);
    assert_eq!(found.len(), 1, "{found:?}");
    let file = nlog::read_file(&found[0]).unwrap();
    assert_eq!(file.header.reason, "snapshot:device-lost");
    let lost: Vec<_> = file
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Fault {
                key,
                values,
                message,
                ..
            } if file.event_name(*key) == Some("gpu.device_lost") => {
                Some((values.clone(), message.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0].0, vec![Value::U64(1)]);
    assert!(lost[0].1.as_deref().unwrap().starts_with("Destroyed"));

    // Later reports of the same loss record nothing more.
    nana_gpu::__framework::mark_lost(
        &gpu,
        nana_gpu::GpuDeviceLost {
            reason: nana_gpu::GpuLossReason::Unknown,
            message: "again".into(),
        },
    );
    assert_eq!(snapshots(&crash).len(), 1);
    drop(_guard);
    let _ = std::fs::remove_dir_all(dir);
}
