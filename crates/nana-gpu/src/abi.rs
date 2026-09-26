//! Backend-neutral logical GPU ABI.
//!
//! This module describes what a renderer needs.  It deliberately contains no
//! WGPU handles; a backend realizes these declarations for its own API.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::{DeviceGeneration, GpuBuffer, GpuError, GpuSampler, GpuTexture};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceClass {
    Persistent,
    Transient,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicalBindingType {
    UniformBuffer,
    StorageBuffer { read_only: bool },
    SampledTexture,
    StorageTexture,
    Sampler,
    DynamicBufferSlice,
    ResourceArray,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicalBinding {
    pub binding: u32,
    pub stages: &'static [ShaderStage],
    pub ty: LogicalBindingType,
    pub class: ResourceClass,
    pub optional: bool,
    pub array_size: Option<NonZeroU32>,
    pub min_size: Option<u64>,
}

impl LogicalBinding {
    pub const fn new(binding: u32, ty: LogicalBindingType, stages: &'static [ShaderStage]) -> Self {
        Self {
            binding,
            stages,
            ty,
            class: ResourceClass::Persistent,
            optional: false,
            array_size: None,
            min_size: None,
        }
    }
    pub const fn class(mut self, class: ResourceClass) -> Self {
        self.class = class;
        self
    }
    pub const fn optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }
    pub const fn array(mut self, size: NonZeroU32) -> Self {
        self.array_size = Some(size);
        self
    }
    pub const fn min_size(mut self, size: u64) -> Self {
        self.min_size = Some(size);
        self
    }
    pub fn validate_range(&self, offset: u64, size: u64, alignment: u32) -> Result<(), GpuError> {
        if !matches!(self.ty, LogicalBindingType::DynamicBufferSlice) {
            return Err(GpuError::InvalidBindingRange);
        }
        let alignment = u64::from(alignment.max(1));
        if !offset.is_multiple_of(alignment)
            || size == 0
            || self.min_size.is_some_and(|minimum| size < minimum)
        {
            return Err(GpuError::InvalidBindingRange);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTable {
    bindings: Arc<[LogicalBinding]>,
    layout_key: u64,
    generation: Option<DeviceGeneration>,
}

#[derive(Debug, Clone)]
pub enum LogicalResource {
    Buffer {
        buffer: GpuBuffer,
        offset: u64,
        size: u64,
    },
    Texture(GpuTexture),
    Sampler(GpuSampler),
    TextureArray(Vec<GpuTexture>),
}

#[derive(Debug, Clone)]
pub struct ResourceBinding {
    pub binding: u32,
    pub resource: Option<LogicalResource>,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceSet {
    bindings: Vec<ResourceBinding>,
}

impl ResourceSet {
    pub fn new(mut bindings: Vec<ResourceBinding>) -> Result<Self, GpuError> {
        bindings.sort_by_key(|binding| binding.binding);
        if bindings
            .windows(2)
            .any(|pair| pair[0].binding == pair[1].binding)
        {
            return Err(GpuError::DuplicateBinding);
        }
        Ok(Self { bindings })
    }
    pub fn bindings(&self) -> &[ResourceBinding] {
        &self.bindings
    }
    pub fn binding(&self, slot: u32) -> Option<&ResourceBinding> {
        self.bindings.iter().find(|binding| binding.binding == slot)
    }
    pub fn validate_generation(&self, generation: DeviceGeneration) -> Result<(), GpuError> {
        for binding in &self.bindings {
            let found = match binding.resource.as_ref() {
                Some(LogicalResource::Buffer { buffer, .. }) => buffer.generation(),
                Some(LogicalResource::Texture(texture)) => texture.generation(),
                Some(LogicalResource::Sampler(sampler)) => sampler.generation(),
                Some(LogicalResource::TextureArray(textures)) => {
                    for texture in textures {
                        if texture.generation() != generation {
                            return Err(GpuError::DeviceMismatch {
                                expected: generation,
                                found: texture.generation(),
                            });
                        }
                    }
                    generation
                }
                None => continue,
            };
            if found != generation {
                return Err(GpuError::DeviceMismatch {
                    expected: generation,
                    found,
                });
            }
        }
        Ok(())
    }
}

impl ResourceTable {
    pub fn new(mut bindings: Vec<LogicalBinding>) -> Result<Self, GpuError> {
        bindings.sort_by_key(|binding| binding.binding);
        if bindings
            .windows(2)
            .any(|pair| pair[0].binding == pair[1].binding)
        {
            return Err(GpuError::DuplicateBinding);
        }
        for binding in &bindings {
            if binding.stages.is_empty()
                || (matches!(binding.ty, LogicalBindingType::ResourceArray)
                    != binding.array_size.is_some())
                || (binding.min_size.is_some()
                    && !matches!(
                        binding.ty,
                        LogicalBindingType::UniformBuffer
                            | LogicalBindingType::StorageBuffer { .. }
                            | LogicalBindingType::DynamicBufferSlice
                    ))
            {
                return Err(GpuError::ShaderInterfaceMismatch);
            }
        }
        let mut key = 0xcbf29ce484222325u64;
        for binding in &bindings {
            key ^= u64::from(binding.binding);
            key = key.wrapping_mul(0x100000001b3);
            key ^= binding_type_key(binding.ty);
            key = key.wrapping_mul(0x100000001b3);
            key ^= binding.optional as u64;
            key = key.wrapping_mul(0x100000001b3);
            key ^= binding.array_size.map_or(0, NonZeroU32::get) as u64;
            key = key.wrapping_mul(0x100000001b3);
            key ^= binding.min_size.unwrap_or_default();
            key = key.wrapping_mul(0x100000001b3);
            key ^= match binding.class {
                ResourceClass::Persistent => 1,
                ResourceClass::Transient => 2,
                ResourceClass::External => 3,
            };
            let stage_mask = binding.stages.iter().fold(0u64, |mask, stage| {
                mask | match stage {
                    ShaderStage::Vertex => 1,
                    ShaderStage::Fragment => 2,
                    ShaderStage::Compute => 4,
                }
            });
            key = key.wrapping_mul(0x100000001b3) ^ stage_mask;
        }
        Ok(Self {
            bindings: bindings.into(),
            layout_key: key,
            generation: None,
        })
    }
    pub fn empty() -> Self {
        Self::new(Vec::new()).expect("empty table is valid")
    }
    pub fn bindings(&self) -> &[LogicalBinding] {
        &self.bindings
    }
    pub const fn layout_key(&self) -> u64 {
        self.layout_key
    }
    pub const fn generation(&self) -> Option<DeviceGeneration> {
        self.generation
    }
    pub const fn for_generation(mut self, generation: DeviceGeneration) -> Self {
        self.generation = Some(generation);
        self
    }
    pub fn binding(&self, slot: u32) -> Option<&LogicalBinding> {
        self.bindings.iter().find(|b| b.binding == slot)
    }
    pub fn validate_generation(&self, generation: DeviceGeneration) -> Result<(), GpuError> {
        if let Some(found) = self.generation
            && found != generation
        {
            return Err(GpuError::DeviceMismatch {
                expected: generation,
                found,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
    pub location: u32,
    pub components: u8,
    pub stride: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderInterface {
    bindings: ResourceTable,
    vertex: Arc<[VertexAttribute]>,
    instance: Arc<[VertexAttribute]>,
    wgsl: Arc<str>,
    key: u64,
}

impl ShaderInterface {
    pub fn new(bindings: ResourceTable, wgsl: impl Into<Arc<str>>) -> Self {
        let wgsl = wgsl.into();
        let key = bindings.layout_key() ^ fxhash(wgsl.as_bytes());
        Self {
            bindings,
            vertex: Arc::<[VertexAttribute]>::from([]),
            instance: Arc::<[VertexAttribute]>::from([]),
            wgsl,
            key,
        }
    }
    pub fn with_vertex_layout(mut self, attributes: impl Into<Arc<[VertexAttribute]>>) -> Self {
        self.vertex = attributes.into();
        self.key = self.recompute_key();
        self
    }
    pub fn with_instance_layout(mut self, attributes: impl Into<Arc<[VertexAttribute]>>) -> Self {
        self.instance = attributes.into();
        self.key = self.recompute_key();
        self
    }
    pub fn bindings(&self) -> &ResourceTable {
        &self.bindings
    }
    pub fn vertex_layout(&self) -> &[VertexAttribute] {
        &self.vertex
    }
    pub fn instance_layout(&self) -> &[VertexAttribute] {
        &self.instance
    }
    pub fn wgsl(&self) -> &str {
        &self.wgsl
    }
    pub const fn key(&self) -> u64 {
        self.key
    }
    /// Stable identity to place in [`crate::PipelineKey::shader`] together
    /// with the resource layout identity.
    pub const fn pipeline_identity(&self) -> u64 {
        self.key
    }

    fn recompute_key(&self) -> u64 {
        let mut key = self.bindings.layout_key() ^ fxhash(self.wgsl.as_bytes());
        for attribute in self.vertex.iter().chain(self.instance.iter()) {
            key = key.wrapping_mul(0x100000001b3) ^ u64::from(attribute.location);
            key = key.wrapping_mul(0x100000001b3) ^ u64::from(attribute.components);
            key = key.wrapping_mul(0x100000001b3) ^ u64::from(attribute.stride);
        }
        key
    }
}

fn fxhash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
fn binding_type_key(ty: LogicalBindingType) -> u64 {
    match ty {
        LogicalBindingType::UniformBuffer => 1,
        LogicalBindingType::StorageBuffer { read_only: false } => 2,
        LogicalBindingType::StorageBuffer { read_only: true } => 3,
        LogicalBindingType::SampledTexture => 4,
        LogicalBindingType::StorageTexture => 5,
        LogicalBindingType::Sampler => 6,
        LogicalBindingType::DynamicBufferSlice => 7,
        LogicalBindingType::ResourceArray => 8,
    }
}

impl fmt::Display for ResourceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Persistent => "persistent",
            Self::Transient => "transient",
            Self::External => "external",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VERTEX: &[ShaderStage] = &[ShaderStage::Vertex];

    #[test]
    fn resource_tables_sort_and_reject_duplicates() {
        let table = ResourceTable::new(vec![
            LogicalBinding::new(3, LogicalBindingType::Sampler, VERTEX),
            LogicalBinding::new(1, LogicalBindingType::UniformBuffer, VERTEX),
        ])
        .unwrap();
        assert_eq!(
            table
                .bindings()
                .iter()
                .map(|b| b.binding)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(matches!(
            ResourceTable::new(vec![
                LogicalBinding::new(1, LogicalBindingType::Sampler, VERTEX),
                LogicalBinding::new(1, LogicalBindingType::Sampler, VERTEX)
            ]),
            Err(GpuError::DuplicateBinding)
        ));
        let a = ResourceTable::new(vec![LogicalBinding::new(
            0,
            LogicalBindingType::Sampler,
            &[ShaderStage::Vertex, ShaderStage::Fragment],
        )])
        .unwrap();
        let b = ResourceTable::new(vec![LogicalBinding::new(
            0,
            LogicalBindingType::Sampler,
            &[ShaderStage::Fragment, ShaderStage::Vertex],
        )])
        .unwrap();
        assert_eq!(a.layout_key(), b.layout_key());
    }

    #[test]
    fn interface_key_includes_shader_source_and_layout() {
        let table = ResourceTable::new(vec![LogicalBinding::new(
            0,
            LogicalBindingType::UniformBuffer,
            VERTEX,
        )])
        .unwrap();
        let a = ShaderInterface::new(
            table.clone(),
            "@vertex fn main() -> @builtin(position) vec4f { return vec4f(); }",
        );
        let b = ShaderInterface::new(
            table,
            "@vertex fn main() -> @builtin(position) vec4f { return vec4f(1.0); }",
        );
        assert_ne!(a.key(), b.key());
        let c = a.clone().with_vertex_layout(Arc::from([VertexAttribute {
            location: 0,
            components: 2,
            stride: 8,
        }]));
        assert_ne!(a.key(), c.key());
    }

    #[test]
    fn dynamic_ranges_require_alignment_and_minimum_size() {
        let binding =
            LogicalBinding::new(0, LogicalBindingType::DynamicBufferSlice, VERTEX).min_size(16);
        assert!(binding.validate_range(256, 16, 256).is_ok());
        assert_eq!(
            binding.validate_range(4, 16, 256),
            Err(GpuError::InvalidBindingRange)
        );
        assert_eq!(
            binding.validate_range(256, 8, 256),
            Err(GpuError::InvalidBindingRange)
        );
    }
}
