#![recursion_limit = "256"]

use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use nana_gpu::GpuContext;

static GPU_TEST_STATE: Mutex<(u64, usize)> = Mutex::new((0, 0));
static GPU_TEST_FREE: Condvar = Condvar::new();

thread_local! {
    static THREAD_TOKEN: u64 = {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    };
}

/// Thread-reentrant guard serializing GPU test execution across `gpu_contracts`.
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

pub(crate) fn gpu_lock() -> GpuTestGuard {
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

/// Holds a test `GpuContext` together with the binary-wide GPU guard.
/// Field order matters: `gpu` drops before `_guard` releases the lock.
pub(crate) struct TestGpu {
    gpu: GpuContext,
    _guard: GpuTestGuard,
}

impl TestGpu {
    pub(crate) fn new(gpu: GpuContext, guard: GpuTestGuard) -> Self {
        Self { gpu, _guard: guard }
    }
}

impl Deref for TestGpu {
    type Target = GpuContext;

    fn deref(&self) -> &Self::Target {
        &self.gpu
    }
}

/// Process-wide WGPU adapter; keeps `wgpu::Instance` alive until process exit.
pub(crate) fn shared_adapter() -> &'static wgpu::Adapter {
    static ADAPTER: OnceLock<(wgpu::Instance, wgpu::Adapter)> = OnceLock::new();
    let _guard = gpu_lock();
    &ADAPTER
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::from_env().unwrap_or_default(),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
                &instance, None,
            ))
            .expect("GPU contract tests require a WGPU adapter");
            (instance, adapter)
        })
        .1
}

mod contract;
mod pipeline_cache;
