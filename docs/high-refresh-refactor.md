# 高刷新与大规模 UI 重构

本轮目标：10 万保留节点、百万虚拟数据项、局部实时 GPU 视口 120Hz。
性能目标是验收要求，不是已有测量结论。业务数据仍由应用持有。

## 已接入的合同

- Runtime 帧收敛后一次性发布抽取和无障碍变化；空闲统计不遍历图元。
- `SceneDelta` 提供变化节点集合，计数位于 `stats`。
- `UiScene::frame_plan` 缓存结构计划，外部资源冲突仍在消费时检查。
- `UiScene::draw_primitive` 返回当前继承变换和裁剪；`primitive` 提供保留几何。
- 普通仿射滚动通过共享属性和可见索引范围平移更新，透视场景保守重建。
- 命中索引按稳定节点寻址，滚动偏移在查询时继承。
- `SceneWgpuPainter::paint_target` 按 `RenderTargetId` 隔离合成纹理；关闭目标调用 `remove_target`。
- HostTexture 内容更新复用已准备绘制数据；替换 view 或尺寸使绑定失效。
- Quad/Mesh 保留上传镜像，以对齐区间更新 GPU buffer；局部颜色变化不全量上传这些 buffer。
- `SceneResourceProducerRegistry::encode_scene` 接收宿主 encoder，返回 `PreparedSceneResources`。
  宿主成功提交后调用 `submitted`，失败则丢弃 encoder 和准备结果。
- `FrameDemand` 分离按需、截止时间和持续刷新；显示器实际刷新率仍限制呈现。
- `TextureSlot` 通过稳定名称更新注册纹理；注册表订阅只通知引用资源的窗口。
- `RuntimeApplication<State>` 默认处理文档路由与窗口生命周期；自定义宿主继续使用 `RuntimeProgram`。
- `VirtualViewport` 统一视口参数，`VirtualScrollAnchor` 保存项内偏移。
  数据重排后，应用先按业务 key 解析新的索引，再恢复锚点。
- Rust `materialize_virtual_{list,table,tree}_in` 接收统一视口；Vue 三种组件使用相同视口字段。
- 固定像素宽高的静态容器可以设置 `LayoutStyle::layout_isolation`；自动尺寸、浮动、
  定位与 grid 容器仍向上传播。修改隔离容器自身尺寸也会向上传播。

## 虚拟窗口定位与活动项保留（续建）

- Rust `VirtualAlignment::{Nearest, Start, Center, End}` 与 Vue 的同名小写字符串使用相同定位规则。
  `VirtualListLayout::reveal_index` / `VirtualTableLayout::reveal_cell` 更新 `VirtualViewport`；
  无效索引返回 `false`，表格不会只更新其中一个轴。业务 key 由应用通过已有数据索引解析，
  定位不扫描中间数据，也不创建中间控件。
- Vue 列表和树通过组件 ref 暴露 `await scrollToIndex(index, alignment)`；表格暴露
  `await scrollToCell(row, column, alignment)`。先提交 Vue 物化窗口，再通过节点的
  `scrollTo(x, y)` 将双轴写成一个 `setScrollOffset` host op，最终仍由宿主帧提交。
  调用方在返回成功后定位已挂载控件并请求焦点；组件不猜测一个复杂行中的焦点目标。
- Vue 列表/树新增 `retainedKeys`、`keyAt` 和 `indexOfKey`。活动编辑会话由应用将业务 key
  放进 `retainedKeys`，会话结束后移除；草稿和选择等持久状态继续由业务 key 管理。
  稀疏范围之间保留真实尺寸的间隔，不将远处编辑项追加到可见窗口尾部。
  `indexOfKey` 返回缺失或不能与 `keyAt` 互证时释放该项，避免重排后保留错误数据。
- Rust `VirtualListMaterializer::prepare_retained` 复用现有两阶段提交，
  `VirtualListLayout::retained_ranges` 提供有序不相交范围；绘制适配器按范围和前缀尺寸放置项。
  **AppContext 的旧物化入口尚未自动接入这些稀疏布局及焦点/IME 生命周期。**
- Vue 视口订阅宿主 `ResizeObserver`，首次布局和后续尺寸发布会重新计算窗口；卸载时断开。
  Agent 的 JS 初始化改用与产品一致的 `initialize_with_web_api`，补齐观察器与事件 shim。

Vue 列表、树和表格现在捕获后代 focus/blur 与 compositionstart/compositionend，
分别保留焦点与 IME 所有者；两者结束才释放。默认索引 key 可直接使用；自定义 key
通过 `indexOfKey`（表格按轴提供）跟随重排。没有逆索引时只在原位置 key 仍匹配时保留，
不扫描逻辑数据。数据删除或树折叠使 key 失效时结束保留，重新插入不会复活旧会话。
显式 retainedKeys 仍可覆盖应用自定义编辑会话。持久草稿、选择和展开状态仍由业务 key 管理。

程序化 `.focus()` / `.blur()` 根据宿主返回的实际前后焦点分发祖先捕获事件；重复 focus
不重复通知，非活动元素 blur 不清除其他元素的焦点。内部 host op `setFocus` / `clearFocus`
返回 `{previous, current}`，`clearFocus(node)` 只清除该节点，省略参数保留清除当前焦点语义。
Rust 列表/树的定位及活动保留入口见下节；Rust 表格、完整 OS IME 交互及完整编辑合同仍未完成。

## 冻结表格与同帧几何更新（续建）

Vue `NanaVirtualTable` 新增 `frozenRows` / `frozenColumns`。冻结区域复用同一个单元格实例，
通过绘制变换抵消对应轴滚动，保持原始占位、裁剪和业务 key。定位会扣除冻结区域占用的
视口；冻结前缀覆盖全部视口时，不物化被遮住的普通窗口，无法显示的非冻结目标返回失败。
即使冻结数量配置为百万，实际冻结挂载也只覆盖可见前缀。

`retainedRowKeys` / `retainedColumnKeys` 配合 `rowKeyAt` / `columnKeyAt` 和
`rowIndexOfKey` / `columnIndexOfKey` 保留当前活动行列。挂载数是所需行列范围的交叉积，
包括冻结前缀、可见窗口、overscan、显式保留项和自动活动项。槽参数增加 `frozenRow` / `frozenColumn`。
布局 spacer 不参与命中，避免透明间隔拦截冻结单元格。

Rust 提供相同几何规则：`VirtualFrozenWindow`、`VirtualTableFrozenWindow`、
`window_with_frozen`、`offset_for_frozen_index` / `reveal_cell_with_frozen`；表格参数
`frozen: [usize; 2]` 按 `[列数, 行数]` 排列。这些是现有数据布局的查询结果，未建立新数据容器。
`materialize_virtual_table_retained_in` 已将冻结布局、焦点与 Runtime IME 保留接入同一次物化事务。

同帧滚动与冻结变换不再误走纯滚动命中更新；事务发布保留无障碍子树变化，避免等下一次
交互才更新边界。无障碍几何发布目前仍遍历受影响子树；十万非虚拟保留项滚动时的成本
还需优化与实测，不能视为工作量门禁通过。

## Rust 定位物化与活动项保留（续建）

`AppContext::materialize_virtual_list_retained_in` 和 `materialize_virtual_tree_retained_in`
复用 `VirtualListItems` / `VirtualTreeItems` 与既有两阶段 materializer，使用 `VirtualViewport`。
调用方传入当前 key 正反索引和可选显式 retained keys，框架沿当前焦点祖先找到所属项，
并检查当前或上次观察到的输入目标的 Runtime IME 状态；不遍历所有数据或全部文档。
焦点或 IME 仍有效时保留，逆索引缺失或不能与正向 key 互证时释放。树折叠后的展开序列
不包含被折叠 key，因此不会为了保留焦点重新实例化折叠子树。

每个挂载项增加一个非命中 Stack 容器，按 Fenwick 前缀定位与定高；List 作为 ScrollView
内容根，框架管理其总高度及防收缩约束，宽度由应用决定。编辑组件自身和嵌套内容不因
重排或滚动重新创建/投影。纯滚动不改变窗口时不提交 Runtime mutation。
物化前验证全部已挂载项的归属，创建、删除、容器位置及内容高度统一提交；成功后才
发布组件存储与 materializer 状态。卸载同时清理嵌套组件视图及事件处理器。
事件处理器维护 source/observer 到订阅桶的反向依赖索引；卸载只访问相关桶，并同步移除反向边。
单项卸载测试在 1000 个无关订阅桶存在时仍只访问 1 个相关桶。

此入口是定位物化的显式选择；旧 `materialize_virtual_list[_in]` / tree 入口保持原先
仅管理身份、由调用方安排布局的合同。不要在同一个非空 items 实例混用两种入口。
应用需在视口或数据变化时再次调用物化，待布局发布后通过 `scroll_to` 更新 ScrollView；
本轮未添加自动调度绑定；Rust 冻结表格使用下述定位入口。

## Rust 冻结表格与目标 GPU 存储（续建）

`materialize_virtual_table_retained_in` 复用 `VirtualTableItems`，接受 `VirtualViewport`、
`frozen: [列数, 行数]`、显式保留的业务 key 对，以及两轴 key 正反索引。
仅物化正文窗口、视口内冻结前缀和活动编辑项的行列组合。每个单元格的嵌套组件保持身份；
移除列时完整清理单元格子树。框架管理 Table/Row/Cell 的绝对定位和冻结变换，未指定背景的
单元格使用 Surface 背景以遮住下面的正文。全部位置与身份更改成功提交后才发布组件存储。
百万行、万列的原生验证挂载单元格数为 20 → 30 → 20，冻结角/行头/列头/正文真实点击均正确。

`RenderTargetId` 隔离已准备命令、Quad/Mesh/Icon 缓冲、文字图集与 renderer、旋转文字缓存、
滤镜工作纹理及 HostTexture 绑定。着色器、RenderPipeline、字体系统与整形缓存按设备共享。
目标关闭调用 `remove_target` 一并释放这些资源；默认 `paint` 保留自己的独立状态。
16 目标交替绘制涵盖不同 DPI、文字、图标、笔画、HostTexture 与背景模糊，复用帧与首帧像素一致，
命令重新组装耗时和 GPU 上传字节数均为零。图标几何仅上传变化字节，投影 uniform 仅在首次使用或尺寸变化时上传。

`HostedGpuResources::generation()` 标识宿主设备上下文代次，克隆保持同值，重建设备产生新值。
宿主恢复路径重建 painter，清除旧纹理绑定并调用消费者 `rebuild_gpu`；真实窗口主动销毁 Device 后代次 1 → 2 并再次呈现已验证；其他驱动异常与多窗口恢复仍待压力验收。

## 宽命中树的有序范围索引（续建）

命中索引在根序列和每个节点的子序列上维护包围盒树。局部几何变化和滚动仅更新各祖先
对应的一个叶槽，查询先剪掉无关范围，再按原前后顺序访问候选。未知透视范围仍保守保留。
删除把稳定槽标为空，直接修复祖先包围盒；插入或重排时才压缩槽并重建对应兄弟序列。
父节点的权威 children Arc 用于验证已有顺序号，普通叶更新无需搜索宽兄弟列表。
跨父节点移动按条目当前归属清理，防止后处理旧父节点时删掉新父节点已发布的条目。
十万兄弟的点查询与局部改位测试均访问少于 100 个范围；命中相关 38 项行为验证通过，
包含嵌套滚动、裁剪、菜单顺序、重排及两种顺序的跨父移动。

## Rust 应用入口

实现 `ApplicationState::initialize` 和 `build`，使用
`run_runtime::<RuntimeApplication<MyState>>(settings)`。
`ApplicationWindow` 持有 document、textures、renderers、producers 与 demand。
业务状态仍在 `MyState`，持久化格式没有调整。

可运行示例：

```powershell
cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts --locked
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts --locked -- --measure-first-frame
```

低层宿主在目标关闭时调用 `SceneWgpuPainter::remove_target`。
`SceneDelta` 的变更集合与 `stats` 分离；访问当前绘制几何请用 `draw_primitive`，
`primitive` 继续表示保留的原始几何。结构计划可以通过 `frame_plan` 共享复用。

## 验证记录

同机时间、原始报告和截图链接见[阶段性能记录](high-refresh-performance.md)。

- 改动前 `cargo test -p nana-ui-scene --lib --locked`：88 项通过。
- Runtime 改动前报告：`target/performance/high-refresh-before-runtime.json`。
- 第一轮真实 GPU 绘制测试：173 项通过；后续新增路径仍需最终复验。
- Scene 新增结构缓存及一万节点滚动验证后：90 项通过。
- Vue 组件包新增百万项测量锚点测试后：171 项通过。
- Core/Runtime/Scene 测试：172 / 778 / 90 项通过；后续 Scene 包再次通过 90 项。
- 最新 Runtime 回归：781 项通过；最终 `scene_paint::` GPU 绘制套件：178 项通过。
- Vue runtime 包：76 项通过；续建新增双轴 host op 测试后 77 项通过。
- Vue 组件包最终回归：172 项通过；补充了小数索引、非有限 count 与大逻辑窗口的输入边界验证。
- `cargo check --workspace --lib --bins --examples --locked` 通过。
- `cargo clippy -p nana-ui --lib --features hosted --locked --no-deps -- -D warnings` 通过。
- 当时的全目标检查被尚未迁移的 Markdown 测试阻断；该测试随后已按宿主 presenter 合同迁移，见本节后文的最终验证记录。
- Issue #8 四份 release 报告已生成并检查，首次检查有三项时间门禁失败；具体数字与复测见性能记录。阈值没有调整。

续建验证：Core 174 项通过，新增表格定位测试后最终虚拟化回归 18 项通过；Vue 组件包 174 项、真实 Vue 调度 fixture 7 项通过。
Windows 原生 Agent 的百万项跳转、真实坐标点击和保留项释放通过，证据见性能记录。

最终验收尚在进行。不能将上述阶段结果解释为 120Hz、十万节点或全部平台已达标。

## 尚未实现或验收的范围

最新回归：Core 177、Runtime 803、Scene 94 项通过；真实 GPU `scene_paint::`
195 项通过；原生 Agent 百万项列表与冻结表格的两项截图/点击测试通过。
Scene 现在使用文本实际持有的自身裁剪范围提前剔除屏外内容，不借用会在滚动时保持
静止的祖先裁剪。Stroke 使用真实路径与端点外扩范围，视口保留一个物理像素的抗锯齿余量。
未裁剪文本、透视变换和不可靠的合成依赖仍保守保留，尚未实现通用文本 ink bounds。

后续无障碍投影优化：单次事务缓存祖先变换与边界，delta 不重复投影受影响节点。
Runtime 全量 805 项与原生 Agent 两项复验通过。新增首次系统阶段诊断，最新标准
Runtime 5000 节点首次系统处理 P95 为 20.111 ms，仍未通过 8 ms 门禁；并发编译干扰
与原始报告见性能记录，不宣称严格 A/B 收益。

Vue 后续优化：按选择器 AST 决定是否生成树关系匹配信息，简单规则下插入/删除不重算
无关兄弟。桥接层以指针索引保留大尺寸 SemanticWidget，借用和快照 API 不变。
776 项 Vue 回归通过；5000 项构造 P95 25.459 ms 已通过 40 ms 门禁，Issue #8
当前仍剩 Runtime 首次系统处理和全量布局两项时间失败。

以下是实际未完成的工作，不能用当前测试替代：

- Windows 系统 IME 候选窗口与输入法切换的实机行为；当前 Rust/Vue 测试覆盖框架 IME 会话及活动项保留。
- 文本及部分复杂绘制内容的精确 ink bounds；当前对不可靠范围及合成依赖保守保留。
- 全部图元类型的增量上传、完整的百万项 UI 操作负载、多窗口设备丢失和反复关窗/虚拟滚动后的内存压力验收；单窗口主动销毁 Device 后重新呈现的探针已通过。
- 120Hz Surface 门禁仍失败；已补充 60 秒真实窗口回调间隔、1/4/16 共享及独立资源的 GPU timestamp、进程内存峰值和 CPU 准备线程分配计数。仍缺完整复杂 UI/多窗口矩阵、全线程分配压力验证和 120Hz 显示反馈。
- Issue #8 时间门禁通过与所有目标测试；其他平台没有本轮证据。

因此这次代码不能标为整个方案完成，也没有确认 120Hz 的最终时间门禁达标。

Android 无障碍动作链路随后补齐：`SlotActions` 通过队列接收 TalkBack 请求，
`AccessTreeProjector::project_action` 统一校验后调用 Runtime typed action。该路径已
通过 Android host crate 编译检查；仍需真机 TalkBack、滚动和虚拟列表证据，不能替代
桌面平台验收。


后续布局优化：视口依赖按文档索引，外部文档 dirty ID 不再进入当前布局岛；叶节点完成
自身几何和 padding 写回后跳过空子布局准备。Runtime 811 项和真实 GPU Agent 两项通过。
完整 5000 节点布局 P95 15.022 ms，仍未通过 8 ms 门禁。阶段诊断及原始证据见性能记录。
尚需处理滚动指标发布的全树扫描。布局缓存跨文档失效已在后续修复，见下。


布局缓存后续按 DocumentId 隔离；全量布局只清理当前文档。关闭或停放最后一个根节点
自动释放 AppContext 中的对应布局缓存，低层宿主可调用 `remove_document`。
跨文档 dirty、交替全量/局部布局、空文档与停放后重新挂载的 Runtime 回归通过。


测量缓存后续修复了旧约束复用错误：自动高度内容从 20px 增长到 35px，再切回旧视口
曾恢复旧高度并错排后续行。现在按节点失效全部旧约束，跨帧每节点最多两组约束，单次
布局内部缓存不变。连续 256 次缩放均与全量布局比较一致且缓存数量受限。
删除节点立即清理其布局、测量、隔离位置与 padding 条目，无需扫描无关节点。
Runtime 815 项和严格 Clippy 通过；完整内存压力与 120Hz 目标仍未验收通过。


滚动指标发布后续改用实际局部布局范围选取 ScrollView 及其祖先，合并目标并保持文档
顺序，避免局部更新后再扫描完整文档。包含 1000 个无关节点、跨文档、嵌套顺序、停放、
内容缩小钳制和锚点恢复的回归通过。全量布局仍完整发布；受影响容器内部的内容范围
计算仍递归遍历后代，增量包围盒尚未实现，不能视为滚动成本目标全部达成。


后续已接入 UiWorld 布局内容边界索引，替代滚动内容范围的重复后代遍历。首次查询
惰性构建，后续单点几何变化沿祖先链更新子节点最大边界；结构变化仅重建相应父级聚合。
一万兄弟节点缩小测试重算 2 个节点、更新不超过 16 个聚合区间。显隐、重排、停放、
删除与原始遍历结果一致，反复增删和跨文档隔离回归通过；Runtime 821 项与严格 Clippy
通过。完整内存峰值、结构大批量变化及 120Hz 性能矩阵仍待验收，不能据此宣称全部完成。


内容索引进一步限制为已查询节点：其他从未查询过内容范围的文档不会分配脏索引工作。
首次查询仍从当前 UiWorld 构建，插入时已有父级的结构失效保证新节点正确接入。
新增 1000 个其他文档节点的回归，验证已缓存文档不重算且脏集合不增长；Runtime 822 项通过。
新增 `nana-framework-benchmark --profile-scroll-bounds`，用于 1万/5万/10万节点内容索引的
独立 CPU 诊断；它不替代完整 UI、GPU 或 Surface 的 120Hz 验收。


范围诊断进一步暴露了标量写入验证的拓扑复制：SetScrollOffset、SetScrollMetrics 和
WriteLayout 原本为了存在性检查物化 PlannedNode，并复制容器的完整子列表。
改为验证事务中的存在状态，仍识别批内创建/删除。新增一万子节点验证不物化拓扑、
新增后更新成功及删除后更新整批回滚的回归；Runtime 823 项通过。
浮层宿主的一致性验证后续改为反向依赖索引，详见下文；无浮层的内容索引计时不覆盖此复杂场景。


浮层校验与清理现在共享 `目标节点 → 引用宿主` 反向索引，事务阶段仅复验实际受影响的
宿主。文本、样式与标量几何的存在性检查不再复制子列表。活动表面的语义或父级变化
仍进行完整合同校验，无效批次仍整体拒绝。删除与停放通过统一写入口解除引用并释放空桶。

焦点校验的宿主候选按 DocumentId 索引；暂存文档顺序复用已有根索引并合并批内根变化，
不再为寻找根扫描其他文档。分离状态也在事务中暂存，使已分离内容退出暂存顺序和焦点
可见性，重新插入时恢复。多个模态浮层仍需枚举当前文档中的宿主候选，但根节点顺序查找
已改为一次索引构建后的常数时间查询。以上是增量工作量改进，不等同于完整 120Hz 性能
验收通过。

活动运行时浮层的顺序竞争已进一步优化：查询期间先建立当前文档的稳定 ID 顺序索引，
活动宿主不再逐个对整份文档调用线性 `position` 查找。z-index、文档顺序、可达性和
浮层类型过滤合同保持不变；Overlay 顺序行为回归及严格 Clippy 已通过。

删除节点还会根据删除前保存的 DocumentId 直接清理根集合，不再逐节点遍历所有文档。
1000 文档删除宿主的回归同时验证其他根仍保留。Runtime 827 项通过。


无障碍坐标投影后续取消对旧命中投影的优先读取，直接从当前 UiWorld 合成祖先变换，
并在单次投影内缓存。新增测试复现：提交祖先平移 100px、尚未重建命中索引时，原实现
仍返回 x=20，正确为 x=120。修复后重建前后投影一致，并与重建后的真实命中一致。
Runtime 828 项、真实 GPU 虚拟列表/冻结表格交互 2 项通过。完整性能门禁仍未通过。


全量命中重建后续直接将前序构建数据写入稳定条目索引，自底向上生成有序子节点范围
与包围盒，取消中间递归 HitEntry 树、再次展平以及单独遍历计数。局部子树替换继续使用
原有路径。对照原树构建验证多根、z 顺序、隐藏父级的可见后代、裁剪、滚动、全部条目
边界与 1600 个位置的候选序列；Runtime 829 项、严格 Clippy、真实 GPU Agent 2 项通过。
这是首次/结构重建路径的工作量改进，稳定帧与 120Hz 的验收要求保持不变。


GraphCanvas 大图元回归发现跨类别 slot 冲突：每类 300 项时，预期 1803 个图元只留下
430 个。现在边、节点框、分隔线、节点标签、端口与两类附属标签各占独立高位区间，
低位保存项序号，保持类别绘制顺序。回归验证 300→2→300→0 的完整数量、ID 唯一性、
文字数量及删除清理。此修复适用于已有 GraphCanvas，不增加百万二维对象专用接口。
Scene 控件侧栏集成测试也显式声明 controls 特性依赖；最小构建不再引用未启用控件。


GraphCanvas 后续增加真实 GPU 大类别验证：复用一个 painter 和目标，连续渲染
300→2→300→0 行，每行分别检查边线、节点框、端口像素，累计 3600 个采样断言。
缩减后消失的行均恢复背景色。三种规模截图已查看并归档；标签文字仍由此前文字测试
覆盖，未据此宣称整套复杂工作区或 120Hz 已验收。


Markdown Scene 测试迁移已完成：通过 RuntimeDocument 验证 300 段内容的完整文本投影
与删除，及公式/图表 presenter 槽稳定且不重复绘制文本。原先要求内建 SVG 的测试改为
当前宿主渲染合同；多于 256 个图元的身份和清理由独立 GraphCanvas 1803 图元回归覆盖。
Scene components 配置 101 项库测试、2 项文档测试、5 项侧栏测试与严格 Clippy 通过。
两个窗口消费者补齐 focus_on_show=true、constrain_to_work_area=false（与构造器默认
值相同）。此前一次工作区 all-targets check 已通过，构建迁移阻塞已消除。


消费者迁移后首次 workspace all-targets test 完成：39 个套件、2793 项通过，无失败或
忽略项。此结果早于下一项样式共享优化，日志已归档，不外推为后续版本的全量结果。
样式解析后续在结果与父节点完全相同时共享不可变 ComputedStyle，不引入全局驻留表。
1000 节点验证共享、局部颜色覆盖、父级字号更新以及旧快照不可变；Runtime 830 项与
严格 Clippy 通过，新的工作区全量验证另行记录。

样式共享后的 workspace all-targets test 也完成，2794 项通过。性能复测未证明整体收益：
首次系统处理标准 P95 21.516 ms，仍未达 8 ms，且高于前份报告。保留退化证据并继续
诊断，不以较好的单独样式阶段耗时替代完整门禁。

### 最终配置复验

当前 `rich-text` 组合执行 `cargo check -p nana-ui-scene --tests --features rich-text --locked`
与 `cargo test -p nana-ui-scene --features rich-text --lib --locked`，前者通过，后者 **97 项
通过，0 项失败**。它包含 Markdown 的 300 段文本投影、公式/图表 presenter identity、
以及 Scene 的可见集、滚动几何复用和 FramePlan 回归；此前移除的 `blocks` / `drawing`
接口已不再是该组合的构建缺口。

`python scripts/check-component-features.py` 也通过，覆盖 base、calendar、charts、controls、
image-viewer、rich-text、components 各自的 Runtime 注册表、Vue 声明和宿主适配器，并验证
关闭组件族后 Rust API 和族专属依赖不可达。

同次 `cargo check --workspace --all-targets --locked` 未能完成，但失败点已不是 NanaUI
源码：`v8 152.2.0` 的构建脚本在 Windows 上创建 `E:/codex-build/nanaui-high-refresh/debug/gn_root`
符号链接时收到 `PermissionDenied`。需要启用创建符号链接的系统权限或在具备该权限的
Windows 环境复跑，不能将这次环境失败计为工作区全目标通过。

同进程交替对照后，5000 节点 shared 总处理 P95 15.151 ms、控制路径 17.459 ms，60 对
中 46 对共享更快；10000 节点 49/60 对更快。保留共享优化，831 项 benchmark 配置
Runtime 回归及严格 Clippy 通过。此证据不替代原标准报告，不意味着 8 ms 门禁通过。


布局输入后续共用单节点投影：LayoutInputMap 缓存命中只查找一次，未命中直接投影，
取消为单个输入建立临时 Vec；完整预取直接使用文档顺序，不再复制 missing ID 列表。
公开批量投影接口和缺失节点行为不变。新增回归先复现额外批量分配，再验证缓存重复
读取、缺失节点与单节点物化数量。Runtime 831 项、严格 Clippy、真实 GPU Agent 2 项通过。
标准复测仍失败：首次系统 P95 18.940 ms，完整布局 P95 29.723 ms，不能宣称整体达标。

布局引擎的内部数字键 HashMap 后续采用已锁定的 hashbrown 0.16.1，不替换 UiWorld
存储或公共合同。旧/新 release 程序交替诊断显示布局阶段 P95 从 12.735–14.157 ms
降至 8.267–8.806 ms；共享工作区构建和系统波动仍限制因果归因。Runtime 832 项、
严格 Clippy、真实 GPU Agent 2 项通过。标准完整布局 P95 11.575 ms、首次系统
P95 18.149 ms，两项 8 ms 门禁仍失败；保留证据并继续优化，不宣称全目标完成。

隐藏容器局部命中更新修复：原实现跳过隐藏父级的条目，却把可见后代提升到祖先的
子范围中；局部更新随后可能直接返回成功而留下旧位置，或只取多个后代中的最后一个。
进一步验证发现同一省略还会丢失隐藏容器的裁剪和滚动入口。当前方案保留隐藏容器的
结构命中条目，自身不可命中，由条目持有后代的裁剪、变换、滚动偏移和兄弟顺序。
隐藏叶节点及不生成布局盒的子树不建立条目。之前的祖先范围扩张方案已移除；隐藏
根下的可见叶更新同样只重建 1 个条目，滚动不重建条目，容器隐藏/显示不再提升后代。
回归覆盖裁剪、文档顺序、滚动、后代移动、隐藏根和反复隐藏/显示；834 项 Runtime
测试和严格 Clippy 通过。4 项真实 GPU Agent 测试包含隐藏滚动容器的 Row 0/1→Row 1/2
截图、点击焦点、祖先平移和百万数据虚拟化回归。完整性能及所有复杂合成组合仍待验收。

无可命中内容的保留分支进一步在包围盒索引中标为 Inactive，与删除后的 Empty 以及
几何未知的 Unknown 分开。Inactive 不参与查询，但保留稳定槽位；子节点重新启用时
沿祖先路径恢复包围盒，不重建无关兄弟。自身不接受命中的容器仍合并可命中后代的范围。
修复前 1 万个不可交互根仍产生 1 万个范围候选；现在为 0。10 万不可命中槽位只访问
顶层范围；单个未知几何候选恢复后访问少于 40 个范围。1 万根/嵌套条目的删除、启用、
再禁用与完整重建一致。836 项 Runtime、4 项真实 GPU Agent 回归通过，截图已查看。

命中条目的 StableNodeId→IndexedHit 映射改用已引入的 hashbrown，其他 UiWorld 存储和
有序兄弟范围不变。保留旧二进制交替对照，5000 节点命中阶段 P50 从 3.474/3.512 ms
降到 2.864/2.840 ms；10000 节点也下降。836 项 Runtime、4 项 GPU Agent 和严格
Runtime Clippy 通过。标准首次系统 P95 16.692 ms，仍超过 8 ms；对照期间有其他
编译进程，不能将全部独立报告差异归因于映射替换。

无障碍投影的 transforms/bounds 两个临时数字键缓存也采用 hashbrown。对抽取/命中
共用 AncestorMemo 的三个映射替换未显示收益，已撤回。最终组合仍通过 836 项 Runtime、
4 项 GPU Agent 与严格 Runtime Clippy。阶段对照有显著波动，完整首次处理 P95
17.712 ms，门禁仍失败，不据此宣称完整性能改善。

隐藏祖先的无障碍链已补齐：此前可见孩子引用被省略的隐藏父节点，投影形成断链。
现在保留所需的中性 Generic 结构容器，隐藏容器不输出自己的标签、值、描述、原角色、
状态或交互能力；隐藏叶及不生成布局盒的子树仍省略。根和父容器隐藏、孩子独立隐藏/
显示的增量结果与完整投影一致。837 项 Runtime 测试通过；AccessKit 集成验证连接、
无操作的 GenericContainer、可见按钮操作、角色恢复与子树删除清理。4 项 GPU Agent
也验证点击后编辑器的完整无障碍祖先链及焦点。实际 Windows 屏幕阅读器会话仍待验证。

Windows 原生 UIA 验证进一步发现：直接把 Generic Runtime 容器作为窗口根时，
焦点进入编辑器后该根被 AccessKit 过滤，原生枚举变空，而 Runtime 祖先链仍完整。
hosted 适配器现在保留独立的 Window 根，文档挂载/替换/清空及控件焦点变化均不
改变原生窗口根身份。嵌入式 AccessTreeProjector 合同不变。22 项无障碍回归通过；
修复后的原生证据包含编辑后的持续枚举、Window 类型、真实聚焦/写值与呈现截图。
完整原生隐藏/显示脚本及屏幕阅读器会话仍分别记录验收结果。

后续完整脚本已确认隐藏→显示→隐藏的原生语义切换及正常退出，首条 stdin 的 UTF-8
BOM 由探针显式处理。该次运行被系统前台焦点条件挡住，整体结果仍为失败；不把多次
运行中的部分成功拼接为一次完整通过。实际 Narrator 会话尚未验证。

RuntimeProgramContext 使用显式 Clone 实现，共享已有 GPU 资源、调度入口与任务
队列，取消 derive 引入的 Message: Clone 限制。move-only 消息回归通过，最小原生
探针使用非 Clone 消息并从 stdin 线程 dispatch。该修复不更改 Workspace/Dock 格式。

无障碍增量链路进一步去掉每次局部更新构造完整 current_tree 的副本。平台激活与
更新共享一个保留投影；激活按需生成完整快照，原生事件回调前释放锁。嵌入式
AccessTreeProjector 初始化也不再生成随后丢弃的完整更新。
纯语义更新保留拓扑和文本运行 ID，跳过全树可达性、根重建、文本运行重整及父级
孩子列表查找；焦点用稳定 ID 集合增量维护，保持原有排序和删除回退行为。
结构或文本输入角色变化仍可能全树重整，尚未宣称所有局部结构更新均为增量。
24 项无障碍回归、1 项公共 AccessKit 集成、严格 Clippy 与 Windows UIA 语义模式
通过。万节点连续编辑无完整快照/全树重整，删除后再次激活返回最新完整树。
独立 release 转换诊断的 1万/5万/10万节点 P95 为 1.3/1.0/1.0 微秒，每次输出 2 个
节点；此结果不含布局、平台事件和呈现，完整性能门禁及原生前台焦点要求仍待验收。

未呈现的无障碍变化现在按目标累积：单批保持增量，多批升级为恢复时的一次最终
快照，空批不会覆盖已有工作。发布移到成功呈现之后、应用的 presented / bind 回调
之前，避免这些回调提交下一帧工作后被提前投影。目标消失不会写入主窗口的待发布
队列。27 项无障碍/待发布队列测试、41 项 scene_host 测试、6 项 hosted_context
测试及严格 Clippy 通过（这些过滤集有重叠，不能直接相加）。

原生故障注入还发现，直接丢弃已获取的 Surface 帧后，下一帧可能进入准备却无法
再次获取图像。新增 discard_frame / discard_surface_frame，在释放 encoder 和
帧 view 后标记目标，下次获取时仅重建受影响的 Surface，继续共享原 Device / Queue。
资源生产、原生内容准备及 painter 错误路径均接入，失败帧不提交。
静态 Text 同时修正为 AccessKit Label.value，使 Windows UIA Name 能读到文字。

Windows Vulkan 和 DX12 原生 UIA 语义模式均通过：两次资源编码失败后，成功帧
提交第三批独立变化，前两批新增文本/编辑器改名仍保留；恢复后的 UIA 写值、隐藏
切换和正常退出通过。失败阶段没有 submitted 或 presented 成功回调。
这不代替前台焦点、Narrator、辅助窗口失败、DComp commit 失败或性能矩阵验收。
同一宿主测试构建串行运行 434 项全部通过。默认并行运行此前长时间无进展，已
主动停止并保存记录，未把该次运行记为通过。

首次发布也已延后到成功呈现：预先 flush 的文档在首次资源编码失败时，原生 UIA
曾提前显示编辑器和 GPU 节点。主窗口和辅助窗口现在都先建立只有 Window 根的
适配器，取消初始化时的整份文档投影。首次发布不能以局部增量建立缺少基础节点的
树，因此补一次当前文档快照；显式完整源仍直接采用。首次发布后继续增量处理。
30 项无障碍回归、43 项 scene_host 回归（过滤集重叠）及严格 Clippy 通过。
Windows Vulkan / DX12 均验证首次失败期间只有 Window 根、恢复后发布完整内容，
随后继续通过两批失败后的增量恢复、UIA 写值和隐藏切换。普通按需 UI 的原生语义
路径也通过；未将这项减少预投影的改动声明为已测得的帧耗时收益。

辅助窗口的真实隔离验证已补上。双窗口探针让辅助目标经历首次未呈现失败、后续
增量失败与恢复，同时通过 UIA 编辑主窗口；最后关闭辅助窗口并再次编辑主窗口。
Vulkan / DX12 均通过：辅助目标未呈现时仍只有 Window 根，主窗口更新可呈现；
辅助目标恢复后包含全部预期变化，关闭事件到达应用，主窗口保留原值并继续更新。
单窗口回归、严格 Clippy 及脚本语法检查通过。本轮未修改产品代码，补充的是
此前缺失的真实多窗口行为证据；尚不等于多次开关后的内存上界或复杂工作区性能验收。

异步 URL 图片完成通知现在携带资源键。Scene host 在每个窗口最近一次提交的 Scene
上维护 `URL -> WindowId` 反向索引，图片解码完成只将引用该 URL 的目标加入重绘队列；
不再通过同格式 painter 扫描并唤醒全部窗口。窗口关闭、设备重建和 Scene 更新会同步
替换/清理索引。`SceneWgpuPainter::set_image_waker` 保留无参兼容入口，宿主使用新的
`set_image_update_waker` keyed 入口；图片内容仍不制造 Runtime 节点脏工作。
