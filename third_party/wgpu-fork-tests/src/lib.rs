//! Tests for NanaUI's changes to its wgpu fork (`third_party/wgpu`).
//!
//! The library only holds what the GPU tests share. A test that finds no
//! adapter for its backend skips; with `NANA_REQUIRE_WGPU_FORK_TESTS=1` it
//! fails instead, so CI on a machine that has the backend cannot pass by
//! skipping.

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

/// Thread-reentrant process guard serializing GPU test execution across libtest threads.
pub struct GpuTestGuard;

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

pub fn gpu_lock() -> GpuTestGuard {
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

/// Whether a missing adapter must fail the test instead of skipping it.
pub fn adapter_required() -> bool {
    std::env::var_os("NANA_REQUIRE_WGPU_FORK_TESTS").is_some_and(|value| value == "1")
}

/// Skip (or, when required, fail) a test that has no adapter to run on.
pub fn no_adapter(test: &str, backend: wgpu::Backends) {
    assert!(
        !adapter_required(),
        "{test}: no {backend:?} adapter, and NANA_REQUIRE_WGPU_FORK_TESTS=1"
    );
    println!("{test}: skipped, no {backend:?} adapter");
}

/// An adapter and device on `backend` with `features` (those the adapter
/// offers), or `None` when the backend has no adapter here.
pub fn device(
    backend: wgpu::Backends,
    features: wgpu::Features,
) -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    type CachedInstances = Mutex<Vec<(u32, wgpu::Instance, Option<wgpu::Adapter>)>>;
    static INSTANCES: OnceLock<CachedInstances> = OnceLock::new();
    let _guard = gpu_lock();
    let cache = INSTANCES.get_or_init(|| Mutex::new(Vec::new()));
    let adapter = {
        let mut entries = cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, _, adapter)) = entries.iter().find(|(bits, _, _)| *bits == backend.bits()) {
            adapter.clone()
        } else {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: backend,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok();
            entries.push((backend.bits(), instance, adapter.clone()));
            adapter
        }
    }?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & features,
        ..Default::default()
    }))
    .ok()?;
    Some((adapter, device, queue))
}
