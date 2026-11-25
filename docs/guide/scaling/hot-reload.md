# 热重载

你改完界面之后，能多快再看到它，取决于改的是什么。热重载住在 `nana-ui-dev`。这个 crate 在没有 `debug_assertions` 时拒绝编译。`[profile.dist]` 继承 `release`，所以产品依赖一旦带上它，构建直接失败。没有逃生开关。

| 改什么 | 发生什么 | 量级 |
| --- | --- | --- |
| `.vue` 里只改静态文字或静态 `<style>` 属性 | 替换文本表或开发期样式补丁，节点、焦点和状态不动 | 不重编译、不重启 |
| Rust 源码 | 重编译，交接窗口几何和状态，再重新执行 | 1.5–3.6 s，链接占大头 |

macOS / Apple Silicon、默认 dev profile、已经预热的增量构建，`cargo build -p component-gallery`：应用 crate（1.8 万行）1.5–1.6 s，含重新链接 131 MB 的 debug 二进制；改过 `nana-ui-runtime`（12.7 万行）后再建应用是 3.6 s，连带重编 `nana-ui-scene` 和 `nana-ui`。Windows 的链接更慢。在你自己的机器上用 `cargo build -p <你的包> --timings` 量一次。窗口加 GPU 重建的 0.4–1.5 s 没有重新测量。

`[profile.dev] debug = "line-tables-only"` 试过，没有采纳：同一负载 1.53–1.77 s，基线 1.49–1.58 s，二进制从 131.6 MB 到 125.5 MB。macOS 的 `split-debuginfo` 默认是 `unpacked`，收窄 debug 几乎不影响链接，却换掉调试器里的变量。

## Rust 应用

Rust 不能在进程里换掉代码。`TypeId` 跨编译不稳定，`RuntimeProgram` 不是 object-safe，事件闭包指向正在运行的代码，macOS 上 `dlclose` 卸不掉带 Objective-C 元数据或 TLS 的镜像。诚实的做法是重新执行。完整例子在 `crates/nana-ui-dev/examples/l3-dev-entry.rs`。

```rust
nana_ui_dev::run_with_restart::<MyProgram>(settings, Path::new("target/nana-dev-handoff"))?;
```

`initialize` 里：

```rust
let watcher = nana_ui_dev::watch_and_rebuild(
    &DevConfig::new_rust("src"),
    RebuildCommand::cargo_package("my-app"),
    context,
)?;
```

消息要实现 `From<DevSignal>`。`Rebuilt` 时交出几何和一段框架不解释的状态，然后退出。Unix 用 `exec`，Windows 先 spawn 再退出。`BuildFailed` 带着诊断文本。

```rust
nana_ui_dev::request_relaunch(DevHandoff { /* 几何 */ }.with_state(&self.session));
```

下一次 `initialize` 取回来。类型对不上时 `state_as` 返回 `None`。`restored_handoff()` 只消费一次，在 `run_with_restart` 下面可以从 `initialize` 里调。

```rust
let session = nana_ui_dev::restored_handoff()
    .and_then(|handoff| handoff.state_as::<Session>())
    .unwrap_or_default();
```

只改 `.vue` 里的静态文字或静态 `<style>` 声明时不必重启。构建脚本在 debug 下打开热模式，dev 入口用 `templates` feature 的 `watch_templates`（消息类型实现 `From<DevSignal> + From<TemplateText> + From<TemplateStyle>`，或直接用 `watch_template_events`）。`runtime` 字符串要和 `Compiler::new` 的那一个相同。形状变了（结构、绑定、脚本、文字跨行，或新增 `transition` / `@keyframes` / 组合器 / 交互伪类），或者改的不是 `.vue`，就照常重建。带插值的文字和属性值不在替换表里。

```rust
let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
nana_ui_sfc::Compiler::new("::nana_ui::runtime").hot(debug).build("views")?;

let watcher = nana_ui_dev::watch_templates(
    &DevConfig::new_rust("src"),
    "views",
    "::nana_ui::runtime",
    RebuildCommand::cargo_package("my-app"),
    context,
)?;
```

`update` 里对 `TemplateText` 和 `TemplateStyle` 分别调用 `apply()`。节点、焦点、滚动和状态不动，下一帧换成新文字或新样式。

## 还没覆盖的

L3 没有默认的「重读数据文件」钩子。要在进程里换树，先 `AppContext::remove_view(root)` 再 `mount_view_root`。连续两次挂载会叠出两棵树。

无头看结果时，给 dev bin 加上 `headless`：

```bash
cargo build -p my-app --bin dev --features dev,headless
```

同一份 JSON 可以问无障碍树、截图和点击。

步骤、测量条件和为什么不能热替换，写在 [热重载](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/hot-reload.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/store">
    <p class="next-step-link">Store</p>
    <p class="next-step-caption">重载留不住的那份嵌套状态，平时怎么按字段更新。</p>
  </a>
  <a class="next-step" href="/guide/scaling/sfc">
    <p class="next-step-link">.vue 方言</p>
    <p class="next-step-caption">热模式要在构建脚本里打开。</p>
  </a>
  <a class="next-step" href="/guide/scaling/packaging">
    <p class="next-step-link">打包</p>
    <p class="next-step-caption">dev 监听不会进 dist 产物。</p>
  </a>
</div>
