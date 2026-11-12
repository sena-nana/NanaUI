# NanaUI's wgpu fork

[中文](README.zh-CN.md)

These are the published sources of wgpu 30.0.1 and the crates it is built from
(`wgpu`, `wgpu-core`, `wgpu-core-deps-*`, `wgpu-hal`, `wgpu-types`,
`wgpu-naga-bridge`, `naga`, `naga-types`), vendored by
`scripts/vendor-wgpu.py`. NanaUI changes them here: owning the path from naga
IR to each backend (shader and pipeline caches, backend output) without giving
up wgpu's validation and resource tracking.

Package names and versions stay upstream's, and the crates depend on each
other by path, so:

- **Every graph holds this one wgpu.** `scripts/check-engine-boundary.py`
  rejects a wgpu or naga crate from anywhere else.
- **An application that only uses NanaUI needs nothing.** A git dependency on
  this repository brings the fork; reach wgpu through `nana_gpu::wgpu`
  (feature `wgpu-interop`) instead of depending on `wgpu` directly.
- **An application that also uses a crate built on crates.io wgpu** (egui-wgpu,
  glyphon, ...) patches crates.io onto this repository, at the same rev as its
  NanaUI dependencies:

  ```toml
  [patch.crates-io]
  naga = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  naga-types = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-core = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-core-deps-apple = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-core-deps-emscripten = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-core-deps-windows-linux-android = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-hal = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-naga-bridge = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  wgpu-types = { git = "https://github.com/sena-nana/NanaUI.git", rev = "<rev>" }
  ```

  Those crates then build against the fork; they compile as long as the fork
  has not changed an API they use. NanaUI's own workspace does the same with
  paths for its egui example.

## Changing the fork

- Keep the public `wgpu::` API additive where possible; put breaking changes
  in `wgpu-core` / `wgpu-hal` internals and in new APIs.
- List every change to a public API under *Changes from upstream* below.
- A change to naga's HLSL output or to how `wgpu-hal`'s DX12 backend compiles
  shaders must regenerate the precompiled DXIL in `nana-ui`'s
  `scene_paint::precompiled`: its wgpu version check cannot see fork changes.
- These directories are excluded from the workspace: their own tests and
  dev-dependencies do not build here. Each manifest ends in an empty
  `[workspace]` (added by `scripts/vendor-wgpu.py`), so cargo treats every
  crate as its own workspace root and does not look further up. Without it,
  `cargo fmt --all` fails in a checkout nested inside another NanaUI checkout
  (agent worktrees under `.claude/worktrees/`): the outer workspace claims
  the crate without listing it. `cargo fmt --all` checks these crates too;
  they are rustfmt-clean, and fork edits keep them so.
- Test each fork change in `third_party/wgpu-fork-tests` (a workspace member,
  so `cargo test --workspace` runs it):
  - `dx12_cache_format` compiles `wgpu-hal/src/dx12/pipeline_cache.rs` in
    directly, so that module's own unit tests run there, on every OS;
  - `pipeline_cache` covers the DX12 cache, `set_default_pipeline_cache` and
    `PipelineCache::pipelines_built` through the `wgpu` API;
  - `surface_reconfigure` (Windows) reconfigures a small window's surface on
    DX12 and Vulkan while another thread submits;
  - `vulkan_present_tracking` (Windows) resizes a window with frames in flight
    on Vulkan once per present-tracking path the adapter has, with validation
    requested, and fails on any error the backend logs. Without
    `VK_LAYER_KHRONOS_validation` it checks the driver's messages only;
    `NANA_REQUIRE_VULKAN_VALIDATION=1` makes a missing layer fail;
  - `timed_poll` races zero-timeout waits against other threads' polls and
    submissions on DX12 and Vulkan for a few seconds;
  - `device_lost_poll` loses a device without a GPU reset (Vulkan through
    the hal's test-only `simulate_device_lost`, DX12 through
    `ID3D12Device5::RemoveDevice`) and checks that polls report
    `PollError::DeviceLost`, the lost callback runs once and pending
    callbacks resolve.

  A GPU test without an adapter skips; `NANA_REQUIRE_WGPU_FORK_TESTS=1`
  makes it fail instead, as the Windows CI job does for DX12.
  Keep any GPU load in these tests small and bounded.

## Moving to a new upstream version

1. `python scripts/vendor-wgpu.py <version>` and commit.
2. Re-apply the fork's changes: `git diff <previous vendoring commit> <last fork commit> -- third_party/wgpu`.
3. Update the version in the root `Cargo.toml`.

## Changes from upstream

### Surface reconfiguration waits for that surface only

`Surface::configure` (a window resize) no longer waits for the whole device to
go idle. Upstream drains every queue submission first and fails with
`ConfigureSurfaceError::GpuWaitTimeout` when another thread submits during
that wait, so applications serialized all queue work against resizes. The fork
waits only for the work that still uses that surface's swapchain images;
validation is unchanged (an output still acquired is `PreviousOutputExists`).

- **`wgpu-core`**: each `Surface` keeps the index of the last submission that
  used one of its textures (stamped at submit). `configure_surface` takes the
  presentation first, so no new texture can be acquired, then waits for that
  index instead of `wait_indefinitely`. Submissions made meanwhile from other
  threads are not waited for and no longer fail the configure.
  `TextureInner::Surface` gains a `last_submission` field (a public enum of
  `wgpu-core`; nothing in `wgpu` exposes it).
- **`wgpu-hal` DX12**: the queue signals a fence right after each `Present`;
  `configure` / `unconfigure` wait for that value before `ResizeBuffers` /
  release, instead of draining the queue (the device's present-queue idler is
  gone).
- **`wgpu-hal` Vulkan**: `configure` no longer calls `vkDeviceWaitIdle`. The
  old swapchain, passed as `oldSwapchain` and therefore retired, is destroyed
  once its presents are known done. How that is known depends on what the
  adapter offers, in this order:
  - **Present fences** (`VK_EXT_swapchain_maintenance1`, else
    `VK_KHR_swapchain_maintenance1`, with the matching
    `VK_*_surface_maintenance1` instance extension and the
    `swapchainMaintenance1` feature): every present carries a
    `VkSwapchainPresentFenceInfoEXT` fence, which signals once the present's
    semaphores and image may be freed, as the extension defines. The old
    swapchain is freed at `configure` if its fences have signaled, otherwise at
    a later acquire once they have (polled, never blocking). `unconfigure`
    waits for the fences (up to 100 ms) instead of the device. The EXT is
    preferred because ash 0.38's structures carry its names and validation
    checks the chained structure against it.
  - **Present wait** (`VK_KHR_present_id` + `VK_KHR_present_wait`): every
    present carries an id. `vkWaitForPresentKHR` is not valid on a retired
    swapchain, so `configure` waits, up to 100 ms, for the old swapchain's
    last present to be shown, then frees it at once. On a 60 Hz FIFO swapchain
    this costs about two refresh intervals per configure; past the timeout the
    swapchain falls back to the heuristic below.
  - **Neither (heuristic)**: the old swapchain is kept until a submission that
    drew to its successor completes (checked without blocking at the next
    acquire). That submission was queued after the old swapchain's last
    present, but Vulkan does not promise the present is done by then; this is
    a timing assumption, much like upstream's reliance on device idle.

  The instance enables `VK_KHR_get_surface_capabilities2` and the
  `VK_*_surface_maintenance1` extensions when present. The environment
  variable `NANA_WGPU_VULKAN_PRESENT_TRACKING` (`fence`, `wait` or `none`),
  read when a device is opened, limits the choice so tests can run each path.
  `unconfigure` and dropping a retired swapchain without present fences still
  wait for the device.
- **Metal, GLES**: their `configure` did not wait; they get the `wgpu-core`
  change only.

The `wgpu` API is unchanged. `ConfigureSurfaceError::GpuWaitTimeout` stays and
is still returned when the wait itself times out.

### Shader and pipeline cache on DX12

- **`wgpu::Device::set_default_pipeline_cache(Option<&PipelineCache>)`** (new).
  A pipeline created with `cache: None` uses the device's default cache. The
  device holds it weakly (a `PipelineCache` holds its device), so it applies
  while the application keeps the handle. A cache from another device is a
  validation error; backends without caches ignore the call. Behind it:
  `wgpu-core` `Device::set_default_pipeline_cache` /
  `Global::device_set_default_pipeline_cache` and lock rank
  `DEVICE_DEFAULT_PIPELINE_CACHE`; the dispatch trait method has a no-op
  default, so custom backends still compile.
- **`Features::PIPELINE_CACHE` on DX12.** `wgpu-hal`'s DX12 `PipelineCache` holds
  compiled shader bytecode (DXIL from DXC, DXBC from FXC) keyed by a 128-bit
  hash of the HLSL naga wrote, the entry point, the target profile and the
  compile flags. A stage found there skips the compiler; a stage compiled is
  added to it (also one the device's in-memory cache already had).
  `create_pipeline_cache` with `data` loads it and `PipelineCache::get_data`
  saves it (format in `wgpu-hal/src/dx12/pipeline_cache.rs`: per-entry
  checksums, unused entries dropped after 8 saves). Bytecode does not depend
  on the adapter, so `wgpu-core`'s header uses a constant adapter key on DX12
  and the validation key names the compiler (kind, version and DXC commit):
  cache bytes move between adapters but not between compilers. PSOs are not
  cached (`ID3D12PipelineLibrary`); drivers keep their own.
- **`wgpu::PipelineCache::pipelines_built() -> Option<u64>`** (new). How many
  pipelines were created through the cache, named in their descriptor or as
  the device default; a hit counts too. A count unchanged since the last
  `get_data` means nothing could have been added, so a host skips fetching
  and saving. `wgpu-core` keeps an atomic counter on its `PipelineCache`,
  bumped after a render or compute pipeline is created with it, and exposes
  `Global::pipeline_cache_pipelines_built`; the dispatch trait method
  defaults to `None`, so custom and WebGPU backends still compile.

### A timed poll racing another thread's poll reports a timeout

`Device::poll` with a `Wait` that times out reads the fence, then triages the
queue. If the GPU finishes and another thread's poll triages every
submission in between, the queue is empty while the fence value this poll
read is still below its wait target. Upstream asserts that this cannot
happen ("If the queue is empty, the current submission index … should be at
least the wait submission index …") and panics. In `wgpu-core`'s
`Device::maintain` the fork returns `QueueEmpty` only when the fence it read
reached the wait target; otherwise the poll reports `Err(PollError::Timeout)`,
which is what its own wait did. The `wgpu` API is unchanged.

### A lost device is a poll error, not a panic

Upstream `wgpu::Device::poll` panicked ("Error in Device::poll: Parent device
is lost") when the backend's fence wait or fence read reported the device
lost, as Vulkan does with `VK_ERROR_DEVICE_LOST` after a TDR or driver reset.
On DX12 a removed device's fences read `u64::MAX`, which counted as "all work
finished": polls succeeded and the loss went unnoticed.

- **`wgpu::PollError::DeviceLost`** (new variant; a break for exhaustive
  matches on `PollError`). `Device::poll` returns it for a device lost during
  this poll or before; every later poll returns it again.
- **`wgpu-core`**: `Device::maintain` no longer returns as soon as the fence
  wait or read fails. `Device::handle_hal_error` already marked the device
  invalid and ran the device-lost callback (once, with
  `DeviceLostReason::Unknown`); the poll then counts every submission as
  finished, so `on_submitted_work_done` and `map_async` callbacks run (a map
  resolves with an error or on the lost device's memory), and the
  invalid-and-empty path releases the device's resources. It reports the
  loss last. `WaitIdleError::to_poll_error` maps `DeviceError::Lost` and
  `OutOfMemory` (both lose the device there) to `PollError::DeviceLost`.
  Submits on the lost device were already rejected without a panic.
- **`wgpu-hal` DX12**: a fence that reads `u64::MAX` is checked against
  `GetDeviceRemovedReason`; a removed device makes `get_fence_value` and
  `wait` return `DeviceError::Lost`.
- **`wgpu-hal` Vulkan**: `vulkan::Device::simulate_device_lost()`
  (`#[doc(hidden)]`, tests only) waits for the device to go idle, then makes
  fence waits, fence reads and queue submits return `DeviceError::Lost`, so
  tests reach this path without a real reset.
