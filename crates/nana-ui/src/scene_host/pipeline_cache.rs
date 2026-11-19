//! The scene host's side of the persisted pipeline cache.
//!
//! The startup thread restores the cache before the first painter compiles
//! and saves it after (`startup::spawn_device_request`). From then on this
//! keeps the store in step with the device the host renders on: a
//! [`PipelineCacheSaver`] picks up pipelines compiled later, a replacement
//! device starts from the store before anything rebuilds on it, and shutdown
//! saves what grew since the last save.

use std::sync::Arc;
use std::time::Duration;

use nana_gpu::{GpuContext, PipelineCacheLoad, PipelineCacheSaver};

use crate::PipelineCacheStore;

pub(super) struct HostPipelineCache {
    store: Arc<dyn PipelineCacheStore>,
    saver: Option<PipelineCacheSaver>,
    interval: Duration,
}

impl HostPipelineCache {
    pub(super) fn new(store: Arc<dyn PipelineCacheStore>) -> Self {
        Self::with_interval(store, PipelineCacheSaver::DEFAULT_INTERVAL)
    }

    pub(super) fn with_interval(store: Arc<dyn PipelineCacheStore>, interval: Duration) -> Self {
        Self {
            store,
            saver: None,
            interval,
        }
    }

    /// Saves `gpu`'s cache from now on. Called once the startup thread is
    /// done with the store (its painter arrived or it stopped), so a save
    /// here cannot land before its restore. Does nothing when already saving
    /// that device.
    pub(super) fn watch(&mut self, gpu: &GpuContext) {
        if self
            .saver
            .as_ref()
            .is_some_and(|saver| saver.gpu().same_device(gpu))
        {
            return;
        }
        self.stop_saver();
        self.saver = PipelineCacheSaver::spawn(gpu, Arc::clone(&self.store), self.interval);
    }

    /// The host moved to `gpu` (after a loss, usually). What the old device
    /// compiled is saved unless it was lost, then `gpu` starts from the
    /// store, before the painters and program renderers rebuild on it, so
    /// their pipelines skip the compiler as they did at startup. Runs on the
    /// event thread during recovery, which already blocks on the new device.
    pub(super) fn replace_device(&mut self, gpu: &GpuContext) -> PipelineCacheLoad {
        self.stop_saver();
        let restored = gpu.restore_pipeline_cache(self.store.as_ref());
        self.watch(gpu);
        restored
    }

    /// Clean shutdown: saves once more if the cache grew.
    pub(super) fn finish(&mut self) {
        self.stop_saver();
    }

    fn stop_saver(&mut self) {
        if let Some(saver) = self.saver.take() {
            // `finish` skips a lost device itself.
            saver.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nana_gpu::{__framework, GpuDeviceLost, GpuLossReason, PipelineCacheKey};

    use super::*;
    use crate::test_gpu::fresh_context;
    use crate::{GpuTextureFormat, SceneWgpuPainter};

    #[derive(Default)]
    struct CountingStore {
        bytes: Mutex<Option<Vec<u8>>>,
        saves: Mutex<usize>,
    }

    impl PipelineCacheStore for CountingStore {
        fn load(&self, _key: &PipelineCacheKey) -> Option<Vec<u8>> {
            self.bytes.lock().unwrap().clone()
        }
        fn save(&self, _key: &PipelineCacheKey, bytes: &[u8]) {
            *self.bytes.lock().unwrap() = Some(bytes.to_vec());
            *self.saves.lock().unwrap() += 1;
        }
    }

    /// A pipeline no painter builds, through the device default cache.
    fn build_pipeline(gpu: &GpuContext, color: f32) {
        let device = __framework::device(gpu);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("host pipeline cache test shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "@vertex fn vs() -> @builtin(position) vec4<f32> {{ return vec4<f32>(0.0); }}
                     @fragment fn fs() -> @location(0) vec4<f32> {{ return vec4<f32>({color:?}); }}"
                )
                .into(),
            ),
        });
        let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("host pipeline cache test pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
    }

    impl CountingStore {
        fn saves(&self) -> usize {
            *self.saves.lock().unwrap()
        }
    }

    /// A replacement device after a loss starts from what the lost one
    /// saved, so its painter compiles nothing new; the lost device is not
    /// saved over the store.
    #[test]
    fn a_device_replaced_after_a_loss_starts_from_the_store() {
        let _gpu_guard = crate::test_gpu::lock();
        let lost = fresh_context();
        let Some(key) = lost.pipeline_cache_key() else {
            // This backend keeps no pipeline cache.
            return;
        };
        let store = Arc::new(CountingStore::default());
        // What the startup thread does.
        let _painter = SceneWgpuPainter::new(&lost, GpuTextureFormat::BGRA8_UNORM_SRGB);
        assert!(lost.persist_pipeline_cache(store.as_ref()));
        let saved = store.bytes.lock().unwrap().clone().expect("saved bytes");
        let mut host = HostPipelineCache::new(store.clone());
        host.watch(&lost);

        // Something new compiles, then the device is lost before a save.
        build_pipeline(&lost, 0.5);
        __framework::mark_lost(
            &lost,
            GpuDeviceLost {
                reason: GpuLossReason::Unknown,
                message: "simulated loss".into(),
            },
        );

        let replacement = fresh_context();
        assert_eq!(host.replace_device(&replacement), PipelineCacheLoad::Loaded);
        assert_eq!(store.saves(), 1, "the lost device was saved");
        let _painter = SceneWgpuPainter::new(&replacement, GpuTextureFormat::BGRA8_UNORM_SRGB);
        if key.backend() == crate::GpuBackend::Dx12 {
            // Every stage came from the store: nothing grew.
            let after = replacement.pipeline_cache_data().expect("cache bytes");
            assert_eq!(after.len(), saved.len());
            assert!(!replacement.persist_pipeline_cache_if_grown(store.as_ref()));
        }
        host.finish();
        if key.backend() == crate::GpuBackend::Dx12 {
            assert_eq!(store.saves(), 1, "shutdown saved an unchanged cache");
        }
    }

    /// Pipelines compiled after startup reach the store when the host shuts
    /// down, and a shutdown with nothing new saves nothing.
    #[test]
    fn shutdown_saves_late_pipelines_and_only_them() {
        let _gpu_guard = crate::test_gpu::lock();
        let gpu = fresh_context();
        let Some(key) = gpu.pipeline_cache_key() else {
            return;
        };
        let store = Arc::new(CountingStore::default());
        let _painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::BGRA8_UNORM_SRGB);
        assert!(gpu.persist_pipeline_cache(store.as_ref()));
        let before_late = gpu.pipeline_cache_data().map_or(0, |bytes| bytes.len());
        // Long enough that only `finish` saves.
        let mut host = HostPipelineCache::with_interval(store.clone(), Duration::from_secs(3600));
        host.watch(&gpu);
        host.finish();
        assert_eq!(store.saves(), 1, "nothing changed, yet shutdown saved");

        host.watch(&gpu);
        build_pipeline(&gpu, 0.5);
        let after_late = gpu.pipeline_cache_data().map_or(0, |bytes| bytes.len());
        host.finish();
        if after_late <= before_late && key.backend() != crate::GpuBackend::Dx12 {
            assert_eq!(store.saves(), 1);
            return;
        }
        assert_eq!(store.saves(), 2, "the late pipelines were not saved");
    }
}
