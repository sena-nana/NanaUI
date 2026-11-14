//! The DX12 pipeline cache and `Device::set_default_pipeline_cache`, through
//! the public `wgpu` API on a real DX12 adapter (WARP counts), and
//! `PipelineCache::pipelines_built`.
//!
//! The bytes `PipelineCache::get_data` returns are wgpu-core's 64-byte header
//! (the validation key, which names the shader compiler, at 32..48) followed
//! by wgpu-hal's DX12 format (`NDXSHC01`, then the entry count).

use nana_wgpu_fork_tests::{device, no_adapter};

const HEADER: usize = 64;
const VALIDATION_KEY: core::ops::Range<usize> = 32..48;

fn dx12(test: &str) -> Option<wgpu::Device> {
    let found = device(wgpu::Backends::DX12, wgpu::Features::PIPELINE_CACHE)
        .filter(|(_, device, _)| device.features().contains(wgpu::Features::PIPELINE_CACHE));
    if found.is_none() {
        no_adapter(test, wgpu::Backends::DX12);
    }
    found.map(|(_, device, _)| device)
}

/// # Safety
/// `data` is either `None` or bytes `PipelineCache::get_data` returned,
/// possibly damaged: wgpu-core validates the header and the DX12 backend
/// parses the rest without trusting it.
unsafe fn cache(device: &wgpu::Device, data: Option<&[u8]>, fallback: bool) -> wgpu::PipelineCache {
    unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("fork test cache"),
            data,
            fallback,
        })
    }
}

/// Compiled shader stages in `cache`.
fn entries(cache: &wgpu::PipelineCache) -> u32 {
    let data = cache.get_data().expect("a DX12 cache has data");
    assert_eq!(&data[HEADER..HEADER + 8], b"NDXSHC01");
    u32::from_le_bytes(data[HEADER + 8..HEADER + 12].try_into().unwrap())
}

/// A render pipeline whose two stages are new to every cache, unless `seed`
/// repeats.
fn pipeline(device: &wgpu::Device, seed: u32, cache: Option<&wgpu::PipelineCache>) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fork test shader"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "@vertex fn vs() -> @builtin(position) vec4<f32> {{ return vec4<f32>(f32({seed}u)); }}
                 @fragment fn fs() -> @location(0) vec4<f32> {{ return vec4<f32>(f32({seed}u)); }}"
            )
            .into(),
        ),
    });
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fork test pipeline"),
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
        cache,
    });
    assert!(pollster::block_on(scope.pop()).is_none());
}

/// Whether creating a cache from `data` with `fallback: false` raised an
/// error.
fn refused(device: &wgpu::Device, data: &[u8]) -> bool {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _cache = unsafe { cache(device, Some(data), false) };
    pollster::block_on(scope.pop()).is_some()
}

#[test]
fn the_default_cache_serves_pipelines_created_without_one() {
    let Some(device) = dx12("the_default_cache_serves_pipelines_created_without_one") else {
        return;
    };
    let default = unsafe { cache(&device, None, false) };
    let named = unsafe { cache(&device, None, false) };
    assert_eq!(entries(&default), 0);

    device.set_default_pipeline_cache(Some(&default));
    pipeline(&device, 1, None);
    let after_default = entries(&default);
    assert!(
        after_default >= 2,
        "both stages land in the default cache, got {after_default}"
    );

    // A pipeline that names a cache uses that one, not the default.
    pipeline(&device, 2, Some(&named));
    assert_eq!(entries(&default), after_default);
    assert!(entries(&named) >= 2);

    // Unset, pipelines without a cache use none.
    device.set_default_pipeline_cache(None);
    pipeline(&device, 3, None);
    assert_eq!(entries(&default), after_default);

    // The device holds the default weakly: once dropped, it is not used.
    device.set_default_pipeline_cache(Some(&named));
    drop(named);
    pipeline(&device, 4, None);
}

#[test]
fn saved_bytes_load_back_with_every_stage() {
    let Some(device) = dx12("saved_bytes_load_back_with_every_stage") else {
        return;
    };
    let first = unsafe { cache(&device, None, false) };
    pipeline(&device, 10, Some(&first));
    pipeline(&device, 11, Some(&first));
    let saved = first.get_data().unwrap();
    let stages = entries(&first);
    assert!(stages >= 4);

    assert!(!refused(&device, &saved));
    let loaded = unsafe { cache(&device, Some(&saved), false) };
    assert_eq!(entries(&loaded), stages);
    // Stages it already holds add nothing.
    pipeline(&device, 10, Some(&loaded));
    assert_eq!(entries(&loaded), stages);
}

#[test]
fn bytes_from_another_compiler_are_refused() {
    let Some(device) = dx12("bytes_from_another_compiler_are_refused") else {
        return;
    };
    let first = unsafe { cache(&device, None, false) };
    pipeline(&device, 20, Some(&first));
    let mut other = first.get_data().unwrap();
    other[VALIDATION_KEY.start] ^= 0x01;

    // Strict: a validation error. With fallback: an empty cache.
    assert!(refused(&device, &other));
    let fallback = unsafe { cache(&device, Some(&other), true) };
    assert_eq!(entries(&fallback), 0);
}

#[test]
fn damaged_bytes_are_refused_or_lose_only_the_damaged_stage() {
    let Some(device) = dx12("damaged_bytes_are_refused_or_lose_only_the_damaged_stage") else {
        return;
    };
    let first = unsafe { cache(&device, None, false) };
    pipeline(&device, 30, Some(&first));
    let saved = first.get_data().unwrap();
    let stages = entries(&first);

    // Truncated: wgpu-core's recorded length no longer matches.
    assert!(refused(&device, &saved[..saved.len() - 1]));
    let fallback = unsafe { cache(&device, Some(&saved[..saved.len() - 1]), true) };
    assert_eq!(entries(&fallback), 0);

    // One byte of the last stage's bytecode flipped: that stage is dropped,
    // the others load.
    let mut flipped = saved.clone();
    *flipped.last_mut().unwrap() ^= 0x5a;
    let loaded = unsafe { cache(&device, Some(&flipped), false) };
    assert_eq!(entries(&loaded), stages - 1);
}

#[test]
fn pipelines_built_counts_pipelines_through_the_cache() {
    let Some(device) = dx12("pipelines_built_counts_pipelines_through_the_cache") else {
        return;
    };
    let default = unsafe { cache(&device, None, false) };
    let named = unsafe { cache(&device, None, false) };
    assert_eq!(default.pipelines_built(), Some(0));

    device.set_default_pipeline_cache(Some(&default));
    pipeline(&device, 21, None);
    assert_eq!(default.pipelines_built(), Some(1));
    // A hit counts too: the count says a pipeline went through, not that
    // the cache grew.
    pipeline(&device, 21, None);
    assert_eq!(default.pipelines_built(), Some(2));
    // A pipeline naming another cache counts there only.
    pipeline(&device, 22, Some(&named));
    assert_eq!(default.pipelines_built(), Some(2));
    assert_eq!(named.pipelines_built(), Some(1));
    // Fetching the data does not move it.
    let _ = default.get_data();
    assert_eq!(default.pipelines_built(), Some(2));
}
