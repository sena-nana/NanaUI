# Runtime 与 Scene 渲染抽取合同

本文面向修改 `nana-ui-runtime`（保留树）或 `nana-ui-scene`（绘制图抽取与合成）的框架维护者。应用层心智模型请见 [框架如何运行](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/how-it-works.md)，宿主入口签名请见 [应用 API](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/application-api.md)。

产品主线严格保持单向：`UiWorld` → `ExtractedNode` → `UiScene` → `SceneWgpuPainter`。`nana-ui-runtime` 与 `nana-ui-scene` 均**不依赖 WGPU**；`nana-ui-scene` 不持有任何应用业务状态、窗口句柄或 GPU 设备对象。

## 1. `UiWorld` 的单一权威边界

Escape 先关闭当前活动 Panel／Dialog 内允许关闭的 Popover／ActionMenu，再关闭外层卡片。
背景卡片里的菜单不能抢走前景模态框的 Escape；一次按键只结束当前这一层交互。

`UiWorld` 是以下全部状态的唯一权威：节点身份（`StableNodeId`）、文档归属（`DocumentId`）、父子层级、节点种类、计算样式、文本与已提交选区、未滚动的布局盒（`LayoutResult` / `LayoutBox`）、滚动偏移（`scroll_offset`）、指针 hover / press / capture 状态、事件路由表、焦点链与 IME、无障碍投影、文本呈现器（text presenter）以及自定义渲染槽声明。

- **外部身份隔离**：对外只暴露 `StableNodeId` 与类型化的 `Entity<V>`；内部节点表存储结构的演进不得使外部句柄、诊断记录或持久化数据失效。
- **原子批式提交**：结构变更先排入帧内的 `MutationQueue`，整批校验通过后才一次性 commit；校验失败不会留下半成品层级，已销毁节点的 ID 永不复用。

## 2. 增量帧调度与动效生命周期

Runtime 按节点的脏标记（`DirtyMask`）确定性地生成各阶段工作列表（样式、文本整形、布局、命中索引、焦点与 IME、无障碍、渲染抽取）。静止的 `UiWorld` 返回空工作集，不请求持续重绘；局部变异只向语义上受影响的节点传播，遇到已处于相同脏状态的节点即停止。

`RuntimeDocument::flush` 在单次有界帧事务中调用宿主的 `TextShaper`，再由 `RuntimeLayoutEngine` 根据视口、样式与整形度量写回布局。当视口尺寸变化时，仅将文档根、`position: fixed` 节点及消费了变化轴约束（如 `vw` / `vh`）的盒子作为种子做增量重排，未受影响的子树直接复用保留缓存。若帧事务未收敛，已取出的脏工作会退回调度器，在 settle 之前不对外发布 Scene 或无障碍增量。

### 统一 Motion IR 与完成事件保证

所有动画（Rust L3 的 `ViewContext::transition`、`Spring::to`、`Timeline::parallel | sequence`，内置控件的 hover / switch / spinner / sidebar 过渡，以及 CSS 动效）统一编译进 `nana-ui-core::motion` 的 `AnimationSpec`，由宿主传入单调时钟求值，Runtime 自身不创建计时线程：

- **完成事件精确送达**：每一条已启动的动画都保证以**恰好一次** `Finished` 或 `Cancelled` 事件收尾。被同 ID 新动画中途替换的轨道上报 `Cancelled`（对齐 CSS `transitioncancel`），已到达终点但尚未被 `advance_animations` 回收的轨道上报 `Finished`。由提交触发的结束（`stop_animation`、`finish_animation`、替换、节点移除或停放）会一直保留到被消费为止（后续再有其他提交也不会冲掉它们）；只要还有未送达的结束事件，`next_animation_deadline` 就会报告当前动画时间以唤醒宿主派发，不依赖窗口是否正在 present。对于比估算 settle 时间略慢的弹簧动画，到达估算点后若尚未落定会按下一帧间隔继续检查，不会卡死在 `Duration::MAX`。
- **常驻节点的显隐生命周期（`PresenceLifecycle`）**：用于管理通过淡入淡出（可附带位移）切换显隐的常驻节点（如工具窗面板、浮动卡片、临时工具栏）。它复用视图层的 `Transition` 描述进出场，调用 `set_shown(cx, shown, now)` 时从当前呈现值直接出发（中途反向即 retarget），**只写合成器轨道而不改逻辑样式**；开始隐藏时先移出内部焦点，再立即将节点设为 `inert`（不命中、Tab 与编程聚焦跳过、无障碍动作被拒，但照常绘制与动画），开始显示时恢复它自己原来的 `inert`（首次挂载需静默隐藏时用 `set_shown_at_once`）。将每帧的 `AnimationFrame` 喂给 `apply_events(cx, &frame.events)` 后，它会在最后一条轨道结束时返回 `PresenceSettled::Shown` 或 `Hidden`；落定隐藏后自动将节点设为 `visibility: hidden`（移出绘制、命中与无障碍树，但保留布局身份）。在系统开启“减少动态效果”时自动以零时长、无位移执行并照常上报落定。
- **浮层退场终值**：模态框与菜单的透明度、位移轨道独立结束；先结束的退场轨道通过 forwards fill 保留终值，直到重开 retarget 或节点销毁，不回落到逻辑样式。宿主等所有退场轨道完成后才发 `OverlayChanged { active: None }`，应用收到后才能停放子树；`OverlayClosing` 和组件 `open=false` 表示确认已关闭，不表示可以立即停放。保留终值不产生动画唤醒。
- **稀疏采样与唤醒分离**：`advance_animations` 对 CPU / Layout / Paint 类轨道按 `frame_interval` 采样，对合成器安全（`AnimationClass::Compositor`）轨道仅注册启动与结束两个 deadline。`compositor_needs_tick` 只把存在活动合成轨道的那扇窗口切入轻量 present，不进入 CPU 动画 deadline，也不触发其他窗口重绘。活动指还会随时间变：已播完、靠 forwards fill 停在终值的轨道（显隐落定后的淡出、浮层退场终值）与暂停的轨道不算，它们的层照旧保留、终值照旧显示，但不再让窗口持续出帧；等待提升或降级的保持期仍算。

### 文档呈现时钟（Issue #300）

一个文档（一个 `UiWorld`，即一个 `RuntimeDocument`）只有一份呈现时间 `animation_now`。轨道、合成器 overlay、揭示与图表过渡都在它上面采样。宿主的单调时钟只是播放驱动：`advance_animations(now)`、`sync_presentation_clock(now)` 与输入时间戳传入的都是宿主时间，由文档的呈现时钟映射到 `animation_now`。

- **播放与停住**：`AppContext::set_presentation_playback(PresentationPlayback::Playing { rate } | Paused)`。默认 `Playing { rate: 1.0 }` 且锚点为 0，映射是恒等，与此前直接写入宿主时间相同。停住后宿主帧、合成器 present 与输入都不再改它；重新播放时，下一次观测到的宿主时间与停住的时间配对，停住期间不计入。速率非有限或不为正时按停住处理。停住只影响这一份文档。
- **设定时间**：`AppContext::seek_presentation(t)` 一步完成两条求值路径：`animation_now` 设为 `t`（合成器与 `sync_presentation_clock` 读它），所有运行中的轨道无论下一 deadline 在何处都在 `t` 采样，Layout 类属性写成 `t` 时刻的绝对值，因此可以倒回。合成器轨道只读时间，不写 `UiWorld`。seek 越过终点的轨道照常结束，`Finished` 留给下一次 advance 送达，仍是恰好一次；已结束的轨道不会因倒回而复活。往返 seek 的结果与两次之间是否 present 无关。在程序回调里 seek 后照常返回该窗口的 `redraw`。
- **步进与帧间隔**：`step_presentation(interval)` 等于 seek 到 `now + interval`，并把 `interval` 记为当前帧间隔；播放时帧间隔是宿主报告的显示间隔（`set_presentation_display_interval`，窗口宿主按合成器节拍与 `max_frame_rate` 取小）。`seek_presentation_frame(t, interval)` 是绝对时间版本：seek 到 `t`（可前可后），同时把帧间隔记为 `interval`，供按步长出图使用（Issue #304，见 [窗口外输出](output.md)）。快门与取样容差读 `presentation_frame_interval()`。
- **唤醒**：`AppContext::next_animation_deadline` 返回宿主时间：文档 deadline 经呈现时钟反向映射，停住时只对已到期的事项（待送达事件）立即唤醒。停住的文档 `RuntimeDocument::compositor_needs_tick()` 为假，没有其他刷新原因时窗口保持 `FrameDemand::OnDemand`。
- **仍用墙钟**：输入、滚轮与触摸惯性滚动、光标闪烁、tooltip / hover card 延迟、拖动与防抖读宿主时间（`presentation_host_now`），不随文档停住。
- **整段替换轨道（Issue #325）**：`UiWorld::animation_tracks(node)` 列出节点上运行中的轨道；`MutationQueue::replace_animation(spec)` 以同一 id 整段替换，按原样采用新 spec，不从旧值 retarget，不发 `Cancelled`，不新建 id，由下一次 advance 在文档当前时间求值。停住的文档因此停在当前帧显示改后的曲线，而不是从 0 重播。id 不存在时提交失败（`MissingAnimation`）。拖动播放头只走 `seek_presentation`。

### 按呈现时间取样的资源（Issue #303）

视频帧、图像序列和精灵帧是不同的生产者，用同一份映射对齐文档时间；槽仍然是 HostTexture，解码、音频与封装留在应用里。

- **映射**：`Video` 与 `GpuTextureView` 的 `media_time: Option<MediaTimeMapping>`（`MutationQueue::set_media_time` 是底层写法）。`media_time = in_point + rate * (now - origin)`，`now` 是文档的 `animation_now`，资源时间以秒计。`rate` 为 0 时停在入点，为负时倒放；设了 `out_point` 时结果夹在入点与出点之间，停在到达的那一端。倒放和变速只改映射（`MediaTimeMapping::retimed(now, rate)` 从当前位置接着走），节点、自定义绘制与布局都不重建。`UiWorld::media_time(node)` 读当前值。
- **绘制前的请求**：`UiWorld::media_sample_requests(document)` / `RuntimeDocument::media_sample_requests()` 交出每个可见（已挂载、在文档树上、未被隐藏）且带映射的宿主纹理节点的 `MediaSampleRequest`：节点、槽名、`media_time`、文档帧间隔与 `MediaPacing`（播放时 `Realtime`，停住或按步长推进时 `Stepped`）。宿主在 `prepare_window_frame` 里读它，此时呈现时钟已同步、flush 尚未开始，再用 `FrameExchange` 发布对应时刻的帧，资源时间记在帧载荷上（见 [GPU 合同](gpu.md#按资源时间取帧-issue-303)）。没有这类节点时请求为空，也不为它唤醒。
- **唤醒**：播放中的文档在有可见资源的时间仍在走时持续出帧；映射停在端点、`rate` 为 0 或文档停住时不唤醒，静止的文档保持按需刷新。改映射只标记该节点重绘，下一帧按新时间请求。
- **进度条**：`MediaTransportBar::source(Some(node))` 让进度条的位置读该节点的 `media_time`，`duration` 仍由应用给出，条上不另存播放头。拖动只改条上的读数，不写映射，也不写文档 `now`；拖动期间宿主对进度条的写入不会把预览读数当成位置。松手后的 `MediaTransportEvent::Seek(t)` 由应用写进映射（`MediaTimeMapping::seeked(now, t)`）或文档时间。

### 动画属性分类表

逻辑值存放在 `UiWorld`，瞬态呈现值存放在 `PresentationStore`（通过 `UiWorld::presentation_pair` 或 `applied_value()` 查询，严禁每帧把 `transform` 或 `opacity` 写回 `UiWorld`）：

透明度动画作用于完整节点的合成结果。有子节点、自定义 painter 或复合控件绘制的节点，先将自身与子内容画进同一个 dest 隔离组，再应用一次当前呈现透明度；不能分别淡化卡面、文字与控件，否则重叠区域会叠加 alpha。分组从首帧开始生效，不等待 compositor layer 提升，覆盖退场、反向与终点 hold；组内仍保留子节点自己的透明度。组承担的透明度不再传入 CPU 图元或 Quad 的 GPU motion id，模态 scrim 仍在卡片自身分组之外独立淡化，触发型菜单的触发器也不进入自身浮层的分组（见下节）。只有分组成员变化时更新保留绘制顺序和图元；时间采样不触发 Runtime 抽取、布局或几何重建。纯透明度为 1 的组可省略离屏合成。

| 属性 | 默认分类 | 执行路径 | 回退与性能边界 |
| --- | --- | --- | --- |
| `transform` / `opacity` | `Compositor` | 呈现 overlay；Quad（含 `QuadBatch` / `QuadColorBatch`）剥离 overlay 走 GPU `evaluate()`；Text / Icon / Mesh / HostTexture 走 CPU overlay | 稳态不写 `UiWorld`；仅 Quad 走 GPU 求值，避免剥离了 overlay 却无法在着色器还原 |
| `clip` / `clip-path` | `Compositor` | 呈现 overlay（`compositor_gpu_motion_ids` 当前仅绑定 transform / opacity） | 稳态不写 `UiWorld`；裁剪呈现走 CPU overlay |
| `shader-parameter` | `Compositor` | 呈现 overlay（需注册类型化编解码器） | 无默认 Quad GPU 路径；图表动画走专用呈现时钟（见下节） |
| `filter` | `Compositor` | 呈现 overlay；场景在文档 `now` 采样整份 `ColorFilter`，写入该节点隔离组的 uniform（见下文“隔离组效果”） | 稳态不写 `UiWorld`，不脏布局和文字 |
| `color` / `background` / `blur` / `shadow` | `Paint` | CPU 插值（按 `frame_interval` 稀疏采样） | 每次采样可能标脏 paint / extract。文字色、背景色和控件自己的 `box-shadow` 留在这里，按钮 hover 不改道 |
| `width` / `height` / `padding` / `margin` | `Layout` | CPU 布局求值，采样写入 px 并标脏 `LAYOUT` | 子树测量与命中均使用真实几何，严禁用 `scale` 伪造 |
| `font-size` / `font-axis` | `Layout` | 非合成器路径（触发文本整形与排版） | [#85](https://github.com/sena-nana/NanaUI/issues/85) 不强制走 GPU |
| `display` | `Discrete` | `snap_discrete`（结束前保持起点，结束时切到终点） | 不做连续插值 |

## 3. Scene 抽取、合成层滞回与像素吸附

`flush` 将脏节点抽取为 `ExtractedNode` 增量并交给 `UiScene::apply_delta` 更新保留绘制图。合成器类的呈现属性（`transform`、`opacity`、`clip`、`shader-parameter`）不烤进 `ExtractedNode` 的逻辑样式，而是由 `UiScene::apply_presentation` 直接从 `PresentationStore` 与 `MotionDescriptorStore` 引用绑定到 `CompositorLayer`（严禁每帧克隆描述符表，仅时间推进绝不触发重新抽取）。

- **合成层提升与降级滞回（Hysteresis）**：层的身份即节点的 `StableNodeId`。`LAYER_PROMOTE_HOLD` 为 `16 ms`（低于一帧的瞬时闪烁不提升），`LAYER_DEMOTE_HOLD` 为 `120 ms`（动画结束后保留层 120 ms 再降级，避免连续动画频繁建拆层）。`OpacityGroup` 与 `FilterGroup` 是目标纹理（dest）隔离组，不等同于 motion layer。竖向裁剪、样式带 `PaintStyle::scroll_edge_fade` 的滚动区也是 `OpacityGroup`：组上的 `EdgeFadeMask` 记着裁剪框和按当前偏移算出的两条带长，合成时 alpha 在带里按目标像素行线性降到 0。组的成员身份只看样式，滚到两端也不重排；两条带都为 0 时 `is_identity()` 为真，绘制器不合成它。它的滚动条图元用去掉渐隐的同一组（`primitive_opacity_groups`）；浮层表面（`VisualNode::Overlay`：触发型弹层表面 `TRIGGERED_OVERLAY_SURFACE_SLOT` 与展开的 `Select` 菜单）与无触发器 `Popover` 按各自在 Runtime 解析并写入 `ExtractedNode::overlay_visual_parent` / `ExtractedNode::visual_parent` 的 `VisualParent`（`Root`、`TopLayer`、`Modal(id)`、`Node(id)`、`Overlay(id)`）沿视觉父链继承分组、不透明度、绘制顺序与命中栈，而不是污染逻辑 DOM 父链；普通逃逸滚动裁剪链的节点（含 `position: fixed` 与逃逸 `#201` 绝对定位盒）与 `VisualNode::Overlay` 均会剥离自己已脱离的滚动容器 `edge_fade`。只做渐隐的组（`is_edge_fade_only`）不挡子树的视口剔除。
- **首帧呈现不等待层提升**：`PendingPromote` 期间，Scene 保留当前 Presentation 的 CPU 投影，绘制、祖先透明度、变换、裁剪和可见范围查询从 0 毫秒起就使用它；不修改 Runtime 逻辑样式，不创建合成缓存层，也不提前绑定 GPU motion id。达到 16 毫秒后由正常层接手，不能出现先按基础 opacity=1 绘制、再跳回淡入中的一帧。等待期间结束或移除的轨道会清除临时投影并失效相应绘制缓存。
- **模态框（Dialog / ConfirmDialog / Drawer）的支点与遮罩分离**：模态框在单个节点内同时生成背景遮罩（scrim）与中心卡片图元。其 motion layer 作用于**卡片**：卡片及插槽内容绕卡片中心（`WidgetPaintOutput::presentation_pivot`，由 `ModalFrameVisual::presentation_pivot` 提供）做位移、缩放与淡入（命中测试、CPU 呈现与 GPU `evaluate()` 共用同一支点）；而遮罩图元不叠加节点自身的 opacity 轨道（仅继承外层祖先不透明度），改按 `ModalFrameGeometry::scrim_opacity` 独立淡入，并向外预留卡片位移与缩放的最大外扩边距以始终铺满窗口（查询单图元呈现使用 `UiScene::compositor_primitive_opacity` / `compositor_primitive_encode`）。
- **触发型菜单（Popover / ActionMenu / HoverCard）的触发器与浮层分支分离（`VisualNode::{Node, Overlay}`）**：菜单节点本身作为 `VisualNode::Node(id)` 留在逻辑与文档流里承载触发器（底板、文字、图标、头像与 `trigger_content` 子树），始终按祖先分组和逻辑样式合成；而挂出去的浮层面（`TRIGGERED_OVERLAY_SURFACE_SLOT`）与 `Select` 展开菜单属于 `VisualNode::Overlay(id)`，承载开合轨道（`MENU_OPACITY` 淡入、`MENU_POP` 0.9→1 缩放，菜单项通过 `VisualParent::Overlay(menu_id)` 挂在其下）。所有浮层原语（`Popover`、`ActionMenu`、`AnchoredActionMenu`、`ContextMenu`、`HoverCard`、`Select`、`Dropdown`、`SearchDropdown`、`Tooltip`）通过 `OverlayMount` 声明挂载范围：默认 `NearestModal`（页面中解析为 `Root`，模态框内部解析为 `Modal(id)`，避开模态框内滚动渐隐与中间组并正确排序于高 z-index 模态框之下），`TopLayer` 解析到全窗口最顶层，`Inline` 解析为 `Node(parent)` 随直接祖先一起渐隐与受滤镜影响（普通 `Stack` / `NodeStyle` 无隔离开关）。浮层的缩放由 CPU 投影承担（`UiScene::hanging_transform`，菜单节点不绑定 GPU transform id）；Runtime 的命中测试、无障碍坐标、`presentation_input_bounds` 和浮层面命中沿同一 `VisualParent` 链计算栈序并从同一支点套用同一变换，提升前后一致。
- **ReorderList 活行拖动**：被拖行跟手和其它行让位都是行节点上的合成器 transform 轨道（跟手是随指针替换的保持轨道，让位是带 Retarget 的过渡并在终点保持），不写行的逻辑样式和布局盒，落点仍按布局盒判定。落点变化只给位移目标变了的行重启轨道；指针移动既不重新抽取列表，也不重新抽取行。被拖行的层级 `REORDER_PREVIEW_Z_INDEX` 低于 `MENU_OVERLAY_Z_INDEX`，只在拖动开始和结束时让被拖子树各重新抽取一次。拖动结束（松手、Escape、`Cancel`）时取消全部轨道，不留呈现。
- **列表重排 FLIP**：在终点（Last）布局已提交后，仅播放一条合成器平移轨道（L3 为 `node().flip(first, last)`，可选配 `animate_size()` 另开一条 Layout 类宽高轨道）。
- **设备像素吸附（Pixel Snapping）**：画笔将图元及其每一级裁剪区的**最终合成纯平移分量**吸附到整数设备像素（`clip::paint_transform`，旋转、缩放与透视分量保持不变）。滚动偏移、CSS `translate` 与冻结行列反向平移会先合成为总平移再一次性吸附，从而保证同一滚动容器下的 Quad、图标、文字、路径网格、HostTexture 与裁剪边永远同步移动相同数量的整像素；同时 `ScrollOffset`、命中测试与无障碍边界仍保留浮点数精度，因此触控板的亚像素横向滚动不会触发文本重新解析（#223）。

## 4. 图表专用呈现时钟（Chart Presentation Clock）

图表（`Chart`）的入场、数据更新过渡与悬停高亮不经过通用 Motion IR：布局阶段直接为每个图元写好起点 `from` 与终点 `to`（由 `ChartMarks::transition` 记录起始时刻、时长与缓动，悬停记录 `HoverState::since`；连续更新数据时，新布局的 `from` 直接取上一布局在当前时刻的瞬时插值位置，因此连续刷新平滑无跳变）。

图表 WGSL 着色器直接按 `motion_time` 算出插值进度——当一帧中只有时间推进时，Runtime 不写 `UiWorld`、Scene 不重新抽取、painter 也不重新上传顶点数据。Scene 通过 `chart_live` 记录动画到期时刻并汇入 `compositor_needs_tick()`，在到期前驱动窗口轻量出帧并跳过 dest 缓存复用。

## 5. 裁剪路径（`clip-path`）、投影滤镜与背景毛玻璃

### CSS `clip-path` 与圆角 `overflow`

祖先节点的 `clip-path` 与圆角 `overflow` 进入 `UiScene::ClipRegion` 链，并按复杂度分流到硬件 Scissor、片元着色器 SDF 或目标合成组（dest group）：

- **轴对齐矩形**：走硬件 GPU Scissor + 顶点 `FragmentClip`。
- **`clip-path: inset(... round R)` 与圆角 `overflow`**：圆角半径写入 `FragmentClip.corner_radius`（非平移变换或旋转下仍保留精确的圆角矩形 SDF，不会把半径清零；HostTexture 同样通过 `clip_inv_ef.z` 共用同一套圆角 SDF）。单层圆角裁剪留在顶点属性中完成；多层嵌套圆角或带旋转的外层裁剪自动升级进 dest 合成组。图元画到的像素（每个合批命令记录的范围）都在某层轴对齐圆角裁剪的矩形之内、又碰不到它四角 `r × r` 的圆角区时，这层裁剪在那里处处覆盖为 1，不再开组（常见的是圆角卡片里输入框的文字）；GPU 节点、遮罩与组照旧开组。节点自身表面（slot 0）只挂直角裁剪，避免对本身已是圆角的填充和描边重复抗锯齿。
- **`clip-path: polygon(...)`**：Scene 同时保存轴对齐包围盒（AABB）与局部多边形顶点。节点自身的 Quad 在片元着色器内直接做点在多边形内测试；子节点、文本与 HostTexture 则通过 dest 合成组在合成 pass 中执行环绕数（winding）多边形裁剪。

### CSS `filter: drop-shadow(...)`

`filter: drop-shadow(offset-x offset-y blur color)` 通过 dest 合成组实现：先将子树绘制到独立的 group 纹理层，合成时采样该层的 **alpha 轮廓**，按偏移平移 UV 后用与 `filter: blur()` 相同的高斯核模糊并着色（模糊半径上限为 `16 px`，选择放宽的组为 `64 px`，见下节），最后以 source-over 垫在原图下方。多层 `drop-shadow`、`spread`、`inset` 及未知滤镜函数严格按 fail-closed 跳过。

### 隔离组效果（Issue #302）

隔离组（dest 合成组）上的效果按文档呈现时间 `now` 求值，结果进入组 uniform，不写回 `UiWorld`。没有镜头类型：调色、英雄区转场、打开快门的组都是这个组的用法。一个节点仍只有一份 `ColorFilter`；第二层效果是嵌套的隔离组，外层包住内层。

- **滤镜按呈现时间求值（#312）**：`filter` 是 `Compositor` 类。轨道的值是整份滤镜 `MotionValue::Filter(FilterValue)`：亮度、饱和度、对比度、色相、反相、滤镜不透明度、模糊半径和一层投影（偏移、模糊、sRGB 颜色）逐通道插值；没有投影即透明投影，没有滤镜即恒等滤镜，所以 `none` 与滤镜之间可以过渡。`logical_motion_value(Filter)` 返回节点的逻辑滤镜，因此视图的隐式 `transition` 不用新 API 就走这条路径；CSS `transition: filter` 和 `@keyframes` 里的 `filter` 也编成这种整份滤镜轨道，停点里随主题变化的投影色在播放开始时按当前主题求值。`UiScene::apply_presentation` 在 `now` 采样，覆盖该节点组的 `OpacityGroup::filter`；采样只改组 uniform 与 `presentation_epoch`，不重建图元、不抽取、不排布局。有滤镜轨道的节点从首帧起就是隔离组（叶子也是），它自己 quad 上烘焙的滤镜随之跳过，滤镜只施加一次；轨道移除后回到逻辑滤镜。触发型菜单的滤镜像它的淡入一样作用于挂出的浮层面，触发器不受影响。
- **模糊上限可选放宽（#314）**：普通节点和未选择放宽的组，元素模糊与投影模糊上限是 `ColorFilter::MAX_BLUR_RADIUS`（16 px）；`PaintStyle::wide_filter_blur` 让这个节点的组放到 `MAX_WIDE_BLUR_RADIUS`（64 px）。上限在场景里按组各自施加（`ColorFilter::with_blur_cap`），所以别的组放宽不会放宽按钮，Rust 直接写入的大半径也按所在组的上限处理；CSS 解析只把值限在 64 px。16 px 及以下仍是原来的 5×5 核，结果不变；放宽后采样随半径增加，间距不超过 8 px，64 px 时为 17×17。`backdrop-filter` 保持自己的上限。
- **快门（#323）**：`PaintStyle::motion_shutter` 是曝光占文档帧间隔（`presentation_frame_interval`）的比例，默认 0 即关闭，大于 1 按 1 处理。打开且该节点自己的 `transform` 轨道生效时，组按该轨道在 `now` 的速度乘以曝光时长得到拖影向量（页面逻辑像素，最长 `MAX_MOTION_STREAK` 64 px，超出截断），合成时沿拖影平均采样，拖影落在运动方向的后方。零长度跳变的速度为 0，不拉出残影；子节点在组内的相对运动不计入；命中、布局和无障碍仍按未拖影的框。宿主在 `apply_presentation` 前调用 `UiScene::set_presentation_frame_interval`（`RuntimeDocument` 已这样做）。投影不随拖影拉长。
- **祖先链深度（#324）**：合成顺序是先画后代，再施加这一层的滤镜和混合，父级采到的是已处理过的结果。同一条视觉祖先链上非恒等 `ColorFilter` 最多 `ColorFilter::MAX_CHAIN_DEPTH`（4）层：第 5 层起保持恒等，组本身、它的不透明度和子树仍在，并为该节点报告一次 `runtime.paint.filter_depth_exceeded`。只有不透明度的组不计入。叶子 quad 自带的滤镜也算一层：绘制器在祖先已有 4 层时不再施加它。

### CSS `backdrop-filter: blur(Npx)`

`backdrop-filter: blur(Npx)` 是**逐节点**的实时背景模糊效果（与 `nana-window` 的整窗系统级 Mica / Acrylic 材质无关）：

- 在绘制该节点自身填充之前，painter 从当前目标纹理（`dest.color` 或外层 opacity-group 层）采样该节点包围盒背后已按文档顺序绘制的全部内容（包括排在它前面的 Quad、HostTexture 与自定义 GPU 节点），经可分离高斯模糊（separable Gaussian blur）后再按节点的圆角、`clip-path` 与 `mask-image` 合成回节点区域，最后叠上节点自身的半透明填充或渐变。
- **降采样金字塔与边缘延展**：模糊半径 `r` 为逻辑像素标准差（自动乘 DPI 缩放到设备像素，采样核覆盖 $3\sigma$ 并利用双线性采样合并相邻纹素，杜绝条纹伪影）。当标准差超过 `8` 个设备像素时，先将待模糊区域按 2 的幂次盒式平均缩小，在缩小的副本上执行模糊后再双线性放大，因此即使 `64 px` 的大半径模糊在任何缩放下也只需几十次采样；采样边界向外按边缘像素延展（clamp-to-edge），不会在边缘渗入透明黑边。
- **与节点不透明度联动**：模糊后的背景与节点填充一起受节点自身 `opacity` 控制——半透明毛玻璃呈现半强度的模糊，淡入的模态遮罩会让背景模糊一起平滑淡入，而不是在第一帧突兀地整块变糊。
- **MSAA 与目标交错策略**：包含 HostTexture、自定义 GPU 节点、dest 合成组或 `backdrop-filter` 的帧统一使用单采样目标交错合成（`sample_count = 1`）；纯矢量/文本 UI 帧则可使用 `4x MSAA` 绘制方块与网格，并在 resolve 之后绘制清晰文本（绝不在自定义 GPU 节点两侧反复 resolve）。

## 6. 文本高亮呈现（`TextPresenter`）

代码高亮等富文本着色通过声明式的 `HighlightRequest` 与 `TextPresentation` 表达，具体词法分析算法由按名称注册的 `TextPresenter`（经 `ExtensionRegistrar::register_presenter` 安装）提供。Presenter 仅读取已提交的 UTF-8 文本，IME 组字（preedit）期间保持单色；当语言未知或对应 presenter 未注册时，Scene 安全退回单色文本。

`HighlightRequest::with_overlay` 允许宿主叠加额外的语义着色区间（重叠处以 overlay 优先）。`SelectableRichText` 的 `RichSpan::color` 即复用该通道：所有分段仍合并为同一段连续文本与单个文本图元，带色分段按 UTF-8 字节区间写入 overlay，并在 Scene 抽取时按当前激活主题的调色板解析为最终颜色。

## 平滑滚轮的交互状态

行滚轮的目标属于 `AppContext` 的短期交互状态；`UiWorld::ScrollOffset` 仍是当前有效位置的唯一权威。该行为复用 Motion IR 的 easing 求值与 Runtime 绝对时钟，通过 `advance_animations` / `next_animation_deadline` 推进，按 8 毫秒采样间隔请求宿主帧，不另建定时器或变换动画。每次实际移动沿原有滚动提交路径同步命中、无障碍、滚动条与虚拟列表，不写未滚动布局盒，不触发整棵树布局。

连续输入从当前采样位置重定向：同向累加目标，反向按当前可见位置计算新目标。公开定位 API 和像素滚动保持即时语义并取消过渡；减少动态效果使过渡立即落到目标。范围变化时重新夹紧目标，节点卸载、移除或文档销毁时丢弃短期状态，空闲没有滚动 deadline。

滚动采样只在存在对应 `ScrollChanged` / `UserScroll` 监听器时进入事件组件更新；没有监听器时，偏移提交直接提供绘制、命中和无障碍失效，不创建空事件事务。有监听器时保留原有事件事务、观察者更新及顺序：先提交 `ScrollChanged` 回调的修改，再读取有效位置发送 `UserScroll`。自动隐藏滚动条的保持计时与订阅无关。活动容器的采样列表复用内存，回调取消或移除容器不会使过渡复活。

## 触摸滚动与 `ScrollPhysics`

`PointerType::Touch` 的拖动、甩动与回弹属于 `AppContext` 的短期交互状态；`UiWorld::ScrollOffset` 仍是当前有效位置的唯一权威。宿主用 `AppContext::set_scroll_physics` 选档位（`ScrollPhysics::Ios`，默认；`ScrollPhysics::Android`），档位只改变触摸滚动，滚轮、滚动条、键盘与 `scroll_to` 在两档下相同。已经在跑的甩动按开始时的档位走完。

**手势。** 按下只记下候选，按压照常落到控件上。手指离按下点超过 8 逻辑像素（slop）后，按位移较大的那一轴锁定（双轴容器也只沿这一轴走），从命中节点向上找 `ScrollView` 或 `overflow: auto|scroll` 盒（遇到阻挡型浮层的根、`position: fixed` 节点停止，`TextArea` / `RichTextEditor` 不参与）：取最内层在该方向还能滚的容器；都到边时取最内层在该轴有滚动范围的容器（iOS 借此拉出橡皮筋）；该轴无可滚容器则不接管，手势之后的样本按普通指针处理。接管时先取消子控件的按压（`release_pointer_press`，松手不再触发点击），再把指针 capture 给容器，并清除该指针的 hover；位移从越过 slop 的点量起，不跳 8 像素。嵌套滚动只在接管时决定一次：内层还能滚就交给内层，到边才交给外层，同一次拖动中途不换容器。

**与控件的让渡。** 已经 capture 指针的控件保留手势（画布、拖柄、排序列表、`NodePointerHooks` 等），两个例外：横向的 `RangeField` / 区间滑块只把纵向拖动让给滚动，并按取消结束拖动（值恢复到按下前，不发 `RangeChanged`），横向拖动仍归滑块；文本的拖选（输入框、可选文本）让给滚动，触摸平台上的选区由长按与平台负责。触摸不命中滚动条轨道，也不抓取滑块。

**速度。** 抬起时取最近 100ms 的样本（含抬起点）沿锁定轴做最小二乘直线，斜率为手指速度；手指停住超过 100ms 再抬起速度为零。内容速度与手指相反。

**iOS。** 拖过边缘时显示的越界量为橡皮筋 `(1 − 1/(x·0.55/d + 1))·d`，`x` 是手指越过边缘的距离，`d` 是该轴视口长度；被抓住的回弹从当前画出的位置反解 `x` 继续。抬起后：位于边缘外则以临界阻尼弹簧（`damped_harmonic`，刚度 250、质量 1）带着抬起速度回到边缘；否则速度按每毫秒乘 0.998 衰减，位移 `v·(0.998^ms − 1)/ln 0.998`（`v` 以 px/ms 计）。衰减越过边缘时，在解析算出的越界时刻以当时的速度转入同一弹簧，所以任何采样频率下轨迹相同。剩余位移小于 0.25 像素即落到终点。

**Android。** 移植 AOSP `SplineOverScroller`：`INFLEXION = 0.35`、`DECELERATION_RATE = ln 0.78 / ln 0.9`、摩擦 0.015、物理系数 `9.80665 · 39.37 · 160 · 0.84`（逻辑像素即 dp），100 段样条表按 AOSP 的二分法生成，`update` 的线性插值相同；低于 50 dp/s 不甩，高于 8000 dp/s 截断。拖动与甩动都在边缘钳住，不越界、不做拉伸或光晕。

**越界偏移。** 只有触摸路径经 `UiMutation::SetScrollOverscroll` 写入可越界的偏移：它与 `SetScrollOffset` 走同一条只平移的提交（命中索引按差值平移、只重新抽取该滚动容器、`update_hit_test_scroll` 与虚拟列表照旧），不写布局盒、不触发布局；下一次滚动范围变化会把它钳回范围内。`ScrollChanged` / `UserScroll` 与滚动条在越界期间照常收到当前位置（滑块几何自行钳位）。

**调度。** 甩动与回弹没有独立定时器：`next_animation_deadline` 并入最早一次采样时刻（上次采样后 8ms），`advance_animations(now)` 按绝对时间求出位置，宿主不必持续出帧，晚到的唤醒直接落在该时刻应在的位置；结束后不留 deadline。偏移被其他路径改动（`scroll_to`、像素滚轮、拖滚动条、辅助技术、观察者回调）即结束甩动；`scroll_to` 即使位置不变也结束。容器卸载时丢弃甩动；隐藏或打开减少动态效果时甩动立即结束，越界的落回边缘。手指按在正在甩动的容器上会停住它，这次按压不触发点击。
