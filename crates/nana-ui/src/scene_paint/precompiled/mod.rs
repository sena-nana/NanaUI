//! Framework shader stages compiled to DXIL ahead of time.
//!
//! On DX12 a pipeline family's WGSL becomes HLSL (naga) and then DXIL (DXC)
//! when its pipelines are created. For the quad family that is over a
//! second, for mesh and text a few hundred milliseconds each, on every
//! launch, since DXC keeps nothing between runs, and the first frame waits
//! for the slowest. The families every application and skin share are
//! compiled here instead (see `tests::regenerate`) and their stages load as
//! passthrough DXIL: the driver then only lowers DXIL to its own code, which
//! it caches across launches.
//!
//! Only framework shaders, and only from the binary. A passthrough module is
//! neither parsed nor validated by naga, so DXIL never comes from a file, a
//! skin or a plugin, and nothing outside this module creates one. A family
//! records what its DXIL was compiled from; the stages are used only while
//! the program and resource layout still match (see [`Family::fresh`]; tests
//! hold the wgpu version), and when the driver refuses them the family
//! compiles its program as it always did.

#[cfg(any(test, target_os = "windows"))]
mod generated;

#[cfg(test)]
pub(super) mod tests;

#[cfg(any(test, target_os = "windows"))]
pub(super) use generated::{
    GROUP, GROUP_BLEND, HOST_TEXTURE, ICON, MESH, QUAD, TEXT, TEXT_DUAL_SOURCE,
};

/// One stage of a family, compiled for shader model 6.0, so any DX12 device
/// with DXIL support runs it.
#[cfg(any(test, target_os = "windows"))]
pub(super) struct Stage {
    pub(super) entry: &'static str,
    /// The pipeline constants it was specialized with, as the family's
    /// pipeline passes them.
    pub(super) constants: &'static [(&'static str, f64)],
    pub(super) dxil: &'static [u8],
}

#[cfg(any(test, target_os = "windows"))]
pub(super) struct Family {
    /// [`fnv1a`] of the program the stages were compiled from, as the
    /// device is given it (the embedded WGSL, without comments).
    pub(super) wgsl: u64,
    /// The family's resource layout key when they were.
    pub(super) layout: u64,
    pub(super) stages: &'static [Stage],
}

#[cfg(any(test, target_os = "windows"))]
impl Family {
    /// Whether the stages were compiled from `wgsl` laid out as `layout`.
    pub(super) fn fresh(&self, wgsl: &str, layout: u64) -> bool {
        !self.stages.is_empty()
            && self.stages.iter().all(|stage| !stage.dxil.is_empty())
            && self.wgsl == fnv1a(wgsl.as_bytes())
            && self.layout == layout
    }

    /// The stage for `entry` specialized with `constants`.
    pub(super) fn stage(&self, entry: &str, constants: &[(&str, f64)]) -> Option<&Stage> {
        self.stages.iter().find(|stage| {
            stage.entry == entry
                && stage.constants.len() == constants.len()
                && stage
                    .constants
                    .iter()
                    .zip(constants)
                    .all(|(a, b)| a.0 == b.0 && a.1 == b.1)
        })
    }
}

/// Incremental FNV-1a, 64 bits. Carriage returns are skipped: a CRLF checkout
/// compiles to the same DXIL, and must hash like an LF one.
pub(crate) const fn fnv1a_update(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\r' {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        index += 1;
    }
    hash
}

/// FNV-1a, 64 bits. Tells a family's WGSL from the one its DXIL was compiled
/// from; it is not a security check. Carriage returns are skipped: a CRLF
/// checkout compiles to the same DXIL, and must hash like an LF one.
pub(crate) const fn fnv1a(bytes: &[u8]) -> u64 {
    fnv1a_update(0xcbf2_9ce4_8422_2325_u64, bytes)
}

/// [`fnv1a`] over `files` joined with `'\n'`, matching [`super::wgsl::joined`].
#[cfg(any(test, not(any(feature = "runtime-wgsl", target_os = "windows"))))]
pub(crate) const fn fnv1a_joined(files: &[&str]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < files.len() {
        if index > 0 {
            hash = fnv1a_update(hash, b"\n");
        }
        hash = fnv1a_update(hash, files[index].as_bytes());
        index += 1;
    }
    hash
}

/// [`fnv1a`] over `parts` concatenated without separators.
#[cfg(any(test, not(any(feature = "runtime-wgsl", target_os = "windows"))))]
pub(crate) const fn fnv1a_concat(parts: &[&str]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < parts.len() {
        hash = fnv1a_update(hash, parts[index].as_bytes());
        index += 1;
    }
    hash
}

/// Whether `device` runs precompiled stages: DX12 with passthrough shaders.
/// Only Windows has DX12; the platform test lets every other target drop the
/// DXIL from its binary.
#[cfg(any(test, target_os = "windows"))]
pub(super) fn runs_on(gpu: &nana_gpu::GpuContext, device: &wgpu::Device) -> bool {
    cfg!(target_os = "windows")
        && gpu.capabilities().backend() == nana_gpu::GpuBackend::Dx12
        && device
            .features()
            .contains(wgpu::Features::PASSTHROUGH_SHADERS)
}

/// [`runs_on`] for a family built where only the device is at hand.
#[cfg(target_os = "windows")]
pub(super) fn runs_on_device(device: &wgpu::Device) -> bool {
    device.adapter_info().backend == wgpu::Backend::Dx12
        && device
            .features()
            .contains(wgpu::Features::PASSTHROUGH_SHADERS)
}

#[cfg(not(any(test, target_os = "windows")))]
pub(super) fn runs_on(_gpu: &nana_gpu::GpuContext, _device: &wgpu::Device) -> bool {
    false
}

/// The pipeline `build` makes from `family`'s stages for `stages` (entry
/// point and pipeline constants each), or `None` when the family has no such
/// stage or the device refused them; the caller then compiles the program.
#[cfg(any(test, target_os = "windows"))]
pub(super) fn pipeline<const N: usize>(
    device: &wgpu::Device,
    family: &Family,
    stages: [(&str, &[(&str, f64)]); N],
    build: impl FnOnce([&wgpu::ShaderModule; N]) -> wgpu::RenderPipeline,
) -> Option<wgpu::RenderPipeline> {
    let mut found = [None; N];
    for (slot, (entry, constants)) in found.iter_mut().zip(stages) {
        *slot = Some(family.stage(entry, constants)?);
    }
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = |stage: &Stage| {
        // SAFETY: the DXIL is the framework's own, compiled from the program
        // `Family::fresh` matched, built into the binary and never replaced.
        unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some("nana-ui.scene.precompiled"),
                entry_points: std::borrow::Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint {
                    name: stage.entry.into(),
                    workgroup_size: (0, 0, 0),
                    metal_sized_bindings: std::borrow::Cow::Borrowed(&[]),
                }]),
                dxil: Some(std::borrow::Cow::Borrowed(stage.dxil)),
                ..Default::default()
            })
        }
    };
    let modules = found.map(|stage| module(stage.expect("every stage was found")));
    let pipeline = build(modules.each_ref());
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
