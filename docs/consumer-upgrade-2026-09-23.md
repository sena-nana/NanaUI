# 消费方升级记录（2026-09-23）

本轮落地 Issue #226 的框架核心（Issue #228 与 #225 见后两节）：package manifest、`.nrpack` 资源包（逐块压缩、XChaCha20-Poly1305 认证加密、Ed25519 发布者签名）、`nana-packager`（Windows / macOS / Linux 布局、Steam 输出、最终产物校验）。详见[打包与分发](packaging.md)。

## API 变化

### 打包工具

- `nana-app-icon` 删除了 `nana-package-app` 二进制，以及 `MacAppPackage` / `package_macos_app`。原因：它们没有 feature 门，会随 `hosted` 编进每个应用（包括其中的 `strip` 调用）。
  - 替代：`cargo run -p nana-packager -- macos-app`，参数不变；版本号改为取自二进制的身份标记，不再写死 0.1.0。
  - 完整产品包用 `nana-packager package`。
- `nana-app-icon` 新增 `encode_icns`。

### 应用身份

- 新增 `nana_ui_platform::application_identity!(id:, name:, version:[, vendor:])`，返回 `ApplicationIdentity`，同时在二进制里留下 `NANA-IDENTITY-V1` 标记。要用 `nana-packager` 打包的应用必须用它声明身份，否则打包直接拒绝。

### 资源包（可选 feature）

- `nana-ui` 新增 feature `packaged-resources`，默认关闭，也不在 `full` 里。开启后可以使用：
  - `NanaApplicationBuilder::resource_packs(ResourcePackOptions)`；
  - `NanaApplication::package_manifest()`；
  - 再导出的 `nana_ui::nana_package`（`KeyProvider`、`StaticKeys`、`TrustPolicy`、`PublisherKey` 等）。
- `nana-ui-core` 新增 `nana://res/` 相关函数：`packaged_logical_path`、`read_packaged`、`install_packaged_source`、`PackagedResourceSource` 等。以下资源加载路径都认这个前缀：
  - 图片 `url()`；
  - `@font-face`；
  - Vue 的 `@import` 与 `@font-face`。
- `nana-diagnostics` 新增 `Domain::RESOURCE`（0x0008）与 `Domain::PACKAGE`（0x0009），以及对应的 `framework::resource` / `framework::package` 描述符。

## 行为变化

- **相对 URL 的基准：** 安装版与便携版布局下，没有显式 base 的相对 `url(...)` / `@font-face` 以 `ApplicationPaths::runtime_resources()` 为基准，不再以进程 CWD 为基准。开发布局（从 `target/` 运行）和没有 `ApplicationPaths` 的嵌入宿主不变。之前依赖“从哪个目录启动就读哪里”的应用，要把资源移到 runtime resources，或改用 `nana://res/`。绝对路径与 `file:` URL 的 jail 不变（宿主设置的 base，否则 cwd）。
- **打包样式表：** `nana://res/` 样式表里的相对 `url()` 在加载时改写为包内绝对地址；没有文件系统 `stylesheet_base` 的 Vue 文档也能 `@import "nana://res/…"`。
- **自检入口：** 启用 `packaged-resources` 的应用，若以 `NANA_PACKAGE_VALIDATE=1` 启动，会在开窗前做只读自检，打印一行 JSON 后退出（成功 0，失败 3）。输出不含密钥。

## Issue #228：未变化的写入不再有成本

### API 变化

- `ComponentView` 新增超 trait `PartialEq`：`ComponentView: Clone + PartialEq + Send + 'static`。自定义组件需要派生或实现 `PartialEq`，`project` 读到的每个字段都要参与比较。字段里有闭包的，按 `Arc::ptr_eq` 比较（参考 `CalendarHeatmap`）。泛型组件 `CalendarHeatmap<T>` 相应要求 `T: PartialEq`。
- 新增 `AppContext::reproject_component(entity)`：组件自身字段没变、但它在 `project` 里读的 world 状态变了，用它重新投影。
- `ComponentView` 新增关联常量 `ALWAYS_REPROJECT: bool`（默认 `false`）。满足下面任一条件的组件设为 `true`，相等的写入也会投影一次，投影不产生写入时照样提前返回：
  - `project` 读取共享的内部可变状态（clone 之后仍指向同一份）；
  - `project` 会往别的组件拥有、也会自己投影的节点上打补丁（容器让托管的内容撑满）。
  框架内置的 `Workspace`、`PaneTree`、`PaneChrome`、`AppShell`、`DesktopShell`、`AppTitleBar`、`SidebarFrame`、`SidebarRow`、`SidebarSection`、`SettingsRow`、`SettingsCollapsibleCard`、`GraphCanvas`、`NativeMarkdown`、`SelectableRichText` 已经声明。

### 行为变化

- **`update_component` 空写短路：** 闭包执行后，如果组件与原值相等、没有排 mutation 和事件，`update_component` 直接返回，不投影、不提交、不跑 lifecycle 和 assembler。只重新投递 program message 的写入同样短路，消息照常送达。
  - 应用侧不需要再按行指纹跳过未变化的行。NanaLive 的 `paint_card_items` 可以删掉 `row_fingerprint`，也可以删掉 `restate_toggle`：用户点翻 Switch 之后，应用下次写入原值就会生效。
  - **以前用 `update_component(e, |_, _| {})` 强制重新投影的写法不再生效，要改成 `reproject_component(e)`。**
- **结构性空写不再弄脏世界：** 下面几种写入在 world 里直接跳过，不失效布局，不 bump generation，也不跑挂载生命周期：
  - `Insert` 把孩子放回它已经在的位置，例如对已经是最后一个孩子的节点再 `append_child`；
  - 对已停放的根再 `park_subtree`；
  - 对已经 unlink 的根再 `detach`。
- **`append_child` 的语义不变：** 对已经挂着、但不在末尾的孩子，`append_child` 仍然会把它挪到末尾。`set_list_item_slots` 自己会插入并排好槽节点，调用前不要再 `append_child` 槽节点。NanaLive 的 `bind_row_thumbnail` 每次刷新都先 `append_child(item, thumb)` 再 `set_list_item_slots`，结果是每行两次真实换位，缩略图被挪到末尾又挪回来，每次刷新都要重排整行。删掉那次 `append_child` 之后，40 行卡片重写一遍相同值：写入约 0.065 ms，flush 空闲。
- **observer 的状态会被投影：** observer 处理器原地修改的组件，在事件投递后会重新投影。以前要等之后某次写入才顺带投影。只有 handler 调用了 `cx.reassemble()` 时才会跑 assembler，这一点没变。

## Issue #225：两阶段启动

合同见[两阶段启动](startup.md)。不配置 Early Splash 的应用不需要改代码。

### API 变化

- 新增 `nana_ui::startup` 模块，并在 crate 根再导出：`StartupOptions`、`SplashSpec` / `SplashLogo` / `SplashAnimation` / `SplashBackground`、`SplashOutcome`、`StartupHandle`、`StartupStatus`、`StartupPhase`、`StartupTicket`、`StartupTakeover`、`StartupTimeline`、`StartupWork`、`StartupError`。
- 入口：`NanaApplicationBuilder::early_splash(spec)` / `startup(options)`、`run_runtime_with_startup`、`with_startup`（给包装 `run_runtime` 的前端用，例如 Vue）。
- `RuntimeProgram` 新增两个有默认实现的方法：`startup_takeover()`（默认 `Immediate`）与 `startup_changed(..)`。`ApplicationState` 同名。`RuntimeProgramContext::startup()` 返回启动记录。
- `nana-window` 新增 `NativeSplash` 及其类型；原生句柄仍只在 `nana-window` 内。
- `nana-diagnostics` 在 `framework::host` 追加事件 `STARTUP_PHASE`（id 4）、`SPLASH_OUTCOME`（5）、`STARTUP_FAILED`（6）和 gauge `STARTUP_LONGEST_BLOCK_NS`（metric id 5）。
- JS：`Nana.startup`（`state`、`deferTakeover`、`takeOver`、`cancelTakeover`、`onChange`）。框架占用的宿主 API 名为 `startupStatus`、`startupDeferTakeover`、`startupTakeOver`、`startupCancelTakeover`，应用自己的 `HostApiRegistry` 不能再用这几个名字。

### 行为变化

- **设备不再在事件线程上请求。** 独立宿主在窗口线程创建窗口与 surface，adapter、设备和第一个 scene painter 的 pipeline 在 `nana-startup-gpu` 线程上建。`initialize` 仍在窗口线程、仍在设备就绪之后调用，程序看到的顺序不变。
- **图标异步应用。** macOS 不再为窗口属性栅格化默认图标（winit 在 macOS 上不用窗口图标）；主窗口的图标与 Dock 图标在后台渲染，到达时应用，可能比窗口首次显示晚一点。
- **窗口清屏色改为线性。** 宿主以前把主题的 sRGB 背景直接当线性清屏色，文档没盖住的区域（加载页、live resize 的边缘）显示成 `#565656`，而不是暗色主题的 `#181818`。现在与画布上的颜色一致。依赖过旧颜色的截图需要重看。
- **macOS 上报减少动态效果。** `RuntimeProgramContext::reduced_motion()` 在 macOS 上读取系统设置（以前恒为 `false`）；运行中切换仍不发送事件。副窗口 `build` 时的上下文现在也带着这个值（以前恒为 `false`）。
- **macOS live resize 的事务 present 生效了。** `set_present_transaction` 以前把视图根层当作 `CAMetalLayer`，而 wgpu 30 把它插为子层，所以固定从未成功；现在能找到子层，live resize 期间的 present 真正与 Core Animation 事务同步。
- **有 splash 时**：窗口在 `initialize` 之前带着 Logo 显示；`initialize` 返回的 startup 消息改走普通消息队列。没有 splash 时两者都和以前一样。

### 各应用

| 应用 | 要做的 |
| --- | --- |
| 所有应用 | 无需改动。自定义 `HostApiRegistry` 若注册了上面四个 `startup*` 名字，需要改名 |
| 想要启动 Logo 的应用 | builder 加 `.early_splash(SplashSpec::new(SplashLogo::png(include_bytes!(..))))`；`initialize` 里的重活改为任务；要等数据再切界面的，实现 `startup_takeover()` 返回 `Deferred`，准备好后 `context.startup().take_over(ticket)` |
