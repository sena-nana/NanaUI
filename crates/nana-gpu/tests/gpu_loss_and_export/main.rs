#[cfg(feature = "wgpu-interop")]
mod controlled_device_loss;
#[cfg(feature = "wgpu-interop")]
mod device_lost_diagnostics;
#[cfg(all(windows, feature = "native-export"))]
mod native_export;
#[cfg(all(
    feature = "wgpu-interop",
    any(target_os = "linux", target_os = "windows", target_os = "android")
))]
mod poll_device_lost;
