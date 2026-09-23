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
