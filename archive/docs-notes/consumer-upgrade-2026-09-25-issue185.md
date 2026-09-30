# Issue #185：Logical GPU ABI

`nana-gpu` now exposes backend-neutral `LogicalBinding`, `ResourceTable` and
`ShaderInterface` declarations. They carry binding visibility, resource class,
optional/array shape, dynamic minimum size and a stable layout key. A table can
be pinned to a `DeviceGeneration`; a replaced device is rejected before WGPU
validation.

`GpuContext::create_resource_layout` is the WGPU realization boundary. It
creates an opaque `GpuResourceLayout`, while the WGPU bind-group layout remains
private to the backend. Portable WGSL remains the reference source. Resource
arrays are capability checked and unsupported devices return a structured
`GpuError::UnsupportedCapability` outcome.

`GpuBuffer`, `GpuSampler`, `LogicalResource`, `ResourceBinding` and
`ResourceSet` provide the value side of the contract. `GpuContext::create_resource_group`
validates generation, binding completeness, usage and dynamic ranges before
creating the opaque WGPU bind group. Raw bind groups are never part of the
ordinary public API.
`GpuContext::write_buffer` uploads to logical buffers after checking generation,
`COPY_DST` usage and bounds under the host submission guard.

The built-in quad, icon, mesh, motion, text atlas, HostTexture, backdrop
copy/blur/composite, destination blit and DefaultGpuView paths now realize
their ordinary resource layouts through the logical ABI. Their pipeline keys
include ABI layout identities. The reading-blend temporary copy remains an
explicit framework-only WGPU operation; it is not exposed as a consumer ABI
requirement.

`GpuCapabilities` now reports resource arrays, indirect and multi-draw,
timestamps, external-resource and transient-hint outcomes, together with
buffer limits and alignment. `GpuDeviceState` remains the owner of per-device
pipeline/resource lifetime and retirement; logical layout keys are safe inputs
for that registry.

The public ABI intentionally does not model Scene, UI, Live2D or application
state. Renderer-specific WGPU commands remain an explicit interop escape hatch
until a later contract adds the corresponding logical operation.
