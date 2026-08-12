# NanaUI

NanaUI 是 Nana 系列应用使用的 Rust 原生 UI 框架。当前基座为 Iced
`0.15.0-dev` 与 WGPU `30.0.0`，不依赖 WebView、DOM、CSS 或 JavaScript。

## 工作区框架

`crates/nana-ui` 以 LiliaUI 的视觉层级和工作区交互为基线，提供：

- `WorkspaceController`：统一管理 Region 布局、折叠、可见性、尺寸约束、
  分隔条拖动、双击复位、按需帧驱动的折叠过渡、窗口尺寸和 DPI；
- `WorkspaceRegions` / `workspace_view`：按注册顺序接收任意 Region 内容，根据
  role、placement 与 scope 编排起始区、主区、结束区和上下区域；
- `WorkspaceSlots`：标准六区应用的便捷适配器，不限制框架的动态区域模型；
- `WorkspaceLayout`：提供稳定/自定义 Region ID、响应式策略、JSON 持久化以及
  逻辑/物理像素几何；
- `SidebarFrame` / `SidebarSection` / `SidebarRow` / `SidebarFooter`：
  提供可滚动主体、固定外槽、带高度与箭头过渡的分区折叠、层级导航与真实
  Footer 操作；
- `SettingsModel` / `SettingsState`：提供稳定设置 Tab、别名恢复、普通/full-page
  页面和可序列化导航状态；
- `AppearanceSettings` / `ThemeTokens`：提供可序列化的标准圆角设置，内部派生
  四档圆角，并让宿主以 `UI_METRICS` 为默认值即时覆盖公共控件外观；
- `settings_sidebar` / `settings_page` / `SettingsRow` / `SettingsCard`：
  提供与 LiliaUI 一致的紧凑设置布局；
- `app_shell` / `app_title_bar`：提供可复用的应用壳层；
- light/dark 主题、LiliaUI 同源的 Noto Sans SC 400/500/600/700 字体、
  基础控件、浮层和 WGPU 内容插槽。

`examples/component-gallery` 是独立的标准 UI Demo crate。它以单个 220px
`Resources` Region 承载控件、表面、反馈和工作区分类及固定设置 Footer；进入设置
后，同一位置切换为设置分类。主题、面板显隐、尺寸复位、返回导航和所有组件操作
都连接到 Demo 自己的 Rust 状态，`nana-ui` 不包含或导出 Gallery 展示模型。

`ThemeMode`、`AppearanceSettings`、`SettingsState` 与 `WorkspaceLayout` 均可
序列化。NanaUI 不选择配置目录或自行写盘，消费应用负责将这些状态组合进自己的
配置文件。

消费应用创建 Iced application 时应注册 `ui_font_sources()` 并将
`ui_font(Normal)` 设为默认字体；仓库中的所有 Demo 和离屏 renderer 已执行该
注册。字体文件由 LiliaUI 使用的同一组 Noto Sans SC WOFF2 无损转换为 TTF，
许可见 `crates/nana-ui/assets/fonts/OFL.txt`。

## 按需构建

`nana-ui` 默认不启用可选 feature，只编译基础主题、Shell、Workspace、Sidebar、
窗口合同、操作按钮和菜单。消费者显式启用实际使用的组件族：

```toml
[dependencies]
nana-ui = { path = "../NanaUI/crates/nana-ui", features = ["controls", "surfaces"] }
```

Cargo 不会根据消费代码中的 `use` 自动推断 feature。可选组件族包括
`calendar`、`controls`、`feedback`、`image-viewer`、
`overlays`、`popover`、`selects`、`settings-components`、`surfaces` 与
`xy-pad`；`components` 一次启用全部组件。`gpu` 启用宿主纹理与自定义 WGPU
View，`bundled-fonts` 才会把四个 Noto Sans SC 字体资源编入目标；需要完整库
能力时可直接启用 `full`。

组件仍可从 crate 根导入以保持兼容，也可以通过
`nana_ui::components::<family>` 使用稳定的职责子模块。Rust 原生静态链接不等同于
Web 动态 `import()`：未启用的组件由 Cargo feature 在编译期排除，已启用但未引用
的函数由 Release 链接裁剪。独立 Gallery crate 显式依赖完整组件集合，运行时只
构造当前视图；日历模型与菜单数据分别在首次访问反馈页和首次打开菜单时初始化。

运行 UI Gallery：

```bash
cargo run -p component-gallery
```

运行自定义 WGPU 内容插槽：

```bash
cargo run -p nana-ui --example gpu-view-demo --features bundled-fonts,gpu
```

运行由宿主掌握窗口与 WGPU 上下文的组合 Demo：

```bash
cargo run -p nana-ui --example hosted-gpu-demo --features bundled-fonts,gpu
```

`hosted-gpu-demo` 由宿主创建 `winit::Window`、事件循环、WGPU
`Instance`、`Device`、`Queue` 与 `Surface`。Iced 和宿主场景复用同一 GPU
上下文，宿主纹理通过 `GpuTextureView` 直接进入 UI 合成，没有 CPU 回读、
图片编码或第二套 Device。

## Vue + JavaScript 源码兼容

`nana-ui-vue` 提供的是 WebView 中常见 Vue 3 源码的 Nana 兼容子集，不是
WebView 或 Tauri 运行时。消费应用以 Vite 编译 SFC、TypeScript 与 CSS，在自己的
入口中从 `@nanaui/nanavue-runtime` 调用 `createNanaApp()`；产出的 IIFE 由
QuickJS 或 V8 执行，语义树最终仍只由 NanaUI/Iced/WGPU 绘制。

框架默认只注册 renderer、DOM 子集与 Web API。应用业务命令和鉴权由消费宿主
通过 `HostApiRegistry` 提供，名称与框架 API 冲突时初始化直接失败。网络通过
应用显式配置的 `FetchHost` 执行，`FetchPolicy` 默认拒绝所有 origin；兼容面是
缓冲式 `fetch`、`Headers`、`Request`、`Response` 与 `AbortSignal`，不包含
流式 body、cookie、cache、CORS、WebSocket 或 Tauri API。

最小消费入口和锁定构建见
[`crates/nana-js-engine/fixtures/vue-sfc-compat`](crates/nana-js-engine/fixtures/vue-sfc-compat)。
该夹具只用于源码兼容验收，不是外部 bundle loader 或 Demo CLI。

## 当前依赖基线

- Iced：`0.15.0-dev`，固定到 `sena-nana/iced`
  `f6fddd3ce0bc123ec64ce77c1839d56dc8465ba6`；
- WGPU：`30.0.0`，依赖图中只有一个 WGPU 主版本；
- Cryoglyph：随 Iced 分叉同步迁移到 WGPU 30，并固定到
  `sena-nana/cryoglyph` `3fe41b131eda1288d08df89ad5ba56de97713308`；
- Rust edition：2024，最低 Rust `1.92`。

Iced 与 Cryoglyph 均使用完整 Git revision，`Cargo.lock` 记录解析后的 commit。
GitHub Actions 从独立 checkout 运行 `--locked` 测试与全目标检查，不依赖相邻
仓库。

## 验证

```bash
cargo fmt --all -- --check
cargo check -p nana-ui --lib --no-default-features --locked
cargo check -p component-gallery --bin component-gallery --locked
cargo test --workspace --all-targets --locked
cargo check --workspace --all-targets --locked
cargo check -p nana-ui --all-targets --all-features --locked
cargo check -p component-gallery --all-targets --all-features --locked
cargo check -p vue-counter --all-targets --features windowed --locked
cargo check -p vue-counter --all-targets --no-default-features --features engine-v8,windowed --locked
(cd crates/nana-js-engine/fixtures/vue-sfc-compat && npm ci && npm run build)
cargo check -p nana-css-parity --all-targets --features webview-ref --locked # macOS
cargo clippy --workspace --all-targets --locked
cargo clippy -p nana-ui -p component-gallery --all-targets --all-features --locked --no-deps -- -D warnings
```

Workspace application crates intentionally make QuickJS and V8 mutually exclusive, so the legal
feature combinations are checked independently instead of enabling `--all-features` globally.
Workspace-wide Clippy reports the existing cross-package lint backlog; NanaUI and Gallery enforce
zero warnings for the public UI path.

生成 Gallery 的 Workspace、组件状态与 dark/light 验收快照：

```bash
cargo run --release -p component-gallery --bin ui-snapshots \
  --features snapshots --locked
```

PNG 输出到 `target/ui-snapshots`。快照工具会执行 GPU→CPU 读取；正式窗口、
`GpuView` 和 `GpuTextureView` 渲染路径不会使用该流程。

Issue #1 的实现与验收记录见
[`docs/issue-1-acceptance.md`](docs/issue-1-acceptance.md)。本阶段没有修改
NanaShader。
