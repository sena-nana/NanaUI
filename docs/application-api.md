# 应用 API

查入口用。第一次写应用先看 [开始](start.md) 和 [框架如何运行](how-it-works.md)。签名以 rustdoc 为准，这篇不复制每一份类型。

## 你该依赖什么

| 消费方 | crate / 包 | 入口 |
| --- | --- | --- |
| 新的桌面界面 | `nana-ui`（feature `hosted`） | `nana_ui::runtime`、`ApplicationState`、`RuntimeApplication`、`run_runtime` |
| 窗口设置 / 输入类型 | 通常经 `nana-ui` 再导出；需要时直接 `nana-ui-platform` | `WindowSettings`、`WindowCommand`、`InputEvent` |
| Vue 宿主 | `nana-ui-vue` + `nana-js-v8` | `nana_ui_vue::prelude`（`VueRuntimeProgram::run`） |
| Vue 控件 | `@nanaui/nanavue-components` | `NanaButton` 等 |
| Vue renderer | `@nanaui/nanavue-runtime` | `createApp()` |

不要直接依赖 `nana-ui-devtools`、`nana-css-parity` 来画产品界面。前者是无头调试，后者是 CSS 对照测试。

新代码从 `nana_ui::runtime` 引入控件。crate 根再导出是兼容面。`runtime::internal` 给 Gallery 和宿主适配器，不是第二套产品 API。`runtime::host` 是 Scene / GPU slot 类型；`runtime::perf` 是帧计数，不是视图状态。

`ActionDescriptor` 只有一个，定义在 Runtime（`nana_ui::runtime`，`nana_ui` 再导出同一个类型）：keymap 读 `id` / `enabled` / `when`，命令面板另外读 `label` / `category` / `keywords`。只绑快捷键的宿主用 `ActionDescriptor::new(id)`，要进面板的用 `ActionDescriptor::labeled(id, label)`。`ActionRegistry` 同样只有一个，按注册顺序保序，`search` / `available` 供面板检索。

Vue 产品窗口需要 `nana-ui-vue` 的 `hosted`（隐含 `scene-view`，把 UiScene 交给 `SceneWgpuPainter`）。没有 `scene-view` 的构建只做 flush / 对照，不画产品帧。

## Cargo feature

`nana-ui` 默认 `[]`。按职责打开：

| feature | 作用 |
| --- | --- |
| `hosted` | `run_runtime`、winit 0.31-line（workspace git pin，非 crates.io 0.31.0）、AccessKit；隐含 `gpu` |
| `gpu` | `SceneWgpuPainter`、`HostTexture`、`GpuTextureView`、`GpuView` |
| `bundled-fonts` | 嵌入 Noto Sans SC |
| `components` | 下面组件族的聚合 |
| `full` | fonts + components + hosted + syntax-highlighting |
| `calendar` / `charts` / `controls` / `graph-canvas` / `image-viewer` / `rich-text` | 适配器再导出。历史 no-op 别名（`overlays`、`selects`、`qr-code` 等）已删除，改用 `components` / `full` |
| `syntax-highlighting` | `TextArea` 的 `"highlight"` presenter |
| `accesskit-tree` | `AccessTreeProjector` 的 TreeUpdate 投影导出，给自接平台适配器的宿主（如 Android `accesskit_android`）；`hosted` 已隐含 |

Cargo 不会因你写了 `CalendarHeatmap` 就自动打开 `calendar`。

## RuntimeProgram

普通 Rust 应用优先实现 `ApplicationState`，由 `RuntimeApplication<State>` 管理
每个窗口的 `RuntimeDocument` 和资源注册表。最小可运行程序见
`crates/nana-ui/examples/application-counter.rs`。
下列低层合同仍用于 Vue 和自行管理窗口/文档的嵌入式宿主。

`ApplicationWindow::demand` 与低层 `RuntimeProgram::frame_demand` 使用同一
`FrameDemand`：默认 `OnDemand`，单次截止时间 `At(Instant)`，持续刷新
`Continuous(NonZeroU32)`。120 代表请求 120Hz，不是显示器刷新率保证。
资源更新使用 `TextureSlot`；详细变更和当前验收范围见
[高刷新重构](high-refresh-refactor.md)。

应用实现这个 trait，再 `run_runtime::<App>(RuntimeWindowSettings::new("…"))`。

| 方法 | 职责 |
| --- | --- |
| `initialize` | 建程序实例；可返回要在第一帧 `update` 的消息 |
| `with_document` / `with_document_mut` | 按 `WindowId` 在访问闭包中交出 `RuntimeDocument` |
| `update` | 宿主级消息；保持便宜 |
| `theme_mode` | 深色 / 浅色 |
| `window_material_mode` | 可选；默认实色 |
| `host_textures` | 默认；slot → `HostTexture` |
| `prepare_window_frame` | flush 前准备纹理 |
| `window_frame_presented` | present 后释放旧资源 |
| `scene_gpu_renderers` | 高级。`None` = 演示 `"gpu-view"`；空表 = 不画 |
| `scene_resource_producers` | 高级。按图离屏；第一次可忽略 |
| `bind_window` | present 之后填内容 |
| `rebuild_gpu` | 设备丢失后重绑资源 |
| `window_event` | 窗口生命周期 |
| `window_event` 里发 `WindowCommand::SetMenuBar` | 原生菜单栏；选中项用 `take_menu_activations()` 每帧 drain，见 [窗口](window.md#菜单栏) |
| `input_event` | Runtime 派发之后的原始输入，唯一的输入钩子。参数 `RoutedInput` 同时带 `event`、`pointer_hit`（仅指针与滚轮）和 `disposition`；已消费事件仍派发，应用快捷键应检查 `disposition.prevent_default` |
| `next_wakeup` / `wake` | 与重绘无关的定时工作 |
| `host_failure` | 宿主已从该错误恢复；默认忽略 |

`RuntimeProgramContext` 提供 `window_id`、`geometry`、`gpu()`、`material()`、`dispatch`、`run_task`。原生窗口句柄不穿过这条边界。

`RuntimeProgramUpdate.redraw` 支持 `None`、`Window(id)`、`Windows(ids)`、`All`。
合并局部更新会保留实际窗口集合；`RuntimeRedraw::for_windows` 会排序去重。
`RuntimeRedraw` 现在持有窗口列表，只实现 `Clone`，不再实现 `Copy`；穷尽匹配需处理 `Windows`。
Vue 输入按语义变化和已挂载节点消费的 Canvas／HostTexture 版本选择窗口。

## 建树

```text
RuntimeDocument::new(DocumentId)
AppContext::build(document, |ui| {
    ui.column(12.0, |ui| {
        let save = ui.child("save", Button::new("…"));
        ui.on(save, |_, Activate, cx| { cx.dispatch_program_all(Msg); });
    })
})
mount { scope.child("key", …) }          // 动态区增删
update_component(entity, |view, _| { … }) // 改单个字段
set_component(entity, Button::new(…))     // 整体换 props，保留交互态
```

`build` 是初次整页（一次 commit）。`mount` 是 keyed 子树协调，不是第二套渲染器。点击 handler 不要再 `build` 一遍。Vue 不得用 `create_component` / `build` 分配 ID，它绑定自己已有的节点。细则见 [L3 组成式建树](l3-authoring.md)。

从应用状态整体重建一个组件时用 `set_component`，不要在 `update_component` 里写
`*view = 新的()`：后者连运行时拥有的交互态一起覆盖，表现为刷新一下菜单就收起、
过滤框光标被重置。`set_component` 走 `ComponentView::reconcile`，由每个组件决定
什么该活下来（`Select` / `Dropdown` / `SearchDropdown` 保留展开与高亮，
`SearchDropdown` 还保留用户已输入的查询与光标）。props 真的变了仍会重置交互态。

已经持有实体、切换时还要保留其状态的区域，可用
`reconcile_children(parent_id, &[child_id, ...]) -> Result<bool, FrameworkError>`。
它发布父节点的完整子节点顺序，省略的子树停放而不销毁，合法跨父节点移动以及
停放父节点上的装配都支持；同序返回 `false`。缺失节点、重复项、环和跨文档移动
会在同一 Runtime 事务中失败，不会先停放其他孩子。它不创建组件或替代带语义的
slots / overlay 组装接口；`mount` 仍用于按 key 构造并销毁缺席组件的动态区域。

`create_component` / `append_child` / `on` 仍是底层 primitive。

对外身份是 `StableNodeId` / `Entity<V>`。不要依赖内部实体编码。

## 扩展控件

| 目标 | 路径 |
| --- | --- |
| 进入布局、命中、Scene | `UiExtension` + `register_component`；Vue tag 为 `ComponentTypeId` 去掉 `nana.`（与 HTML 同语义用原生标签；不同语义换名） |
| 仅 JS 命令 / props 白名单 | `NativeComponentRegistry` + `Nana.components.call` |
| GPU 内容 | `GpuTextureView` + 宿主纹理；直写见 `GpuView` |

不支持动态 dylib。

## JavaScript 产物形态

Vue / JS 入口交给宿主的是一份 `RuntimeArtifact`，两种形态：

| 形态 | 构造 | 说明 |
| --- | --- | --- |
| 源码 | `RuntimeArtifact::from_source(name, source)` | UTF-8 JavaScript（通常是你 Vite 打出的 IIFE）。框架在加载前用 `compose_runtime_artifact` 把 Web API shim 拼到前面。 |
| Binary Release | `RuntimeArtifact::from_v8_snapshot(name, bytes)` | V8 `SnapshotCreator::create_blob` 的快照。`is_binary_release()` 为真，**原样加载**，框架不再拼 shim。 |

因此快照必须在 `compose_runtime_artifact` **之后**编译：shim 要一起进快照，否则运行时找不到 `__nanaWebApi`。源码形态下框架会检测 `__nanaWebApi` 是否已存在，已拼过的不会重复拼。

`name` 同时是样式表解析的基准：相对 `@import` 与 `url()` 都相对它兑现（见[布局](layout.md)的 `stylesheet_base`）。

## 性能上你不用手写的

`build` 把整棵子树收成一次 commit。mutation 提交后 Runtime 自己调度脏工作。无变更不刷帧。大列表走 `materialize_virtual_*`。GPU 换纹理升 generation，不重建布局。

消息有两个入口，按类型选：`dispatch_program` **按 Rust 类型只保留最后一条**，适合「后一条取代前一条」的状态消息（resize、主题变了、请求重绘）；`dispatch_program_all` 按派发顺序全部送达。业务消息通常是一个 `enum`，那就是**同一个类型**——用 `dispatch_program` 会让同一帧内的两次点击塌成一次、悄悄丢掉第一次，这种情况用 `dispatch_program_all`。两者都在下一帧进入 `update`。

控件需要先于默认编辑处理按键时，用 `AppContext::on_key` 或 `on_view_key` 注册一个策略；后者读取当前保留的控件值。返回 `true` 表示消费，重复注册替换旧策略，删除视图会移除策略。`RuntimeInputAdapter` 在浮层处理后、默认编辑前调用 `dispatch_focused_key`，只投递给当前文档中已挂载且未禁用的焦点节点，IME 组合期间跳过业务策略。

保留编辑器绑定到另一个任务、文件或草稿身份时，即使文本相同也应调用 `clear_text_history(node)`，清除原对象的 undo/redo；它不改变文本、选区或正在进行的 IME。业务对象身份和是否允许重绑定仍由应用判断。

自接指针的组件可用 `UiWorld::pointer_layout_position` 将窗口坐标转换到布局坐标，反向用 `layout_pointer_position`。两者使用当前命中投影，包括祖先滚动和透视变换；无投影、已 park 或不可逆变换返回 `None`。输入之前应由既有帧流程刷新布局与命中投影。

## Rust 虚拟列表和树

需要真实滚动占位与屏外编辑保留时，将 List 放在 ScrollView 内，使用
`materialize_virtual_list_retained_in`；树使用 `materialize_virtual_tree_retained_in`
和只含展开行的 `VirtualTreeLayout`。列表示例（`list` 已挂载，`items` 跨帧保存）：

```rust
cx.materialize_virtual_list_retained_in(
    list, &mut items, &layout,
    VirtualViewport::vertical(offset, height, overscan),
    &[], // 额外业务编辑会话；焦点与 Runtime IME 自动保留
    |index| model.key_at(index),
    |key| model.index_of_key(key),
    |index, _key| TextInput::new(model.value_at(index)),
)?;
```

每行要绑事件时用 `materialize_virtual_list_retained_with`，它多收一个 `on_mount`
回调：**只**为本次新建的行调用一次（滚回已挂载的行不会重复调用），在提交之后执行，
可以直接 `cx.on(entity, ...)`。滚走释放的行连同 handler 一起释放。

项身份与内容按 key 保持；框架在组件外放置一个非命中容器以维护逻辑位置，
并管理 List 的完整内容高度。数据重排必须提供最新逆索引；删除、折叠或 key
不匹配会释放对应项。离屏导航先查询目标偏移并物化，布局发布后调用 `scroll_into_view(scroll, target, margin)`
（已在视口内的目标不动容器，只按最小距离滚动；虚拟化行仍需先物化才有布局盒），
再将焦点移到目标项内的具体控件。业务持久化草稿和选择仍按 key 保存。
旧物化入口仅协调身份，不要与定位入口混用于同一个非空 `items`。

## Rust 冻结表格

`materialize_virtual_table_retained_in(table, items, layout, viewport, frozen, retained_cells,
row_key_at, row_index_of, column_key_at, column_index_of, build_row, build_cell)` 使用同一
`VirtualTableLayout` 和 key 数据模型。`frozen` 按 `[列数, 行数]` 排列，`retained_cells`
是额外保留的 `(行 key, 列 key)`；焦点及 Runtime IME 所在单元格自动保留。
框架管理表格内容尺寸、行列绝对位置、冻结变换和冻结区域顺序；未指定单元格背景时使用
Surface。应用维护最新两轴逆索引，在视口/数据变化后调用入口；不要再覆盖受管理的变换。
离屏导航先 `reveal_cell_with_frozen`，物化、发布布局、滚动，再聚焦目标内的控件。
列卸载会释放单元格的全部嵌套视图和订阅。持久选择和草稿仍由应用按业务 key 保存。

## 非目标

- 完整浏览器、Tauri、裸 `@vue/runtime-dom` 产物、把整窗 WebView 当产品 UI
- 第二套 Device / Queue、CPU 回读伪装零拷贝
- 控件拿窗口句柄，或在 UI 画完后把原生 WebView 盖在 Surface 上
- 以 crate 根控件表或 Vue 的 DOM facade 定义新的框架合同

应用内打开网页见 [应用内浏览器](gpu.md#应用内浏览器)（未实现）。`nana-ui` 没有 `browser` feature。


### 多文档布局缓存

`RetainedLayoutCache` 按 `DocumentId` 隔离布局框、测量结果、隔离容器位置和已解析
padding。同一 `UiWorld` 交替布局多个文档时，一个文档的全量布局不会清空其他文档。
独立调用 `RuntimeLayoutEngine` 的宿主在文档关闭后调用
`RetainedLayoutCache::remove_document(document)` 释放该文档缓存；对空文档布局也会释放缓存。
`AppContext` 在删除、停放或 detach 最后一个文档根节点时自动释放布局缓存。
停放仍保留控件状态，重新插入后重新建立布局缓存。


保留测量结果按节点失效：内容或样式影响布局时，该节点及真实依赖祖先的全部历史
约束失效，避免切回旧视口时恢复过期尺寸。每节点跨帧最多保留两组约束；超过容量只会
重新测量，单次布局内的测量缓存不受此限制。低层宿主删除节点后可调用
`RetainedLayoutCache::remove_node(document, id)` 立即释放条目；AppContext 的删除
事务自动完成这一步，其他节点缓存不受影响。


### 局部布局后的滚动指标

局部布局仅为实际重排节点及其祖先 ScrollView 发布滚动指标，重复目标合并，并保持
文档顺序。其他文档、已停放节点及无关子树不参与目标选择。全量布局仍执行完整发布。
`restore_scroll_anchor` 会将容器加入下一次布局工作，确保没有样式变化时也能在测量后
恢复锚点。受影响滚动容器的内容范围由 UiWorld 的惰性布局边界索引计算。首次查询构建子树索引；
普通几何写回只更新变更节点及祖先的最大边界，结构变化重建对应父级的子节点聚合。
索引不包含绘制阴影、滤镜或滚动偏移，保持既有布局滚动范围语义；删除节点同步释放条目。

### Workspace 借用已有控件作为区域

`WorkspaceRegionSlot::new` 表示 Workspace 管理的结构区域，使用 Generic 语义且自身不接收指针。
如果直接将已有控件作为区域表面，使用 `WorkspaceRegionSlot::borrowed`：Workspace 仅投影区域布局和表面样式，
控件继续持有当前输入及无障碍语义；重新装配不会恢复旧的 label、disabled 或 focusable 快照。
`DesktopShell` 对直接借入的 ScrollView 自动选择此合同，普通结构区域保留原行为。
创建 Workspace 后须调用 `AppContext::assemble_workspace` 建立区域轨道，再进行布局和命中索引构建。

### 隐藏层级中的无障碍语义

`UiWorld::project_accessibility` 与增量投影会保留连接可见控件所需的结构祖先。
`visibility:hidden` 容器输出中性的 Generic 节点，其自身标签、值、描述、原角色及
交互状态均不对外提供；显式 `visibility:visible` 的孩子继续拥有完整的父子链和语义。
隐藏叶节点及不生成布局盒的子树不进入投影。变回可见后恢复当前业务状态中的语义。

启用 `accesskit-tree` 的嵌入式宿主可使用 `AccessTreeProjector` 消费完整节点或
`AccessibilityDelta`。隐藏结构容器对应没有点击/聚焦操作的 GenericContainer，
孩子的操作与焦点保留。Workspace/Dock 的持久化格式不受这项投影变化影响。

原生 hosted 适配器额外持有一个稳定的 Window 无障碍根。挂载、替换或清空文档，
以及焦点进入普通控件时，都不会替换窗口提供者的根身份；Runtime 节点仍保留自己的
角色与稳定 ID。无控件聚焦时，原生焦点回退到窗口根，不借用隐藏的结构容器。
这层窗口包装不改变 `AccessTreeProjector` 的嵌入式投影合同。

原生适配器共享当前保留投影，平台激活或再次激活时按需构造已发布状态的完整快照；更新和激活
共用同一份已提交状态，进入原生事件回调前释放投影锁。`AccessTreeProjector::new`
也只建立保留状态；嵌入式宿主需要首次完整树时显式调用 `full_update()`。
纯标签、值、焦点和边界更新复用根及文本运行索引，焦点按稳定 ID 增量维护；
结构或文本输入角色变化仍进行必要的重新检查。
静态 `Text` 的可访问内容映射到 AccessKit `Label.value`，让原生 UIA 的 Name 属性
包含文本；普通控件仍分别使用 label 和 value，编辑控件的名称与输入值不会混合。

hosted 宿主在布局收敛后暂存无障碍变化，在成功呈现后、应用的 `presented` / 窗口
绑定回调之前发布。未呈现期间的一批变化保留增量；累积多批变化时，仅标记恢复后
需要当前文档的完整快照，避免中间删除或重挂载使批次拼接失真。空闲重试不会覆盖
已有变化；各窗口独立保留待发布状态，关闭窗口不会转而修改主窗口的队列。

编码失败后，低层 `HostedGpuContext` 消费者先释放引用 Surface 帧的 view 和未提交
encoder，再调用 `discard_frame`（主窗口）或 `discard_surface_frame`（辅助窗口）。
下次获取时仅重建受影响的 Surface，继续使用已有 Device / Queue；不提交失败帧，
也不通知生产者提交成功。标准 Runtime 宿主自动处理这条路径。

新建主窗口和辅助窗口的无障碍适配器先只提供稳定的 Window 根，不预先读取文档。
首次成功呈现后才发布内容；应用在挂载前已经调用 `flush`，也不会使未呈现控件提前
进入原生树。冷启动没有基础树，首批局部增量需要补一次当前文档快照；有效的显式
完整快照可直接使用。首次发布后的局部更新继续沿用增量合同。

`RuntimeProgramContext<Message>` 可直接克隆并移入后台任务，仅要求 `Message: Send`，
不要求消息实现 `Clone`。克隆共享宿主 GPU 资源、消息入口与任务队列；后台任务通过
`dispatch` 唤醒宿主并提交消息，业务状态仍由应用更新回调处理。
