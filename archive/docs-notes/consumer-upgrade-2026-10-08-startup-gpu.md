# 启动更快：DX12 用随包的 DXC，painter 并行编译

独立宿主从入口到 `UiReady` 的时间，大头是启动线程上建 scene painter 时的着色器编译。这次改了三件事：

- painter 不再编译只给测试回读用的 motion 求值 pipeline。各 pipeline 家族并行编译。pipeline registry 在锁外编译。
- 请求合成后端时，设备请求直接从建窗前合成探测枚举过的 adapter 里选，不再枚举第二次。
- DX12 的着色器编译器改为：exe 旁有 `dxcompiler.dll` 时按完整路径用 DXC，否则用 FXC。`WGPU_DX12_COMPILER` 设了就以它为准。

RTX 5060 Ti 上 `startup-splash --probe --composition` 的 `UiReady`：release 从约 5.1 s 降到约 2.8 s（FXC）或约 1.6–2.3 s（DXC）；debug 从约 10.4–12.1 s 降到约 6–9 s（FXC）或约 3.0–3.5 s（DXC）。数字和拆分见 [两阶段启动](../../docs/reference/startup.md#dx12-着色器编译器)。

## 需要改的地方

- 想要 DXC 的 Windows 应用，把 x64 `dxcompiler.dll`（DXC v1.8.2502 或更新）和 exe 放在同一目录发行。不需要 `dxil.dll`，也不需要打开任何 feature。不放就照旧用 FXC，只是启动慢。
- `StartupWork` 新增 `device_request: Option<Duration>` 与 `painter_build: Option<Duration>`。用结构体字面量构造 `StartupWork` 的代码要补上这两个字段。
- `nana-diagnostics` 在 `framework::host` 追加 gauge `STARTUP_DEVICE_REQUEST_NS`（metric id 7）和 `STARTUP_PAINTER_BUILD_NS`（metric id 8）。

## 行为变化

- 以前 wgpu 的默认 `Auto` 会在 DLL 搜索路径（含 `PATH`）里找 `dxcompiler.dll`。现在只认 exe 旁的那一个。依赖 `PATH` 上的 DXC 的环境，改为把 DLL 放到 exe 旁，或设 `WGPU_DX12_COMPILER=dxc`。
- `WGPU_DX12_COMPILER` 以前被宿主忽略，现在生效。
- 首帧画面不变：painter 仍在 `UiReady` 前建好全部产品路径要用的 pipeline。
