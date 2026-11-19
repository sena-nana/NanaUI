---
title: 代码阅读指南
---

# 代码阅读指南

本章回答一个实际问题：**面对 `crates/` 下 25 个 crate（24 个工作区成员 + 独立 `nana-icons-tabler`）的仓库，应该从哪个文件开始读、按什么顺序读、怎么跑最小验证、读到哪一步算真正掌握了主干**。

## 第一步：按顺序读完这五份短文档

在一头扎进几千行的源码文件之前，先按以下顺序花 20 分钟建立全局坐标系：

| 顺序 | 文档路径 | 为你建立什么认知 |
| --- | --- | --- |
| 1 | `AGENTS.md`（仓库根目录） | 全仓架构合同的最高浓缩：唯一产品路径、支撑边界与九条非协商红线 |
| 2 | [`docs/reference/how-it-works.md`](/reference/how-it-works) | 核心运行模型：一棵保留树、宿主拥有 GPU、一帧的 8 步生命周期 |
| 3 | [`docs/reference/architecture.md`](/reference/architecture) | Crate 边界、Motion IR 分流表与全局对象所有权表 |
| 4 | [模块与分层](/learn/modules-and-layers) | 25 个 crate 的依赖深度层级与 58 条工作区成员普通依赖边实读 |
| 5 | [合同与不变量](/learn/contracts-and-invariants) | 九条非协商合同的设计因果与只读验证命令 |

`AGENTS.md` 值得在阅读源码时随时对照：仓库里几乎所有「为什么这段代码要分两步、为什么不直接传句柄」的设计，最终都对应其中一条非协商合同。

## 仓库目录地形图

```text
NanaUI/
├── Cargo.toml                  Workspace 根清单：24 个工作区成员 crate + tools + platform + 5 个示例
├── AGENTS.md                   架构合同与开发守则（最重要的一份根文档）
├── crates/                     全部 25 个框架 crate（24 成员 + nana-icons-tabler，全表见 /learn/coverage-matrix）
├── examples/                   5 个工作区示例包（component-gallery、native-rhi-probe、
│                               runtime-host-fixture、package-fixture、reactive-sfc）
├── crates/nana-ui/examples/    26 个单文件最小专题示例（计数器、GPU 视图、透明窗、Splash、无障碍等）
├── docs/                       VitePress 文档站源码（含 :::api 双写法校验脚本）
├── platform/android/           Android 实验性 GameActivity 宿主
├── tools/css-parity/           CSS 子集布局一致性测试夹具与比对工具
├── perf/                       性能基准场景与门禁定义
└── scripts/                    架构边界与合规检查脚本（check-engine-boundary.py 等）
```

## 三条推荐源码阅读路线

### 路线一：一帧的渲染主线（状态 → 像素）

1. 先读 [端到端数据流](/learn/flows) 的「流程一」与 [框架如何运行](/reference/how-it-works) 的「一帧」章节；
2. 读 [Scene 提取](/learn/scene-extraction)，对应源码：`crates/nana-ui-scene/src/runtime_document.rs:109`（`RuntimeDocument::flush`）与 `crates/nana-ui-scene/src/scene.rs:1672`（`UiScene::apply_delta`）；
3. 读 [GPU 与帧生命周期](/learn/gpu-frame)，对应源码：`crates/nana-gpu/src/context.rs`（`GpuContext`）、`crates/nana-gpu/src/frame.rs`（`FrameContext`）、`crates/nana-ui/src/scene_paint/mod.rs`（`SceneWgpuPainter`）以及 `crates/nana-ui/src/scene_host/present.rs:114`（宿主 `redraw` 35 步全序）。

### 路线二：保留树与增量布局主线（变异 → 几何）

1. 读 [保留树：UiWorld](/learn/runtime-tree)，从 `crates/nana-ui-runtime/src/mutation.rs:15` 的 `UiMutation` 枚举开始，看清保留树允许的全部操作词表，再到 `world/mutation.rs:2915` 看 `UiWorld::commit`；
2. 读 [布局引擎](/learn/runtime-layout)，从 `crates/nana-ui-runtime/src/layout_engine.rs:89`（`RuntimeLayoutEngine`）与 `layout_frontier.rs`（依赖图与增量种子传播）看增量布局如何只重排受影响的子树。

### 路线三：输入路由主线（原生事件 → 回调）

1. 读 [输入链路](/learn/runtime-input)；
2. 打开 `crates/nana-ui-input/src/canonical.rs:44`（`CanonicalInputEvent` 与 `InputEndpoint`），然后直接跳到全仓唯一的路由入口 `crates/nana-ui-runtime/src/framework/input.rs:397`（`AppContext::route_input`）与四条分发链 `framework/input/dispatch.rs`。

## 运行最小示例亲自观察

`crates/nana-ui/examples/*.rs` 下的每个单文件示例都只聚焦演示一个特性，是极佳的动态阅读入口：

```bash
# 1. 全量内置控件与工作区画廊
cargo run -p component-gallery

# 2. 最小应用壳计数器（ApplicationState 模式）
cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts

# 3. 声明式视图计数器（view! 宏模式）
cargo run -p nana-ui --example reactive-counter --features hosted,bundled-fonts,view-macro

# 4. 实时 GPU 画面与普通控件同窗混排
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop,runtime-wgsl

# 5. 两阶段启动与原生 Early Splash 探针
cargo run -p nana-ui --example startup-splash --features hosted,bundled-fonts -- --probe
```

## 调试、诊断与边界检查工具

- **框架诊断与性能指标**：帧路径严禁打日志或做文件 I/O，排查帧耗时、缓存命中率或丢帧时请查看 `nana-diagnostics` 的静态指标与 `.nlog` 导出（见 [诊断与应用路径](/reference/diagnostics) 与 [诊断码](/reference/errors)）。
- **静态架构守门脚本**：修改代码后可随时运行以下只读脚本验证是否触碰了架构红线：
  ```bash
  python3 scripts/check-engine-boundary.py
  python3 scripts/check-api-convergence.py
  python3 scripts/check-package-boundary.py
  python3 scripts/audit-framework-strings.py
  ```
- **提交前卫生检查**：按照 `AGENTS.md` 规范，提交代码前运行 `cargo fmt --all -- --check`；修改文档后运行 `cd docs && npm run build` 与 `git diff --check`。

## 三个自测问题检验理解程度

读完本板块后，可以用以下三个问题做快速自检（答案均在对应章节中）：

1. 为什么在按钮点击回调里不应该调用 `mount_view_root` 重新渲染整页？一次点击从操作系统事件到触发 `on_activate` 依次经过了哪几个 crate？（见 [L3 建树](/reference/l3-authoring) 与 [输入链路](/learn/runtime-input)）
2. 为什么一块实时 GPU 画面（`GpuTextureView`）既能被父容器的圆角裁切，又能被后方的下拉菜单遮挡，还能响应鼠标点击？（见 [Scene 提取](/learn/scene-extraction) 与 [GPU 与帧生命周期](/learn/gpu-frame)）
3. 为什么 `nana-ui-runtime` 的依赖表里根本没有 `nana-ui-css`，但无论是 Rust 的 `view!` 还是 `.vue` 组件都能使用 CSS 样式？（见 [CSS 引擎](/learn/css-engine) 与 [作者工具链](/learn/toolchain)）

## 接着读

- [端到端数据流](/learn/flows)：四条完整链路串起全部子系统
- [25 crate 覆盖矩阵](/learn/coverage-matrix)：全部 25 个 crate 的四要素速查表
- [证据清单](/learn/evidence-checklist)：用只读命令亲手核验每一条架构断言
