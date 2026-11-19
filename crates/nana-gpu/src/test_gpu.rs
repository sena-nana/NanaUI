//! Shared GPU test device and reentrant serialization guard for `nana-gpu` unit tests.

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

/// Process-wide test device and queue, keeping the `wgpu::Instance` alive for
/// the test process lifetime so software Vulkan drivers never tear down mid-suite.
pub(crate) fn device_and_queue() -> (GpuTestGuard, wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue)> =
        OnceLock::new();
    let guard = lock();
    let (_instance, _adapter, device, queue) = DEVICE.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("nana-gpu unit tests require a WGPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("nana-gpu unit tests require a WGPU device");
        (instance, adapter, device, queue)
    });
    (guard, device.clone(), queue.clone())
}
