# Issue #7 Phase 4：Entity / View / Context 与扩展契约

本阶段在 `2026-08-14` 为 Phase 3 runtime 增加 backend-neutral 应用 API。目标是让 Rust 组件、Vue adapter 和后续扩展共享同一个 typed state / action / async contract，不复制 Iced application model，也不在 Runtime 内创建第二个 executor。

## Typed Entity 与 Context

- `Entity<V>` 由 `StableNodeId` + phantom View type 构成；不暴露 Bevy `Entity`，复制 handle 不复制 state。
- `AppContext` 同时持有 `UiWorld` 与 typed View state。`create_view` 先成功创建 retained Entity，再发布 state；伪造错误类型或 stale handle 返回 `FrameworkError`，不会 panic 或移除原 state。
- `update(entity, |view, cx| ...)` 在 closure 内汇集 mutation 与事件，事件处理完后只 commit 一个 `MutationQueue`。
- `remove_view` 先取得完整 subtree，成功 despawn 后统一释放 root/descendant typed state 与两端事件 handler，不遗留 observer。
- `ViewContext` 提供当前 typed entity、frame mutation queue 和 `emit`；不暴露 ECS World。
- `create_component` / `update_component` 将 Nana-native component state 投影到同一 Entity；事件 handler 完成后才投影最终 state，避免 closure event 中的变更落后一个 turn。projection 对当前 UiWorld 做字段级 diff，no-op update 不增加 generation。

View state mutation 本身遵循普通 Rust closure 语义；retained-tree mutation 仍由 `UiWorld::commit` 保证批次原子。业务不应把 `UiWorld` commit error 当作 View state rollback 机制。

## Nana-native component primitives

`Text`、`Button`、`IconButton`、`Card`、`TextInput`、`TextArea`、`Checkbox`、`Switch`、`Slider`、`TabList`/`Tab`、`ScrollView`、`List`/`ListItem` 与 `Table`/`TableRow`/`TableCell` 是不含 Iced/GPUI/ECS implementation type 的 retained primitives。canonical application import 为 `nana_ui::runtime`；Gallery snapshot 已改用这一 public path，不再直接依赖 implementation crate。顶层 Iced-shaped exports 只保留为迁移 compatibility surface，不是新 framework extension contract：

- component state 直接投影到现有 `NodeStyle`、`TextContent`、`InteractionState`、`TextInputState` 与 `AccessibilityState`，不创建第二套组件属性模型；
- element node 可以同时 extraction 为 Quad 与 Text，因此 Button/Input 不需要伪造内部 label subtree；
- `append_child` 建立同一 UiWorld hierarchy；`activate_button` 执行 disabled gate 并发出 typed `Activate`，`replace_text_input_selection` 按 UTF-8 selection 修改 committed state 并发出 `TextChanged`；
- TextInput 的 `TextContent` 是 committed state 的同批 render projection，selection replacement 不会留下旧绘制文本。
- IconButton 将可见 glyph 与 accessibility label 分离；Card 是不抢占 child action 的非交互内容容器；ListItem 的 disabled/selected/activation 与 List hierarchy 共用 retained state，不以 Card click 或字符串 tag 模拟行为。
- Table hierarchy 投影正式的 table/row/cell/column-header accessibility roles；cell 的文本、selection 与 focusability 不另建表格状态树。
- `navigate_table` 从 Runtime hierarchy 与当前 focus 计算 row/cell，应用 backend-neutral `TableNavigation` 并发出 typed `TableCellFocused`；ragged rows 会按实际 cell 数夹取列，不保存第二份 selection matrix。
- Checkbox/Switch 的 checked state 与 Slider 的有限 range/value 是 typed component state；`toggle_*` / `set_slider_value` 执行 disabled、clamp、no-op 门禁并发出 `ToggleChanged` / `SliderChanged`。同一 state 投影到 accessibility 与 backend-neutral `StandardVisual`，不靠 renderer tag 匹配或展示型假控件。
- `select_tab` 校验 direct-child/disabled/no-op，在一次 retained commit 中同步旧/新 Tab selected state 与 focus，commit 成功后才发布 `TabSelected`；不维护第二份 active-index。selected+hover/press 使用主题的 `SelectedHover`/`SelectedPressed` 组合态，而不是退回普通 hover。
- `ScrollView` 只保存轴向/style/label 配置，offset 不在 typed component 复制，唯一保存在 UiWorld。layout backend 以 `ScrollMetrics` 写回 viewport/content extent，Runtime 统一夹取 offset；内容缩小时先提交新实际值再发 typed `ScrollChanged`。`scroll_to` 拒绝非有限/负值、屏蔽未启用轴并保持 exact no-op，自定义 style 不能移除 scroll clip contract。
- `UiWorld` 持有 theme mode 与 per-pointer hover/press；semantic foreground/background/border 叠加 hovered/pressed/focused/disabled state 后在 style system 中解析为 computed paint。Rust component 与 Vue theme/interaction 写入同一 authority。重复设置当前 theme 是 generation/system-work 均不变的 no-op，theme 切换只失效 style/render，不触发布局。
- native Button/TextInput/TableCell 的 text placement、padding、border 与圆角属于 `NodeStyle`/`LayoutStyle`，Scene backend 不再通过 tag 猜测组件布局。Button 使用居中 anchor，输入与表格单元格使用垂直居中 content box；这些语义同时服务后续 backend，而不是 Iced 私有参数。

这是 component/element contract 与真实事件入口。dark/light Runtime Scene 已实际覆盖 Text、Button、IconButton、Card、Input、TextArea、Checkbox、Switch、Slider、Tabs、ScrollView、List/ListItem 与 Table 的主要 normal/selected/disabled 状态；overlay lifecycle 由独立功能门禁覆盖。它不表示为了形式独立而删除仍是最佳实现的 Iced text/platform/WGPU compatibility code。

## Closure event

`on` 处理 View 自身事件；`observe(source, observer, closure)` 支持子 View → 父 View等 typed 跨实体观察，不需要全局字符串消息总线。nested event 记录真实 emitter，并以 FIFO 顺序交付。

单次 update 最多交付 16384 个事件；递归 emission 超限返回 `EventOverflow`，避免一个 UI turn 永不返回。移除 source 或 observer subtree 都会移除相应 handler。

## Action / KeyContext

`ActionId`、`KeyContext`、`ContextPredicate` 已从 Iced-facing `nana-ui::command` 下沉到 `nana-ui-core::action`：

- `nana-ui` 继续 re-export 原名称，现有 `ActionRegistry`、`Keymap` 和调用方不需要迁移；
- `nana-ui-runtime` 使用相同 action identity/context predicate 注册 closure handler；
- 空 ID、重复注册、context 不匹配与缺失 action 都是显式错误；
- dispatch 临时取出 handler，允许 handler 安全访问其余 `AppContext` 状态，并在返回后恢复注册。

Action 的 label/category/search/key binding 仍由现有 NanaUI registry 拥有；Runtime 不复制面向 UI 的 command metadata。

## Task / Subscription 与宿主

- `Task<T>` 拥有一个标准 `Future`，支持 `ready`、`map` 和所有权转交；
- `Subscription<T>` 拥有带稳定 ID 的标准 wake-driven `Stream`；没有逐帧 polling callback；
- drop future/stream 即取消未完成工作，Runtime 不启动线程或 executor；
- `nana-ui::run_task` / `run_subscription` 将合同接入现有 Iced executor/subscription tracker。相同 subscription ID 保持宿主生命周期身份。

## Extension 安装

`UiExtension` 只能写 staging `ExtensionRegistrar`。Registrar 先校验扩展内重复项，`AppContext::install` 再检查与已安装 action 的冲突，全部通过后一次性发布。安装失败不会留下半注册 action，也不能在 install 阶段任意修改运行中的 World。扩展名称为空或重复均拒绝。

当前合同只支持进程生命周期内安装，不提供热卸载；这是有意约束，避免异步 task、View 与 handler 尚未定义取消顺序时暴露不安全的假卸载功能。

## 功能与性能门禁

Runtime 42 项测试覆盖 typed update/read、self/cross-view/nested event、错误类型 handle、事件上限、action context、扩展原子安装、subtree state 清理、Task/Future、Subscription/Stream、Nana-native component/table/toggle/slider/tab/scroll/overlay projection、IconButton glyph/accessible-name 分离、Card hierarchy、ListItem selected/activation、typed activation/change、CJK single/multiline selection、theme/interaction/selected-combination computed paint、scroll hit-test/clip、keyed list/table virtual materialization、跨 List/Table/OverlayHost 所有权拒绝、active overlay 销毁恢复与 no-op generation。既有 NanaUI command tests 通过，证明 Action/KeyContext 下沉保持兼容。

机器可读报告见 [`performance/2026-08-14-issue7-phase4-framework.json`](performance/2026-08-14-issue7-phase4-framework.json)。Apple M4 release，100 次 warmup、1000 个样本：

- typed View update + closure event + text mutation + atomic commit：P50/P95 0.001 ms、P99 0.002 ms；
- context action dispatch：P50/P95/P99 均 < 0.0005 ms。
- Button typed activation + final component projection：P50/P95/P99 0.002 ms；
- unchanged component update：P50/P95/P99 0.001 ms，并在每个样本断言 generation 不变、system work 为空。
- 10k variable-height virtual list 的 visible-window query 与单项 measurement update：P50/P95/P99 均 < 0.0005 ms。
- 10k list 的 keyed visible materialization（约 60 个含 overscan 的 retained rows，120/140 px 交替、复用 overlap、原子 mount/unmount/order）：P50 0.039 ms、P95 0.043 ms、P99 0.046 ms。
- 10k rows × 100 columns 的 keyed 二维 materialization（约 60 × 20 retained 可见交集，水平/垂直窗口交替、复用 overlap）：P50 0.818 ms、P95 0.844 ms、P99 0.874 ms。
- 10k rows × 100 columns virtual table 的二维 visible-window query 与单列 resize：P50/P95/P99 均 < 0.0005 ms。
- 40 个 retained 可见节点的 scroll window 在 120/140 px 间切换：P50 0.003 ms、P95 0.003 ms、P99 0.004 ms；每次只产生 41 个 input/render work node且没有 layout work。报告为 schema v7，并由前两项实际 materializer 证明 list/table 只保留可见窗口，未把完整数据集 retained 后宣称 O(1) 滚动。
- 100–5000 节点间 old/new hover target 切换：P95 均 < 0.0005 ms，且只调度至多 2 个 interaction-styled node。

该 microbenchmark 证明框架层没有可见固定开销，不代表完整渲染帧时间。

验证结果：runtime tests/clippy 通过；NanaUI/Iced adapter check 通过（仅 `vendor/arboard` 既有 warnings）；backend-neutral runtime portability check 通过；Iced compatibility boundary script 通过。

## 阶段结论

Phase 4 已建立 typed Entity/View、closure Context、跨 View event、统一 Action/KeyContext、host-owned Task/Subscription、原子 extension registration，以及 Text/Button/IconButton/Card/Input/TextArea/Checkbox/Switch/Slider/Tab/Scroll/List/ListItem/Table/Overlay/Dialog/Menu/Tooltip Nana-native component contract。list/table 使用 revision-fenced 两阶段 plan 与单次 Runtime commit；OverlayHost 的 active/focus restore 只在 UiWorld，非 active subtree 不参与系统输出。多轮复核移除了 busy-poll subscription、半安装 extension、无界递归事件、subtree state 泄漏、仅能 self-event 的局部模型、no-op 全量 projection、TextInput render state 分叉、component theme/overlay 双权威、selected label/indicator 颜色混用、selected-hover 状态降级、scroll/layout 双写与虚拟列表逐帧重建。`nana_ui::runtime` 现在是明确的 canonical public boundary，compatibility widgets 不再定义新应用的 framework contract。
