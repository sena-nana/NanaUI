//! Framework shader stages compiled to DXIL ahead of time.
//!
//! On DX12 a pipeline family's WGSL becomes HLSL (naga) and then DXIL (DXC)
//! when its pipelines are created. For the quad family that is about two
//! seconds, on every launch, since DXC keeps nothing between runs, and the
//! first frame waits for it. The families every application and skin share
//! are compiled here instead (see `tests::regenerate`) and their stages load
//! as passthrough DXIL: the driver then only lowers DXIL to its own code,
//! which it caches across launches.
//!
//! Only framework shaders, and only from the binary. A passthrough module is
//! neither parsed nor validated by naga, so DXIL never comes from a file, a
//! skin or a plugin, and nothing outside this module creates one. A family
//! records what its DXIL was compiled from; the stages are used only while
//! the WGSL and resource layout still match (see [`Family::fresh`]; tests hold
//! the wgpu version), and when the driver refuses them the family compiles
//! its WGSL as it always did.

mod generated;

#[cfg(test)]
pub(super) mod tests;

pub(super) use generated::QUAD;

/// One stage of a family, compiled for shader model 6.0, so any DX12 device
/// with DXIL support runs it.
pub(super) struct Stage {
    pub(super) entry: &'static str,
    pub(super) dxil: &'static [u8],
}

pub(super) struct Family {
    /// [`fnv1a`] of the WGSL the stages were compiled from.
    pub(super) wgsl: u64,
    /// The family's resource layout key when they were.
    pub(super) layout: u64,
    pub(super) vertex: Stage,
    pub(super) fragment: Stage,
}

impl Family {
    /// Whether the stages were compiled from `wgsl` laid out as `layout`.
    pub(super) fn fresh(&self, wgsl: &str, layout: u64) -> bool {
        !self.vertex.dxil.is_empty()
            && !self.fragment.dxil.is_empty()
            && self.wgsl == fnv1a(wgsl.as_bytes())
            && self.layout == layout
    }
}

/// FNV-1a, 64 bits. Tells a family's WGSL from the one its DXIL was compiled
/// from; it is not a security check.
pub(super) const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// Whether `device` runs precompiled stages: DX12 with passthrough shaders.
pub(super) fn runs_on(gpu: &nana_gpu::GpuContext, device: &wgpu::Device) -> bool {
    gpu.capabilities().backend() == nana_gpu::GpuBackend::Dx12
        && device
            .features()
            .contains(wgpu::Features::PASSTHROUGH_SHADERS)
}

/// The pipeline `build` makes from `family`'s stages, or `None` when the
/// device refused them; the caller then compiles the family's WGSL.
pub(super) fn pipeline(
    device: &wgpu::Device,
    family: &Family,
    build: impl FnOnce(&wgpu::ShaderModule, &wgpu::ShaderModule) -> wgpu::RenderPipeline,
) -> Option<wgpu::RenderPipeline> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = |stage: &Stage| {
        // SAFETY: the DXIL is the framework's own, compiled from the WGSL
        // `Family::fresh` matched, built into the binary and never replaced.
        unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some("nana-ui.scene.precompiled"),
                entry_points: std::borrow::Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint {
                    name: stage.entry.into(),
                    workgroup_size: (0, 0, 0),
                }]),
                dxil: Some(std::borrow::Cow::Borrowed(stage.dxil)),
                ..Default::default()
            })
        }
    };
    let pipeline = build(&module(&family.vertex), &module(&family.fragment));
    #[cfg(test)]
    tests::LOADED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let refused = [
        pollster::block_on(validation.pop()),
        pollster::block_on(internal.pop()),
        pollster::block_on(oom.pop()),
    ]
    .iter()
    .any(Option::is_some);
    (!refused).then_some(pipeline)
}
