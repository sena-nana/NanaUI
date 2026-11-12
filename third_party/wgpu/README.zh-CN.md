# NanaUI 的 wgpu fork

[English](README.md)

这里是 wgpu 30.0.1 及其组成 crate（`wgpu`、`wgpu-core`、`wgpu-core-deps-*`、`wgpu-hal`、`wgpu-types`、`wgpu-naga-bridge`、`naga`、`naga-types`）发布出来的源码，由 `scripts/vendor-wgpu.py` 放进来。NanaUI 在这里修改它们：把从 naga IR 到各后端的这一段（着色器与管线缓存、后端输出）握在自己手里，同时保留 wgpu 的校验与资源追踪。

包名和版本沿用上游，各 crate 之间按路径互相依赖，因此：

- **每个依赖图里只有这一份 wgpu。** `scripts/check-engine-boundary.py` 会拒绝来自别处的 wgpu 或 naga crate。
- **只用 NanaUI 的应用什么都不用做。** 以 git 依赖本仓库就会带上 fork；通过 `nana_gpu::wgpu`（feature `wgpu-interop`）使用 wgpu，不要直接依赖 `wgpu`。
- **还用到基于 crates.io wgpu 的库**（egui-wgpu、glyphon 等）的应用，要把 crates.io 指到本仓库，rev 与它的 NanaUI 依赖相同：

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

  这些库随后就基于 fork 编译；只要 fork 没改到它们用的接口，就能通过。NanaUI 自己的工作区也用路径这样处理它的 egui 示例。

## 修改 fork

- 公开的 `wgpu::` 接口尽量只增不改；不兼容的改动放在 `wgpu-core` / `wgpu-hal` 内部和新接口上。
- 每一处公开接口的改动都记在下面的"与上游的差异"里。
- 改动 naga 的 HLSL 输出，或 `wgpu-hal` 的 DX12 后端编译着色器的方式时，必须重新生成 `nana-ui` 里 `scene_paint::precompiled` 的预编译 DXIL：它的 wgpu 版本检查看不到 fork 的改动。
- 这些目录不在工作区成员里：它们自己的测试和 dev-dependencies 不在这里构建。每个清单末尾都有一个空的 `[workspace]`（由 `scripts/vendor-wgpu.py` 加上），让 cargo 把每个库当作独立的工作区根，不再往上找。少了它，在嵌套于另一个 NanaUI 检出里的检出（`.claude/worktrees/` 下的 agent worktree）中，`cargo fmt --all` 会失败：外层工作区认领了这个库，却没把它列为成员。`cargo fmt --all` 也会检查这些库；它们符合 rustfmt，fork 的改动也要保持如此。
- 每项 fork 改动的测试写在 `third_party/wgpu-fork-tests`（工作区成员，`cargo test --workspace` 会跑它）：
  - `dx12_cache_format` 直接把 `wgpu-hal/src/dx12/pipeline_cache.rs` 编译进来，该模块自带的单元测试也在这里跑，所有系统都跑；
  - `pipeline_cache` 通过 `wgpu` 接口测试 DX12 缓存、`set_default_pipeline_cache` 与 `PipelineCache::pipelines_built`；
  - `surface_reconfigure`（Windows）在另一个线程提交的同时，对一个小窗口的 surface 在 DX12 与 Vulkan 上反复重配置；
  - `vulkan_present_tracking`（Windows）在 Vulkan 上、带着在途帧反复缩放窗口，适配器支持的每种 present 跟踪方式各跑一遍，并请求校验层；后端记下任何错误即失败。没有 `VK_LAYER_KHRONOS_validation` 时只检查驱动自己的消息；设 `NANA_REQUIRE_VULKAN_VALIDATION=1` 则缺少校验层就失败；
  - `timed_poll` 在 DX12 与 Vulkan 上让零超时的等待与其他线程的 poll、提交竞争几秒；
  - `device_lost_poll` 不重置 GPU 就让设备丢失（Vulkan 用 hal 的测试专用 `simulate_device_lost`，DX12 用 `ID3D12Device5::RemoveDevice`），检查 poll 报 `PollError::DeviceLost`、丢失回调只调一次、挂起的回调都有结果。

  没有适配器时 GPU 测试会跳过；设 `NANA_REQUIRE_WGPU_FORK_TESTS=1` 则改为失败，Windows CI 对 DX12 就是这样跑的。这些测试里的 GPU 负载要小且有上限。

## 升级到新的上游版本

1. 运行 `python scripts/vendor-wgpu.py <版本>` 并提交。
2. 重新应用 fork 的改动：`git diff <上一次放入的提交> <最后一个 fork 提交> -- third_party/wgpu`。
3. 更新根 `Cargo.toml` 里的版本。

## 与上游的差异

### Surface 重配置只等这个 surface 的工作

`Surface::configure`（窗口缩放）不再等整个设备空闲。上游会先等所有队列提交完成，等待期间另一个线程提交就以 `ConfigureSurfaceError::GpuWaitTimeout` 失败，所以应用只好让所有队列工作与缩放串行。fork 只等仍在使用这个 surface 交换链图像的工作；校验不变（仍持有已获取的输出时报 `PreviousOutputExists`）。

- **`wgpu-core`**：每个 `Surface` 记下最后一次用到它纹理的提交序号（提交时盖章）。`configure_surface` 先取走 presentation，使等待期间不能再获取新纹理，然后只等这个序号，而不是 `wait_indefinitely`。期间其他线程的提交不再被等待，也不再让 configure 失败。`TextureInner::Surface` 多了 `last_submission` 字段（`wgpu-core` 的公开枚举；`wgpu` 不暴露它）。
- **`wgpu-hal` DX12**：队列在每次 `Present` 之后立即 signal 一个围栏；`configure` / `unconfigure` 在 `ResizeBuffers` / 释放前只等这个值，不再排空整个队列（设备上的 present 队列 idler 已删除）。
- **`wgpu-hal` Vulkan**：`configure` 不再调用 `vkDeviceWaitIdle`。旧交换链作为 `oldSwapchain` 传入后已退役，确知它的 present 都已完成后才销毁。怎样确知取决于适配器支持什么，依次为：
  - **Present 围栏**（`VK_EXT_swapchain_maintenance1`，否则 `VK_KHR_swapchain_maintenance1`，并需要对应的 `VK_*_surface_maintenance1` 实例扩展和 `swapchainMaintenance1` 特性）：每次 present 都带一个 `VkSwapchainPresentFenceInfoEXT` 围栏，按扩展的定义，它在该次 present 的信号量和图像可以释放时 signal。旧交换链若在 `configure` 时围栏都已 signal 就当场释放，否则在之后某次 acquire 时检查到 signal 再释放（轮询，不阻塞）。`unconfigure` 改为等这些围栏（最多 100 ms），不再等设备空闲。优先用 EXT，因为 ash 0.38 的结构体用的是它的名字，校验层也按它检查挂上的结构体。
  - **Present wait**（`VK_KHR_present_id` + `VK_KHR_present_wait`）：每次 present 都带一个 id。`vkWaitForPresentKHR` 不能用于已退役的交换链，所以 `configure` 先最多等 100 ms，等旧交换链最后一次 present 显示出来，然后当场释放。60 Hz 的 FIFO 交换链上，每次 configure 约多花两个刷新间隔；超时则该交换链退回下面的启发式。
  - **两者都没有（启发式）**：旧交换链保留到绘制其后继交换链的某次提交完成为止（在下一次 acquire 时非阻塞检查）。这次提交排在旧交换链最后一次 present 之后，但 Vulkan 并不保证那时 present 已完成；这是一个时序假设，与上游依赖设备空闲的做法类似。

  实例在可用时启用 `VK_KHR_get_surface_capabilities2` 与 `VK_*_surface_maintenance1`。环境变量 `NANA_WGPU_VULKAN_PRESENT_TRACKING`（`fence`、`wait` 或 `none`）在打开设备时读取，用来限制选择，让测试能分别跑每条路径。没有 present 围栏时，`unconfigure` 与销毁仍未释放的退役交换链仍等待设备空闲。
- **Metal、GLES**：它们的 `configure` 本来不等待，只受 `wgpu-core` 改动影响。

`wgpu` 的接口没有变化。`ConfigureSurfaceError::GpuWaitTimeout` 仍保留，在等待本身超时时返回。

### DX12 上的着色器与管线缓存

- **`wgpu::Device::set_default_pipeline_cache(Option<&PipelineCache>)`**（新增）。
  以 `cache: None` 创建的管线改用设备的默认缓存。设备只弱引用它（`PipelineCache`
  本身持有设备），应用留着句柄它才生效。别的设备的缓存是校验错误；没有缓存的后端忽略这个调用。
  背后是 `wgpu-core` 的 `Device::set_default_pipeline_cache` /
  `Global::device_set_default_pipeline_cache` 与锁等级 `DEVICE_DEFAULT_PIPELINE_CACHE`；
  dispatch trait 上的方法带空的默认实现，自定义后端照样编译。
- **DX12 上的 `Features::PIPELINE_CACHE`。** `wgpu-hal` 的 DX12 `PipelineCache`
  存放编译好的着色器字节码（DXC 的 DXIL、FXC 的 DXBC），键是 naga 写出的 HLSL、入口点、
  目标 profile 与编译选项的 128 位哈希。缓存里有的阶段不再经过编译器；编译出来的阶段加进去
  （设备内存缓存里已有的也加）。`create_pipeline_cache` 带 `data` 时载入，`PipelineCache::get_data`
  导出（格式见 `wgpu-hal/src/dx12/pipeline_cache.rs`：逐条校验和，连续 8 次保存未用到的条目丢弃）。
  字节码与适配器无关，所以 `wgpu-core` 的头部在 DX12 上用固定的适配器键，校验键写的是编译器
  （种类、版本和 DXC 提交）：缓存字节可以换适配器使用，不能换编译器。不缓存 PSO
  （`ID3D12PipelineLibrary`），驱动自有缓存。

- **`wgpu::PipelineCache::pipelines_built() -> Option<u64>`**（新增）。经过这个缓存创建的管线数，
  无论是描述符里指名的还是作为设备默认缓存；命中缓存的也算。自上次 `get_data` 以来计数不变，
  就说明不可能新增了条目，宿主可以不取数据、不保存。`wgpu-core` 在它的 `PipelineCache`
  上放一个原子计数，渲染或计算管线用它创建成功后加一，并提供
  `Global::pipeline_cache_pipelines_built`；dispatch trait 上的方法默认返回 `None`，
  自定义后端与 WebGPU 后端照样编译。

### 带超时的 poll 与别的线程的 poll 竞争时报超时

`Device::poll` 的 `Wait` 超时后先读围栏，再整理队列。若这期间 GPU 完成、另一个线程的 poll 把所有提交都整理掉了，队列为空，而这次 poll 读到的围栏值仍低于它等待的序号。上游断言这不可能发生（"If the queue is empty, the current submission index … should be at least the wait submission index …"）并 panic。fork 在 `wgpu-core` 的 `Device::maintain` 里只有读到的围栏达到等待序号时才返回 `QueueEmpty`，否则报 `Err(PollError::Timeout)`，也就是它自己那次等待的结果。`wgpu` 的接口没有变化。

### 设备丢失是 poll 的错误，不再 panic

上游的 `wgpu::Device::poll` 在后端等围栏或读围栏报设备丢失时 panic（"Error in Device::poll: Parent device is lost"），Vulkan 在 TDR 或驱动重置后返回 `VK_ERROR_DEVICE_LOST` 就是这样。DX12 上被移除的设备围栏读出 `u64::MAX`，被当成"工作全部完成"：poll 成功，丢失无人察觉。

- **`wgpu::PollError::DeviceLost`**（新增变体；对 `PollError` 做穷尽匹配的代码需要改）。设备在这次 poll 中或之前丢失时 `Device::poll` 返回它，之后每次 poll 都再返回它。
- **`wgpu-core`**：`Device::maintain` 在等围栏或读围栏失败时不再立即返回。`Device::handle_hal_error` 已把设备标为无效并调用设备丢失回调（只一次，原因为 `DeviceLostReason::Unknown`）；随后这次 poll 把所有提交当作已完成，`on_submitted_work_done` 与 `map_async` 的回调都会执行（映射以错误结束，或映射到丢失设备的内存上），"无效且队列为空"的路径释放设备的资源，最后报告丢失。`WaitIdleError::to_poll_error` 把 `DeviceError::Lost` 与 `OutOfMemory`（两者在这里都会让设备丢失）映射为 `PollError::DeviceLost`。丢失设备上的提交本来就会被拒绝，不会 panic。
- **`wgpu-hal` DX12**：围栏读出 `u64::MAX` 时查 `GetDeviceRemovedReason`；设备已被移除则 `get_fence_value` 与 `wait` 返回 `DeviceError::Lost`。
- **`wgpu-hal` Vulkan**：`vulkan::Device::simulate_device_lost()`（`#[doc(hidden)]`，仅供测试）先等设备空闲，之后等围栏、读围栏与队列提交都返回 `DeviceError::Lost`，测试不必真的重置就能走到这条路径。
