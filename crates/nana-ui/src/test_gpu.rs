//! GPU device for unit tests.
//!
//! The whole test process shares one `wgpu::Device`, created on first use.
//! Creating devices while tests run deadlocks on Windows with the NVIDIA Vulkan
//! ICD: device creation loads libraries under the driver's lock while
//! libtest's per-test threads exit under the loader lock, and `cargo test`
//! hangs with every worker stuck in a GPU test. Tests only rely on their own
//! painters, targets and resources, never on owning the device.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

static GPU_TEST_STATE: Mutex<(u64, usize)> = Mutex::new((0, 0));
static GPU_TEST_FREE: Condvar = Condvar::new();

thread_local! {
    static THREAD_TOKEN: u64 = {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    };
}

/// Thread-reentrant process guard serializing GPU test execution across libtest worker threads.
pub(crate) struct GpuTestGuard;

impl Drop for GpuTestGuard {
    fn drop(&mut self) {
        let mut state = GPU_TEST_STATE
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.1 = state.1.saturating_sub(1);
        if state.1 == 0 {
            state.0 = 0;
            GPU_TEST_FREE.notify_one();
        }
    }
}

pub(crate) fn lock() -> GpuTestGuard {
    let token = THREAD_TOKEN.with(|token| *token);
    let mut state = GPU_TEST_STATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    while state.1 > 0 && state.0 != token {
        state = GPU_TEST_FREE
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner);
    }
    state.0 = token;
    state.1 += 1;
    GpuTestGuard
}

/// Runs an isolated child test binary with a wall-clock timeout so a hung
/// child never stalls the parent `cargo test` runner.
pub(crate) fn run_child_with_timeout(test_filter: &str, env_key: &str, env_val: &str) {
    let exe = std::env::current_exe().expect("locate test executable");
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--exact", test_filter, "--test-threads=1", "--nocapture"])
        .env(env_key, env_val)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("spawn isolated test child");
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

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "isolated child test `{test_filter}` timed out after 15s"
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "failed waiting on isolated child test `{test_filter}`: {error}"
                ));
            }
        }
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    match status {
        Ok(status) => assert!(
            status.success(),
            "isolated child test `{test_filter}` failed ({status}):\n{}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr),
        ),
        Err(message) => panic!(
            "{message}:\n{}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr),
        ),
    }
}

use nana_gpu::{
    __framework, GpuContext, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages,
};

/// The process-wide test device and queue.
pub(crate) fn device() -> (wgpu::Device, wgpu::Queue) {
    let gpu = context();
    (
        __framework::device(&gpu).clone(),
        __framework::queue(&gpu).clone(),
    )
}

/// A sampled RGBA8 texture on the test device.
pub(crate) fn texture(width: u32, height: u32) -> GpuTexture {
    context()
        .create_texture(&GpuTextureDescriptor {
            label: Some("nana-ui test texture"),
            width,
            height,
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST,
        })
        .expect("test texture")
}

/// The texture behind a test view, on the test device.
pub(crate) fn wrap_view(view: &wgpu::TextureView) -> GpuTexture {
    __framework::texture_from_wgpu(&context(), view.texture().clone())
}

/// The process-wide test device adopted again, as a context of its own: its
/// policy counters and frame slots belong to the calling test alone. Tests
/// running beside it share [`context`], so a test that counts what its own
/// frames did, or must not wait on another test's frame, uses this one.
pub(crate) fn private_context() -> GpuContext {
    let gpu = context();
    __framework::adopt(
        __framework::adapter(&gpu).clone(),
        __framework::device(&gpu).clone(),
        __framework::queue(&gpu).clone(),
    )
}

/// The process-wide test device as the GPU contract sees it.
pub(crate) fn context() -> GpuContext {
    static CONTEXT: OnceLock<GpuContext> = OnceLock::new();
    CONTEXT.get_or_init(fresh_context).clone()
}

/// A device of its own, created as [`context`]'s was.
pub(crate) fn fresh_context() -> GpuContext {
    let _guard = lock();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .expect("GPU tests require a WGPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nana-ui test device"),
        // What the hosted context asks for too, when the adapter has it.
        required_features: adapter.features()
            & (wgpu::Features::DUAL_SOURCE_BLENDING | wgpu::Features::PIPELINE_CACHE),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .expect("GPU tests require a WGPU device");
    __framework::adopt(adapter, device, queue)
}
