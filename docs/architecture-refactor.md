# NanaUI 架构重构与迁移记录

本轮在当前分支实施。保留 `UiWorld → ExtractedNode → UiScene → SceneWgpuPainter`，Window、Surface、Device、Queue 仍由宿主拥有。

## 文档访问

`RuntimeProgram::document/document_mut` 已删除。实现 `with_document` / `with_document_mut`，返回 `Result<Option<R>, DocumentAccessError>`。`None` 仅表示窗口没有文档；`Busy` 表示重入或正在借用，`Poisoned` 表示前一次作用域异常。宿主在作用域结束后调用 `host_failure`，进入 JS 前释放借用。

Vue 的共享句柄复用 `Arc<Mutex<NanaTreeDocument>>`，其中独占持有 `RuntimeDocument`。这是计划中锁结构的具体调整：DOM facade 与 Runtime 共用一把锁，避免引入第二把文档锁及锁顺序问题。不存在 `UnsafeCell`、手写 Send/Sync 或从共享引用返回裸可变引用。共享句柄使用 `try_lock`；JS DOM host-op 也使用非阻塞锁并返回 JS 错误。

## 职责位置

| 子系统 | 实现位置与边界 |
| --- | --- |
| Runtime 树 | `world/` 分离 mutation、input、style、hit_test、text、accessibility、animation、extraction；`UiWorld` 是唯一树权威 |
| 控件几何 | `world/geometry/` 按可选控件族派生几何；Scene 不重新计算控件布局 |
| AppContext | `framework/` 分离注册、事件、生命周期、帧、虚拟化、选择、模态和输入；编辑会话状态归 `TextEditSession` |
| 布局 | `layout_engine/` 的 measure、flow、flex、grid、inline、placement 共用原入口和增量缓存 |
| Scene | composition、order、primitives；控件几何绘制函数只接收投影数据并向统一出口发出图元，不访问 Scene 索引 |
| 原生宿主 | `scene_host/` 分离窗口、调度、输入、无障碍和呈现 |
| Vue | `bridge/semantic,cascade,motion,resources`；`host/` 分离 JS 回调、帧、输入投影和资源操作 |
| JS | nodes 保持唯一 wrapper cache，events 保持唯一监听表，styles 保持唯一批处理队列；windowContext 负责路由 |
| Web API shim | `shim/manifest.txt` 固定组合顺序，build.rs 生成同一个共享脚本作用域；测试也按该清单加载 |
| 验证工具 | `perf/contract_support/` 分离 schema、invariants、extractors、reports、comparison、process、CLI 和自检；原 CLI 入口保留 |
| Gallery | 快照 catalog、workspace、selection、feedback、evidence 与 fixture_values 分离，示例状态留在示例中 |

`PendingHostOps` 保留提交前读取；`flush_host_frame` 提交 Runtime 事务。语义索引从已提交 Runtime 拓扑刷新；`LayoutBoxStore` 是布局/滚动投影，不向 Runtime LayoutBox 回写滚动结果。移除和窗口销毁清理对应缓存，JS 模块不各自创建节点身份表。

## Dock 迁移

删除 `DockLayout`、旧 Dock 节点类型、旧 controller 持树逻辑及 `hosted_dock_update`。调用者拥有 `DockWorkspace`；`DockController` 只保留指针、时间和显示器环境。应用通过 `workspace.execute(DockCommand::...)` 或现有 Runtime 操作修改树，效果统一交给 `runtime_dock_window_update`。

新增命令涵盖隐藏/显示、锁定、重排、跨表面投放、分割尺寸和浮动；失败投放原子回滚。Runtime 计算两侧尺寸限制，宿主处理拖拽驻留和显示器边界。`MoveFloating` 同时转交位置和尺寸。持久化仍为原 JSON 字段/版本，保留 `locked`、浮窗 `monitor`，不持久化实时内容节点。

## 控件族与构建

六个 feature 名称及公开归属不变：calendar、charts、controls、graph-canvas、image-viewer、rich-text。它们贯穿 Runtime 模块/注册、Scene 绘制、Vue 和顶层再导出。基础 Button、TextInput、Workspace、Settings 不依赖这些开关。`components` 聚合六族，`full` 是完整入口。`syntax-highlighting` 必须显式启用，`scene-view` 不再隐式启用它。Markdown 依赖随 rich-text 裁剪。

`component_descriptors` 统一注册 type id、tag、feature 和实际编译状态；关闭后的 Vue tag 显式报告缺失 feature，Rust API 不可达。`ComponentSupport.migration` 和 `ComponentMigrationState` 已删除，历史资格资料在 [component-qualification-history.md](component-qualification-history.md)，不再用于运行时能力决策。

根清单删除本机 V8 路径补丁，锁定版本保持不变；本机覆盖只放入不提交的配置。CI 删除不存在的 Live2D target/feature，保留真实 GPU 场景。架构脚本检查传递依赖、单一 WGPU 主版本、feature 声明与 CI Cargo target。独立 feature 矩阵避免 workspace feature 合并掩盖裁剪问题。

## 验证记录

2026-09-03，当前 macOS 工作区验证。下述构建均使用锁定依赖。

| 验证 | 结果 |
| --- | --- |
| `cargo test --workspace --locked -- --test-threads=1` | 2602 通过，0 失败，1 个原有文档示例忽略；Runtime 796、Scene 88、Vue 791、nana-ui 412 |
| `cargo clippy --workspace --all-targets --locked --no-deps -- -D warnings` | 通过 |
| 全 feature 构建 | `cargo check --workspace --all-targets --all-features --locked` 通过；最终 renderer 定向复验 46 项通过 |
| 独立 feature 矩阵 | base、六族各自开启、components 共 8 个独立构建通过；注册表、Vue 可用性、关闭后的 Rust API 与专属依赖检查通过 |
| JS 测试与 SFC fixture | runtime 76、components 166 通过；Vite fixture 构建通过，提交目录中的 bundle 已同步 |
| V8 | 21 项串行测试通过；真实 Vue hosted acceptance 的 6 项测试通过，含原生组件、辅助窗口、Canvas、WebGPU 和 Video |
| 架构与性能工具 | 依赖边界检查、4 项边界脚本测试、两组性能工具 self-test 通过 |
| Runtime/Scene 性能 | 现有 timing/scale validator 通过；5000 节点布局 P95 **4.759 ms**（门槛 8 ms）；21 个真实报告的 Issue #8 场景不变量全部通过 |
| Agent 与 GPU | Agent 12 项测试通过；实际会话验证点击后计数 0→1、可访问树及可见布局；原生 hosted GPU demo 完成首次 Surface 呈现 |
| Gallery | 生成 1904 张 PNG；人工检查日历、图表、GraphMinimap、Workspace 等代表场景。GraphMinimap 夹具包含实际节点和视口；Gallery 快照宿主提供缩略图使用的真实纹理 |
| 格式 | `cargo fmt --all -- --check`、`git diff --check` 通过 |

计时在构建与截图完成后单独执行。Framework 参数使用 `python3 perf/runners/nana/run.py --print-framework-window-args` 给出的目录参数（含 160 px overscan），不使用独立程序的 200 px 默认值。第一次并发采样因负载和参数不匹配未过门禁；没有修改阈值或算法来适配结果。

本机证据位于：

- `target/performance/refactor-{runtime,vue,scene}.json` 与 `refactor-framework-catalog.json`；Issue #8 报告在 `target/performance/refactor-issue8/`。
- `target/refactor-agent.png` 与 `target/ui-snapshots/`。这些产物留在构建目录，不作为新的视觉基准提交。

Gallery 的现有参考图回退机制可能使用 Runtime 输出；生成成功不等于 LiliaUI 视觉等价。本轮只对实际打开的图片作视觉确认，没有重新认证全部控件资格。未运行 Windows/Android 真机效果、远程 CI 或百万节点扩展规模；macOS 结果不能替代这些证据。

## 验收中修复的问题

同步修复了完整测试暴露的 CSS scrollbar 规则合并、带分号字体 URL 的声明拆分、FLIP 完成后 transform 清理，以及 GraphCanvas 内容投影排队问题。Vue 原生组件的子节点插入现在遵守已注册组件身份；字符串形式的宽高属性按像素尺寸解析，避免 Canvas 高度被无单位 CSS 清空。宿主与共享文档的错误路径保持显式返回。补齐 CSS L1 基准程序过期的 MatchContext 初始化，使 benchmark feature 也能在全 target 构建中通过。

测试整理保留行为断言；字体测试使用实际字体数据，文本缓存计数测试串行隔离全局字体注册影响。分位数测试按既有算法校正断言，没有改变性能统计定义。

## 提交前精简

组件声明只指定一次 feature，由声明生成可用性；tag 查询仅规范化输入，不再重复分配每个候选 tag。Dock 的 Item/Tabs 共用轴向尺寸限制计算，删除标签遍历中的临时节点构造。性能 CLI 保留公开入口，移除对内部私有辅助函数的重新导出。

复用已有用例验证独立 feature 矩阵、Dock 命令与宿主交互、性能工具 self-test 和 Issue #8 报告，不新增与私有实现绑定的测试。
