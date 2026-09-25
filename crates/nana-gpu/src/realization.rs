//! WGPU realization of the logical ABI.  The handles are opaque to consumers.

use std::sync::Arc;

use crate::{
    DeviceGeneration, GpuContext, GpuError, LogicalBindingType, LogicalResource, ResourceSet,
    ResourceTable, ShaderInterface, ShaderStage,
};

pub(crate) fn create_resource_layout_raw(
    device: &wgpu::Device,
    generation: DeviceGeneration,
    table: &ResourceTable,
) -> Result<GpuResourceLayout, GpuError> {
    table.validate_generation(generation)?;
    if table
        .bindings()
        .iter()
        .any(|binding| matches!(binding.ty, LogicalBindingType::ResourceArray))
    {
        return Err(GpuError::UnsupportedCapability("resource_arrays"));
    }
    let entries = table
        .bindings()
        .iter()
        .map(|binding| {
            let visibility =
                binding
                    .stages
                    .iter()
                    .fold(wgpu::ShaderStages::empty(), |flags, stage| {
                        flags
                            | match stage {
                                ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
                                ShaderStage::Fragment => wgpu::ShaderStages::FRAGMENT,
                                ShaderStage::Compute => wgpu::ShaderStages::COMPUTE,
                            }
                    });
            let ty = match binding.ty {
                LogicalBindingType::UniformBuffer | LogicalBindingType::DynamicBufferSlice => {
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: matches!(
                            binding.ty,
                            LogicalBindingType::DynamicBufferSlice
                        ),
                        min_binding_size: binding.min_size.and_then(std::num::NonZeroU64::new),
                    }
                }
                LogicalBindingType::StorageBuffer { read_only } => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: binding.min_size.and_then(std::num::NonZeroU64::new),
                },
                LogicalBindingType::SampledTexture => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                LogicalBindingType::StorageTexture => wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                LogicalBindingType::Sampler => {
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                }
                LogicalBindingType::ResourceArray => unreachable!(),
            };
            wgpu::BindGroupLayoutEntry {
                binding: binding.binding,
                visibility,
                ty,
                count: binding.array_size,
            }
        })
        .collect::<Vec<_>>();
    Ok(GpuResourceLayout {
        generation,
        key: table.layout_key(),
        layout: Arc::new(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("nana logical resource layout"),
                entries: &entries,
            }),
        ),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuBufferUsages(u32);

impl GpuBufferUsages {
    pub const UNIFORM: Self = Self(1 << 0);
    pub const STORAGE: Self = Self(1 << 1);
    pub const COPY_SRC: Self = Self(1 << 2);
    pub const COPY_DST: Self = Self(1 << 3);
    pub const VERTEX: Self = Self(1 << 4);
    pub const INDEX: Self = Self(1 << 5);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for GpuBufferUsages {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuBufferDescriptor<'a> {
    pub label: Option<&'a str>,
    pub size: u64,
    pub usage: GpuBufferUsages,
}

#[derive(Clone)]
pub struct GpuBuffer {
    pub(crate) generation: DeviceGeneration,
    pub(crate) size: u64,
    pub(crate) usage: GpuBufferUsages,
    pub(crate) buffer: Arc<wgpu::Buffer>,
}

impl std::fmt::Debug for GpuBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuBuffer")
            .field("generation", &self.generation)
            .field("size", &self.size)
            .field("usage", &self.usage)
            .finish()
    }
}

impl GpuBuffer {
    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }
    pub const fn size(&self) -> u64 {
        self.size
    }
    pub const fn usage(&self) -> GpuBufferUsages {
        self.usage
    }
}

impl GpuBuffer {
    pub(crate) fn wrap_raw(
        gpu: &GpuContext,
        buffer: wgpu::Buffer,
        size: u64,
        usage: GpuBufferUsages,
    ) -> Self {
        Self {
            generation: gpu.generation(),
            size,
            usage,
            buffer: Arc::new(buffer),
        }
    }
}

#[derive(Clone)]
pub struct GpuSampler {
    pub(crate) generation: DeviceGeneration,
    pub(crate) sampler: Arc<wgpu::Sampler>,
}

impl std::fmt::Debug for GpuSampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuSampler")
            .field("generation", &self.generation)
            .finish()
    }
}

impl GpuSampler {
    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }
}

impl GpuSampler {
    pub(crate) fn wrap_raw(gpu: &GpuContext, sampler: wgpu::Sampler) -> Self {
        Self {
            generation: gpu.generation(),
            sampler: Arc::new(sampler),
        }
    }
}

#[derive(Clone)]
pub struct GpuResourceLayout {
    pub(crate) generation: crate::DeviceGeneration,
    pub(crate) key: u64,
    #[allow(dead_code)]
    pub(crate) layout: Arc<wgpu::BindGroupLayout>,
}

#[derive(Clone)]
pub struct GpuResourceGroup {
    generation: DeviceGeneration,
    layout_key: u64,
    dynamic_offsets: Arc<[u32]>,
    #[allow(dead_code)]
    pub(crate) bind_group: Arc<wgpu::BindGroup>,
}

impl std::fmt::Debug for GpuResourceGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuResourceGroup")
            .field("generation", &self.generation)
            .field("layout_key", &self.layout_key)
            .finish()
    }
}

impl GpuResourceGroup {
    pub fn generation(&self) -> DeviceGeneration {
        self.generation
    }
    pub const fn layout_key(&self) -> u64 {
        self.layout_key
    }
    pub fn dynamic_offsets(&self) -> &[u32] {
        &self.dynamic_offsets
    }
}

impl std::fmt::Debug for GpuResourceLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuResourceLayout")
            .field("generation", &self.generation)
            .field("key", &self.key)
            .finish()
    }
}

impl GpuResourceLayout {
    pub fn generation(&self) -> crate::DeviceGeneration {
        self.generation
    }
    pub const fn key(&self) -> u64 {
        self.key
    }
    #[allow(dead_code)]
    pub(crate) fn raw(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }
}

impl GpuContext {
    /// Upload bytes into a logical buffer while preserving the host submission
    /// ordering contract. The buffer must belong to this device generation and
    /// have been created with `COPY_DST`.
    pub fn write_buffer(
        &self,
        buffer: &GpuBuffer,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), GpuError> {
        self.check_device(buffer.generation)?;
        if !buffer.usage.contains(GpuBufferUsages::COPY_DST)
            || offset
                .checked_add(bytes.len() as u64)
                .is_none_or(|end| end > buffer.size)
        {
            return Err(GpuError::InvalidBindingRange);
        }
        let _submission = self.lock_submission();
        self.inner.queue.write_buffer(&buffer.buffer, offset, bytes);
        Ok(())
    }

    pub fn create_buffer(
        &self,
        descriptor: &GpuBufferDescriptor<'_>,
    ) -> Result<GpuBuffer, GpuError> {
        if descriptor.size == 0
            || descriptor.size > self.limits().max_buffer_size
            || descriptor.usage.bits() == 0
        {
            return Err(GpuError::InvalidBufferDescriptor);
        }
        let mut usage = wgpu::BufferUsages::empty();
        if descriptor.usage.contains(GpuBufferUsages::UNIFORM) {
            usage |= wgpu::BufferUsages::UNIFORM;
        }
        if descriptor.usage.contains(GpuBufferUsages::STORAGE) {
            usage |= wgpu::BufferUsages::STORAGE;
        }
        if descriptor.usage.contains(GpuBufferUsages::COPY_SRC) {
            usage |= wgpu::BufferUsages::COPY_SRC;
        }
        if descriptor.usage.contains(GpuBufferUsages::COPY_DST) {
            usage |= wgpu::BufferUsages::COPY_DST;
        }
        if descriptor.usage.contains(GpuBufferUsages::VERTEX) {
            usage |= wgpu::BufferUsages::VERTEX;
        }
        if descriptor.usage.contains(GpuBufferUsages::INDEX) {
            usage |= wgpu::BufferUsages::INDEX;
        }
        let buffer = self.inner.device.create_buffer(&wgpu::BufferDescriptor {
            label: descriptor.label,
            size: descriptor.size,
            usage,
            mapped_at_creation: false,
        });
        Ok(GpuBuffer {
            generation: self.generation(),
            size: descriptor.size,
            usage: descriptor.usage,
            buffer: Arc::new(buffer),
        })
    }

    pub fn create_sampler(&self) -> GpuSampler {
        GpuSampler {
            generation: self.generation(),
            sampler: Arc::new(
                self.inner
                    .device
                    .create_sampler(&wgpu::SamplerDescriptor::default()),
            ),
        }
    }

    pub fn create_resource_group(
        &self,
        layout: &GpuResourceLayout,
        table: &ResourceTable,
        set: &ResourceSet,
    ) -> Result<GpuResourceGroup, GpuError> {
        self.check_device(layout.generation)?;
        table.validate_generation(self.generation())?;
        set.validate_generation(self.generation())?;
        if layout.key != table.layout_key() {
            return Err(GpuError::ShaderInterfaceMismatch);
        }
        for binding in set.bindings() {
            if table.binding(binding.binding).is_none() {
                return Err(GpuError::MissingBinding(binding.binding));
            }
        }
        let array_views: Vec<Vec<&wgpu::TextureView>> = table
            .bindings()
            .iter()
            .map(|declaration| {
                match set
                    .binding(declaration.binding)
                    .and_then(|binding| binding.resource.as_ref())
                {
                    Some(LogicalResource::TextureArray(textures)) => {
                        textures.iter().map(|texture| texture.raw_view()).collect()
                    }
                    _ => Vec::new(),
                }
            })
            .collect();
        let mut entries = Vec::with_capacity(table.bindings().len());
        let mut dynamic_offsets = Vec::new();
        for (index, declaration) in table.bindings().iter().enumerate() {
            let value = set
                .binding(declaration.binding)
                .and_then(|binding| binding.resource.as_ref());
            let Some(value) = value else {
                if declaration.optional {
                    return Err(GpuError::UnsupportedCapability(
                        "optional_resource_fallback",
                    ));
                }
                return Err(GpuError::MissingBinding(declaration.binding));
            };
            let type_matches = matches!(
                (declaration.ty, value),
                (
                    LogicalBindingType::UniformBuffer | LogicalBindingType::DynamicBufferSlice,
                    LogicalResource::Buffer { .. }
                ) | (
                    LogicalBindingType::StorageBuffer { .. },
                    LogicalResource::Buffer { .. }
                ) | (
                    LogicalBindingType::SampledTexture | LogicalBindingType::StorageTexture,
                    LogicalResource::Texture(_)
                ) | (LogicalBindingType::Sampler, LogicalResource::Sampler(_))
                    | (
                        LogicalBindingType::ResourceArray,
                        LogicalResource::TextureArray(_)
                    )
            );
            if !type_matches {
                return Err(GpuError::BindingTypeMismatch(declaration.binding));
            }
            let resource = match value {
                LogicalResource::Buffer {
                    buffer,
                    offset,
                    size,
                } => {
                    let required_usage =
                        if matches!(declaration.ty, LogicalBindingType::StorageBuffer { .. }) {
                            GpuBufferUsages::STORAGE
                        } else {
                            GpuBufferUsages::UNIFORM
                        };
                    if !buffer.usage.contains(required_usage) {
                        return Err(GpuError::InvalidBindingRange);
                    }
                    let alignment = u64::from(
                        if matches!(declaration.ty, LogicalBindingType::StorageBuffer { .. }) {
                            self.limits().min_storage_buffer_offset_alignment
                        } else {
                            self.limits().min_uniform_buffer_offset_alignment
                        },
                    );
                    if *offset % alignment != 0
                        || *size == 0
                        || declaration.min_size.is_some_and(|minimum| *size < minimum)
                        || *size
                            > if matches!(declaration.ty, LogicalBindingType::StorageBuffer { .. })
                            {
                                self.limits().max_storage_buffer_binding_size
                            } else {
                                self.limits().max_uniform_buffer_binding_size
                            }
                        || offset
                            .checked_add(*size)
                            .is_none_or(|end| end > buffer.size)
                    {
                        return Err(GpuError::InvalidBindingRange);
                    }
                    let dynamic = matches!(declaration.ty, LogicalBindingType::DynamicBufferSlice);
                    if dynamic {
                        let dynamic_offset =
                            u32::try_from(*offset).map_err(|_| GpuError::InvalidBindingRange)?;
                        dynamic_offsets.push(dynamic_offset);
                    }
                    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer.buffer,
                        offset: if dynamic { 0 } else { *offset },
                        size: std::num::NonZeroU64::new(*size),
                    })
                }
                LogicalResource::Texture(texture) => {
                    let required = if matches!(declaration.ty, LogicalBindingType::StorageTexture) {
                        crate::GpuTextureUsages::STORAGE
                    } else {
                        crate::GpuTextureUsages::SAMPLED
                    };
                    if !texture.usage().contains(required) {
                        return Err(GpuError::MissingUsage(required));
                    }
                    if matches!(declaration.ty, LogicalBindingType::StorageTexture)
                        && texture.format() != crate::GpuTextureFormat::RGBA8_UNORM
                    {
                        return Err(GpuError::UnsupportedFormat(texture.format()));
                    }
                    wgpu::BindingResource::TextureView(texture.raw_view())
                }
                LogicalResource::Sampler(sampler) => {
                    wgpu::BindingResource::Sampler(&sampler.sampler)
                }
                LogicalResource::TextureArray(textures) => {
                    if !self
                        .capabilities()
                        .supports(crate::GpuCapability::ResourceArrays)
                    {
                        return Err(GpuError::UnsupportedCapability("resource_arrays"));
                    }
                    if declaration
                        .array_size
                        .is_none_or(|count| count.get() as usize != textures.len())
                        || textures.iter().any(|texture| {
                            !texture.usage().contains(crate::GpuTextureUsages::SAMPLED)
                        })
                    {
                        return Err(GpuError::InvalidBindingRange);
                    }
                    wgpu::BindingResource::TextureViewArray(&array_views[index])
                }
            };
            entries.push(wgpu::BindGroupEntry {
                binding: declaration.binding,
                resource,
            });
        }
        let bind_group = self
            .inner
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nana logical resource group"),
                layout: &layout.layout,
                entries: &entries,
            });
        Ok(GpuResourceGroup {
            generation: self.generation(),
            layout_key: layout.key,
            dynamic_offsets: dynamic_offsets.into(),
            bind_group: Arc::new(bind_group),
        })
    }

    pub fn create_resource_layout(
        &self,
        table: &ResourceTable,
    ) -> Result<GpuResourceLayout, GpuError> {
        table.validate_generation(self.generation())?;
        if table.bindings().len() > self.limits().max_bindings_per_group as usize {
            return Err(GpuError::UnsupportedCapability("max_bindings_per_group"));
        }
        for binding in table.bindings() {
            if matches!(binding.ty, LogicalBindingType::ResourceArray)
                && !self
                    .capabilities()
                    .supports(crate::GpuCapability::ResourceArrays)
            {
                return Err(GpuError::UnsupportedCapability("resource_arrays"));
            }
        }
        let entries = table
            .bindings()
            .iter()
            .map(|binding| {
                let visibility =
                    binding
                        .stages
                        .iter()
                        .fold(wgpu::ShaderStages::empty(), |flags, stage| {
                            flags
                                | match stage {
                                    ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
                                    ShaderStage::Fragment => wgpu::ShaderStages::FRAGMENT,
                                    ShaderStage::Compute => wgpu::ShaderStages::COMPUTE,
                                }
                        });
                let ty = match binding.ty {
                    LogicalBindingType::UniformBuffer | LogicalBindingType::DynamicBufferSlice => {
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            // The logical offset is baked into the binding
                            // value. This keeps `GpuResourceGroup` opaque and
                            // avoids exposing backend-specific dynamic-offset
                            // arrays to ordinary renderers.
                            has_dynamic_offset: matches!(
                                binding.ty,
                                LogicalBindingType::DynamicBufferSlice
                            ),
                            min_binding_size: binding.min_size.and_then(std::num::NonZeroU64::new),
                        }
                    }
                    LogicalBindingType::StorageBuffer { read_only } => wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only },
                        has_dynamic_offset: false,
                        min_binding_size: binding.min_size.and_then(std::num::NonZeroU64::new),
                    },
                    LogicalBindingType::SampledTexture => wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    LogicalBindingType::StorageTexture => wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    LogicalBindingType::Sampler => {
                        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                    }
                    LogicalBindingType::ResourceArray => wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                };
                wgpu::BindGroupLayoutEntry {
                    binding: binding.binding,
                    visibility,
                    ty,
                    count: binding.array_size,
                }
            })
            .collect::<Vec<_>>();
        let policy = self.policy();
        let layout =
            policy.resource_layout(table.layout_key(), || {
                Arc::new(self.inner.device.create_bind_group_layout(
                    &wgpu::BindGroupLayoutDescriptor {
                        label: Some("nana logical resource layout"),
                        entries: &entries,
                    },
                ))
            });
        Ok(GpuResourceLayout {
            generation: self.generation(),
            key: table.layout_key(),
            layout,
        })
    }

    pub fn validate_shader_interface(&self, interface: &ShaderInterface) -> Result<(), GpuError> {
        if interface
            .bindings()
            .bindings()
            .iter()
            .any(|binding| binding.binding >= self.limits().max_bindings_per_group)
        {
            return Err(GpuError::ShaderInterfaceMismatch);
        }
        Ok(())
    }
}
