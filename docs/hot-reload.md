# 开发期热重载

给要**写应用**的人：改了 UI 或逻辑之后，怎么用最小代价看到新界面。

结论先说：**Vue 层可以做到接近 Tauri 的体验，而且更好一点；Rust L3 层做不到进程内热替换。** 两条路径的天花板不同，原因在 [§Rust L3 为什么不行](#rust-l3-为什么不行)。

| 改什么 | 发生什么 | 量级 |
| --- | --- | --- |
| Vue：注册过的 `.css` | 零拆除。节点 id、焦点、滚动位置、GPU 槽位、进行中的动画全部保留 | < 50 ms |
| Vue：`.vue` / `.ts`（即打包产物变了） | 窗口、位置、wgpu `Device`/`Queue`/`Surface`、宿主纹理全部保留；只重挂 UI 树 | ~200 ms |
| Rust L3：源码 | 重编译 → 交接窗口几何与状态 → re-exec。窗口闪一次 | 1.5–4 s（实测，见下表），**链接主导** |
| Rust L3：数据文件 | 应用自己重读并重建（框架不提供默认钩子，见 [§已知空缺](#已知空缺)） | < 100 ms |

Rust 一行的实测（macOS / Apple Silicon，默认 dev profile，热增量，`cargo build -p component-gallery`）：

| 改哪一层 | 重建耗时 | 说明 |
| --- | ---: | --- |
| 应用自己的 crate（`component-gallery`，1.8 万行） | **1.5–1.6 s** | 三次连续测量；含重新链接 131 MB 的 debug 二进制 |
| `nana-ui-runtime`（12.7 万行）后再建应用 | **3.6 s** | 连带重编 `nana-ui-scene` / `nana-ui` |

这些数字明显低于本文早先给的 4–10 s / 30–90 s。差异来自平台与冷热：上面是 macOS 上已经预热的增量构建，Windows 的链接更慢。**在自己的机器上量一次再决定要不要为此调整架构**，`cargo build -p <你的包> --timings` 就够。窗口 + GPU 重建的 0.4–1.5 s 没有重新测量。

**试过但没有采纳：`[profile.dev] debug = "line-tables-only"`。** 同一台机器同一负载，重建 1.53–1.77 s（基线 1.49–1.58 s），二进制 131.6 → 125.5 MB。macOS 的 `split-debuginfo` 默认就是 `unpacked`，DWARF 留在 `.o` 里、链接器只写一张 debug map，所以收窄 debug 级别在这里几乎不影响链接，却换掉了调试器里的变量信息。Linux / Windows 把 debuginfo 嵌进二进制，那里的账可能不一样——要加就先在目标平台上量。

Vue 路径比浏览器刷新更好的地方：Tauri 的 devserver 刷新是整个 webview reload，这里 **GPU 设备从头到尾没有感知**。

不好的地方也说清楚：**这是 reload，不是 Vite HMR。** Vue 组件里的 `ref`/store 不跨重载存活。

---

## 开起来

热重载全部住在 `nana-ui-dev`。这个 crate **在没有 `debug_assertions` 时拒绝编译**，而 `[profile.dist]` 继承 `release`，所以产品 crate 一旦误依赖它会直接构建失败，不会把文件监听打进用户手里。没有逃生开关。

### Vue 应用

release 入口（`src/main.rs`）一行不改。新增一个 dev bin：

```toml
# Cargo.toml
[[bin]]
name = "dev"
required-features = ["dev"]

[features]
dev = ["dep:nana-ui-dev"]
```

```rust
// src/bin/dev.rs
use nana_js_v8::V8Engine;
use nana_ui_dev::DevConfig;
use nana_ui_vue::VueRuntimeProgram;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dev = DevConfig::new("web/dist/app.iife.js") // artifact，同时是 jail 根
        .watch("web/dist")
        .css("web/dist/app.css");                    // 显式开启 CSS 快路径
    let artifact = nana_ui_dev::artifact_from_path(
        dev.artifact().expect("artifact"),
        dev.jail_root(),
    )?;
    VueRuntimeProgram::run_dev(
        RuntimeWindowSettings::new("My App"),
        V8Engine::new,   // 工厂，不是实例：每次重载跑在全新 isolate 里
        artifact,
        HostApiRegistry::new(),
    )?;
    Ok(())
}
```

程序的 `initialize` 里起监听（返回值要活着，drop 掉监听就停）：

```rust
let watcher = nana_ui_dev::watch_and_reload(&dev, context)?;
```

监听线程读文件(带 jail、体积上限、半截保存检查),然后 `context.dispatch(VueMessage::Dev(..))`;程序在两帧之间应用它。

> `VueRuntimeProgram` 的消息类型是 `VueMessage`,把「宿主命令」和「控件输入」分开。重载是发给宿主的,不是发给某个控件的 —— 它没有 widget id,也没有对应的 JS 事件名。

**注意**：监听的是**打包产物**，不是源码。宿主读的是 bundle，改 `.vue` 只有在打包器把它变成新字节之后才有意义 —— 所以旁边要跑着 `vite build --watch`。

### Rust L3 应用

完整可编译示例在 [`crates/nana-ui-dev/examples/l3-dev-entry.rs`](../crates/nana-ui-dev/examples/l3-dev-entry.rs)。骨架：

```rust
// main
nana_ui_dev::run_with_restart::<MyProgram>(settings, Path::new("target/nana-dev-handoff"))?;

// RuntimeProgram::initialize —— 返回的 watcher 存进 program，drop 即停
let watcher = nana_ui_dev::watch_and_rebuild(
    &DevConfig::new_rust("src"),
    RebuildCommand::cargo_package("my-app"),
    context,
)?;

// RuntimeProgram::update
Message::Dev(DevSignal::Rebuilt) => {
    nana_ui_dev::request_relaunch(
        DevHandoff { /* 几何 */ }.with_state(&self.session),
    );
    RuntimeProgramUpdate { exit: true, ..Default::default() }
}
Message::Dev(DevSignal::BuildFailed(diagnostics)) => { eprintln!("{diagnostics}"); .. }
```

`cargo build` 跑在监听线程上，不阻塞事件循环。构建成功后进程交接几何与一段**框架不解释**的状态串，然后 re-exec（Unix 用 `exec`，Windows 用 spawn + 退出）。

### 让重启不丢状态

状态串框架不解释，但不必手写 JSON 往返：`DevHandoff::with_state(&T)` 存，`state_as::<T>()` 取，`T` 是你自己的 `Serialize + Deserialize`。

值得存的是**视图状态**——当前在哪一页、滚到哪、选中了谁。几何由 `run_with_restart` 恢复，看到的东西由这段决定；两者都恢复，一次重建才像刷新而不是重启。

```rust
// initialize —— 第一次启动，或状态类型自上次构建以来变了，都回到默认值
let session = nana_ui_dev::restored_handoff()
    .and_then(|handoff| handoff.state_as::<Session>())
    .unwrap_or_default();
```

`state_as` 在类型对不上时返回 `None` 而不是报错：改自己正在恢复的那个类型，正是开发循环里最常发生的事，不该让它变成一次失败的重启。

`restored_handoff()` 消费一次即止，且在 `run_with_restart` 下可以安全地从 `initialize` 里调——入口点会把它读到的那份**留给**程序，而不是让程序去重读一个已经被删掉的文件。

---

## CSS 快路径的前提

只有**注册过**的样式表走快路径。`DevConfig::css(path)` 注册，或者应用自己用 `injectStylesheet(css, href)` 的第二个参数 —— `href` 就是 key，白得一个。

**Vite 的 IIFE 默认把 CSS 内联进 JS bundle。** 那种情况下改 `.css` 会改到 bundle，走的是全量重载而不是快路径。要吃到快路径，dev 构建必须把 CSS 单独产出。

同一个 key 重复 inject 是替换而非叠加，所以一个每次 mount 都注入样式表的应用不会随重载慢慢变胖。

---

## 一次全量重载做了什么

顺序是有讲究的，改动前先读这一节。实现在 [`crates/nana-ui-vue/src/dev.rs`](../crates/nana-ui-vue/src/dev.rs)。

1. 向 `globalThis.__nanaDevSaveState` 要一段状态串（应用可选提供）。
2. 关掉除主窗口外的所有窗口。**重载是整 runtime 粒度**：一个 isolate、一张模块图被所有 Vue 窗口共享，单窗口重载在这个结构下不可表达。辅窗由重新求值的 artifact 自己打开回来。
3. `clearMount` 拆树。这是已有的生产 op：逐个卸载原生子树、从文档移除（保留 `html`/`body` 脚手架）、丢 layout box、从 bridge 反注册。焦点、IME、指针捕获、进行中的动画由 `UiWorld` 在同一次 despawn 里清掉。
4. 清掉比节点活得久的东西：作者样式表、排队的 bridge 事件、定时器、rAF、在途 fetch、开着的 socket、进程级滚动偏移。**漏清一项只会在二十次重载之后才显形**，所以有专门的回归测试盯着计数。
5. **先 shutdown 旧引擎，再造新引擎。** V8 创建 isolate 时进入、drop 时退出，且要求严格 LIFO；先造后丢会在第一次重载就 abort 进程。
6. 重新 initialize：重注册整套 host API、求值 artifact、为每个活窗口重绑 event bridge。
7. 恢复几何与主题。

**第 6 步之前不调 `bind_host_gpu`**，也不该调 —— GPU 绑在 `VueHost` 上而不是 engine 上，所以 `Device`、`Queue`、`Surface` 和每一张宿主纹理全程不动。这一行就是本方案和"重启进程"的全部差别。

求值失败（改坏了语法）会把**上一份能跑的 artifact** 放回去，并通过诊断通道抛出异常。白屏且无解释是最糟也最容易发生的开发体验。

### 为什么原地重建，而不是换掉整个 document

看着更简单的做法是造一个新 `RuntimeDocument` 塞进活窗口。那会**静默打断读屏**：`AccessibilityProjector` 用一个单调 `generation` 守卫更新，而 `HostedAccessibility` 每窗口只建一次、之后不再重建。全新的 `UiWorld` 把 generation 归零、`StableNodeId` 从 1 重排，于是投影器永久拒绝之后的每一次更新。

在同一个 `UiWorld` 里原地重建两个问题都没有：节点 id 永久退休不复用，generation 只增不减。有一条测试专门盯着这件事。

### 为什么换 isolate，而不是原地重新求值

Rust 侧其实没问题：`register_host_api` 是替换而非追加，event bridge 按名字重新解析全局。问题在 JS 侧 —— Web API shim 自带 `installed` 守卫，于是 `window`/`document` 的监听器列表、node wrapper 缓存、窗口注册表全部存活，每次重载叠一代打进死闭包的监听器；Vue 自己的组件注册表也活着，改个组件名会静默解析到旧的。给 shim 做一套 reset 比丢掉整个堆更脆。

新建 isolate + 重新求值一个 1–3 MB 的 IIFE 在几十毫秒量级，远在"手感即时"预算内。

---

## Rust L3 为什么不行

不是难，是有几条在不重构框架的前提下无解：

- **`TypeId` 是树协调键，且跨编译不稳定。** `UiBuilder::child` 比较 `TypeId::of::<C>()`，`remove_view` 按它 downcast。重新编译的 dylib 对同一个类型给出不同 id，于是每个 keyed child 都失配、整棵树照样重建，而 `remove_view` 直接失败。这一条单独就足以否掉"保住树、只换代码"。
- **`RuntimeProgram` 不是 object-safe**（`with_document` 是泛型方法，进不了 vtable），跨 FFI 要手写一套 C-ABI 影子层 —— 正是 `AGENTS.md` 禁止的第二套实例化 ABI。
- **事件处理闭包是指向 dylib 的代码指针，而宿主持有它们。** 要安全卸载就得先拆完整棵树，那 dylib 相比重建没带来任何东西。
- **`dlclose` 在关键平台不可靠。** macOS 不会卸载含 Objective-C 元数据或 TLS 的镜像，本仓库两样都有。
- **静态量会重复。** dylib 若重新链接 `wgpu` 或 `v8` 就会出现两套 `Instance` / 两个 V8 platform。

所以 L3 的诚实答案是重启，并且要说清楚代价的构成：这一秒几秒里**链接占大头**，窗口和 GPU 重建只有 0.4–1.5 s。真实量级见开头那张实测表——macOS 上应用一行 1.5 s、改 Runtime 后 3.6 s；Windows 的链接更慢，给 dev profile 配 `lld` 在那里收益最大。

改 `nana-ui-runtime`（12.7 万行）会连带重编 `nana-ui-scene` / `nana-ui`，比改应用贵一倍多，但仍在个位数秒。热重载帮你**基于**框架开发，不帮你开发框架本身。

---

## 无头验证

重建完还得看一眼才知道改动落没落。开窗口不是唯一办法，L3 尤其不该是——每次重建都要重新点回出问题的那个界面，比等编译还贵。

### Rust L3：同一个 dev bin 兼作无头会话

`nana-ui-dev` 的 `headless` feature 引入 `nana-ui-devtools` 的 Vue-free `runtime-agent` 层（离屏像素 + JSON 协议，不带 V8）。把建树抽成一个函数，两个入口共用：

```rust
fn build_document(session: &Session) -> RuntimeDocument { /* build / child / on */ }
```

窗口路径用它，无头路径也用它——于是截图是关于**真实应用**的证据，不是关于一个长得像它的 fixture 的。`child` 的 key 就是 Agent 的 `agent_path` 选择器。

```bash
cargo build -p my-app --bin dev --features dev,headless
```

```bash
printf '%s\n' \
  '{"id":1,"cmd":"a11y"}' \
  '{"id":2,"cmd":"screenshot","path":"target/agent/after.png"}' \
  '{"id":3,"cmd":"click","agent_path":"open"}' \
  | ./target/debug/dev --stdio --width 320 --height 200 --theme dark
```

没带 `--stdio` / `--screenshot` / `--a11y` 时二进制照常进窗口循环，所以一个 bin 两用，不必维护两份。完整写法见 [`crates/nana-ui-dev/examples/l3-dev-entry.rs`](../crates/nana-ui-dev/examples/l3-dev-entry.rs)，命令与选择器全集见 [`$nanaui-agent-debug`](../.agents/skills/nanaui-agent-debug/SKILL.md)。

### Vue：直接 reload

Agent 会话支持 `reload`，不需要窗口也不需要 GPU：

```json
{"cmd":"reload","js":"web/dist/app.iife.js"}
{"cmd":"reload","css":"web/dist/app.css"}
```

`nana-runtime-agent`（Vue-free 层）会明确回答它做不到 —— 它的应用是编译进去的 Rust。要驱动自己的 Rust 应用，走上面那条路，不是这个内置 fixture 二进制。

---

## 已知空缺

- **Vue 组件状态不跨 JS 重载存活。** 真 HMR 需要应用打包器侧的 `import.meta.hot` 模块图记账加宿主侧模块注册表；这里 V8 求值的是一段扁平 classic script，没有模块图可打补丁。
- **`localStorage` 和 `location` 有意保留**（符合浏览器刷新语义）；`documentElement` 的 dataset 与内联 style 有意清掉（那是上一版应用写的，新版不该继承），主题在重载后立即重新注入。
- **L3 没有 `rebuild` 钩子。** 数据驱动的应用要"进程不动、重读数据文件、重建树"，正确做法是先 `AppContext::remove_view(root)` 再 `build` —— 连续两次 `build` 会叠出两棵互相覆盖的树，因为 root 层的 keyed child 不会被复用也不会被回收。框架没有提供默认实现：唯一正确的公开 despawn 需要类型化的 `Entity<V>`，应用自己持有 root 才能调，而一个默认实现只能是错的。应用可以自己写这两行。
- **重载不能在 JS 在栈上时发生**，推迟到下一次 `RuntimeProgram::update`。
