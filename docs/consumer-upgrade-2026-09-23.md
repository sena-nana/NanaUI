# 消费方升级记录（2026-09-23）

本轮落地 Issue #226 的框架核心：package manifest、`.nrpack` 资源包（逐块压缩、XChaCha20-Poly1305 认证加密、Ed25519 发布者签名）、`nana-packager`（Windows / macOS / Linux 布局、Steam 输出、最终产物校验）。详见[打包与分发](packaging.md)。

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
