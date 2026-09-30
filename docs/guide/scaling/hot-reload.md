# 热重载

你改完界面之后，能多快再看到它，取决于改的是哪一层。热重载住在 `nana-ui-dev`。这个 crate 在没有 `debug_assertions` 时拒绝编译。`[profile.dist]` 继承 `release`，所以产品依赖一旦带上它，构建直接失败。没有逃生开关。

| 改什么 | 发生什么 | 量级 |
| --- | --- | --- |
| 注册过的 Vue `.css` | 节点、焦点、滚动、GPU 槽和进行中的动画都留着 | < 50 ms |
| Vue 打包产物 | 窗口和 wgpu `Device` / `Queue` / `Surface` 留着，只重挂 UI 树 | ~200 ms |
| Rust 源码 | 重编译，交接窗口几何和状态，再重新执行 | 1.5–3.6 s，链接占大头 |

这是 reload，不是 Vite 的组件热替换。Vue 里的 `ref` 和 store 不跨这次重载留下来。

macOS / Apple Silicon、默认 dev profile、已经预热的增量构建，`cargo build -p component-gallery`：应用 crate（1.8 万行）1.5–1.6 s，含重新链接 131 MB 的 debug 二进制；改过 `nana-ui-runtime`（12.7 万行）后再建应用是 3.6 s，连带重编 `nana-ui-scene` 和 `nana-ui`。Windows 的链接更慢。在你自己的机器上用 `cargo build -p <你的包> --timings` 量一次。窗口加 GPU 重建的 0.4–1.5 s 没有重新测量。

`[profile.dev] debug = "line-tables-only"` 试过，没有采纳：同一负载 1.53–1.77 s，基线 1.49–1.58 s，二进制从 131.6 MB 到 125.5 MB。macOS 的 `split-debuginfo` 默认是 `unpacked`，收窄 debug 几乎不影响链接，却换掉调试器里的变量。

## Vue 应用

release 入口不动。另开一个 dev bin，feature 带上 `nana-ui-dev` 的 `vue`。`watch_and_reload` 在这个 feature 后面。

```toml
[[bin]]
name = "dev"
required-features = ["dev"]

[features]
dev = ["dep:nana-ui-dev", "nana-ui-dev/vue"]
```

```rust
use nana_js_engine::HostApiRegistry;
use nana_js_v8::V8Engine;
use nana_ui::WindowDescriptor;
use nana_ui_dev::DevConfig;
use nana_ui_vue::VueRuntimeProgram;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dev = DevConfig::new("web/dist/app.iife.js")
        .watch("web/dist")
        .css("web/dist/app.css");
    let artifact = nana_ui_dev::artifact_from_path(
        dev.artifact().expect("artifact"),
        dev.jail_root(),
    )?;
    VueRuntimeProgram::run_dev(
        WindowDescriptor::new("My App"),
        V8Engine::new,
        artifact,
        HostApiRegistry::new(),
    )?;
    Ok(())
}
```

`V8Engine::new` 是工厂。每次重载在新的 isolate 里跑。在 `initialize` 里把监听留住，drop 掉就停：

```rust
let watcher = nana_ui_dev::watch_and_reload(&dev, context)?;
```

程序的消息要实现 `From<DevReload>`。`VueRuntimeProgram` 的消息是 `VueMessage`，重载是 `VueMessage::Dev`。监听的是打包产物，旁边要跑着 `vite build --watch`。

Vite 的 IIFE 默认把 CSS 内联进 JS。那种改动会改到 bundle，走全量重载。要吃到小于 50 ms 的快路径，dev 构建把 CSS 单独产出，并用 `DevConfig::css` 注册。`injectStylesheet(css, href)` 的第二个参数也是这份 key，同一个 key 再注入是替换。

全量重载按这个顺序：向 `__nanaDevSaveState` 要状态；关掉主窗口以外的窗口；`clearMount` 拆树；清掉样式表、bridge 事件、定时器、rAF、在途 fetch 和 socket；先关掉旧引擎再造新的；重新注册 host API、求值、重绑；恢复几何和主题。第 6 步之前不调用 `bind_host_gpu`。GPU 绑在 `VueHost` 上，设备、队列、Surface 和宿主纹理留在原地。求值失败时放回上一份能跑的 artifact。

原地重建、不换 `RuntimeDocument`，是因为 `AccessibilityProjector` 的 generation 只增不减，`HostedAccessibility` 每窗口只建一次。换一棵新的 `UiWorld` 会让之后的无障碍更新被永久拒绝。

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

只改 `.vue` 里的静态文字时不必重启。构建脚本在 debug 下打开热模式，dev 入口用 `templates` feature 的 `watch_templates`。`runtime` 字符串要和 `Compiler::new` 的那一个相同。形状变了（结构、绑定、脚本，或文字跨行），或者改的不是 `.vue`，就照常重建。带插值的文字和属性值不在替换表里。

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

`update` 里对 `TemplateText` 调用 `apply()`。节点、焦点、滚动和状态不动，下一帧换成新文字。

## 还没覆盖的

`localStorage` 和 `location` 会留着。`documentElement` 的 dataset 和内联 style 会清掉，主题在重载后重新注入。没有 Vue 组件级 HMR：求值的是一段扁平脚本。

L3 没有默认的「重读数据文件」钩子。要在进程里换树，先 `AppContext::remove_view(root)` 再 `mount_view_root`。连续两次挂载会叠出两棵树。重载不能发生在 JS 还在栈上的时候，会推迟到下一次 `update`。

无头看结果时，给 dev bin 加上 `headless`：

```bash
cargo build -p my-app --bin dev --features dev,headless
```

同一份 JSON 可以问无障碍树、截图和点击。Vue 会话还可以 `{"cmd":"reload","js":"web/dist/app.iife.js"}` 或 `"css"`。编译进二进制的 Rust 应用不走 `nana-runtime-agent` 的这条 reload。

步骤、测量条件和为什么不能热替换，写在 [热重载](../../reference/hot-reload.md)。

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
