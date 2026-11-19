# 合同与不变量

NanaUI 有一批**非协商架构合同（Non-negotiable Contracts）**：它们不是代码风格建议——一旦违反其中任何一条，框架就会长出第二套权威、第二棵树或第二个 GPU 设备。本章将九条核心合同逐一列出，每条配齐「是什么」「为什么」与「违反后果」，并给出可在当前仓库直接复跑的验证读数。

面向框架维护者的合同参考全文见 [架构合同](/reference/architecture)；本章侧重为架构学习者剖析每一条合同背后的设计因果。

## 合同是如何被强制执行的

框架通过三道防线守住这些不变量：

1. **Cargo 依赖图分层**：通过单向依赖让下层 crate 在编译期根本看不见上层类型（详见 [模块与分层](/learn/modules-and-layers)）。
2. **自动化边界守门脚本**：CI 每次必跑 `scripts/check-engine-boundary.py`、`scripts/check-api-convergence.py`、`scripts/check-package-boundary.py` 与 `scripts/audit-framework-strings.py`。
3. **评审准则**：仓库根目录的 `AGENTS.md` 将这九条合同列为不可妥协的评审基线。

## 合同一：产品路径单向流动，只有一棵权威保留树

```text
UiWorld → ExtractedNode → UiScene → SceneWgpuPainter
```

- **是什么**：树拓扑、计算样式、未滚动布局、命中测试、焦点、IME 与无障碍的唯一权威是 `UiWorld`（`nana-ui-runtime`）；绘制图的唯一权威是 `UiScene`（`nana-ui-scene`）；上屏绘制器只有 `SceneWgpuPainter`（`nana-ui`）。布局输出 `LayoutResult` 只是投影，不构成第二套树权威；滚动偏移绝不写回 Runtime 布局盒。
- **为什么**：一旦引入「第二棵树」，布局、命中、动画、焦点与无障碍就必须跨两棵树双向同步，不仅带来固定开销，而且在异步输入和过渡动画下永远无法完全对齐。
- **违反后果**：变更经 `MutationQueue` 在帧边界原子提交的保证被打破；查询到的几何盒与真实命中/绘制区域分叉。
- **核查方式**：`cargo metadata` 普通依赖图严格前向无环（见 [模块与分层](/learn/modules-and-layers)）；两条作者入口如何汇入同一棵树见 [作者入口](/learn/authoring-paths)。

## 合同二：CSS 只有一个引擎（`nana-ui-css`）

- **是什么**：样式表的词法分析、语法解析、选择器匹配与级联排序只在 `nana-ui-css` 中实现。`nana-ui-core`、`nana-ui-runtime`、`nana-ui-scene`、`nana-ui` 四个核心 crate 不含任何样式表解析代码，只消费级联产物 Style Model（`LayoutStyle`）。
- **为什么**：CSS 级联语义（来源次序、特异性三元组、`@layer`、`!important`、`@container` 区间分桶）极为精细，两套实现必然导致同一份样式在 `view!`、`.vue` 与 `tools/css-parity` 之间算出不同的布局。
- **违反后果**：`view!` 与 `.vue` 的样式表现不一致，`tools/css-parity` 基线失守。
- **当前仓库实跑读数**：
  - 在 `nana-ui-runtime` 中搜索 `specificity | Selector | cascade | stylesheet | CssParser | tokenizer`：全部命中均为注释、测试事件名或 `view/style.rs` 对构建期编译产物 `Sheet` / `StylePatch` 的消费（其模块头明确声明："No CSS is parsed and no selector is matched at run time, and this crate has no CSS in it"）。
  - 在 `nana-ui-core` 中搜索：仅有 `SemanticColorRole::from_css_token_name`（`style_model.rs:341`，将 `--nana-*` 变量名映射为语义角色）与单值关键字解析，无样式表解析器。
  - 依赖图实读：`nana-ui-css` 在 `crates/` 内的直接消费方只有 `nana-ui-view-codegen`（构建期）。

## 合同三：输入路由只有一个消费方（`AppContext::route_input`）

- **是什么**：`nana-ui-input` 拥有规范输入事件（`CanonicalInputEvent`）、端点（`InputEndpoint`）、序列器（`InputSequencer`）与宿主服务（`HostServices`）合同；各平台宿主将原生事件降维为规范事件，而**路由器的唯一实现位于 `nana-ui-runtime`**——即每个 `AppContext` 的 `route_input`（`crates/nana-ui-runtime/src/framework/input.rs:397`）。
- **为什么**：输入事件的序列号、端点代际（generation）、乱序/重复丢弃以及按键与文本抑制（`CommittedText`）必须在一处集中裁决，否则指针捕获、焦点遍历与输入法组字的组合行为无法保持确定性。
- **违反后果**：两条路由路径对同一串事件给出不同的「按下 / 释放 / 激活」判定。
- **当前仓库实跑读数**：
  - `pub fn route_input` 全仓仅定义于 `crates/nana-ui-runtime/src/framework/input.rs:397`（无头测试变体 `HeadlessInput::route_shaped` 位于同模块的 `input/headless.rs:119`，内部仍调用 `context.route_input`）。
  - `nana-ui`（`scene_host/host_services.rs`）与 `nana-ui-devtools`（`agent/runtime.rs:120`）全部只是将事件降维或转发给 `AppContext::route_input`，不存在第二套路由。

## 合同四：全进程单一 GPU 设备与队列，只有一份 wgpu

- **是什么**：WGPU 是唯一正式图形后端，依赖图里只有一份 wgpu：NanaUI 自己的 wgpu fork（`third_party/wgpu`，沿用上游包名与版本 30.0.1）。以 git 依赖本仓库就会带上它；应用若还用到 egui-wgpu 之类依赖 crates.io wgpu 的库，需在自己的工作区用 `[patch.crates-io]` 把 wgpu 与 naga 指到本仓库的同一个 rev（写法见 `third_party/wgpu/README.zh-CN.md`）。严禁创建第二个 `Device` / `Queue`，严禁在产品呈现路径上执行 CPU 回读（readback）。
- **为什么**：两套设备意味着两套独立的提交时序与围栏，实时画面与 UI 控件无法在同一帧、同一次 `present` 中严格对齐；而 CPU 回读会直接打断 GPU 异步流水线。
- **违反后果**：窗口缩放或高刷动画下画面撕裂、延迟一帧或跨设备句柄崩溃。
- **核查方式**：`python3 scripts/check-engine-boundary.py` 强制检查公开签名——`wgpu::*` 只能在开启 `wgpu-interop` feature 时出现在白名单允许的四个 crate（`nana-gpu`、`nana-frame-exchange`、`nana-ui`、`nana-ui-devtools`）中，框架内部源码则通过私有模块 `nana_gpu::__framework` 访问；同一脚本还要求依赖图里的 wgpu 与 naga 全部来自 `third_party/wgpu`。

## 合同五：原始窗口句柄绝不进入普通控件

- **是什么**：`HWND`、`NSWindow` 等平台原始窗口句柄严格封闭在 `nana-window` 与 `nana-ui-platform` 边界内，普通控件在编译期和运行期都拿不到句柄。系统材质申请失败时，必须如实返回**实际生效的结果（`MaterialOutcome`）**或显式回退原因，绝不静默伪造成功或擅自改试另一种材质家族。
- **为什么**：一旦普通控件直接持有原生句柄调系统 API，就会绕过框架的布局、裁剪、命中与多窗口生命周期。
- **违反后果**：控件私自绘制的原生内容无法被圆角裁剪或浮层遮挡，且无法在无头测试中验证。

## 合同六：逻辑状态与呈现 Overlay 分离

- **是什么**：`UiWorld` 只存逻辑目标状态；过渡动画途中的瞬时呈现值存放在 `PresentationStore` overlay 中。合成器动画（`AnimationClass::Compositor`）严禁把每帧采样值写回节点的基础样式。
- **为什么**：若将 `opacity: 0 → 1` 的中间值 `0.01`、`0.02` 每帧写回 `UiWorld`，不仅业务代码读到的是错误的过渡态，还会迫使保留树每帧标脏、重跑提取。
- **违反后果**：`UiWorld` 在纯合成动画期间每帧变脏，`perf/README.md` 中的 `compositor-steady` 零工作门禁失败。

## 合同七：诊断统一走 `nana-diagnostics`，帧路径零 I/O

- **是什么**：框架诊断统一使用 `event!`、`metric!`、`fault!`、`span!` 宏配静态描述符（`nana_diagnostics::framework`，ID 只增不改）。帧路径上严禁使用外部日志 crate、`eprintln!`、字符串格式化或文件 I/O；高频数据必须记录为原子 `metric!`，严禁作为逐帧 `event!` 发送。
- **为什么**：在 144 Hz / 240 Hz 渲染下，每帧一次 `eprintln!` 或格式化分配就会直接击穿帧时间预算。
- **违反后果**：帧耗时出现周期性尖刺；随意改动诊断 ID 会破坏现有 `.nlog` 离线解析器。

## 合同八：共享单一 `ComponentRegistry`，无第二套实例化 ABI

- **是什么**：所有内置控件与扩展控件共用同一张 `ComponentRegistry` 与同一个 `register_component` 入口。`ComponentSupport` 不是实例化 ABI。
- **为什么**：两套组件 ABI 意味着每个控件的布局、焦点与无障碍投影都要重复实现两遍，能力永远无法对齐。
- **违反后果**：扩展控件拿不到内置控件的增量布局、焦点导航与无障碍投影支持。

## 合同九：WebView 不是产品 UI 壳

- **是什么**：NanaUI 没有整窗 WebView 壳。窗口内嵌网页仅限显式的宿主原生例外 `runtime::BrowserView`（当前仅 macOS `WKWebView` 后端）；无头 `WebSurface` 页面则是**宿主内容生产者**（将捕获帧送入应用的 sink），不属于 UI 树。盒模型对照工具 `tools/css-parity-webview` 留在 workspace 之外，绝不链接进产品 crate。
- **为什么**：用整窗 WebView 充当 UI 壳会让整个界面退出原生保留树与宿主 GPU 管线，失去同窗文档序实时画面合成的核心价值。
- **违反后果**：原生 WebView 内容无法进入离屏截图，且在遇到非矩形裁剪、透明度/滤镜组或上层控件遮挡时只能整体隐藏。

## 一句话审阅判据

合同一至九共同指向同一个架构审阅问题：

> **「这件事的唯一权威是谁？」** —— 保留树、CSS 级联、文本排版、输入路由、GPU 设备、原生窗口，每一个领域在全仓都只有一个 crate 拥有最终决定权；其余 crate 只能向它提交意图或消费它的投影，绝不能私自维护第二份答案。

## 接着读

- [模块与分层](/learn/modules-and-layers)：25 个 crate 依赖图的完整实读
- [作者入口](/learn/authoring-paths)：两条编写路径如何汇入同一棵树
- [证据清单](/learn/evidence-checklist)：九条合同的自动化复核命令与源码行号
