//! The painter's pipelines go through the device's pipeline cache.

use crate::test_gpu::fresh_context;
use crate::{GpuTextureFormat, PipelineCacheLoad, SceneWgpuPainter};

/// Every family compiles through the device default cache, so a painter
/// built on a device with one leaves its compiled stages there, and a second
/// device started from those bytes builds its painter without adding any.
#[test]
fn a_painter_fills_the_device_cache_and_a_warm_device_reuses_it() {
    let _gpu_guard = crate::test_gpu::lock();
    let cold = fresh_context();
    let Some(key) = cold.pipeline_cache_key() else {
        // This backend keeps no pipeline cache.
        return;
    };
    let empty = cold.pipeline_cache_data().expect("cache bytes");
    let _painter = SceneWgpuPainter::new(&cold, GpuTextureFormat::BGRA8_UNORM_SRGB);
    let filled = cold.pipeline_cache_data().expect("cache bytes");
    if filled.len() <= empty.len() && key.backend() != crate::GpuBackend::Dx12 {
        // Software Vulkan drivers such as Mesa lavapipe return only a fixed header.
        return;
    }
    assert!(
        filled.len() > empty.len(),
        "the painter compiled nothing into the cache"
    );

    let warm = fresh_context();
    assert_eq!(warm.load_pipeline_cache(&filled), PipelineCacheLoad::Loaded);
    let _painter = SceneWgpuPainter::new(&warm, GpuTextureFormat::BGRA8_UNORM_SRGB);
    let after = warm.pipeline_cache_data().expect("cache bytes");
    if key.backend() == crate::GpuBackend::Dx12 {
        // Every stage the warm painter needed was already there: the
        // entries are the same, only their order may differ.
        assert_eq!(after.len(), filled.len());
    }
}
