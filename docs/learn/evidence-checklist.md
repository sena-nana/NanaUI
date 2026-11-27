---
title: 证据清单：关键架构断言怎么核
---

# 证据清单：关键架构断言怎么核

本页将 [架构学习](/learn/) 板块中反复出现的**核心架构断言**落实为「断言 → 只读核对命令 → 当前工作区实跑读数」三列对照表。所有命令均可在仓库根目录下直接复跑，不修改任何文件。

## A. 分层与依赖方向

### A1. 依赖沿 `core → runtime → scene → nana-ui` 单向流动、严格无环

**为什么重要**：一旦出现反向依赖边，底层合同就会开始感知上层树或渲染器的实现细节，单向分层随之瓦解（见 [合同与不变量](/learn/contracts-and-invariants)）。

```powershell
# 解析工作区普通依赖边（cargo metadata 本身解析成功即证明无环，Cargo 拒绝环形依赖图）
cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
# 读法：遍历每个成员包的 dependencies[]，凡 path 落在 crates/ 内且 kind 为空（normal）的即一条普通依赖边
```

**当前读数**：`crates/` 内共有 **58 条工作区成员普通依赖边**（24 个工作区成员 crate 之间 57 条 + `nana-ui → nana-icons-tabler` 可选边 1 条；另加非成员 `nana-icons-tabler → nana-ui-core` 1 条），**全部严格前向**。核心主线：`nana-ui-core` 仅依赖 `nana-diagnostics`；`nana-ui-runtime` 依赖 `{core, text, input, view-schema, diagnostics}` 与可选 `{charts, diagram, view-macros}`；`nana-ui-scene` 依赖 `{runtime, core, diagnostics, text(可选 benchmark)}`，已移除 `nana-ui-charts`；`nana-ui` 依赖 `{scene, runtime, core, …}`。没有任何一条低层指回高层的普通边。仅用于测试宿主的 4 组 `dev-dependencies` 例外已单独列于 [模块与分层](/learn/modules-and-layers#dev-边与例外)，不进入产品二进制。

### A2. 四个 UI 主线 crate 均不依赖 CSS 引擎

```powershell
rg "nana-ui-css" crates/nana-ui-core/Cargo.toml crates/nana-ui-runtime/Cargo.toml crates/nana-ui-scene/Cargo.toml crates/nana-ui/Cargo.toml
```

**当前读数**：`0` 命中。`nana-ui-css` 在 `crates/` 内的消费者仅有 `nana-ui-view-codegen`（构建期），详见 B1。

## B. 唯一权威实现清单

### B1. CSS 解析与级联只在 `nana-ui-css`

```powershell
cargo tree -p nana-ui-css --depth 1                          # css -> 仅 core，零外部依赖，无保留树依赖
rg -i "selector|cascade" crates/nana-ui-runtime/src -g '!tests'  # 命中均为注释/文档/ICU selectordinal
rg "nana_ui_css" crates/*/src                                # 仅 view-codegen（构建期）
```

**当前读数**：`crates/nana-ui-runtime/src/view/style.rs:8-9` 模块文档写明 `"No CSS is parsed and no selector is matched at run time, and this crate has no CSS in it"`；`nana-ui-core` 无样式表解析器、选择器匹配或级联实现（仅在 `box_layout.rs` / `geometry.rs` / `style_model.rs:341` 中提供单值枚举关键字解析、`calc()` AST 与 `--nana-*` 语义色令牌名映射）；`nana-ui-scene` 仅一处基准测试注释；`nana-ui` 为 `0` 命中。

### B2. 输入路由的唯一消费方是 Runtime 的 `AppContext::route_input`

```powershell
cargo tree --workspace -i nana-ui-input --edges normal       # 普通边消费方：runtime + platform（再导出）
rg "pub fn route_input" crates                               # 唯一定义：crates/nana-ui-runtime/src/framework/input.rs:397
rg "route_input|route_shaped" crates -g '!*tests*'           # 各宿主全部汇入 context.route_input
```

**当前读数**：`route_input` 的唯一实现在 `crates/nana-ui-runtime/src/framework/input.rs:397`（`impl AppContext`）；`nana-ui-devtools` 调用 `route_shaped`（`crates/nana-ui-devtools/src/agent/runtime.rs:123`）→ 内部在 `crates/nana-ui-runtime/src/framework/input/headless.rs:126` 仍然调用 `context.route_input`，不存在任何旁路路由。

### B3. 文本引擎唯一、GPU 后端唯一

```powershell
rg "cosmic_text|cryoglyph" crates/nana-text/src              # 仅 lib.rs:25 文档声明本身
rg "nana_gpu::__framework" crates                            # __framework 仅被允许的框架 crate 内部使用
python3 scripts/check-engine-boundary.py                     # 自动化守门脚本
```

**当前读数**：`crates/nana-text/src/lib.rs:25` 仅有一处陈述该禁令本身的模块注释（`"No cosmic_text or cryoglyph identifier appears under src/"`）；GPU 侧以 WGPU 为唯一后端，公开签名中的 `wgpu::*` 仅允许在 `wgpu-interop` feature 下出现在白名单中的四个 crate 内，由 `scripts/check-engine-boundary.py` 强制校验通过。

## C. 帧路径关键不变量锚点

| 架构断言 | 源码锚点位置 | 核对要点 |
| --- | --- | --- |
| 变更在帧边界原子提交 | `crates/nana-ui-scene/src/runtime_document.rs:109`（`flush`） | 有界 8-pass 收敛，布局、提取、命中与无障碍共享同一 `layout_generation`（`:28`） |
| 滚动不改写 Runtime 布局盒 | `crates/nana-ui-runtime/src/world/mutation.rs:2758`（`scroll_to_clamped`） | 仅更新 `scroll_offset` 与命中滚动偏移，`LayoutBox` 保持不变 |
| 单一 `Device` / `Queue`，跨代际资源拒绝 | `crates/nana-gpu/src/context.rs:19`（`DeviceGeneration`）、`:715`（`check_device`） | 设备丢失粘性记录首次原因，旧代际纹理或目标在入口即返回 `DeviceMismatch` |
| 产品呈现路径无 CPU 回读 | `crates/nana-ui-devtools/src/offscreen.rs`（Agent 会话） | CPU 回读仅存在于 devtools 离屏快照；`run_runtime` 的 `present` 路径绝不经过它 |
| 帧路径零字符串格式化、零文件 I/O | `crates/nana-diagnostics/src/framework.rs:2` | `event!` / `metric!` 均基于静态描述符，ID 只增不改 |

## D. 一帧 35 步全序入口锚点

一帧从调度、输入消息排空到 `present` 的完整 35 步顺序表见 [GPU 与帧生命周期](/learn/gpu-frame)，四条跨层端到端数据流见 [端到端数据流](/learn/flows)。源码入口锚点如下：

```powershell
rg "fn redraw" crates/nana-ui/src/scene_host/present.rs      # pub(super) fn redraw（第 :114 行起）
```

## 接着读

- [合同与不变量](/learn/contracts-and-invariants)：九条核心合同的设计缘由与违反后果
- [模块与分层](/learn/modules-and-layers)：58 条工作区成员普通依赖边全量清单与 dev 边说明
- [25 crate 覆盖矩阵](/learn/coverage-matrix)：每个 crate 的四要素速查表
