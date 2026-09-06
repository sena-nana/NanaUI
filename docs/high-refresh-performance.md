# 高刷新重构阶段性能记录（2026-09-05）

这是阶段证据，不是完整 120Hz 验收通过报告。

## 环境及可比性

- Windows；AMD Ryzen 7 5800X，8 核 / 16 线程；NVIDIA GeForce RTX 5060。
- 系统也列出了 Virtual Desktop Monitor；本次未确认实际 Surface 刷新率。
- release 基准；工作区包含本次修改及其他任务的并行修改、构建。
  报告整理时 HEAD 为 `6d22e10bac9d46b0759a7724d836f2560e9cc0d9`，并非干净提交。
- Runtime 有同机改动前后报告；Scene/GPU 没有同负载的改动前基线。
  下列时间不能用于宣称重构带来了确定的百分比收益。

## Scene 规模采样

每档预热 2 秒，再串行采样至少 60 秒；窗口为 1920×1080。
负载为仿射 Quad，保持相同可见内容，逐帧改变一个节点颜色并滚动根容器。
每帧请求间隔为 1/120 秒，但线程 sleep 不代表 Surface 呈现。

| 保留节点 | 可见操作 | 有效样本 | 局部更新 P95 / ms | 滚动 P95 / ms | 可见查询 P95 / ms |
|---:|---:|---:|---:|---:|---:|
| 10,001 | 540 | 5,879 | 0.0868 | 0.0154 | 0.0382 |
| 50,001 | 540 | 6,938 | 0.0616 | 0.0132 | 0.0321 |
| 100,001 | 540 | 6,831 | 0.0782 | 0.0153 | 0.0505 |

三档滚动后代图元重建数均为 0，结构计划重编译数均为 0。
样本数低于理想的 7,200，且存在长尾；不能据此声称保持了 120Hz。
这些耗时只覆盖 Scene 更新与可见查询，不包含 Runtime、文字、GPU 或呈现。

原始 P50/P95/P99、最大值和首次构建耗时：
[`high-refresh-scene-scale.json`](performance-data/high-refresh-2026-09-05/high-refresh-scene-scale.json)。

```powershell
cargo run --release --locked -p nana-ui-scene --features benchmark --bin nana-scene-benchmark -- --high-refresh --output target/performance/high-refresh-scene-scale.json
```

## Runtime 前后观察

| 节点 | 局部绘制系统 P95 前 → 后 / ms | 指针悬停 P95 前 → 后 / ms | 稳态系统 P95 前 → 后 / ms |
|---:|---:|---:|---:|
| 100 | 0.006 → 0.006 | 0.003 → 0.003 | 0.090 → 0.089 |
| 500 | 0.010 → 0.012 | 0.005 → 0.006 | 0.185 → 0.247 |
| 1,000 | 0.015 → 0.016 | 0.006 → 0.006 | 0.397 → 0.591 |
| 5,000 | 0.019 → 0.026 | 0.007 → 0.009 | 2.055 → 4.259 |
| 10,000 | 0.022 → 0.027 | 0.008 → 0.013 | 5.086 → 8.707 |

若干时间指标变差，原因尚未通过隔离实验确认，不能直接归因于并行负载并忽略。
需要在固定、安静且源码冻结的环境重跑，才能判断回归及收益。

Issue #8 四份报告的首次检查失败：

| 门禁 | 测得 P95 | 阈值 |
|---|---:|---:|
| Runtime 5,000 节点首次系统处理 | 40.886 ms | 8 ms |
| 标准 Runtime 5,000 节点布局 | 51.942 ms | 8 ms |
| Vue 5,000 节点构建 | 74.896 ms | 40 ms |

Runtime 改动前同项首次系统处理 P95 为 21.638 ms，也超过 8 ms。
复查发现新增的隔离布局放置缓存不应覆盖所有普通节点，已收窄为仅记录显式隔离边界。
该修正后的隔离布局行为测试通过；最终复测应与首次报告分开读取。
门禁失败未豁免，阈值未放宽。

修正后的最终复测仍有相同三项失败：Runtime 首次系统处理 P95 **37.930 ms**、
标准布局 P95 **19.818 ms**、Vue 构建 P95 **83.073 ms**。
布局耗时下降不构成可控 A/B 收益证明，其他并行负载及源码变化仍未隔离。

最终原始数据：[Runtime](performance-data/high-refresh-2026-09-05/high-refresh-final-runtime.json)、
[Framework](performance-data/high-refresh-2026-09-05/high-refresh-final-framework.json)、
[Vue](performance-data/high-refresh-2026-09-05/high-refresh-final-vue.json)、
[Scene](performance-data/high-refresh-2026-09-05/high-refresh-after-scene.json)、
[门禁输出](performance-data/high-refresh-2026-09-05/high-refresh-final-issue8.txt)。

```powershell
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-final-runtime.json --framework target/performance/high-refresh-final-framework.json --vue target/performance/high-refresh-final-vue.json --scene target/performance/high-refresh-after-scene.json
```

原始报告：[改动前](performance-data/high-refresh-2026-09-05/high-refresh-before-runtime.json)、
[改动后](performance-data/high-refresh-2026-09-05/high-refresh-after-runtime.json)。
这两份沿用 Issue #8 的采样协议，没有伪装成 60 秒 Surface 负载。

## 正确性证据

- 真实 GPU：纹理内容更新复用批次，上传量为 0；替换 view 后像素正确，解绑被拒绝。
- 真实 GPU：同格式目标交替呈现，分别保留各自红/绿内容。
- 真实 GPU：资源生产第二步失败后，前一步编码没有提交或改写目标；重试提交后才通知两位生产者。
- 真实 GPU：局部 Quad 颜色变化显示最新像素，该负载上传量低于首次的四分之一。
- 最终 GPU 绘制套件 178 项通过，包含自定义 renderer 动态默认与显式版本复用的行为验证。
- Windows `hosted-gpu-demo --measure-first-frame` 成功呈现并退出：debug 首帧 3525.631 ms，material 为 transparent。
  首帧数据不参与稳定帧指标。
- Vue Agent 实际点击 `increment` 返回 handled，a11y 显示 `count = 1`，按钮边界大于 8px；
  已打开检查[初始截图](performance-data/high-refresh-2026-09-05/high-refresh-agent.png)和[点击后截图](performance-data/high-refresh-2026-09-05/high-refresh-agent-click.png)。

完整交付与尚未完成的验收范围见[重构说明](high-refresh-refactor.md)。


## 续建：百万项虚拟窗口的真实操作验证

新增可复现 fixture：`crates/nana-js-engine/fixtures/vue-sfc-compat/src/VirtualNavigation.js`。
通过 `build-virtual.mjs` 打包，再运行 `verify-virtual.py`，驱动当前 Windows Agent 的
V8 / Runtime / Scene / WGPU 无头路径。它不是 Surface 刷新率或 GPU 时间基准。

- 逻辑数据 1,000,000 条；初始挂载 5 项，跳转并保留一个业务 key 后 6 项，释放后回到 5 项。
- 保留项的稳定节点 ID 在跳转前后相同；跳转后可见行位置为 y=64/96/128/160/192，行高 32。
- 跳转后的真实坐标点击使业务状态变为 `Selected 500000`；释放后屏外项从无障碍树移除。
- 含保留项时无障碍节点总数 29；本轮没有测量可见图元总数、GPU 时间或分配量。
- 截图已打开检查，可见行与点击位置一致；按钮文字仍出现已有的省略显示，未把该结果称为视觉完整验收。

[原始命令响应](performance-data/high-refresh-2026-09-05/virtual-navigation/agent.jsonl)、
[机器可读结果](performance-data/high-refresh-2026-09-05/virtual-navigation/report.json)、
[点击后截图](performance-data/high-refresh-2026-09-05/virtual-navigation/jumped.png)。

本次操作验证修复了 Agent 初始化缺少 Web API shim，以及点击坐标错误使用未滚动
Runtime LayoutBox 的问题。该轮仅验证显式 key 保留；后续自动保留证据见下节。
前述 Issue #8 失败项及 120Hz 未验收状态没有改变。


续建构建成功，但当前 Agent 构建仍报告 `NativeContentRenderer` dead-code 警告和
Windows linker LNK4098；本轮未将构建成功表述为零警告 Clippy 通过。
Vue 组件包 174 项、runtime 包 77 项和真实 Vue fixture 7 项通过。
最终 `cargo test -p nana-ui-core --lib virtual_ --locked` 18 项通过。


## 续建：冻结表格与首帧命中验证

Windows 原生 Agent 验证使用 1,000,000 逻辑行、10,000 逻辑列（隐式索引，未创建对应数据矩阵）。
冻结首行和首列，320×160 视口中的挂载单元格数为 20；跳转并保留一个屏外行列后为 30，
释放后回到 20。含保留项时无障碍节点 125。可见图元数、CPU/GPU 时间及分配量未测量。

首帧无障碍坐标校验通过，随后依次点击冻结交叉角、表头、冻结列与正文单元格，业务状态
分别变为 `0/0`、`0/8000`、`500000/0`、`500000/8000`。验证检查实际业务值，不仅检查 handled。
截图含冻结首行、首列及正文；现有按钮省略显示仍在，不是完整文字视觉验收。

[原始响应](performance-data/high-refresh-2026-09-05/virtual-table/agent.jsonl)、
[机器可读结果](performance-data/high-refresh-2026-09-05/virtual-table/report.json)、
[点击后截图](performance-data/high-refresh-2026-09-05/virtual-table/jumped.png)。

本轮 Core 虚拟化 20 项、Scene 92 项、Vue 组件 175 项及真实 Vue fixture 7 项通过。
移除临时诊断代码后重新构建 Agent，冻结表格与上一轮百万列表原生操作脚本均通过。
无障碍受影响子树发布仍有线性成本；没有重新测量或放宽 Issue #8 和 120Hz 的时间阈值。


## 续建：自动焦点与 IME 活动项保留

Windows 原生 Agent 使用 `VirtualActivity.js`，无显式 retainedKeys。程序化聚焦第 2 项后
跳转第 500000 项：挂载 5 → 6，保留输入框稳定 ID、离屏坐标和焦点；`type` 命令继续写入
该输入框。点击结束编辑后挂载回到 5。已查看截图，可见窗口仅显示目标附近 5 项。

[原始响应](performance-data/high-refresh-2026-09-05/virtual-activity/agent.jsonl)、
[结果](performance-data/high-refresh-2026-09-05/virtual-activity/report.json)、
[截图](performance-data/high-refresh-2026-09-05/virtual-activity/jumped.png)。

Vue 实例测试验证 focus/IME 独立释放、重排与删除、冻结表格双轴挂载 25 → 36 → 25，
共 10 项通过；组件包 175 项、runtime 包 78 项通过。IME 会话顺序由测试事件驱动，
本轮没有执行 Windows 系统输入法预编辑/候选窗口验收，也没有测量 Surface 刷新率。
Issue #8 失败项和 120Hz 未验收状态不变。Agent 构建仍有既有 dead-code / LNK4098 警告。

Rust 定向验证：`cargo test -p nana-ui-vue --lib focus --locked` 13 项通过，
`cargo test -p nana-ui-vue --lib composition --locked` 1 项通过；覆盖宿主实际焦点返回、
无效焦点请求、非所有者 blur 和组合提交。该 feature 配置另有既有 calendar 测试辅助函数
dead-code 警告。`git diff --check` 通过。


## 续建：Rust 定位物化与活动项保留

新增 Rust 列表/树定位入口与同机 Agent 验证。百万逻辑项使用固定 32 像素行高，
160 像素视口；初始挂载 5 项，聚焦并预编辑第 2 项后跳转到第 500000 项，挂载 6 项。
保留项实体与 IME 文本不变；真实滚动后目标输入框 y=0，点击后焦点正确转移，旧项释放，
回到 5 项。每项一个定位 Stack，活动状态共 14 个 Runtime 节点、15 个 Scene 图元，
最新自身裁剪剔除复测的保守可见操作集为 10 个。该计数包含保守保留依赖，不能当作精确可见像素数。

命令：

```sh
cargo test -p nana-ui-runtime --lib virtual_ --locked
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```

Runtime 虚拟化 12 项通过（含 6 项新测试），覆盖百万 key 查询数量约束、稀疏布局、
焦点/IME、嵌套内容释放、业务 key 重排、变高、树折叠、重复 key 与 Runtime 提交失败回滚。
Windows 原生 GPU 截图/点击测试 1 项通过，截图已打开检查。Agent 构建报告既有
`NativeContentRenderer` dead-code 警告；未将其表述为零警告 Clippy 验收。

[结果](performance-data/high-refresh-2026-09-05/virtual-rust/report.json)、
[截图](performance-data/high-refresh-2026-09-05/virtual-rust/jumped.png)。

未测量本入口的 60 秒 Surface/GPU 时间、系统 IME 候选窗口或总内存峰值；Runtime 中
现有全局 handler 删除扫描、Rust 冻结表格及 120Hz 完整门禁仍未完成。Issue #8 时间阈值不变。


## 续建：Rust 冻结表格、依赖清理及完整目标 GPU 状态

百万行、万列原生表格：挂载单元格 20 → 30 → 20，活动状态 39 个 Runtime 节点、
65 个 Scene 图元。冻结角、列头、行头和正文的真实坐标点击均到达预期单元格；
嵌套编辑器保持 IME，结束活动会话后卸载。截图已打开检查。
[结果](performance-data/high-refresh-2026-09-05/virtual-rust-table/report.json)、
[截图](performance-data/high-refresh-2026-09-05/virtual-rust-table/jumped.png)。

`cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1`
两项通过。`cargo test -p nana-ui-core -p nana-ui-runtime -p nana-ui-scene --lib --locked` 全部通过。
事件卸载反向索引新增测试通过；不再扫描无关 handler 桶。

`alternating_live_targets_keep_prepared_geometry_text_and_bindings` 通过：16 目标各三轮交替绘制，
不同尺寸/DPI，混合 HostTexture、文字/旋转文字、图标、笔画及背景模糊。后两轮像素与对应
目标首轮逐字节一致，batch CPU 时间为零，GPU 上传字节为零；移除目标释放保留状态。
这证明该负载的目标缓存复用，不等于 16 个 1080p Surface 的时间门禁通过。

完整 GPU 首轮 193 通过、1 失败：旧笔画上传测试混用了首次全量基线和后续差异上传。
改为独立冷启动后，16/32/64 边的纯实例上传仍严格翻倍，该测试通过。原有相同几何免上传测试保留。
物理 RTX 5060 显示器当前报告 60Hz，另有 Virtual Desktop Monitor 报告 143Hz；此枚举不能证明
120Hz 实际呈现。Issue #8 失败项、GPU timestamp/Surface 60 秒及设备丢失压力验收仍待完成。


## 续建：1080p GPU timestamp 与真实 Surface 探针

`nana-gpu-scene-benchmark` 新增 `--gpu-timestamps` 和 `--sample-seconds`。生产者和 UI 共用
宿主 encoder，一帧一次 submit；timestamp 分别包围生产者及 UI 命令。预热至少 2 秒，
随后每档串行采样 60 秒。GPU 完成后只读查询结果，不读目标像素；测量模式为离屏逐帧
等待完成，查询回读及等待不计入 CPU 准备时间。CPU 阶段字段保留原义，GPU 实际耗时另列。
框架 CPU 准备包含 `RuntimeDocument::flush` 及 painter 调用；生产者 CPU 编码和 submit 另计。

第一组是多个纹理节点引用同一生产者，夹有静态文字和按钮：

| 纹理节点 | 样本 | CPU 准备 P95 / ms | UI GPU P95 / ms | 生产者 GPU P95 / ms | 进程工作集峰值 / MiB |
|---:|---:|---:|---:|---:|---:|
| 1 | 213569 | 0.0096 | 0.024960 | 0.004672 | 145.55 |
| 4 | 211657 | 0.0123 | 0.026272 | 0.004608 | 143.39 |
| 16 | 197020 | 0.0214 | 0.031136 | 0.004608 | 140.99 |

全部样本的 Runtime 工作轮次、结构计划重编译次数、UI 上传字节及缓冲重分配均为 0。
仅本组明确负载的 CPU/GPU P95 低于 2 ms；不代表复杂工作区、独立生产者或 Surface 已通过。
内存由 Windows 每 100 ms 观察进程统计，包含预热、驱动及样本容器；不包含独立 GPU 显存，
不能解释为纯 UI 缓存占用或 malloc 分配数。

[1 节点](performance-data/high-refresh-2026-09-05/gpu-timestamps/gpu-1.json)、
[4 节点](performance-data/high-refresh-2026-09-05/gpu-timestamps/gpu-4.json)、
[16 节点](performance-data/high-refresh-2026-09-05/gpu-timestamps/gpu-16.json)；
同目录保存各档输入 scenario 和 `.memory.json`（机器、提交、dirty 状态及内存观测）。

```powershell
cargo build --release -p nana-ui --bin nana-gpu-scene-benchmark --features gpu,bundled-fonts --locked
./scripts/measure-high-refresh-gpu.ps1
```

真实 hosted-gpu-demo 的 Device 销毁恢复探针通过：设备代次 **1 → 2**，重建后实际 Surface
再次呈现。它验证本应用主动销毁 Device 后的恢复，不等于所有驱动异常/多窗口丢失场景。
[原始结果](performance-data/high-refresh-2026-09-05/high-refresh-device-recovery.json)。

真实窗口（1650×1080、Vulkan、PreMultiplied）请求 120Hz，同步呈现并预热 2 秒后采样
60.001 秒，共 3600 个回调间隔。P50 **14.2171 ms**、P95 **20.9328 ms**、P99 **21.0276 ms**；
超过 8.33 ms 的比例 **100%**，因此门禁 **失败**。这是 Surface present 回调间隔，没有显示器
scanout 反馈；物理显示器枚举为 60Hz。没有启用撕裂、降低阈值或用离屏结果替代该失败。
[原始结果](performance-data/high-refresh-2026-09-05/high-refresh-surface-present.json)。

```powershell
cargo run --release -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts --locked -- --probe-device-loss --performance-output target/performance/high-refresh-device-recovery.json
cargo run --release -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts --locked -- --measure-present-seconds 60 --performance-output target/performance/high-refresh-surface-present.json
```

图标缓存压力验证通过：最老活跃条目不会阻止其余闲置条目淘汰，连续 32 轮换入内容仍保持
128 个闲置缓存预算；一帧需要超过预算时保留全部活跃条目，下次重新准备时释放峰值。
图标纹理上传也计入实际上传字节，失败绘制不会继续暴露上一目标的成功 GPU 统计。


独立资源复测：同样 1080p、2 秒预热、每档 60 秒串行采样，每个纹理节点使用独立
HostTexture 和生产者。各生产者编码进入同一 encoder，整帧仅一次 submit。

| 独立纹理/生产者 | 样本 | CPU 准备 P95 / ms | UI GPU P95 / ms | 全部生产者 GPU P95 / ms |
|---:|---:|---:|---:|---:|
| 1 | 267601 | 0.0091 | 0.023392 | 0.004224 |
| 4 | 206473 | 0.0114 | 0.025792 | 0.010656 |
| 16 | 99480 | 0.0250 | 0.030720 | 0.040640 |

全部采样帧仍无 Runtime 工作轮次、结构重编译、UI 上传及 GPU 缓冲重分配。
这是给定静态文字/按钮与简单 GPU 生产者的负载；未将这些结果推广为复杂工作区通过。
[1 个生产者](performance-data/high-refresh-2026-09-05/gpu-timestamps-independent/gpu-1.json)、
[4 个生产者](performance-data/high-refresh-2026-09-05/gpu-timestamps-independent/gpu-4.json)、
[16 个生产者](performance-data/high-refresh-2026-09-05/gpu-timestamps-independent/gpu-16.json)。
同目录保存各档 scenario 和进程内存观测。

```powershell
./scripts/measure-high-refresh-gpu.ps1 -IndependentTextures
```

## 最新功能回归及 Issue #8 复测

Core 177、Runtime 803、Scene 94 项通过；`scene_paint::` 195 项真实 GPU 测试通过。
原生 Agent 的百万项列表与冻结表格两项测试再次通过，截图与报告已更新到对应目录。
列表跳转期间保留 6 项、14 个 Runtime 节点、15 个 Scene 图元、10 个保守可见操作；
编辑结束后恢复 5 项。表格保留 30 个单元格，结束后恢复 20 个。

```powershell
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
cargo clippy -p nana-ui --lib --bin nana-gpu-scene-benchmark --features hosted --locked --no-deps -- -D warnings
```

上述命令通过。`cargo check --workspace --all-targets --locked` 仍被 rich-text Markdown
测试引用已经移除的 `blocks` / `drawing` API 阻塞；未删测试或关闭 feature。
WGPU 逆向依赖检查仍为单一 `wgpu 30.0.1`。

四个 release 基准先完成构建，再串行运行。最新门禁仍失败：

| Issue #8 指标 | P95 / ms | 阈值 / ms |
|---|---:|---:|
| Runtime 5000 节点首次系统处理 | 32.338 | 8 |
| Runtime 5000 节点全量布局 | 18.494 | 8 |
| Vue 5000 节点构造 | 81.972 | 40 |

首次系统处理覆盖样式、焦点、无障碍、布局输入、命中和抽取；全量布局负载交替改变
视口宽度。这些全局路径尚未达标，不能由纹理稳态零布局的结果豁免。尚无分阶段证据
把时间归因于单一实现。工作区同时有其他修改，因此不将与历史报告的差值解释为本改动收益。

[Runtime](performance-data/high-refresh-2026-09-05/high-refresh-latest-runtime.json)、
[Framework](performance-data/high-refresh-2026-09-05/high-refresh-latest-framework.json)、
[Vue](performance-data/high-refresh-2026-09-05/high-refresh-latest-vue.json)、
[Scene](performance-data/high-refresh-2026-09-05/high-refresh-latest-scene.json)、
[门禁输出](performance-data/high-refresh-2026-09-05/high-refresh-latest-gates.txt)。

## CPU 准备阶段的分配诊断

GPU 基准新增 `--allocation-counts`，仅该可执行文件安装计数分配器。计数范围是
当前 benchmark 线程的 `RuntimeDocument::flush` 和 painter 调用，包括 WGPU 在这两个
调用内的 Rust 分配。成功 alloc/alloc_zeroed/realloc 各计一次；请求字节把 realloc
的新尺寸计入，不是净增内存或峰值。生产者、其他线程、驱动原生分配、GPU 显存和
报告/采样数组不在范围内。作用域退出或 panic 会关闭计数，两项行为测试通过。

独立生产者 1/4/16 档分别预热 2 秒、串行采样 60 秒：

| 生产者 | 样本 | CPU 准备 P95 / ms | UI GPU P95 / ms | 每帧最多分配次数 | 每帧最多请求字节 |
|---:|---:|---:|---:|---:|---:|
| 1 | 234572 | 0.0094 | 0.024480 | 20 | 6768 |
| 4 | 190250 | 0.0128 | 0.026048 | 28 | 8758 |
| 16 | 102415 | 0.0273 | 0.031360 | 58 | 33338 |

计数模式有诊断开销，应与之前未启用计数的样本分开读取。三档仍无 Runtime 工作轮次、
结构重编译和 UI 上传；没有把这一点写成“零分配”。此结果不覆盖首次挂载、复杂 UI、
全线程分配或关闭窗口后的长期内存稳定性。

[1 档](performance-data/high-refresh-2026-09-05/gpu-allocations-independent/gpu-1.json)、
[4 档](performance-data/high-refresh-2026-09-05/gpu-allocations-independent/gpu-4.json)、
[16 档](performance-data/high-refresh-2026-09-05/gpu-allocations-independent/gpu-16.json)。
同目录保留机器/进程内存记录，输入为 `perf/fixtures/high-refresh-independent-gpu-*.json`。

```powershell
cargo test -p nana-ui --bin nana-gpu-scene-benchmark --features hosted --locked allocations::tests -- --test-threads=1
cargo build --release -p nana-ui --bin nana-gpu-scene-benchmark --features gpu,bundled-fonts --locked
./scripts/measure-high-refresh-gpu.ps1 -IndependentTextures -AllocationCounts
```

## 首次系统处理的阶段诊断与无障碍投影优化

`nana-runtime-benchmark --profile-initial-systems` 运行与标准基准相同的六个系统阶段，
单独计时，5000/10000 节点各预热 10 次、采样 60 次。标准基准仍不插入阶段计时，
验收仍走原报告与门禁。第一份诊断中 5000 节点无障碍投影 P95 为 11.503 ms，
抽取 7.465 ms、命中构建 5.903 ms；各阶段分位数不能相加为整批分位数。

首次投影尚无命中缓存时，原实现为每个节点及其子节点重新计算完整祖先变换。
现在同一投影事务内共享祖先变换和可访问边界；缓存随本次调用释放，下一次滚动/变换
重新计算，不跨事务保留旧坐标。无障碍 delta 对每个受影响节点只投影一次，同时产生
更新与移除集合。抽取和指定节点投影预留输入规模容量，避免增长时反复搬移大结构。

Runtime 全量 805 项、真实 Agent 虚拟列表/冻结表格两项测试通过。新增行为测试对比
命中索引与无索引投影，覆盖先查询后代、逆序查询及连续改变滚动值。Runtime 库与
benchmark 的 Clippy（`--no-deps -- -D warnings`）通过。

优化后标准 Runtime 报告：5000 节点首次系统处理 P50 **15.789 ms**、P95 **20.111 ms**，
仍未通过 8 ms 门禁。本次采样期间观察到其他任务的 rustc 进程，阶段报告存在明显抖动；
不将与上一份报告的时间差解释为严格同负载 A/B 收益。其余三个报告沿用上一轮，门禁
复查仍列出相同三项失败，没有调整阈值。

[阶段诊断（前）](performance-data/high-refresh-2026-09-05/high-refresh-initial-stages.json)、
[加入投影缓存](performance-data/high-refresh-2026-09-05/high-refresh-initial-stages-cached.json)、
[加入预留容量](performance-data/high-refresh-2026-09-05/high-refresh-initial-stages-optimized.json)、
[Runtime 复测](performance-data/high-refresh-2026-09-05/high-refresh-optimized-runtime.json)、
[门禁输出](performance-data/high-refresh-2026-09-05/high-refresh-projection-gates.txt)。

```powershell
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark --locked
target/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-initial-stages-optimized.json
target/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-optimized-runtime.json
```

## Vue 构造与选择器依赖优化

新增 `nana-vue-runtime-benchmark --profile-construction`，对与标准基准相同的构造操作
分阶段计时。5000 项首份诊断的注册阶段 P95 为 58.979 ms，创建 4.779 ms、插入
3.286 ms。诊断逐操作计时，有额外测量开销；门禁仍只读取标准基准。

已完成两类优化：

- 从解析后的选择器 AST 计算是否需要树关系；随样式表、媒体和主题激活重新计算。
  普通类型、类名和属性规则不再构造祖先/兄弟匹配信息。插入和删除不触发无关兄弟
  的样式重算；结构伪类、祖先/兄弟组合器、`:has()`、`:empty` 等保留原完整路径。
  无样式表时，自定义属性仍解析 prop/inline 声明并按原顺序继承。
- `SemanticWidget` 本体实测 5320 字节（其中 LayoutStyle 4768 字节）。桥接索引改存
  `Box<SemanticWidget>`，HashMap 扩容只移动键和指针，避免搬移整份大值。控件内容仍
  分配并保留，额外引入一次间接访问；不是将每项全部存储缩减成 8 字节。公开借用入口、
  owned snapshot 和序列化格式不变，UiWorld 节点存储没有替换。

最终 Vue 全量 **776 项通过**，包括千兄弟节点插入/删除的增量变化集合，以及随后注入
结构伪类的正确性。声明、变量覆盖、字体继承及后续 nth-child 注入也有行为回归。
Vue 库与 benchmark Clippy（`--no-deps -- -D warnings`）通过。

最终代码重新构建 `nana-agent-session` 后，`verify-virtual.py`、
`verify-frozen-table.py`、`verify-activity.py` 三份真实 V8/GPU 验证均通过；
截图已打开检查，对应目录的报告和截图已更新。挂载规模分别保持 5→6→5、
20→30→20、5→6→5，离屏输入内容保持，冻结区域点击命中正确。

| 标准 5000 项 Vue 构造 | P50 / ms | P95 / ms | P99 / ms |
|---|---:|---:|---:|
| 选择器优化后、索引仍内嵌大值 | 43.042 | 57.352 | 60.556 |
| 指针索引后 | 15.276 | **25.459** | 31.491 |

最新 **Vue 构造 P95 已通过 40 ms 门禁**。单次最大值仍有 41.288 ms，不能描述为每次
构造均小于 40 ms。最新门禁组合仍有 Runtime 首次系统处理和全量布局两项失败；其余
三份报告沿用上一轮，不把本次 Vue 构造通过解释为整个 Issue #8 或 120Hz 通过。
工作区仍有并行修改，报告保留绝对测量值，不宣称严格隔离的百分比收益。

[首份阶段诊断](performance-data/high-refresh-2026-09-05/high-refresh-vue-construction-stages.json)、
[选择器优化标准报告](performance-data/high-refresh-2026-09-05/high-refresh-topology-vue.json)、
[指针索引阶段诊断](performance-data/high-refresh-2026-09-05/high-refresh-vue-construction-boxed.json)、
[最终标准报告](performance-data/high-refresh-2026-09-05/high-refresh-boxed-vue.json)、
[最新门禁输出](performance-data/high-refresh-2026-09-05/high-refresh-boxed-gates.txt)。

```powershell
cargo build --release -p nana-ui-vue --features benchmark --bin nana-vue-runtime-benchmark --locked
target/release/nana-vue-runtime-benchmark.exe --profile-construction --output target/performance/high-refresh-vue-construction-boxed.json
target/release/nana-vue-runtime-benchmark.exe --output target/performance/high-refresh-boxed-vue.json
```


## 文档视口隔离与叶节点布局准备

视口依赖索引按 DocumentId 分桶，布局引擎忽略其他文档的 dirty ID，避免共享
UiWorld 下将外部固定定位布局岛按当前窗口视口重新布局。停放或删除节点同步清理索引。
普通叶节点在发布自身布局框、相对偏移和 used padding 后结束 placement，省去空的
子流收集、排序、网格和定位准备；Modal 的专用槽布局仍优先执行。

新增 `--profile-layout` 复用标准 5000 节点 fixture 和真实布局入口，分开测量 tooltip、
布局引擎、写回、滚动指标发布。生产入口没有这些分阶段时钟。预热 100 次，采样 1000 次，
宽度交替 1280/1024，高度 800；这是 Issue #8 固定迭代诊断，不代替 60 秒呈现验收。

| 阶段 | 修改前 P95 / ms | 叶节点优化后 P95 / ms |
|---|---:|---:|
| 布局引擎 | 14.781 | 13.137 |
| 写回 | 1.110 | 1.089 |
| 滚动指标 | 0.430 | 0.455 |

标准完整布局 P50 / P95 / P99 为 **10.538 / 15.022 / 16.180 ms**，仍未通过
8 ms 门禁。分阶段百分位不能相加；工作区存在并行任务，不宣称严格隔离的收益百分比。
门禁组合仅替换本轮 framework 报告，沿用前轮 runtime/vue/scene 报告，仍有 Runtime
首次系统处理与完整布局两项失败。滚动指标全树扫描和共享布局缓存的跨文档失效仍待优化。

Runtime 全量 **811 项通过**；Runtime 库与两个 benchmark 的严格 Clippy 通过。
真实 GPU Agent 的百万项列表保留与冻结表格两项测试通过，截图已打开检查。
新增叶节点回归覆盖隔离布局、相对定位与父尺寸变化后的百分比 padding 写回。

[优化前阶段报告](performance-data/high-refresh-2026-09-05/high-refresh-layout-stages.json)、
[优化后阶段报告](performance-data/high-refresh-2026-09-05/high-refresh-leaf-layout-stages.json)、
[完整布局报告](performance-data/high-refresh-2026-09-05/high-refresh-leaf-framework.json)、
[门禁输出](performance-data/high-refresh-2026-09-05/high-refresh-leaf-gates.txt)、
[Runtime 测试](performance-data/high-refresh-2026-09-05/high-refresh-leaf-tests.txt)、
[GPU Agent 测试](performance-data/high-refresh-2026-09-05/high-refresh-leaf-agent-tests.txt)。

```powershell
cargo test -p nana-ui-runtime --lib --locked
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-framework-benchmark --locked
target/release/nana-framework-benchmark.exe --profile-layout --output target/performance/high-refresh-leaf-layout-stages.json
target/release/nana-framework-benchmark.exe --list-overscan-px 160 --table-overscan-y-px 160 --output target/performance/high-refresh-leaf-framework.json
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```


## 多文档布局缓存隔离与释放

`RetainedLayoutCache` 改为每文档保留独立状态，全量布局仅重建当前文档缓存。
回归验证先布局第二文档，再全量布局第一文档，随后第二文档静态固定尺寸隔离容器的
局部更新只发布该容器。低层 `remove_document` 清理指定文档，空文档布局不创建缓存桶。
AppContext 的成功删除、停放及 detach 事务，在最后一个文档根节点离开后自动释放缓存；
停放后重新挂载的控件重新获得正确的布局与 used padding。

**Runtime 812 项通过，Runtime 库及两份 benchmark 严格 Clippy 通过，真实 GPU Agent
两项通过**，列表与冻结表格截图已打开检查。本轮没有重新采样时间门禁，不将上一轮
布局数值作为当前提交的重新实测结果；Issue #8 和 120Hz 仍未全量通过。
滚动指标发布的全树扫描及完整内存压力矩阵仍未完成。

[Runtime 回归](performance-data/high-refresh-2026-09-05/high-refresh-layout-documents-tests.txt)、
[Clippy](performance-data/high-refresh-2026-09-05/high-refresh-layout-documents-clippy.txt)、
[GPU Agent](performance-data/high-refresh-2026-09-05/high-refresh-layout-documents-agent.txt)。

```powershell
cargo test -p nana-ui-runtime --lib --locked
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```


## 历史测量约束失效与容量边界

复现：自动高度行的内部标签从 20px 增高到 35px，在另一视口尺寸完成局部布局后，
切回旧视口会错误复用 20px 的历史测量，连带恢复旧的兄弟行位置。旧实现只覆盖本次
测量的约束，其他宽高组合仍存活。修复后按节点清理所有受影响的历史约束，失效范围
仍由真实祖先依赖及合法布局隔离决定。

保留测量缓存每节点最多两组约束，单次布局内部仍保存全部测量结果。256 次连续缩放
逐次与全量布局比较一致，同时验证变体数不超过节点数两倍。缓存淘汰仅触发重新测量。
`remove_node(document, id)` 直接清理已删除节点的布局、测量、隔离位置和 padding，
AppContext 的删除事务自动调用；不扫描无关节点。删除后的容量压力完整矩阵仍待验收。

Runtime **815 项通过**，严格 Clippy 通过，真实 GPU Agent 列表/冻结表格 **2 项通过**，
截图已打开检查。修复前失败日志与修复后通过日志均保留。

本轮重新构建 release，串行运行阶段诊断和标准 framework 基准。布局引擎阶段 P95
**13.144 ms**，标准完整布局 P50 / P95 / P99 **11.124 / 15.940 / 18.361 ms**。
完整布局 P95 高于前轮 15.022 ms，仍不通过 8 ms 门禁；不能宣称本次修复带来了时间收益。
构建期间观察到其他任务编译；完整基准运行中一次 rustc 进程查询未返回进程，不能据此
证明整段采样没有外部干扰。记录绝对结果，不作为严格隔离 A/B。
门禁只替换本轮 framework 报告，其他三份沿用前轮，仍有 Runtime 首次系统处理与全量
布局两项失败，阈值不变。

[修复前行为失败](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-regression-before.txt)、
[Runtime 回归](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-tests.txt)、
[Clippy](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-clippy.txt)、
[GPU Agent](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-agent.txt)、
[阶段诊断](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-layout-stages.json)、
[完整报告](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-framework.json)、
[门禁输出](performance-data/high-refresh-2026-09-05/high-refresh-intrinsic-gates.txt)。

```powershell
cargo test -p nana-ui-runtime --lib --locked
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-framework-benchmark --locked
target/release/nana-framework-benchmark.exe --profile-layout --output target/performance/high-refresh-intrinsic-layout-stages.json
target/release/nana-framework-benchmark.exe --list-overscan-px 160 --table-overscan-y-px 160 --output target/performance/high-refresh-intrinsic-framework.json
```


## 局部布局的滚动指标目标选择

局部布局结束后，按实际重排节点的祖先闭包选择 ScrollView，重复目标合并。
只有多个目标需要恢复文档顺序时才遍历相关祖先分支，不进入不含目标的内容子树；
单目标直接发布。全量布局仍执行原有完整文档发布。内容缩小后的偏移钳制和嵌套顺序
保持，延迟锚点恢复显式加入布局工作，避免样式未变时遗漏恢复。

**Runtime 817 项、严格 Clippy、真实 GPU Agent 2 项均通过**，截图已打开检查。
新增回归覆盖 1000 个无关节点、重复重排、跨文档、嵌套与非 ID 顺序、停放、局部缩小
后的滚动钳制，以及锚点请求到布局完成的状态变化。

本轮未重新采样时间门禁，不宣称时间收益或 120Hz 已达标。受影响容器内部仍遍历后代
计算内容范围；多个目标排序时也可能检查相关祖先的直接子列表。尚未实现完整的增量
内容包围盒索引，不能把目标选择改进视为所有滚动工作量门禁已通过。

[Runtime 回归](performance-data/high-refresh-2026-09-05/high-refresh-scoped-scroll-tests.txt)、
[Clippy](performance-data/high-refresh-2026-09-05/high-refresh-scoped-scroll-clippy.txt)、
[GPU Agent](performance-data/high-refresh-2026-09-05/high-refresh-scoped-scroll-agent.txt)。

```powershell
cargo test -p nana-ui-runtime --lib --locked
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```


## 滚动内容的增量布局边界

UiWorld 新增惰性内容边界索引，保留每个已查询节点的子节点最大右边界和下边界。
子节点聚合采用最大值树，普通布局写回使当前节点及祖先路径失效；查询只刷新脏分支，
单点变化以对数复杂度更新父级聚合。首次查询需要构建子树索引，结构变更重建对应父级
聚合，不能把结构变化成本描述为固定 O(1)。没有滚动范围查询的文档不构建内容索引。

写回、display:none 切换、插入、重排、停放、detach 和删除均在真实 UiWorld mutation
入口处理，直接 world_mut 消费者也不会绕过失效。滚动偏移不改变布局范围，不使索引
失效。内容边界保持原有布局语义，不加入阴影、滤镜、绘制裁剪或滚动变换。

一万兄弟节点的末项从最大边界缩回后，仅刷新 **2 个节点**，更新 **不超过 16 个聚合
区间**；真实滚动偏移变化后的重复查询不增加刷新次数。显隐、重挂载、跨父级移动和
删除逐项对照原始后代遍历结果。两份各 256 次的增删回归验证缓存释放、跨文档复用，
以及旧父级一直不查询时，移走后删除的 ID 不残留在脏子节点集合中。

最终 **Runtime 821 项通过，严格 Clippy 通过，真实 GPU Agent 2 项通过**；列表和冻结
表格截图已打开检查。索引成本计数来自行为回归，本轮没有补充 release 60 秒时间、内存
峰值或结构压力测量，不能用这些工作量结果替代整个 120Hz 或内存压力门禁。

[Runtime 回归](performance-data/high-refresh-2026-09-05/high-refresh-scroll-bounds-tests.txt)、
[Clippy](performance-data/high-refresh-2026-09-05/high-refresh-scroll-bounds-clippy.txt)、
[GPU Agent](performance-data/high-refresh-2026-09-05/high-refresh-scroll-bounds-agent.txt)。

```powershell
cargo test -p nana-ui-runtime --lib --locked
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```


## 1万 / 5万 / 10万节点内容索引的 60 秒诊断

新增 `--profile-scroll-bounds`，每规模预热 2 秒、采样 60 秒，三个规模串行运行。
节点数包括一个文档根，其余为直接子节点；交替更新末项布局位置和文档滚动偏移。
这是 CPU 布局内容索引诊断，不运行文字整形、布局引擎、Scene、GPU 或呈现。
每轮测量之外暂停 1ms，限制样本存储和 CPU 占用；循环频率不是呈现刷新率。
时间包含诊断工作计数，提交和查询分别计时，不能把各自百分位相加。

第一轮诊断发现范围查询已保持常量工作，但纯滚动提交仍在验证时复制根节点完整子列表。
SetScrollOffset、SetScrollMetrics、WriteLayout 现使用事务中的存在性检查，不物化拓扑，
保留批内创建、删除和失败回滚语义。未查询文档也不再分配内容索引脏工作。

早期诊断的滚动命中更新队列未消费，最终循环已补齐 `take_scroll_hit_updates`；最终
JSON 明确带 `drains_scroll_hit_updates: true`。早期报告仅保留为问题定位证据，不能
与最终循环作严格 A/B。复测期间观察到其他任务编译，因此只报告绝对测量值。

| 保留节点 | 首次索引 / ms | 几何查询 P95 / ms | 纯滚动提交 P95 / ms | 纯滚动查询 P95 / ms | 几何刷新节点 / 最大区间更新 |
|---:|---:|---:|---:|---:|---:|
| 10000 | 3.3539 | 0.0019 | 0.0031 | 0.0004 | 2 / 15 |
| 50000 | 31.8648 | 0.0019 | 0.0030 | 0.0004 | 2 / 17 |
| 100000 | 50.7005 | 0.0020 | 0.0030 | 0.0005 | 2 / 18 |

最终每规模约 1.95万～1.98万次几何更新及同等数量的纯滚动样本。纯滚动三个规模的
索引刷新节点数与区间更新数均为 0；几何变化最多刷新 2 个节点。首次索引单列，其
开销仍随索引规模增长。一次中途进程观测得到截至该时刻的工作集峰值 125,513,728
字节（约 119.7MiB），它不是完整采样末端或完整 UI/GPU 的内存峰值验收。

Runtime **823 项通过**，严格 Clippy 通过，最终产品代码的真实 GPU Agent **2 项通过**，
截图已检查。D 盘空间不足时使用 E 盘独立构建目录；一次 E 盘输出报告设备未就绪，
确认命令终止后重试，最终测试通过。没有删除现有构建或用户文件。

本轮没有重跑 Issue #8 标准四报告，首次系统处理、全量布局及 120Hz 的完整门禁仍
未确认通过。浮层验证仍可能枚举已注册宿主；此处没有浮层，不能据此验证复杂浮层负载。

[最终 60 秒报告](performance-data/high-refresh-2026-09-05/high-refresh-bounds-scale-final.json)、
[进程中途观测](performance-data/high-refresh-2026-09-05/high-refresh-bounds-scale-final-memory-observation.json)、
[首轮问题诊断](performance-data/high-refresh-2026-09-05/high-refresh-bounds-scale.json)、
[存在性检查修复后的早期诊断](performance-data/high-refresh-2026-09-05/high-refresh-bounds-scale-scalar.json)、
[Runtime 回归](performance-data/high-refresh-2026-09-05/high-refresh-scalar-validation-tests.txt)、
[最终 Clippy](performance-data/high-refresh-2026-09-05/high-refresh-bounds-final-clippy.txt)、
[GPU Agent](performance-data/high-refresh-2026-09-05/high-refresh-bounds-scale-agent.txt)。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
E:/codex-build/nanaui-high-refresh/release/nana-framework-benchmark.exe --profile-scroll-bounds --seconds 60 --output target/performance/high-refresh-bounds-scale-final.json
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```


## 浮层反向依赖与文档内焦点校验

2026-09-05，新增工作量与行为回归覆盖 1000 个分别属于不同文档的浮层宿主：
普通文本、样式、滚动和几何更新不扫描浮层宿主、不为存在性校验复制子列表；修改一个
活动表面的语义只检查引用它的宿主。错误角色和分离活动表面的无效事务仍整体拒绝。
128 次创建/删除活动浮层后，反向引用不会累积；删除所有宿主后两类索引均释放。

焦点候选改为文档级浮层索引，暂存顺序从当前文档根及事务中的节点构建。新增回归验证
1000 文档下的模态范围、分离后的暂存/已提交顺序一致，以及重新插入恢复文档顺序。
多个模态表面的顺序竞争仍遍历当前文档；这些工作量证据不表示焦点查询已经完全增量化。

验证命令：

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```

真实 GPU Agent 的列表编辑保留、滚动命中及冻结表格点击测试 2 项通过，并检查了第
50 万项附近的两张截图。GPU-only 构建仍报告已有 NativeContentRenderer dead_code 警告。
这不是原生 OS IME 或 Surface 呈现间隔证据。本轮没有重新采集 60 秒计时；此前完整
布局和初始系统门禁未通过、物理 120Hz 呈现未验收的结论保持不变。

原始日志与截图存于 `performance-data/high-refresh-2026-09-05/` 中的
`high-refresh-overlay-*` 及 `overlay/`。

本轮最终 Runtime 回归为 827 项通过。删除节点的根索引清理也改为直接访问所属文档，
删除 1000 个宿主后逐文档验证其他根未丢失。未新增此删除路径的耗时结论。


## 2026-09-06：当前阶段诊断与无障碍变换权威

以浮层索引修复后的 release 构建重新运行阶段诊断：5000 节点首次处理的命中构建
P95 5.910 ms、无障碍投影 4.656 ms、抽取 4.651 ms；完整布局阶段 P95 36.575 ms。
这些是不同阶段的分位数，不能相加推导整体 P95。10000 节点样本与布局样本仍有明显
抖动，执行期间观察到其他任务的 rustc 进程，未达到隔离采样条件。原始诊断是当前热点
证据，不是完整四报告门禁复测、60 秒采样或严格前后 A/B。

继续审查发现无障碍投影优先读取命中索引中的旧变换。回归先证明：提交祖先平移
100px、尚未重建命中时，投影错误返回 x=20 而不是 120。现改为使用当前保留状态并
复用事务内祖先缓存；命中重建前后坐标一致，重建后中心点命中一致。已有索引时也不再
为每个无障碍节点重复查询命中祖先链。以上阶段诊断采于这项修复之前，本轮没有测得
这项修复的耗时收益。

验证命令：

```powershell
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-overlay-initial-stages.json
E:/codex-build/nanaui-high-refresh/release/nana-framework-benchmark.exe --profile-layout --output target/performance/high-refresh-overlay-layout-stages.json
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
```

828 项 Runtime 回归通过；真实 GPU Agent 2 项通过，检查了列表与冻结表格第 50 万项
附近的截图。GPU-only 构建仍有已有 NativeContentRenderer dead_code 警告。证据存于
[本轮目录](performance-data/high-refresh-2026-09-06/)，其中 `*-before.txt` 是预期失败
复现，不是最终验证结果。真实 OS IME、120Hz Surface 呈现及完整压力矩阵仍未验收。


## 全量命中索引直接构建

全量重建取消递归 HitEntry 中间树，前序构建数据直接进入平面条目索引，再自底向上
生成有序范围与包围盒。避免中间子树的分配/搬移、再次展平和单独递归计数。局部子树
替换保留现有路径。原树路径作为测试参照，比较多根、z 顺序、隐藏父级、裁剪与嵌套
滚动下全部条目的顺序和边界，并比较 1600 个位置的完整命中候选序列。

Runtime 829 项、严格 Clippy、真实 GPU Agent 2 项通过，已查看列表与冻结表格截图。
新 release 阶段诊断中 5000 节点命中构建 P95 **4.273 ms**；此前 5.910 ms 的诊断与本次
负载条件不完全一致，不能把差值解释为固定比例收益。10000 节点命中构建 P95 12.721 ms。

新标准 Runtime 报告的 5000 节点首次系统处理 P50 **13.892 ms**、P95 **15.445 ms**，
仍未通过 8 ms 门禁；局部绘制工作为 1 个节点。报告中的 allocations 是既有工作计数，
并非全线程分配器测量。本次继续沿用上一轮 Framework/Vue/Scene 报告复查，仍失败于
首次系统处理和完整布局两项；这不是四份报告的同版本、隔离环境重采样。

```powershell
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-direct-hit-stages.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-direct-hit-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-direct-hit-runtime.json --framework target/performance/high-refresh-intrinsic-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

构建、测试和 Clippy 命令同上一节；日志、JSON 与门禁输出归档于
`performance-data/high-refresh-2026-09-06/high-refresh-direct-hit-*`，截图位于该目录
`direct-hit/`。完整 60 秒负载矩阵、120Hz 呈现与内存压力验收仍未通过。


## GraphCanvas 图元身份与可选消费者门控

发现并复现跨类别 slot 重叠：各 300 个节点、边、端口及标签，加背景/网格/分隔线，
应有 1803 图元，旧实现只保留 430。采用互不重叠的类别高位和项序号低位，保留绘制
类别顺序。新回归在 300→2→300→0 项更新后验证所有 ID、总数与文字数量，再删除节点
验证完全清理。没有新增图形数据容器，也没有将此行为修复解释为百万画布支持。

Scene 的 controls/graph-canvas 配置通过 97 项库测试、2 项文档集成测试和 5 项侧栏
集成测试；对应 all-targets 严格 Clippy 通过。真实 GPU 6 项 GraphCanvas 边线测试通过，
覆盖轮廓/端点、MSAA、祖先裁剪、按线段数上传和相同实例复用。这不等同于 1803 图元
完整截图的 GPU 验收，本轮未采集新的计时报告。

```powershell
cargo test -p nana-ui-scene --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo test -p nana-ui-scene --test sidebar_row_label_paints_once --features controls --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo test -p nana-ui-scene --features graph-canvas,controls --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-scene --all-targets --features graph-canvas,controls --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui --lib --features gpu,graph-canvas --locked graph_canvas_stroke -- --test-threads=1
```

侧栏测试 target 现在声明 controls 的 required-features，最小 Scene 构建的 95 项库测试
和 2 项集成测试通过，启用 controls 后仍运行全部 5 项侧栏测试。未删除/放宽侧栏断言。
另有 `cargo check -p nana-ui-scene --tests --features rich-text --locked --target-dir E:/codex-build/nanaui-high-refresh`
仍失败：旧 Markdown 测试要求已移除的 blocks/drawing 字段及 drawing 方法，与当前
宿主负责公式/图表绘制的合同不一致。该迁移缺口尚未解决，不能报告整个工作区通过。

原始输出位于 `performance-data/high-refresh-2026-09-06/high-refresh-graph-slots-*` 和
`high-refresh-scene-*`。`graph-slots-before` 与 `scene-consumer-check` 为失败证据，其余为
修复后选定范围的验证。完整性能门禁与 120Hz 验收结论保持不变。


### GraphCanvas 大规模真实 GPU 像素回归

补充 `graph_canvas_large_families_survive_gpu_updates_and_shrink`：在 96×1200 离屏目标上，
复用 Scene、painter 和目标纹理，连续渲染 300→2→300→0 行。每行分别绘制红色边线、
绿色节点框和白色端口；逐帧检查全部 300 行的三个位置，累计 3600 个像素采样断言。
已移除的行必须恢复蓝色背景，避免静态数据或 GPU 缓存留下旧图元。测试通过，宿主库
hosted/graph-canvas 严格 Clippy 通过，300、2、0 三张截图已查看。

此项证明 900 个可见图形元素能跨数量变化正确绘制和清理。测试中的节点标签为空，
不覆盖 1803 图元夹带复杂文字的完整截图，也不是 1080p、60 秒或 120Hz 性能验收。
读回和可选 PNG 导出仅在测试路径，产品 Surface 没有新增 CPU 读回。

```powershell
$env:NANA_GRAPH_SCALE_SNAPSHOTS = 'D:/PROJECT/workspace/sena-nana/NanaUI/target/performance/graph-scale'
cargo test -p nana-ui --lib --features gpu,graph-canvas --locked graph_canvas_large_families_survive_gpu_updates_and_shrink -- --test-threads=1
cargo clippy -p nana-ui --lib --features hosted,graph-canvas --locked --no-deps -- -D warnings
```

日志归档为 `performance-data/high-refresh-2026-09-06/high-refresh-graph-scale-*`；
截图位于该目录的 `graph-scale/graph-300.png`、`graph-2.png`、`graph-0.png`。


## Markdown 合同迁移与工作区目标检查

迁移两项过期 Markdown Scene 测试，使用 RuntimeDocument 完整 flush：300 段内容必须
完整出现在一份文本投影中，删除视图后 Scene 清理干净；公式与 mermaid 的宿主 presenter
槽必须传递正确内容，重复组装保持身份，隐藏槽不再重复绘制文本。当前合同没有内建
公式/mermaid SVG，测试不再要求已移除的 blocks/drawing 绘制字段。大于 256 图元的
身份/清理断言由前述 1803 图元 GraphCanvas 回归独立承担，而非仅降低原计数要求。

Scene components 配置通过 101 项库测试、2 项 RuntimeDocument 集成测试、5 项侧栏
集成测试及 all-targets 严格 Clippy。全工作区目标检查继续暴露两处窗口消费者未迁移：
runtime-host-fixture 与 window-chrome-multi-window 现显式补齐 focus_on_show=true、
constrain_to_work_area=false，与 WindowSettings::new 的默认值一致。

```powershell
cargo test -p nana-ui-scene --features components --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-scene --all-targets --features components --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo check --workspace --all-targets --locked
```

以上命令均通过。证据为 `performance-data/high-refresh-2026-09-06/` 下的
`high-refresh-markdown-migration-tests.txt`、`high-refresh-markdown-migration-clippy.txt`、
`high-refresh-workspace-current-check.txt`。这取代了此前 Markdown/窗口配置导致全目标
检查失败的状态，但不代表工作区全量测试、全部特性组合或 120Hz 性能验收已通过。


### 工作区全量测试与相同继承样式共享

Markdown/窗口配置消费者迁移后的 `cargo test --workspace --all-targets --locked -- --test-threads=1`
完成，39 个套件共 2793 项测试通过，未报告失败或忽略项。原始日志为
`performance-data/high-refresh-2026-09-06/high-refresh-workspace-current-tests.txt`。
该命令使用默认的工作区特性统一结果，不等同于所有特性组合；V8 相关目标仍报告已有
MSVC LNK4098 链接警告。这份结果采于后续样式共享修改之前。

随后优化相同继承结果的样式分配：子节点 ComputedStyle 与父级完全相同才共享父级 Arc。
本地 NodeStyle 仍独立保存；后续修改重新发布样式，不修改旧投影。1000 节点回归验证
一致结果共享，局部颜色覆盖不会污染其他节点，字号继承更新正确，旧快照仍保留旧值。
Runtime 830 项及 Runtime 库/benchmark 严格 Clippy 通过。没有引入全局样式驻留缓存，
不存在额外的全局缓存回收合同。分配减少的覆盖范围不等同于整体耗时达标。


样式共享后的工作区复查也已完成：同一全量命令运行 39 个套件、2794 项测试，0 失败、
0 忽略。样式共享的回归计入此轮。GPU 与消费者测试通过不代表性能验收通过。

在本任务的全量测试进程结束后采集 release 报告：5000 节点样式阶段 P95 2.423 ms，
首次系统处理标准报告 P50 15.852 ms、P95 **21.516 ms**。相较前一标准报告 15.445 ms
出现耗时退化；初始 commit P95 也从 16.218 ms 升至 24.614 ms，尽管此次产品改动位于
样式解析。不能据此排除实现影响，也不能将退化直接归因于环境；尚需更受控的同进程
对照。当前只能证明相同继承结果共享及行为正确，不能证明总体 CPU 收益。

继续结合沿用的 Framework/Vue/Scene 报告检查，首次系统与完整布局仍超过 8 ms 门槛。
没有申请或给予性能豁免，没有降低阈值，没有提交或合并。

```powershell
cargo test --workspace --all-targets --locked -- --test-threads=1
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-style-sharing-stages.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-style-sharing-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-style-sharing-runtime.json --framework target/performance/high-refresh-intrinsic-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

新日志与原始报告归档于 `performance-data/high-refresh-2026-09-06/high-refresh-style-sharing-*`
及 `high-refresh-workspace-style-sharing-tests.txt`。


### 同进程样式共享对照

增加 `nana-runtime-benchmark --profile-style-sharing`：每种规模先预热 10 对，再采样
60 对，同一进程交替 shared→unshared 与 unshared→shared。两种模式每次创建相同数据
的 UiWorld；构造/提交不计入系统耗时，共享与控制路径复用同一组系统阶段。产品路径
保持编译期共享，关闭共享的控制入口仅在 benchmark 特性中提供。1000 节点、含局部
字体/颜色覆盖的回归确认两条路径得到相同 ComputedStyle；benchmark 配置 Runtime
831 项测试及严格 Clippy 通过。

| 节点 | 共享 style P95 | 控制 style P95 | 共享总处理 P95 | 控制总处理 P95 | 配对差中位数 | 共享更快 |
|---|---:|---:|---:|---:|---:|---:|
| 5000 | 2.490 ms | 3.294 ms | 15.151 ms | 17.459 ms | −0.822 ms | 46/60 |
| 10000 | 5.834 ms | 7.001 ms | 40.163 ms | 41.530 ms | −2.396 ms | 49/60 |

配对差为同一对中 shared−unshared，原始 60 个差值写入 JSON。结果支持在该首次系统
负载中保留共享，不支持把前后两份独立报告的差异直接归因于共享造成退化，也没有
证明历史退化的全部原因。随机哈希布局和系统调度仍会影响每对样本；此诊断有阶段计时
开销、不是固定 60 秒采样，不替代标准报告和 8 ms 门禁，更不覆盖 GPU/呈现。

```powershell
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo test -p nana-ui-runtime --lib --features benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-style-sharing --output target/performance/high-refresh-style-paired.json
```

证据归档为 `performance-data/high-refresh-2026-09-06/high-refresh-style-paired*`。此前
21.516 ms 的标准报告仍保留，不用这份诊断替换或豁免失败门禁。


## 布局输入的单节点投影与预取

LayoutInputMap 原本每次读取先 contains 再 get，缓存未命中还通过 layout_inputs(&[id])
创建临时单元素 Vec。现使用 map entry 一次查找，并直接调用共用的单节点投影。
完整预取只在新建空映射时调用，直接使用传入的文档顺序，取消 missing ID 中间列表。
UiWorld 投影同时复用已取得的 NodeRecord，减少重复读取。公开批量 API 仍保持原行为。

回归先确认旧路径增加一次投影批量分配事件，再验证新路径不产生该事件、重复读取不
重复物化、缺失节点仍返回 None。此计数只覆盖投影输出容器，不声称整个 map 零分配。
Runtime 831 项、严格 Clippy、真实 GPU Agent 两项通过，列表/冻结表格截图已查看。

重新串行运行标准 Framework 与 Runtime 报告：5000 节点完整布局 P50 16.263 ms、
P95 **29.723 ms**；首次系统处理 P50 13.675 ms、P95 **18.940 ms**。这不是严格 A/B，
没有以局部工作量下降推断整体速度收益。两项 8 ms 门禁仍失败，未豁免或放宽。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
cargo build --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-framework-benchmark.exe --list-overscan-px 160 --table-overscan-y-px 160 --output target/performance/high-refresh-layout-input-framework.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-layout-input-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-layout-input-runtime.json --framework target/performance/high-refresh-layout-input-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

Vue/Scene 报告沿用此前版本，不是四报告同版本重采样。原始输出归档为
`performance-data/high-refresh-2026-09-06/high-refresh-layout-input-*`；截图位于该目录
`layout-input/`。工作区 2794 项通过的报告早于此改动，本轮验证范围以以上命令为准。

## 布局缓存 HashMap 对照

布局引擎内部的数字 ID / 约束键缓存改用已在 lockfile 中存在的 hashbrown 0.16.1；
UiWorld 节点存储、公共 API、持久化和 HashSet 不变。没有删除参与布局判断的 padding
缓存，也没有改变缓存失效与文档隔离合同。Runtime 832 项、严格 Clippy 和真实 GPU
虚拟列表/冻结表格交互 2 项通过，截图已查看。

保留改动前的 std-map release 程序，完成构建后串行按 std→hashbrown→hashbrown→std
运行 `--profile-layout`。每轮预热 100 次、采样 1000 次，相同 5000 节点缩放负载：

| 顺序 | 缓存 | layout_engine P50 | layout_engine P95 |
|---|---|---:|---:|
| 0 | std | 9.347 ms | 12.735 ms |
| 1 | hashbrown | 6.207 ms | 8.267 ms |
| 2 | hashbrown | 6.505 ms | 8.806 ms |
| 3 | std | 10.202 ms | 14.157 ms |

结果支持保留此替换，但两个二进制来自共享工作区的不同构建时点，并非隔离检出的严格
A/B；机器调度和其他任务影响没有完全排除。二进制 SHA256 和四份原始输出已归档。
这是阶段诊断，不是 60 秒完整应用或 GPU/呈现验收。

随后串行运行标准 Framework、Runtime：完整布局 P50 **8.079 ms**、P95 **11.575 ms**；
首次系统处理 P50 **13.747 ms**、P95 **18.149 ms**。两项 8 ms 门禁仍失败，未放宽阈值。
门禁沿用此前 Vue/Scene 报告，因此不是四份报告同版本重采样。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test virtual_retention --locked -- --ignored --test-threads=1
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-framework-benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-framework-benchmark.exe --profile-layout --output target/performance/high-refresh-layout-map-1-hashbrown.json
E:/codex-build/nanaui-high-refresh/release/nana-framework-benchmark.exe --list-overscan-px 160 --table-overscan-y-px 160 --output target/performance/high-refresh-layout-map-framework.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-layout-map-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-layout-map-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

本轮原始输出归档于 `performance-data/high-refresh-2026-09-06/high-refresh-layout-map-*`，
截图位于该目录 `layout-map/`。此前失败报告保留；完整目标尚未验收。

## 隐藏祖先的局部命中范围

本节记录中间方案；下节的结构条目方案已替换祖先范围扩张，原始证据保留。

新增回归复现：隐藏父节点下显式 visible 的孩子移动后，scoped hit 更新返回成功，但旧
位置仍能命中。隐藏容器没有自己的索引条目，其可见后代实际属于祖先的子范围；现先
提升到能完整替换该范围的祖先，并合并重复范围。隐藏文档根存在后代时请求全量回退。
普通可见叶节点仍局部替换。隐藏层级可能扩大更新到整个可见祖先子树，这是当前成本
限制，不声称这些情况已满足全部增量工作量门禁。

回归覆盖孩子几何、隐藏父级变换、隐藏文档根、重复隐藏/显示后无重复命中，与完整
重建结果对照。Runtime 833 项通过；Runtime 严格 Clippy 通过。新增真实 GPU Agent
测试确认编辑框平移 160px 后绘制位置、旧位置排除和点击焦点一致，前后截图已查看；
两项百万数据虚拟列表/冻结表格交互回归也通过。

Devtools 严格 Clippy 另暴露旧快照通道的常量 chunks_exact 用法，迁移为 as_chunks::<4>
并保留同样的 BGRA→RGBA 顺序。Devtools 自身检查通过；其 nana-ui 依赖在该特性组合下
仍报告 NativeContentRenderer 未构造警告，不把这次检查表述为整个工作区零警告。

重建 release Runtime 后标准首次系统 P95 **16.778 ms**，仍未达到 8 ms。Framework
沿用本轮缓存替换报告（11.575 ms），Vue/Scene 沿用此前报告；两项门禁仍失败。
不将独立运行的首次系统波动归因于此次主要影响 scoped hit 的修复。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test hidden_hit --test virtual_retention --locked -- --ignored --test-threads=1
cargo clippy -p nana-ui-devtools --features agent --test hidden_hit --locked --no-deps -- -D warnings
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-hidden-hit-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-hidden-hit-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

原始失败、测试、构建和性能输出归档为
`performance-data/high-refresh-2026-09-06/high-refresh-hidden-hit-*`，截图位于该目录
`hidden-hit/`。整体目标仍有未完成项，真实 120Hz 呈现和完整复杂 UI 矩阵尚未验收。

## 隐藏容器保留结构命中条目

新增回归确认被省略的隐藏容器还会丢失 overflow 裁剪，且普通滚动找不到容器条目而
跳过更新。现在只省略隐藏叶节点和 box_visible=false 子树；有后代的隐藏容器保留
结构条目，hittable=false。后代沿真实层级获得剪裁、滚动和顺序，不再提升到祖先。
对应移除上一节的范围扩张与隐藏根全量回退，直接构建也不再需要处理提升产生的序号
冲突。这里的条目属于命中投影，不产生隐藏容器的绘制或业务节点。

行为验证覆盖：屏外内容不能越过隐藏父级裁剪；后来的兄弟仍在前面；滚动后命中移动且
条目重建计数不增加；隐藏根下移动可见叶只重建 1 个条目，移动含两个孩子的隐藏容器
重建 3 个；重复显示/隐藏与完整重建结果一致。Runtime **834 项通过**，严格 Clippy 通过。

真实 GPU Agent **4 项通过**：原有平移、两项虚拟化，以及新增隐藏 ScrollView 下的
四个可见编辑器。64px 视口中截图先显示 Row 0/1，滚动 32px 后显示 Row 1/2，点击新
首行获得正确焦点；裁剪之外的行不能命中。两张新增截图已查看。Devtools 严格 Clippy
也通过，仍保留其 nana-ui 依赖的 NativeContentRenderer dead_code 警告说明。

标准 Runtime 首次系统 P50 **13.291 ms**、P95 **16.184 ms**，仍超过 8 ms。
Framework 沿用 11.575 ms 报告，Vue/Scene 沿用此前报告，两项门禁仍失败。
本轮不是固定 60 秒应用负载或严格 A/B；不据首次系统波动推断此命中修复的速度收益。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test hidden_hit --test virtual_retention --locked -- --ignored --test-threads=1
cargo clippy -p nana-ui-devtools --features agent --test hidden_hit --locked --no-deps -- -D warnings
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-hidden-proxy-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-hidden-proxy-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

失败复现及本轮输出位于 `performance-data/high-refresh-2026-09-06/high-refresh-hidden-proxy-*`，
截图位于该目录 `hidden-proxy/`。复杂层叠/滤镜全矩阵、无可命中后代的保留分支剪枝和
完整 120Hz 验收仍需继续，不将本轮行为覆盖外推到这些未完成项。

## 不可命中保留分支的范围剪枝

包围盒索引新增 Inactive：槽位被保留条目占用，但自身和后代均无命中贡献。它与已删除
的 Empty、可计算的 Known、保守保留的 Unknown 分开。合并与滚动平移保留这一区别，
否则不可命中容器会污染可命中后代的范围，或被当成空存储而丢失可重新启用的兄弟槽位。
自身不命中的容器仍合并后代范围；几何未知的可命中内容仍进入查询。

旧路径在 1 万个不可交互根上仍产生 1 万个范围候选，回归已复现；新路径为 0。
根级/嵌套的 1 万条目验证删除一个节点后其余条目仍保留，启用最后一个节点恢复正确
命中，再禁用后无候选，结果与完整重建一致。10 万 Inactive 槽位的范围查询只检查
顶层；启用一个 Unknown 条目后检查少于 40 个范围且保留候选，再删除后恢复顶层剪枝。
这些是行为与工作量证据，不是完整 UI 的时间验收。

Runtime **836 项通过**，严格 Clippy 通过；真实 GPU Agent **4 项通过**，隐藏滚动与
冻结表格截图已查看。Devtools 严格 Clippy 通过，其 nana-ui 依赖仍有此前记录的
NativeContentRenderer dead_code 警告。

标准首次系统 P50 **16.781 ms**、P95 **23.661 ms**，高于上一份 P95 16.184 ms。
原因尚未查明，保留失败报告，不以候选数量下降解释或豁免完整门禁。随后阶段诊断的
5000 节点 P95：accessibility 12.699 ms、extraction 7.971 ms、hit_test 9.854 ms、
layout_inputs 4.787 ms、style 6.293 ms。阶段诊断采样期间可观察到其他 cargo/rustc
进程；该观察不能反向证明更早标准报告的退化由它们造成，也不能将各阶段 P95 相加。
Framework/Vue/Scene 沿用此前报告，两项 8 ms 门禁仍失败。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test hidden_hit --test virtual_retention --locked -- --ignored --test-threads=1
cargo clippy -p nana-ui-devtools --features agent --test hidden_hit --locked --no-deps -- -D warnings
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-inactive-hit-runtime.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-inactive-hit-stages.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-inactive-hit-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

证据位于 `performance-data/high-refresh-2026-09-06/high-refresh-inactive-hit-*`，最后一轮
Runtime 全库结果为 `high-refresh-inactive-hit-tests-final.txt`（836 项），截图位于
`inactive-hit/`。保留 release 二进制的 SHA256 以便后续对照；本轮未达全部性能目标。

## 命中条目数字键映射对照

只将 HitIndex.entries 的数字 StableNodeId 映射改为 hashbrown 0.16.1，不改变节点存储、
命中合同或兄弟遍历顺序。836 项 Runtime、4 项真实 GPU Agent 和严格 Runtime Clippy
通过；隐藏滚动与百万数据列表截图已查看。

保留上一节 release 程序，按 std→hashbrown→hashbrown→std 串行运行
`--profile-initial-systems`；每种规模预热 10 次、采样 60 次：

| 轮次 | 映射 | 5000 hit P50 | 5000 hit P95 | 10000 hit P50 | 10000 hit P95 |
|---|---|---:|---:|---:|---:|
| 0 | std | 3.474 ms | 4.494 ms | 9.773 ms | 14.962 ms |
| 1 | hashbrown | 2.864 ms | 3.712 ms | 6.657 ms | 9.197 ms |
| 2 | hashbrown | 2.840 ms | 3.709 ms | 6.500 ms | 8.584 ms |
| 3 | std | 3.512 ms | 3.922 ms | 7.901 ms | 9.855 ms |

5000 节点样式阶段 P50 四轮约 2 ms，命中阶段下降支持保留替换。但采样期间仍观察到
其他 cargo/rustc，10000 节点第一轮其他阶段也更慢；这不是隔离机器的严格 A/B 或
60 秒完整 UI 验收，不能把第一轮所有下降归因于映射实现。轮次开始时的进程观察与
两个二进制 SHA256 已一并归档。

标准首次系统 P50 **12.571 ms**、P95 **16.692 ms**。两项 8 ms 门禁仍失败，
Framework/Vue/Scene 沿用此前报告；上一节 23.661 ms 的失败报告保留。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test hidden_hit --test virtual_retention --locked -- --ignored --test-threads=1
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-hit-map-1-hashbrown.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-hit-map-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-hit-map-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

证据为 `performance-data/high-refresh-2026-09-06/high-refresh-hit-map-*`，截图位于
`hit-map/`。整体目标仍未完成，下一阶段需要继续降低系统处理成本并补齐完整负载验收。

## 无障碍临时缓存对照与范围收敛

先试验 ProjectionMemo.transforms/bounds 与 AncestorMemo.live/stacking/color 共五个
数字键映射。首组交替对照中无障碍阶段下降，但共用祖先缓存对应的抽取/命中未显示
收益，10000 节点下略慢。因此撤回 AncestorMemo 的三个替换，最终仅保留无障碍缓存。
没有改变变换、裁剪、语义投影算法或缓存生命周期。

最终组合的第二组 std→hashbrown→hashbrown→std 对照如下，每种规模预热 10 次、
采样 60 次。std 程序来自上一节命中映射完成后的保留 release 二进制：

| 轮次 | 无障碍缓存 | 5000 P50 | 5000 P95 | 10000 P50 | 10000 P95 |
|---|---|---:|---:|---:|---:|
| 0 | std | 4.773 ms | 7.139 ms | 11.823 ms | 20.160 ms |
| 1 | hashbrown | 5.906 ms | 11.351 ms | 7.045 ms | 10.223 ms |
| 2 | hashbrown | 2.773 ms | 3.389 ms | 7.062 ms | 11.745 ms |
| 3 | std | 3.230 ms | 4.004 ms | 7.767 ms | 11.616 ms |

前半段波动明显，未丢弃慢样本或只选后两轮。采样期间有其他 cargo 进程；不能把这组
测量视为隔离机器的严格 A/B。结合首组无障碍阶段的下降保留两个缓存替换，但收益幅度
仍需受控测量确认，尤其不能将阶段结果外推为完整系统性能提升。

最终 Runtime **836 项通过**，严格 Runtime Clippy 通过；真实 GPU Agent **4 项通过**，
隐藏滚动与冻结表格截图已查看。标准首次处理 P50 **15.584 ms**、P95 **17.712 ms**，
仍未达 8 ms，且高于上一份 16.692 ms。Framework/Vue/Scene 沿用此前报告，两项门禁
继续失败，没有豁免。试验中间版本的标准报告 16.212 ms 不作为最终版本成绩。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo test -p nana-ui-devtools --features agent --test hidden_hit --test virtual_retention --locked -- --ignored --test-threads=1
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --profile-initial-systems --output target/performance/high-refresh-projection-map-final-1-hashbrown.json
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-projection-map-final-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-projection-map-final-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

所有试验与最终结果归档为 `performance-data/high-refresh-2026-09-06/high-refresh-projection-map-*`，
其中 `final-*` 对应只替换两个无障碍缓存的最终组合；截图位于 `projection-map/`。
二进制 SHA256 与轮次开始时的进程观察同时保留，整体目标尚未完成。

## 隐藏祖先的无障碍连接修复

回归复现可见按钮引用了被省略的隐藏父节点，导致无障碍投影缺少完整祖先链。
当前保留隐藏容器的中性结构节点，但清除其自身标签、值、描述、原角色、状态及
交互语义；隐藏叶与不生成布局盒的子树仍省略。可见孩子保持原身份、语义和焦点。

验证结果：

- Runtime 全库 **837 项通过**；补充孩子独立隐藏/显示的定向回归通过。隐藏根、隐藏
  父级、角色恢复与增量节点缓存均和完整投影一致。
- AccessKit 集成 **1 项通过**：GenericContainer 的连接、无标签/值、无点击/聚焦
  操作，可见按钮仍可点击，反复隐藏/显示与完整投影一致，删除子树后只保留文档根。
- 真实 GPU Agent **4 项通过**；隐藏滚动和祖先平移测试增加点击后无障碍祖先链及
  焦点断言，截图已查看。
- Runtime、AccessKit 集成目标与 Agent 测试目标的严格 Clippy 均通过；Agent 的
  nana-ui 依赖仍报告此前的 NativeContentRenderer dead_code 警告。

这些验证包含真实 GPU 和 AccessKit 投影，尚不是 Windows UIA / 屏幕阅读器实际会话
证据。标准首次系统 P50 **13.067 ms**、P95 **16.484 ms**，仍未达 8 ms；Framework/
Vue/Scene 沿用此前报告，两项门禁继续失败。该独立运行不证明本次正确性修复的速度收益。

```powershell
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo test -p nana-ui-runtime --lib --locked --target-dir E:/codex-build/nanaui-high-refresh hidden_accessibility_containers_keep_visible_descendants_connected
cargo test -p nana-ui --features accesskit-tree --test accessibility_hidden --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo test -p nana-ui-devtools --features agent --test hidden_hit --test virtual_retention --locked -- --ignored --test-threads=1
cargo clippy -p nana-ui-runtime --lib --features benchmark --bin nana-runtime-benchmark --bin nana-framework-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo clippy -p nana-ui --features accesskit-tree --test accessibility_hidden --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo clippy -p nana-ui-devtools --features agent --test hidden_hit --locked --no-deps -- -D warnings
cargo build -p nana-ui-runtime --release --features benchmark --bin nana-runtime-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-runtime-benchmark.exe --output target/performance/high-refresh-hidden-a11y-runtime.json
python scripts/validate-runtime-performance.py --runtime target/performance/high-refresh-hidden-a11y-runtime.json --framework target/performance/high-refresh-layout-map-framework.json --vue target/performance/high-refresh-boxed-vue.json --scene target/performance/high-refresh-latest-scene.json
```

失败复现及结果归档于 `performance-data/high-refresh-2026-09-06/high-refresh-hidden-a11y-*`，
截图位于 `hidden-a11y/`。公共投影合同同步写入 `application-api.md`。

### Windows 原生无障碍窗口根（2026-09-06）

新增 `accessibility-hidden-probe` 与 `scripts/validate-hidden-accessibility.ps1`，使用
真实 hosted Window/Surface，通过 Windows UI Automation 发现、聚焦、写值及切换
隐藏父容器。仅操作和关闭探针自身；截图在外部脚本进行，不向产品呈现加入 CPU 回读。

原生验证发现了 Runtime/AccessKit 数据单测之外的问题：Generic 窗口根在编辑器获得
焦点后被平台过滤。修复前 Runtime 三节点链完整，但原生枚举为空；修复后 Window 根
稳定，编辑后仍可枚举 Edit，真实截图显示 UIA 写入的值和焦点边框。原生宿主的窗口包装
不改变嵌入式 AccessTreeProjector 合同。根稳定、空树、文档替换及焦点回归共 **22 项
通过**；hosted lib/example 严格 Clippy 通过。这些结果早于共享工作区随后发生的文本
snippet 改动，后续构建结果分别记录。

另外移除了 RuntimeProgramContext 派生 Clone 引入的 Message: Clone 约束。
move-only 消息回归 **1 项通过**；独立管道验证首帧后 dispatch 可以显示容器并正常
退出。第一次原生脚本误选 winit 内部消息窗口，已改为等待产品窗口标题；另有运行未
取得系统前台焦点，这些失败不计为通过。完整脚本和实际屏幕阅读器会话仍分别验收。

```powershell
cargo test -p nana-ui --lib --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh context_clone_accepts_move_only_messages
cargo test -p nana-ui --lib --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh accessibility::tests
cargo clippy -p nana-ui --lib --example accessibility-hidden-probe --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
cargo build -p nana-ui --example accessibility-hidden-probe --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh
powershell.exe -NoProfile -Mta -ExecutionPolicy Bypass -File scripts/validate-hidden-accessibility.ps1 -ExePath E:/codex-build/nanaui-high-refresh/debug/examples/accessibility-hidden-probe.exe
```

证据位于 `performance-data/high-refresh-2026-09-06/high-refresh-native-a11y-*`。
本项是原生正确性修复，未重测完整性能矩阵；此前两项 8 ms 门禁仍失败，不宣称 120Hz
验收通过。实际 Narrator/IME 候选窗、120Hz 硬件呈现与完整复杂 UI 矩阵仍待完成。

后续探针重建完成（6m48s，期间存在共享工作区编译）。`high-refresh-native-a11y-protocol.json`
记录原生隐藏→显示→隐藏成功且退出码为 0；stderr 确认首条命令含 UTF-8 BOM，探针
已显式兼容。该次完整脚本仍因系统未给予前台焦点而失败，不能计为完整通过。
之前编辑后原生树与截图的成功证据独立保留，不与此次部分结果拼接为一轮全通过。

此阶段尚有 HostedAccessibility 每次语义更新构造完整 current_tree，以及
AccessibilityProjector 的全树可达性、文本运行 ID 和焦点扫描。后续优化见下一节。

### 无障碍纯语义增量与惰性激活快照（2026-09-06）

HostedAccessibility 与激活处理器共享同一保留投影，取消每帧额外构造的完整
TreeUpdate。只有平台请求激活快照时才展开完整树；更新锁在原生事件回调前释放。
纯语义更新复用拓扑、根和文本运行 ID，跳过全树重整及父级孩子列表查找，焦点按
稳定 ID 增量维护。结构/文本输入角色变化仍保留全量检查，尚未完成结构变化的全部增量化。

- 无障碍库回归 **24 项通过**；万节点连续编辑只输出编辑器及文本运行，完整快照
  计数为 0，初始化后全树重整计数不增加。再次激活返回最新完整树；删除清理文本
  运行与焦点，旧激活快照保持独立。焦点转移与父子整树删除回退也通过。
- 公共 AccessKit 集成 **1 项通过**，包括隐藏祖先连接、角色恢复、增量/完整投影
  一致及子树删除。
- Windows UIA **Semantics 模式通过**：发现、隐藏/显示、非聚焦写值、编辑保留、
  Runtime 呈现回报及正常退出。该模式不测试系统前台焦点、不取截图；默认 Full 模式
  的焦点与截图要求保留，不能用此结果替代 Full 或 Narrator 验收。
- hosted lib、探针及新增诊断程序的严格 Clippy 通过。

新增 `nana-a11y-benchmark` 测量公开 AccessTreeProjector 的局部转换：每种规模预热
100 次、串行采样 2000 次，交替修改末尾编辑器的值和焦点，检测大父级的线性孩子查找。
输入 delta 构造、初始化、Runtime 布局、原生 UIA 事件和呈现均不计入该时间。

| 保留语义节点 | 每次输出节点 | P50 / 微秒 | P95 / 微秒 | P99 / 微秒 |
| --- | --- | --- | --- | --- |
| 10,000 | 2 | 1.0 | 1.3 | 1.3 |
| 50,000 | 2 | 0.9 | 1.0 | 1.0 |
| 100,000 | 2 | 0.9 | 1.0 | 1.0 |

这些是 release 的独立转换诊断，没有旧版同机计时对照，运行时仍有共享工作区构建。
不能将其解释为端到端 CPU/GPU P95、60 秒完整负载矩阵或 120Hz 呈现验收。此前
Runtime 首次处理和完整布局两项 8ms 门禁继续保持失败状态。

```powershell
cargo test -p nana-ui --lib --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh accessibility::tests
cargo test -p nana-ui --features accesskit-tree --test accessibility_hidden --locked --target-dir E:/codex-build/nanaui-high-refresh
cargo build -p nana-ui --release --features accesskit-tree --bin nana-a11y-benchmark --locked --target-dir E:/codex-build/nanaui-high-refresh
E:/codex-build/nanaui-high-refresh/release/nana-a11y-benchmark.exe
cargo rustc -p nana-ui --example accessibility-hidden-probe --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh -- -C debuginfo=0
powershell.exe -NoProfile -Mta -ExecutionPolicy Bypass -File scripts/validate-hidden-accessibility.ps1 -ExePath E:/codex-build/nanaui-high-refresh/debug/examples/accessibility-hidden-probe.exe -Mode Semantics
cargo clippy -p nana-ui --lib --bin nana-a11y-benchmark --example accessibility-hidden-probe --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
```

证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-incremental-a11y-*`。

### 2026-09-06：失败帧恢复与无障碍批次保留

本轮验证纠正了三个实际问题：未呈现增量被后续批次覆盖；直接丢弃已获取帧后下一次
Surface 获取停滞；静态 Text 在 Runtime 有标签、Windows UIA Name 却为空。
目标级待发布状态保留单批增量，多批仅标记成功呈现时生成最终快照；发布先于应用
presented/bind 回调。失败编码释放 encoder/view/frame，下一次获取仅恢复该 Surface。
静态文字使用 AccessKit Label.value，普通控件保留独立 label/value 合同。

原生探针在第一帧成功后提交三批独立变化：新增文字 → 修改编辑器名称 → 修改视口
语义。前两帧由资源生产者主动返回错误，第三帧正常提交，避免空闲恢复的全量回退
掩盖增量丢失。Windows Vulkan 和 DX12 的 UIA Semantics 模式均通过：

- 三批最终状态全部可枚举；恢复后的写值、隐藏/显示与值保留、正常退出均通过。
- 恰好记录两次生产失败；失败阶段没有 producer submitted 或应用 presented 成功回调。
- 首次失败诊断记录确认恢复消息已处理、下一帧已 prepare，但没有进入第二次编码；
  Surface 恢复后这条停滞消失。中间探针错误使用 HelpText 查询 description，已改用
  名称变化验证第二批事务；该次失败没有被记为产品验收通过。

27 项无障碍/待发布队列测试、41 项 scene_host 测试、6 项 hosted_context 测试与严格
Clippy 通过。过滤集存在重叠，不直接相加。默认并行的全量宿主测试长时间无进展，
已主动停止并保留失败日志；串行运行另行记录，不能把中途结果算成全量通过。
随后同一测试构建串行完成：434 项全部通过，耗时 127.22 秒；这是功能测试时间，
不是 UI 帧耗时，也不证明默认并行运行停滞的原因已经查明。
Hosted GPU Demo 的 debug 短时持续呈现也正常退出，无 stderr 错误。Vulkan / RTX 5060
采样约 2 秒、234 个回调间隔，P95 为 9.009 ms，超 8.33ms 比例约 87.61%，报告的
interval_gate_passed 为 false。该次只作正常路径冒烟检查，不作为 release、60 秒、
1080p 或真实 120Hz 扫描输出证据，也不改变既有性能门禁状态。

以上为恢复和语义正确性证据，不含前台焦点、Narrator、辅助窗口失败、DComp commit
失败，也不是 60 秒负载矩阵或刷新率验收；两项既有 8ms 门禁仍保持失败状态。

```powershell
cargo test -p nana-ui --lib --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh accessibility:: -- --test-threads=1
cargo rustc -p nana-ui --example accessibility-hidden-probe --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh -- -C debuginfo=0
powershell.exe -NoProfile -Mta -ExecutionPolicy Bypass -File scripts/validate-hidden-accessibility.ps1 -ExePath E:/codex-build/nanaui-high-refresh/debug/examples/accessibility-hidden-probe.exe -Mode Semantics -Retry
# Repeat in a fresh shell with WGPU_BACKEND=dx12; each report includes the actual adapter backend.
cargo clippy -p nana-ui --lib --example accessibility-hidden-probe --example hosted-gpu-demo --features hosted,bundled-fonts --locked --target-dir E:/codex-build/nanaui-high-refresh --no-deps -- -D warnings
```

证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-a11y-retry-*`。

### 2026-09-06：首次成功呈现前不发布文档语义

基线探针先在应用 build 中 flush 文档，再令首次资源编码失败。Windows Vulkan
报告只有 prepare / producer_failed 阶段，没有成功呈现，却已能在 UIA 枚举
Visible editor 和一个 Image 节点。此问题来自主窗口/辅助窗口初始化时提前调用
accessibility_snapshot，而不是增量恢复队列。

适配器初始化现在不接触文档，只提供稳定的 Window 根。首次成功呈现才建立文档
语义：局部增量补齐完整基础树，显式完整源不重复投影。该改动取消初始化时的一次
文档扫描，未做独立计时，不推断完整首帧耗时改善。

验证结果：

- 30 项无障碍/发布队列回归、43 项 scene_host 回归及严格 Clippy 通过；过滤集重叠。
- Vulkan 和 DX12 原生 UIA Semantics 模式均通过：首次失败时根为 Window，后代
  数为 0；恢复后完整控件出现，再通过原有两批失败/第三批成功事务、写值、隐藏
  切换、值保留及退出码 0。普通无 GPU 探针的按需语义路径也通过。
- 回归包括冷启动后收到已预处理文档的局部增量、保留显式完整源、发布后的平台
  激活读取新内容。测试构建中的 unused_mut 提示已清理，随后严格 Clippy 通过。

```powershell
powershell.exe -NoProfile -Mta -ExecutionPolicy Bypass -File scripts/validate-hidden-accessibility.ps1 -ExePath E:/codex-build/nanaui-high-refresh/debug/examples/accessibility-hidden-probe.exe -Mode Semantics -Retry -InitialFailure
# Repeat with WGPU_BACKEND=dx12; the report records the actual backend.
```

原生证据仍限主窗口、语义模式，不替代辅助窗口实测、前台焦点或完整性能矩阵。
此前全量 434 项串行通过是上一构建的证据，本轮按改动范围验证上述过滤集。
证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-a11y-initial-*`。

### 2026-09-06：辅助目标失败、主窗口更新与关闭隔离

扩展同一原生探针，主窗口与辅助窗口各持有独立 Runtime 文档，故障资源仅注册在
辅助目标。通过进程 ID 与精确窗口标题查找各自 HWND，避免 MainWindowHandle 误指
另一窗口或内部消息窗口。没有修改产品 Surface 或呈现代码。

Windows Vulkan / DX12 的 UIA Semantics 模式均完整通过：

- 辅助目标首次失败时仅有 Window 根；主窗口仍能通过 UIA 写入
  Primary during failure，并实际进入成功呈现回调。主窗口更新没有提前发布辅助内容。
- 辅助目标恢复后包含新增文字、改名编辑器和视口语义；写值、隐藏切换及值保留通过，
  主窗口的值没有被辅助更新覆盖。失败阶段没有提交/呈现成功回调。
- 显式关闭辅助窗口后，应用收到 WindowId(1) 的 Closed；主窗口随后成功写入并呈现
  Primary after close。整个探针正常退出，退出码 0。

单窗口首次失败回归、严格 Clippy、PowerShell 语法检查与 diff 检查也通过。
这是双窗口恢复/路由的功能证据；没有测量连续开关的缓存内存上界，没有替代复杂
工作区的截图/点击、120Hz 实机或 release 60 秒性能矩阵。此前性能门禁状态不变。

```powershell
powershell.exe -NoProfile -Mta -ExecutionPolicy Bypass -File scripts/validate-hidden-accessibility.ps1 -ExePath E:/codex-build/nanaui-high-refresh/debug/examples/accessibility-hidden-probe.exe -Mode Semantics -Retry -InitialFailure -Auxiliary
# Repeat with WGPU_BACKEND=dx12. Reports contain the actual backend and primary presented states.
```

证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-a11y-aux-*`。

### 2026-09-06：样式解析初始容量优化

Runtime 的阶段基准显示 10k 初始处理主要分布在 style、accessibility、hit_test 和
extraction。样式解析会对本帧 dirty 节点及其父链做去重，之前使用空 HashSet，初始
大文档会反复扩容。现在按 dirty frontier 预留容量，保持父链去重和结果完全不变。

同机 release 重测（其余代码与工作区状态保持不变）：

| 节点 | style P95 | accessibility P95 | hit_test P95 | extraction P95 |
|---:|---:|---:|---:|---:|
| 5,000 | 1.98 ms | 2.65 ms | 3.07 ms | 2.89 ms |
| 10,000 | 6.14 ms | 7.83 ms | 7.32 ms | 7.45 ms |

完整 Runtime 报告的初始系统 P95 为 15.48 / 33.81 ms（5k / 10k），稳态系统 P95
为 2.14 / 5.07 ms；局部绘制系统 P95 均为 0.02 ms。阶段拆分与完整报告的采样
实现不同，不能将阶段值相加，也不能把初始全局操作当作稳定 120Hz 帧。10k 初始
全局 P95 仍超过 8ms，故 Issue #8 门禁继续失败；稳态系统已低于 8ms。

30 项无障碍、43 项 scene_host、6 项 hosted_context 及严格 Clippy 之外，本轮
Runtime 阶段优化没有改变 UIA/GPU 合同。证据归档于
`performance-data/high-refresh-2026-09-06/high-refresh-style-capacity-*`。

### 2026-09-06：VirtualTable 稳态验证快速路径

`VirtualTableItems` 的行、列和单元格映射由 Runtime 私有字段维护，所有变更必须经过
同一 reconciliation。稳态窗口两轴都没有 mount/unmount 时，之前仍重复构造四组
HashSet 并逐行逐列验证同一不变量。现在直接提交已准备好的窗口计划，保留 revision
检查；只有发生挂载、卸载或首轮结构变化时才执行完整校验。

同机 release Framework 基准重测：

| 指标 | 修改前 P95 | 修改后 P95 |
|---|---:|---:|
| 10k×100 VirtualTable materialize | 8.419 ms | 3.861 ms |
| 10k VirtualList materialize | 0.428 ms | 0.195 ms |
| 5000 节点 canonical layout | 17.593 ms | 8.167 ms |

修改后 canonical layout 本次接近但仍略高于 8ms，未将波动解释为已通过；完整布局
和首次全局操作门禁继续按 8ms 验收。VirtualTable 行/列窗口、冻结区域及非法归属
回归测试通过，Runtime 严格 Clippy 通过。快速路径不适用于带 placement 的 retained
入口，后者仍执行完整校验与定位。

证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-virtual-table-fastpath-*`。

### 2026-09-06：布局输入容量复测

完整布局的 `LayoutInputMap` 在预取后按实际输入数量一次性预留，避免收集完整文档
输入时反复扩容。73 项布局行为测试通过，Runtime 严格 Clippy 通过。受控性不足的
同机阶段复测为 layout_engine P95 8.934ms、P50 6.605ms（此前 8.537/6.308ms），
未显示稳定收益，差异按环境波动处理，不据此声称性能提升。

该改动保持按需 scoped 输入路径不预分配整棵文档，且不改变布局结果；完整布局和
首次全局操作仍按 8ms 门禁验收。后续若要继续降低该门禁，需要更细的布局算法剖析，
不能从这次容量变化推断收益。

### 2026-09-06：百万逻辑项 Framework 规模复测

设置 `NANA_PERF_SCALE=large` 后运行 release Framework 基准，覆盖 1,000,000 行列表、
1,000,000×100 表格和 1,000,000 行树。窗口算法只访问可见窗口与 overscan，结果为：

| 类型 | 窗口 P95 | 物化 P95 | 活动 Runtime 实体 | 状态 |
|---|---:|---:|---:|---|
| 列表 | 0.000 ms | 0.173 ms | 61（上限 62） | ok |
| 表格 | 0.005 ms | 6.137 ms | 1342（上限 1426） | ok |
| 树 | 0.000 ms | 0.217 ms | 61（上限 62） | ok |

1M 列表/树的构造分别为 2.913/15.361ms；1M×100 表格构造为 3.282ms。表格本次
物化工作包含 1343 个实体的样式、布局、命中和无障碍处理，仍低于 16.67ms 的
60Hz 帧预算。窗口与物化采样不包含 Surface 呈现、GPU 时间、系统 IME 或峰值内存，
因此不能替代 120Hz 复杂工作区验收。规模基准没有创建百万 `UiWorld` 节点或百万
二维数据矩阵，符合虚拟化合同。

证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-current-large-scale-*`。

Runtime 全量库测试（benchmark feature）本轮 848 项全部通过，耗时 2.45 秒。该结果
覆盖 VirtualTable 快速路径、布局输入预取、命中索引和无障碍投影的行为回归；它是
功能证据，不代表 8ms 全局布局或 120Hz 呈现门禁通过。

### 2026-09-06：Vue 批量注册级联短路

Vue 构造剖析显示无作者样式、无内联声明的初始挂载会在每个节点重复进入完整
CSS 级联。注册阶段现在复用已写入的 kind 默认布局；只有存在样式层或内联声明
时才执行级联，同时保留 revision、脏节点记录和替换节点的 `:has()` 索引失效语义。

同机 release 重测（70 次采样，丢弃前 10 次）：

| 节点 | register P95 修改前 | register P95 修改后 |
|---:|---:|---:|
| 5,000 | 28.496 ms | 17.604 ms |
| 10,000 | 52.341 ms | 35.200 ms |

这降低了无样式批量挂载成本，但 5k/10k 注册仍高于 16.67ms 的单帧预算；初始大文档
应继续分批挂载，稳定帧门禁不把首次构造计入。143 项 Vue bridge 行为测试通过。
证据归档于 `performance-data/high-refresh-2026-09-06/high-refresh-vue-current-stages-after3.json`。

同轮修复 `nana-app-icon` 在透明像素反预乘中的严格 Clippy 检查（使用显式
`checked_div`，结果与 alpha 非零分支一致）。Vue crate 严格 Clippy 现已通过。

### 2026-09-06：虚拟列表反复滚动生命周期回归

在 retained 虚拟列表测试中加入 128 次跨越百万逻辑项的远距离滚动，检查活动挂载项
始终保持在可见窗口上限（含 overscan 与编辑保留项）以内。该测试通过，峰值为 8
行，且原有焦点、IME 草稿和离屏项结束后的卸载断言仍通过；这为缓存不会随访问过的
逻辑项线性增长提供了行为证据。

### 2026-09-06：Runtime release 门禁复测

在无并行 cargo 进程的情况下重跑最新 release Runtime 基准，确认增量路径保持稳定：
5,000 节点稳态系统 P95 **2.468 ms**、局部绘制系统 P95 **0.016 ms**；10,000 节点
分别为 **4.397 ms** 与 **0.017 ms**。首次系统处理 P95 为 16.888 / 35.913 ms，
仍属于首次挂载与全局操作，不纳入稳定 120Hz 帧指标；两档的局部工作量仍仅为单节点。
报告位于 `target/performance/high-refresh-current-runtime-rerun.json`。

随后执行 Runtime 全量库回归：**847 项通过，0 项失败**（release 工作区依赖下的
当前测试集合）。该结果包含虚拟化生命周期、滚动边界索引、布局缓存隔离、焦点/IME
和资源生命周期相关测试。

### 2026-09-06：GPU 目标短时 release 复测

重新构建 `nana-gpu-scene-benchmark`（host-owned WGPU）并在 Vulkan 目标运行 5 秒、
18,001 帧的单纹理场景。GPU 时间戳显示生产者 P95 **0.0052 ms**、UI 合成 P95
**0.0289 ms**（P99 分别 0.0062 / 0.0341 ms），未观察到逐帧结构重编译。该工具是
离屏完成时间，不包含真实扫描输出间隔；因此只能证明 GPU 命令与资源路径的稳态成本，
不能替代 60 秒 120Hz Surface 验收。报告位于
`performance-data/high-refresh-2026-09-06/gpu-1-current.json`。

随后以独立进程重复同一 60 秒场景并每 100ms 采样进程内存，共 549 次观测；峰值工作集
**144.5 MiB**，峰值私有字节 **203.5 MiB**，进程正常退出。该观测包含基准自身的
诊断存储且不包含独立 GPU 显存，只能作为本场景生命周期上界证据。记录位于
`performance-data/high-refresh-2026-09-06/gpu-1-current-60s.memory.json`。

同机 5 秒 GPU 负载矩阵还覆盖 4 与 16 个纹理节点：

| 纹理节点 | 帧数 | CPU 准备 P95 | 生产者 GPU P95 | UI 合成 GPU P95 | 结构重编译 | 缓冲重分配 |
|---:|---:|---:|---:|---:|---:|---:|
| 4 | 16,408 | 0.0131 ms | 0.0053 ms | 0.0289 ms | 0 | 0 |
| 16 | 15,983 | 0.0227 ms | 0.0051 ms | 0.0342 ms | 0 | 0 |

两组报告均为 `offscreen-gpu-completion-serialized`，未测 Surface present 间隔；原始
结果保存在 `gpu-4-current.json` 与 `gpu-16-current.json`。

16 个纹理节点的 60 秒长时复测完成 196,959 帧：生产者 GPU P95/P99 为
**0.0046 / 0.0051 ms**，UI 合成 P95/P99 为 **0.0297 / 0.0378 ms**；结构批次重建和
GPU 缓冲重分配均为 0。报告位于 `gpu-16-current-60s.json`。该结果仍是离屏 GPU
完成时间，真实 Surface 扫描反馈需在目标高刷新显示器上单独采集。

GPU 测量脚本新增可选 `-Binary` 参数，支持传入独立 release target 的绝对路径；默认
仍使用 `target/release/nana-gpu-scene-benchmark.exe`。脚本路径解析已通过 1 秒实际
运行验证（3,851 帧，进程正常退出），避免不同构建目录下误报找不到二进制。

尝试将 GPUI 生成的 `text-table` 与 `virtual-list-10k` fixture 直接交给 Nana GPU
基准时，工具按合同返回 `unsupported`：这些文件是 GPUI TestAppContext（无 GPU
backend）的窗口算法报告，不是 Nana `UiOnly` Scene 场景。该结果被保留为边界证据，
避免把无 GPU 的第三方基准误报为 Nana 复杂 UI 呈现通过；复杂表格的 Nana Runtime
虚拟化数据仍以 Framework 百万项报告为准。
