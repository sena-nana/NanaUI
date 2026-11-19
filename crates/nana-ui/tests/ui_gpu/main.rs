#![cfg(all(feature = "gpu", feature = "wgpu-interop"))]

pub(crate) fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

mod output_trace_owner;
mod precompiled_without_runtime_wgsl;
mod text_device_recreation;
mod text_lost_device;
