//! Device-backed checks of the GPU contract.

#![recursion_limit = "256"]

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use nana_gpu::__framework;
use nana_gpu::{
    GpuBufferDescriptor, GpuBufferUsages, GpuContext, GpuError, GpuTextureDescriptor,
    GpuTextureFormat, GpuTextureRegion, GpuTextureUsages, LogicalBinding, LogicalBindingType,
    LogicalResource, ResourceBinding, ResourceSet, ResourceTable, RetainedWrites, ShaderStage,
};

fn context() -> GpuContext {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .expect("GPU contract tests require a WGPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("GPU contract tests require a WGPU device");
    __framework::adopt(adapter, device, queue)
}

fn texture(gpu: &GpuContext, usage: GpuTextureUsages) -> nana_gpu::GpuTexture {
    gpu.create_texture(&GpuTextureDescriptor {
        label: Some("contract test"),
        width: 4,
        height: 2,
        format: GpuTextureFormat::RGBA8_UNORM,
        usage,
    })
    .expect("4x2 rgba8 texture")
}

#[test]
fn contexts_share_a_generation_only_with_their_clones() {
    let a = context();
    let b = context();
    assert!(a.same_device(&a.clone()));
    assert_eq!(a.generation(), a.clone().generation());
    assert!(!a.same_device(&b));
    let first = a.begin_frame("first");
    let second = a.begin_frame("second");
    assert!(second.id() > first.id());
    assert_eq!(first.generation(), a.generation());
}

#[test]
fn frame_slot_exhaustion_is_observable_and_discard_releases_exact_slot() {
    let gpu = context();
    let first = gpu.try_begin_frame("slot-1").expect("first slot");
    let second = gpu.try_begin_frame("slot-2").expect("second slot");
    let third = gpu.try_begin_frame("slot-3").expect("third slot");
    assert!(gpu.try_begin_frame("slot-4").is_none());
    drop(second);
    assert!(gpu.try_begin_frame("slot-after-discard").is_some());
    drop(first);
    drop(third);
}

#[test]
fn texture_creation_refuses_extents_the_device_cannot_hold() {
    let gpu = context();
    let max = gpu.capabilities().max_texture_dimension_2d();
    for (width, height) in [(0, 1), (1, 0), (max + 1, 1)] {
        let error = gpu
            .create_texture(&GpuTextureDescriptor {
                label: None,
                width,
                height,
                format: GpuTextureFormat::RGBA8_UNORM,
                usage: GpuTextureUsages::SAMPLED,
            })
            .expect_err("invalid extent");
        assert_eq!(error, GpuError::InvalidExtent { width, height, max });
    }
    let compressed = __framework::format_from_wgpu(wgpu::TextureFormat::Bc1RgbaUnorm);
    assert_eq!(
        gpu.create_texture(&GpuTextureDescriptor {
            label: None,
            width: 6,
            height: 4,
            format: compressed,
            usage: GpuTextureUsages::SAMPLED,
        })
        .expect_err("not whole 4x4 blocks"),
        GpuError::InvalidExtent {
            width: 6,
            height: 4,
            max
        }
    );
    assert_eq!(
        gpu.create_texture(&GpuTextureDescriptor {
            label: None,
            width: 1,
            height: 1,
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::empty(),
        })
        .expect_err("no usage"),
        GpuError::EmptyUsage
    );
    let target = texture(
        &gpu,
        GpuTextureUsages::SAMPLED | GpuTextureUsages::RENDER_TARGET,
    );
    let view = target.render_target().expect("render target usage");
    assert_eq!(view.size(), [4, 2]);
    assert_eq!(view.generation(), gpu.generation());
    assert_eq!(
        texture(&gpu, GpuTextureUsages::SAMPLED)
            .render_target()
            .expect_err("no render target usage"),
        GpuError::MissingUsage(GpuTextureUsages::RENDER_TARGET)
    );
}

#[test]
fn uploads_are_validated_before_they_reach_the_queue() {
    let gpu = context();
    let other = context();
    let writable = texture(&gpu, GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST);
    let row = [0u8; 16];
    let pixels = [0u8; 32];
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels, 16),
        Ok(())
    );
    assert_eq!(
        gpu.write_texture(
            &writable,
            GpuTextureRegion {
                x: 1,
                y: 1,
                width: 2,
                height: 1
            },
            &row[..8],
            8
        ),
        Ok(()),
        "a dirty rect uploads only its own bytes"
    );
    assert_eq!(
        other.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels, 16),
        Err(GpuError::DeviceMismatch {
            expected: other.generation(),
            found: gpu.generation(),
        })
    );
    assert_eq!(
        gpu.write_texture(
            &texture(&gpu, GpuTextureUsages::SAMPLED),
            GpuTextureRegion::full(4, 2),
            &pixels,
            16
        ),
        Err(GpuError::MissingUsage(GpuTextureUsages::COPY_DST))
    );
    for region in [
        GpuTextureRegion::full(5, 2),
        GpuTextureRegion {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
        GpuTextureRegion {
            x: 5,
            y: 0,
            width: 0,
            height: 1,
        },
    ] {
        assert_eq!(
            gpu.write_texture(&writable, region, &pixels, 32),
            Err(GpuError::RegionOutOfBounds)
        );
    }
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(0, 2), &[], 0),
        Ok(()),
        "an empty region inside the texture is a no-op, as in WGPU"
    );
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels, 12),
        Err(GpuError::RowTooShort {
            needed: 16,
            provided: 12
        })
    );
    assert_eq!(
        gpu.write_texture(&writable, GpuTextureRegion::full(4, 2), &pixels[..31], 16),
        Err(GpuError::DataTooShort {
            needed: 32,
            provided: 31
        })
    );
}

#[test]
fn a_dropped_frame_rolls_back_what_it_recorded_and_a_submitted_one_settles() {
    let gpu = context();
    let ledger = RetainedWrites::new();

    let mut submitted = gpu.begin_frame("submitted");
    submitted.record_retained_writes(&ledger, 1);
    submitted.record_retained_writes(&ledger, 1);
    let later = gpu.begin_frame("later");
    assert!(ledger.in_flight(1, later.id()));
    let submission = submitted.submit();
    assert_eq!(submission.generation(), gpu.generation());
    assert!(!ledger.in_flight(1, later.id()));
    assert!(!ledger.has_rolled_back());
    drop(later);
    assert!(!ledger.has_rolled_back(), "nothing was recorded into it");

    let mut discarded = gpu.begin_frame("discarded");
    discarded.record_retained_writes(&ledger, 1);
    discarded.record_retained_writes(&ledger, 2);
    discarded.discard();
    let mut keys = Vec::new();
    ledger.drain_rolled_back(|key| keys.push(key));
    keys.sort_unstable();
    assert_eq!(keys, [1, 2]);
    assert!(!ledger.has_rolled_back());
}

#[test]
fn submission_waits_while_a_surface_reconfigures() {
    let gpu = context();
    let (started, waiting) = mpsc::channel();
    let (submitted, submissions) = mpsc::channel();
    let writer = gpu.clone();
    let gate = __framework::lock_reconfigure(&writer);
    let producer = {
        let gpu = gpu.clone();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            gpu.begin_frame("producer").submit();
            submitted.send(()).unwrap();
        })
    };
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        submissions
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "a submit must wait while the surface is reconfiguring"
    );
    drop(gate);
    submissions.recv_timeout(Duration::from_secs(5)).unwrap();
    producer.join().unwrap();
}

#[test]
fn a_loss_marked_by_the_host_is_sticky_and_reported() {
    let gpu = context();
    assert!(!gpu.is_lost());
    __framework::mark_lost(
        &gpu,
        nana_gpu::GpuDeviceLost {
            reason: nana_gpu::GpuLossReason::Destroyed,
            message: "host".into(),
        },
    );
    assert!(gpu.clone().is_lost());
    assert_eq!(gpu.lost_report().unwrap().message, "host");
}

#[test]
fn logical_resource_layout_is_realized_and_generation_checked() {
    let gpu = context();
    let stages: &'static [ShaderStage] = &[ShaderStage::Vertex, ShaderStage::Fragment];
    let table = ResourceTable::new(vec![
        LogicalBinding::new(0, LogicalBindingType::UniformBuffer, stages),
        LogicalBinding::new(1, LogicalBindingType::Sampler, stages),
    ])
    .unwrap()
    .for_generation(gpu.generation());
    let layout = gpu
        .create_resource_layout(&table)
        .expect("logical table maps to a WGPU bind-group layout");
    assert_eq!(layout.generation(), gpu.generation());
    assert_eq!(layout.key(), table.layout_key());
    let other = context();
    assert!(matches!(
        other.create_resource_layout(&table),
        Err(GpuError::DeviceMismatch { .. })
    ));
}

#[test]
fn logical_resource_group_binds_real_buffer_texture_and_sampler() {
    let gpu = context();
    let stages: &'static [ShaderStage] = &[ShaderStage::Vertex, ShaderStage::Fragment];
    let table = ResourceTable::new(vec![
        LogicalBinding::new(0, LogicalBindingType::UniformBuffer, stages),
        LogicalBinding::new(1, LogicalBindingType::SampledTexture, stages),
        LogicalBinding::new(2, LogicalBindingType::Sampler, stages),
    ])
    .unwrap()
    .for_generation(gpu.generation());
    let layout = gpu.create_resource_layout(&table).unwrap();
    let buffer = gpu
        .create_buffer(&GpuBufferDescriptor {
            label: Some("abi"),
            size: 256,
            usage: GpuBufferUsages::UNIFORM | GpuBufferUsages::COPY_DST,
        })
        .unwrap();
    let texture = texture(&gpu, GpuTextureUsages::SAMPLED);
    let sampler = gpu.create_sampler();
    let set = ResourceSet::new(vec![
        ResourceBinding {
            binding: 0,
            resource: Some(LogicalResource::Buffer {
                buffer,
                offset: 0,
                size: 256,
            }),
        },
        ResourceBinding {
            binding: 1,
            resource: Some(LogicalResource::Texture(texture)),
        },
        ResourceBinding {
            binding: 2,
            resource: Some(LogicalResource::Sampler(sampler)),
        },
    ])
    .unwrap();
    let group = gpu.create_resource_group(&layout, &table, &set).unwrap();
    assert_eq!(group.generation(), gpu.generation());
    assert_eq!(group.layout_key(), table.layout_key());
}

#[test]
fn logical_buffer_upload_checks_usage_and_bounds() {
    let gpu = context();
    let buffer = gpu
        .create_buffer(&GpuBufferDescriptor {
            label: Some("upload"),
            size: 16,
            usage: GpuBufferUsages::UNIFORM | GpuBufferUsages::COPY_DST,
        })
        .unwrap();
    gpu.write_buffer(&buffer, 4, &[1, 2, 3, 4]).unwrap();
    assert_eq!(
        gpu.write_buffer(&buffer, 2, &[1, 2]),
        Err(GpuError::InvalidBindingRange)
    );
    assert_eq!(
        gpu.write_buffer(&buffer, 12, &[1, 2, 3, 4, 5]),
        Err(GpuError::InvalidBindingRange),
        "past the end"
    );
    // A length that is not whole words would zero the bytes after it.
    assert_eq!(
        gpu.write_buffer(&buffer, 0, &[1, 2, 3, 4, 5]),
        Err(GpuError::InvalidBindingRange)
    );
    assert_eq!(
        gpu.write_buffer(&buffer, 15, &[1, 2]),
        Err(GpuError::InvalidBindingRange)
    );
    let no_copy = gpu
        .create_buffer(&GpuBufferDescriptor {
            label: None,
            size: 16,
            usage: GpuBufferUsages::UNIFORM,
        })
        .unwrap();
    assert_eq!(
        gpu.write_buffer(&no_copy, 0, &[1]),
        Err(GpuError::InvalidBindingRange)
    );
}

#[test]
fn logical_optional_and_capability_fallbacks_are_structured() {
    let gpu = context();
    let stages: &'static [ShaderStage] = &[ShaderStage::Fragment];
    let optional = ResourceTable::new(vec![
        LogicalBinding::new(0, LogicalBindingType::Sampler, stages).optional(true),
    ])
    .unwrap()
    .for_generation(gpu.generation());
    let layout = gpu.create_resource_layout(&optional).unwrap();
    let empty = ResourceSet::default();
    assert!(matches!(
        gpu.create_resource_group(&layout, &optional, &empty),
        Err(GpuError::UnsupportedCapability(
            "optional_resource_fallback"
        ))
    ));

    let array = ResourceTable::new(vec![
        LogicalBinding::new(0, LogicalBindingType::ResourceArray, stages)
            .array(std::num::NonZeroU32::new(2).unwrap()),
    ])
    .unwrap()
    .for_generation(gpu.generation());
    if !gpu
        .capabilities()
        .supports(nana_gpu::GpuCapability::ResourceArrays)
    {
        assert!(matches!(
            gpu.create_resource_layout(&array),
            Err(GpuError::UnsupportedCapability("resource_arrays"))
        ));
    }
}

#[test]
fn dynamic_buffer_slice_realization_returns_backend_offsets() {
    let gpu = context();
    let stages: &'static [ShaderStage] = &[ShaderStage::Vertex];
    let table = ResourceTable::new(vec![
        LogicalBinding::new(0, LogicalBindingType::DynamicBufferSlice, stages).min_size(16),
    ])
    .unwrap()
    .for_generation(gpu.generation());
    let layout = gpu.create_resource_layout(&table).unwrap();
    let buffer = gpu
        .create_buffer(&GpuBufferDescriptor {
            label: Some("dynamic"),
            size: 512,
            usage: GpuBufferUsages::UNIFORM,
        })
        .unwrap();
    let alignment = gpu.limits().min_uniform_buffer_offset_alignment as u64;
    let set = ResourceSet::new(vec![ResourceBinding {
        binding: 0,
        resource: Some(LogicalResource::Buffer {
            buffer,
            offset: alignment,
            size: 16,
        }),
    }])
    .unwrap();
    let group = gpu.create_resource_group(&layout, &table, &set).unwrap();
    assert_eq!(group.dynamic_offsets(), &[alignment as u32]);
}

#[test]
fn real_transient_pool_reuses_only_matching_resources_and_obeys_budget() {
    let gpu = context();
    let usage = GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST;
    let key = nana_gpu::TransientResourceKey::new(
        gpu.generation(),
        GpuTextureFormat::RGBA8_UNORM,
        usage.bits(),
        4,
        2,
        1,
    );
    let original = texture(&gpu, usage);
    gpu.policy()
        .release_transient_texture(key, original.clone());
    let reused = gpu
        .policy()
        .acquire_transient_texture(key, || panic!("must reuse"))
        .unwrap();
    assert!(original.ptr_eq(&reused));
    let wrong = nana_gpu::TransientResourceKey { width: 8, ..key };
    assert_eq!(
        gpu.policy()
            .acquire_transient_texture(wrong, || Ok(original.clone()))
            .unwrap_err(),
        GpuError::TransientDescriptorMismatch
    );
    gpu.policy()
        .release_transient_texture(wrong, reused.clone());
    assert!(
        gpu.policy()
            .acquire_transient_texture(wrong, || Err(GpuError::EmptyUsage))
            .is_err()
    );
    gpu.policy().release_transient_texture(key, reused);
    gpu.policy().set_transient_budget(0);
    let after_eviction = gpu
        .policy()
        .acquire_transient_texture(key, || Ok(texture(&gpu, usage)))
        .unwrap();
    assert!(!after_eviction.ptr_eq(&original));
    let foreign = context();
    let foreign_texture = texture(&foreign, usage);
    assert!(matches!(
        gpu.policy()
            .acquire_transient_texture(key, || Ok(foreign_texture)),
        Err(GpuError::DeviceMismatch { .. })
    ));
}

#[test]
fn concurrent_transient_cold_requests_create_distinct_real_textures() {
    let gpu = context();
    let policy = gpu.policy().clone();
    let key = nana_gpu::TransientResourceKey::new(
        gpu.generation(),
        GpuTextureFormat::RGBA8_UNORM,
        (GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST).bits(),
        4,
        2,
        1,
    );
    let creates = Arc::new(AtomicU64::new(0));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let policy = policy.clone();
        let gpu = gpu.clone();
        let creates = Arc::clone(&creates);
        workers.push(thread::spawn(move || {
            policy
                .acquire_transient_texture(key, || {
                    creates.fetch_add(1, Ordering::Relaxed);
                    gpu.create_texture(&GpuTextureDescriptor {
                        label: Some("concurrent transient"),
                        width: 4,
                        height: 2,
                        format: GpuTextureFormat::RGBA8_UNORM,
                        usage: GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST,
                    })
                })
                .unwrap()
        }));
    }
    let first = workers.remove(0).join().unwrap();
    let second = workers.remove(0).join().unwrap();
    // Concurrent users must not alias one texture while either operation is
    // in flight; cold misses therefore allocate separate leased resources.
    assert_eq!(creates.load(Ordering::Relaxed), 2);
    assert!(!first.ptr_eq(&second));
    assert_eq!(policy.stats().transient_pool_hits, 0);
}

fn raw_buffer(gpu: &GpuContext, size: u64) -> wgpu::Buffer {
    __framework::device(gpu).create_buffer(&wgpu::BufferDescriptor {
        label: Some("contract upload target"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// Copy `source` back to the CPU with its own submission.
fn read_buffer(gpu: &GpuContext, source: &wgpu::Buffer) -> Vec<u8> {
    let device = __framework::device(gpu);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("contract readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut frame = gpu.begin_frame("contract readback");
    __framework::encoder(&mut frame).copy_buffer_to_buffer(source, 0, &readback, 0, source.size());
    frame.submit();
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = readback.slice(..).get_mapped_range().unwrap().to_vec();
    readback.unmap();
    bytes
}

#[test]
fn frame_uploads_land_with_their_frame_as_one_flush_of_merged_copies() {
    let gpu = context();
    let first = raw_buffer(&gpu, 4000);
    let second = raw_buffer(&gpu, 4000);
    let frame = gpu.begin_frame("uploads");
    let uploads = __framework::frame_uploads(&frame);
    for index in 0..500u32 {
        uploads.write_buffer(&first, u64::from(index) * 4, &index.to_le_bytes());
        uploads.write_buffer(&second, u64::from(index) * 4, &(index * 2).to_le_bytes());
    }
    // Interleaved targets cannot merge; each run of one target can.
    let before = gpu.policy().stats();
    frame.submit();
    let after = gpu.policy().stats();
    assert_eq!(after.upload_flushes - before.upload_flushes, 1);
    assert_eq!(after.upload_writes - before.upload_writes, 1000);
    let bytes = read_buffer(&gpu, &first);
    let words: Vec<u32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    assert_eq!(words[..500], (0..500).collect::<Vec<_>>()[..]);
    let doubled = read_buffer(&gpu, &second);
    assert_eq!(
        u32::from_le_bytes(doubled[4 * 499..4 * 500].try_into().unwrap()),
        998
    );

    // Consecutive writes to one buffer become one copy.
    let frame = gpu.begin_frame("merged");
    let uploads = __framework::frame_uploads(&frame);
    for index in 0..500u32 {
        uploads.write_buffer(&first, u64::from(index) * 4, &[7; 4]);
    }
    let before = gpu.policy().stats();
    frame.submit();
    assert_eq!(gpu.policy().stats().upload_copies - before.upload_copies, 1);
    assert_eq!(read_buffer(&gpu, &first)[..8], [7; 8]);
}

#[test]
fn writes_outside_a_frame_and_from_a_discarded_frame_land_at_the_next_submit() {
    let gpu = context();
    let target = texture(
        &gpu,
        GpuTextureUsages::COPY_DST | GpuTextureUsages::COPY_SRC,
    );
    let pixels: Vec<u8> = (0..32).collect();
    gpu.write_texture(
        &target,
        GpuTextureRegion {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        &pixels,
        16,
    )
    .unwrap();
    let buffer = raw_buffer(&gpu, 16);
    let frame = gpu.begin_frame("discarded");
    __framework::frame_uploads(&frame).write_buffer(&buffer, 0, &[9; 16]);
    drop(frame);
    // One submission lands both.
    let device = __framework::device(&gpu);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture readback"),
        size: 256 * 2,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut frame = gpu.begin_frame("reader");
    __framework::encoder(&mut frame).copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: __framework::texture(&target),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(2),
            },
        },
        wgpu::Extent3d {
            width: 4,
            height: 2,
            depth_or_array_layers: 1,
        },
    );
    frame.submit();
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    {
        let rows = readback.slice(..).get_mapped_range().unwrap();
        assert_eq!(&rows[..16], &pixels[..16]);
        assert_eq!(&rows[256..272], &pixels[16..32]);
    }
    readback.unmap();
    assert_eq!(read_buffer(&gpu, &buffer), vec![9; 16]);
}

#[test]
fn steady_frames_reuse_their_staging_chunks() {
    let gpu = context();
    let target = raw_buffer(&gpu, 4096);
    let device = __framework::device(&gpu);
    for frame_index in 0..20u8 {
        let frame = gpu.begin_frame("steady");
        __framework::frame_uploads(&frame).write_buffer(&target, 0, &[frame_index; 4096]);
        frame.submit();
        // A host polls between frames; the chunk comes back then.
        let _ = device.poll(wgpu::PollType::Poll);
    }
    assert!(
        gpu.policy().stats().upload_ring_allocations <= 4,
        "{:?}",
        gpu.policy().stats()
    );
    assert_eq!(read_buffer(&gpu, &target)[..4], [19; 4]);
}

#[test]
fn a_full_pipeline_blocks_on_the_oldest_frame_and_unsubmitted_slots_never_deadlock() {
    let gpu = context();
    for _ in 0..3 {
        gpu.begin_frame("in flight").submit();
    }
    // Three submitted frames hold every slot until they complete; the next
    // frame waits for the oldest instead of spinning.
    let before = gpu.policy().stats();
    gpu.begin_frame("after").submit();
    let after = gpu.policy().stats();
    assert!(after.frame_slot_waits - before.frame_slot_waits <= 1);
    assert_eq!(after.frame_slot_stalls, before.frame_slot_stalls);

    // Let the submitted frames complete and hand their slots back.
    __framework::device(&gpu)
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    // Three recordings nobody submits hold every slot for good. A fourth
    // frame must still start.
    let held: Vec<_> = (0..3).map(|_| gpu.try_begin_frame("held")).collect();
    assert!(held.iter().all(Option::is_some));
    let (done_tx, done_rx) = mpsc::channel();
    let starter = gpu.clone();
    thread::spawn(move || {
        let frame = starter.begin_frame("unslotted");
        done_tx.send(frame.id()).unwrap();
    });
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("begin_frame must not wait on frames that were never submitted");
    assert_eq!(
        gpu.policy().stats().frame_slot_stalls,
        before.frame_slot_stalls + 1
    );
}

#[test]
fn try_begin_frame_returns_immediately_while_recordings_hold_every_slot() {
    let gpu = context();
    let held: Vec<_> = (0..3).map(|_| gpu.try_begin_frame("held")).collect();
    assert!(held.iter().all(Option::is_some));
    let before = gpu.policy().stats();
    let started = std::time::Instant::now();
    assert!(gpu.try_begin_frame("window").is_none());
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "a full pipeline must not wait for GPU completion"
    );
    assert_eq!(
        gpu.policy().stats().frame_slot_waits,
        before.frame_slot_waits
    );
    drop(held);
    assert!(gpu.try_begin_frame("window").is_some());
}

#[test]
fn a_frame_can_move_to_another_thread() {
    fn send<T: Send>() {}
    send::<nana_gpu::FrameContext>();
    send::<__framework::FrameUploadHandle>();
}
