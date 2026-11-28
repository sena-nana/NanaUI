//! Tests for `wgpu::hal::vulkan::Device::buffer_from_raw`, `BufferMemory`, and
//! HAL buffer memory counter accounting on Vulkan.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use nana_wgpu_fork_tests::{device, no_adapter};
use wgpu::hal::vulkan::ash::vk;

const COMPUTE_WGSL: &str = r#"
@group(0) @binding(0)
var<storage, read_write> out_buf: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i < arrayLength(&out_buf) {
        out_buf[i] = i * 7u + 3u;
    }
}
"#;

fn find_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: vk::MemoryPropertyFlags,
) -> Option<u32> {
    for i in 0..props.memory_type_count {
        if (type_bits & (1 << i)) != 0
            && props.memory_types[i as usize]
                .property_flags
                .contains(required)
        {
            return Some(i);
        }
    }
    None
}

unsafe fn create_raw_storage_buffer(
    hal_device: &wgpu::hal::vulkan::Device,
    byte_size: u64,
) -> (vk::Buffer, vk::DeviceMemory, u64) {
    let raw = hal_device.raw_device();
    let instance = hal_device.shared_instance().raw_instance();
    let physical = hal_device.raw_physical_device();
    let mem_props = unsafe { instance.get_physical_device_memory_properties(physical) };

    let info = vk::BufferCreateInfo::default()
        .size(byte_size)
        .usage(
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let vk_buf = unsafe { raw.create_buffer(&info, None) }.expect("vkCreateBuffer");
    let req = unsafe { raw.get_buffer_memory_requirements(vk_buf) };
    let mem_type = find_memory_type(
        &mem_props,
        req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::empty(),
        )
    })
    .expect("memory type");
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(req.size)
        .memory_type_index(mem_type);
    let memory = unsafe { raw.allocate_memory(&alloc, None) }.expect("vkAllocateMemory");
    unsafe { raw.bind_buffer_memory(vk_buf, memory, 0) }.expect("vkBindBufferMemory");
    (vk_buf, memory, req.size)
}

#[test]
fn vulkan_buffer_from_raw_dedicated_compute_roundtrip_and_counters() {
    let Some((_adapter, device, queue)) = device(wgpu::Backends::VULKAN, wgpu::Features::empty())
    else {
        no_adapter(
            "vulkan_buffer_from_raw_dedicated_compute_roundtrip_and_counters",
            wgpu::Backends::VULKAN,
        );
        return;
    };

    const COUNT: usize = 64;
    const BYTE_SIZE: u64 = (COUNT * std::mem::size_of::<u32>()) as u64;

    let before = device.get_internal_counters().hal;
    let before_buffers = before.buffers.read();
    let before_buffer_mem = before.buffer_memory.read();

    let hal_buffer = unsafe {
        let hal_device = device
            .as_hal::<wgpu::hal::api::Vulkan>()
            .expect("Vulkan hal device");
        let (vk_buf, memory, alloc_size) = create_raw_storage_buffer(&hal_device, BYTE_SIZE);
        hal_device.buffer_from_raw(
            vk_buf,
            &wgpu::hal::BufferDescriptor {
                label: Some("fork-test-vulkan-raw-dedicated"),
                size: BYTE_SIZE,
                usage: wgpu::BufferUses::STORAGE_READ_WRITE
                    | wgpu::BufferUses::COPY_SRC
                    | wgpu::BufferUses::COPY_DST,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
            },
            None,
            wgpu::hal::vulkan::BufferMemory::Dedicated {
                memory,
                offset: 0,
                size: alloc_size,
            },
        )
    };

    let imported = unsafe {
        device.create_buffer_from_hal::<wgpu::hal::api::Vulkan>(
            hal_buffer,
            &wgpu::BufferDescriptor {
                label: Some("fork-test-vulkan-raw-dedicated"),
                size: BYTE_SIZE,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
    };

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fork-test-raw-buf-cs"),
        source: wgpu::ShaderSource::Wgsl(COMPUTE_WGSL.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fork-test-raw-buf-pipeline"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: imported.as_entire_binding(),
        }],
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fork-test-readback"),
        size: BYTE_SIZE,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&imported, 0, &readback, 0, BYTE_SIZE);
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    {
        let mapped = slice.get_mapped_range().unwrap();
        let words: Vec<u32> = mapped
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let expected: Vec<u32> = (0..COUNT as u32).map(|i| i * 7 + 3).collect();
        assert_eq!(words, expected);
    }
    readback.unmap();

    drop(bind_group);
    drop(imported);
    drop(readback);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    let after = device.get_internal_counters().hal;
    assert_eq!(
        after.buffers.read(),
        before_buffers,
        "HAL buffer count should return to baseline after destroy"
    );
    assert_eq!(
        after.buffer_memory.read(),
        before_buffer_mem,
        "HAL buffer_memory counter must not underflow when destroying dedicated raw Vulkan buffers"
    );
}

#[test]
fn vulkan_buffer_from_raw_external_runs_drop_callback() {
    let Some((_adapter, device, _queue)) = device(wgpu::Backends::VULKAN, wgpu::Features::empty())
    else {
        no_adapter(
            "vulkan_buffer_from_raw_external_runs_drop_callback",
            wgpu::Backends::VULKAN,
        );
        return;
    };

    const BYTE_SIZE: u64 = 256;
    let dropped = Arc::new(AtomicUsize::new(0));
    let dropped_cb = Arc::clone(&dropped);

    let (raw_device, vk_buf, memory, hal_buffer) = unsafe {
        let hal_device = device
            .as_hal::<wgpu::hal::api::Vulkan>()
            .expect("Vulkan hal device");
        let raw_device = hal_device.raw_device().clone();
        let (vk_buf, memory, _alloc_size) = create_raw_storage_buffer(&hal_device, BYTE_SIZE);
        let hal_buffer = hal_device.buffer_from_raw(
            vk_buf,
            &wgpu::hal::BufferDescriptor {
                label: Some("fork-test-vulkan-raw-external"),
                size: BYTE_SIZE,
                usage: wgpu::BufferUses::STORAGE_READ_WRITE | wgpu::BufferUses::COPY_SRC,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
            },
            Some(Box::new(move || {
                dropped_cb.fetch_add(1, Ordering::SeqCst);
            })),
            wgpu::hal::vulkan::BufferMemory::External,
        );
        (raw_device, vk_buf, memory, hal_buffer)
    };

    let imported = unsafe {
        device.create_buffer_from_hal::<wgpu::hal::api::Vulkan>(
            hal_buffer,
            &wgpu::BufferDescriptor {
                label: Some("fork-test-vulkan-raw-external"),
                size: BYTE_SIZE,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        )
    };
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(imported);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);

    // External ownership: caller destroys both the VkBuffer and its memory.
    unsafe {
        raw_device.destroy_buffer(vk_buf, None);
        raw_device.free_memory(memory, None);
    }
}
