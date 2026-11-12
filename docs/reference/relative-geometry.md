# 相对几何

一棵子树换了位置，只改它根上的一个偏移：布局、命中索引、提取与 Scene、painter、无障碍都把盒子里的东西存在盒子自己的坐标里，绝对位置在读的时候沿参照系链求和得到。一次编辑的代价因此跟真正变了的东西成正比，而不是跟被挪动的节点数成正比。

本文描述现在的规则：每个子系统在平移、改尺寸、改变换时处理谁，哪些情况仍按节点或整棵处理、为什么，代码在哪，以及改动时要守的冷对照。各阶段的基准数字见 [资源预算 · CPU 时间](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md#cpu-时间)，演进见文末。

## 1. 原则

### 1.1 一个加法

```text
abs(child) = abs(frame) + local(child)
```

每个盒子存「相对参照系原点的偏移 + 尺寸」（`NodeRecord::local_layout`）和参照系本身（`NodeRecord::layout_frame: LayoutFrame`）。绝对坐标按坐标轴**从最上面的参照系往下逐级相加**得到。运行时的 `UiWorld::record_layout_origin`、引擎的 `origin_in` / `content_origin_in`、`RuntimeLayoutEngine::layout_document` 的 `page_boxes`、Scene 的 `UiScene::placed_origin` 和无障碍的 `AccessibleTransform::origin` 用的是同一种加法顺序。冷求解同样先求本地偏移、再这样累加，所以增量与冷对照**逐位相等**，平移不需要做 delta 加法，也就没有「旧值 + 位移」带来的末位漂移。

这条约定是所有冷对照逐位比较的前提。任何一处改成另一种求和顺序（例如先加位移、或从下往上累加），冷对照就会在不整齐的步长（测试里常用 1.37 px）下报末位不同。

### 1.2 参照系 `LayoutFrame`

定义在 `crates/nana-ui-runtime/src/layout_result.rs`：

| 参照系 | 原点 | 谁用 |
| --- | --- | --- |
| `Parent` | 父节点边框盒原点 | 普通流里的盒子；包含块就是父节点的 `absolute` 盒子 |
| `Block(id)` | 包含块的边框盒原点 | 包含块在父节点之上的 `absolute` 盒子（逃逸的绝对定位） |
| `Page` | 文档原点 | 根、`fixed` 盒子、包含块是初始包含块的 `absolute` 盒子、触发式菜单的菜单项（挂在视口上）。`Page` 下的本地值就是绝对值 |

参照系由引擎写回（隐藏的 `UiMutation::PlaceLayout { id, layout, frame }`），外部的 `WriteLayout` 按样式推断（`UiWorld::layout_frame_from_styles`，与裁剪链的规则一致）。`position` 变了而布局还没重跑时，提交末尾把参照系与样式不符的盒子换到新参照系、页面位置不变（`reframe_layouts`）。

### 1.3 对外 API：语义不变，绝对值靠推导

| API | 坐标 | 怎么来 |
| --- | --- | --- |
| `UiWorld::layout_box(id)` | 文档 | 沿参照系链累加，O(深度)，不缓存 |
| `UiWorld::local_layout_box(id)` / `layout_frame(id)` | 参照系 | 记录本身 |
| `UiWorld::layout_result(id)` | 文档 | 返回 `Option<Arc<LayoutResult>>`；放到页面上的结果缓存在 `page_layout_results`，按本地结果的 `Arc` 指针和原点（逐位）判定是否过期 |
| `UiWorld::local_layout_result(id)` | 节点自己 | 边框盒在 (0, 0) |
| `UiWorld::component_geometry(id)` | 文档 | 每次按当前盒子现算；命中、IME、指针读它 |
| `layout_origin(id)`、`layout_box_from(owner, child)`（crate 内） | 文档 / 某个祖先 | 页面原点；子节点相对某个祖先的盒子 |
| `UiScene::primitive_layout_bounds` / `place_rect`，`SceneDraw::placed_bounds` / `place` | 页面 | 图元的本地几何加节点原点 |
| `SceneDraw::geometry_transform` | — | 「先放到原点再变换」的合成矩阵，给不吸附网格的读者 |
| `AccessibilityNode::bounds` + `transform`，`AccessibilityPlacements` | 父节点的无障碍空间 | 节点到父节点是 `translate(bounds.x, bounds.y) · transform`（`AccessibilityNode::to_parent`）；`AccessibilityPlacements` 从相对树累积出视口变换 |

`RuntimeLayoutEngine::layout_document` / `layout_document_with_frontier` 仍返回页面坐标；框架写回用 crate 内的 `layout_document_with_frontier_local`。`MutationQueue::write_layout` 收文档坐标，提交结束时按深度从上往下换成本地值，所以与写入顺序无关；没写的子节点随父节点平移。

「不缓存」是跨帧的：提取与无障碍投影在一次只读遍里记住深节点走过的参照系之和（`PageOriginScope`，`world/page_origins.rs`；深度不超过 32 的照旧直走），下面的盒子接着它往下加——和就是从上往下逐级加到那个参照系的累加值，所以结果是同一个位。遍历结束就丢掉，期间世界被借着、写不了盒子。

运行时内部不走 `layout_result` 的页面缓存：`canonical_layout_box`、`component_content_box`、部件查询、裁剪链都读本地结果再加原点，缓存只在外部调用时才填，不会让每个节点多一份结果（`node_memory` 的预算守着这一点）。

## 2. 节点空间分类 `NodeSpace`

`crates/nana-ui-runtime/src/node_space.rs`。每个消费者在保留「盒子里的东西」时都要问同几个问题：布局从哪量它、它自己的变换只是平移还是正立缩放或转过、它开不开 3D 上下文、它裁不裁剪、它的 visual 读不读页面。`NodeSpace` 是这些答案，每个节点按**自己的**解析样式、参照系、种类和 visual 算一次，存在 `NodeRecord::space` 上。它只含节点自己的记录能决定的东西，所以上面的盒子怎么变都不需要走子树去改它。

### 2.1 位

| 位 | 含义 | 来源 |
| --- | --- | --- |
| `OFF_PARENT` | 参照系不是父节点（`Block` 或 `Page`） | `layout_frame` |
| `FIXED` / `STICKY` | `position: fixed` / `sticky` | 样式 |
| `MOVES` / `SCALES` / `TURNS` | 自己的 2D `transform`：线性部分恒等 / 每轴正缩放（不全是 1） / 旋转、斜切、翻转或非有限；三者至多一个 | 样式 |
| `TRANSFORM_3D` / `PERSPECTIVE` / `PRESERVE_3D` | 自己的 3D 上下文 | 样式 |
| `CLIPS` | 任一轴裁剪溢出 | 样式 |
| `DOCUMENT` | 文档节点 | 种类 |
| `VIRTUAL` | visual 从几何派生无障碍节点或改写自己的无障碍盒：打开的 Select、TreeView、MenuSurface、ModalFrame。关着的 Select 只投影字段，不算 | visual |
| `MODAL` | ModalFrame：无障碍盒是 surface，命中把子节点裁到它 | visual |
| `READS_PAGE` | visual 的几何读页面（`StandardVisual::geometry_reads_page`：只有打开的 MenuSurface） | visual |
| `MENU_ROOM` | 打开的 Select：菜单按页面上的空间适配，上面的盒子一动空间就可能变 | visual |

### 2.2 派生答案

| 方法 | 回答 | 谁读 |
| --- | --- | --- |
| `depth()` | 自己开或加入 3D 上下文 | 无障碍（不交偏移）、命中原地重建（拒绝） |
| `opens_3d_context()` | `perspective` 或 `preserve-3d`，子节点画在 3D 上下文里 | 命中建条目、无障碍变换链往下传 |
| `transformed()` / `frames_itself()` / `upright()` | 有 2D 变换 / 变换不只是平移 / 变换让盒子正立 | 无障碍空间、命中、原地改尺寸的判定 |
| `places_by_offset()` | 无障碍 bounds 能取本地偏移：参照系是父节点、不是 `fixed` / `sticky`、没有自己的 3D | 无障碍 |
| `hands_offsets()` | 下面的盒子能按偏移放置在它的空间里：没有 3D | 无障碍 |
| `pins_accessibility()` | 上面的盒子平移时它或它下面的东西可能保不住无障碍 bounds：不按偏移放置、裁剪、不交偏移或 `VIRTUAL` | 无障碍钉子索引 |
| `keeps_space(after)` | 改变换前后交给子孙的空间相同：两边都没有 3D，且两边都 `frames_itself` 或两边都不 | 提交时决定 `TRANSFORM_KEPT` |
| `aligned_by_style()` | 样式让无障碍盒留在父节点坐标给的位置 | 无障碍 `axis_aligned` |
| `hit_frames_itself(local)` | 命中条目就盒子本身而言把自己的布局坐标交给子节点 | 命中建条目 |
| `page_bound()` | `READS_PAGE` 或 `MENU_ROOM` | 提取（`settle_layout_moves`） |

### 2.3 维护与索引

`NodeStore::note_node_space`（`crates/nana-ui-runtime/src/store.rs`）在决定分类的字段一写时重算该节点：插入（`NodeStore::insert`）、visual 设置（`NodeStore::set_visual`，打开、关上 Select 都改 visual）、样式解析（`note_layout_anchor_change`）、写回或换参照系（`write_local_layout`、`reframe_layouts`）。节点移除时从索引里删掉。

两个索引随分类维护，与分类逐个相同：

- `space_pins`：`pins_accessibility()` 为真的节点。无障碍从这里往上找平移根（`reach_moved_pins`、`reach_turned_pins`），代价 O(钉子数 × 深度)。滚动写入不改分类，不碰这个索引。
- `page_bound`：`page_bound()` 为真的节点。`settle_layout_moves` 只扫这个集合；`NodeStore::open_select_menus` 是它上面按 `MENU_ROOM` 的过滤，不是单独的集合。

守卫：`NodeStore::check_node_spaces`（测试与 `layout-verify`，4 096 个节点以内）在每次提交末尾和每次无障碍投影开头，把每个节点的分类与 `NodeSpace::of` 现算的比对，并核对两个索引不多不少。

### 2.4 依赖整条链的答案不存

分类只看节点自己。下面这些要看上面的整条链，所以**不存、每次现推**，按投影或建索引的一次调用记忆：

- **`AccessibleSpace`**（`world/accessibility/space.rs`）：节点的无障碍空间，三个字段 `local`（按偏移放置）、`unit`（盒子在空间里正立时的每轴缩放）、`framed`（空间是某个盒子的本地布局坐标，而不是视口平移过来的）。由 `accessible_space` 沿链求出，记在 `ProjectionMemo` 里。
- **`moved_root_hands_offsets`**（`world/accessibility/delta.rs`）：平移根的盒子在它的空间里是否正立、链上没有模态框、全局没有动画叠加、长度动画和裁剪 visual。快路径只读一路记录的 `hands_offsets()` 与 `upright()`；不成立时再问 `accessible_space`。
- **命中的 3D 上下文**：`build.rs` 建条目时沿种子往下带 `parent_blocks_3d`，与 `opens_3d_context()` 合成子节点的 `child_blocks_3d`；`local_children` 要求一路没有透视、子节点不在 3D 上下文里。
- **命中的有效样式**：条目按有效样式建。有效样式不是解析样式时（动画叠加、浮层宿主的分支），`NodeSpace::of_style` 按有效样式现分类，不读 `NodeRecord::space`。

### 2.5 有意保留的差异

平面 `transform_3d` 在两边答案不同，分类里分开记（`TRANSFORM_3D` 不与 `PERSPECTIVE`、`PRESERVE_3D` 合并）：

- **命中当 2D**：`hit_frames_itself` 对 `TRANSFORM_3D` 看它求出的单应矩阵；没有透视行、线性部分不是恒等时，条目照样交出本地坐标（`boxes_moved_under_a_planar_3d_turn_answer_as_a_fresh_index`）。
- **无障碍当 3D**：`depth()` 把三种都算 3D，退回投影四角的外接框。

原因：命中只需要一个可逆仿射，平面 3D 变换恰好给得出；无障碍交给平台的 `transform` 是 2D 仿射，现在对平面 3D 交的是外接框，统一成 2D 会改变平台拿到的输出。两边都保持原样，改的时候两边一起改并更新快照与无障碍冷对照。

另一处差异：`pins_accessibility` 把 `fixed` 和裁剪算钉子，命中的 `HitEntry::pinned` 不算。命中把 `fixed` 挂在无父节点处、裁剪存在条目自己的坐标里；无障碍的 bounds 相对树上的父节点、`clip` 在视口坐标里。

## 3. 布局

### 3.1 规则

- 引擎的 `DocumentLayoutCache.boxes` 和 `ContainerPlan` 的原点是相对值，非父节点的参照系另记在 `DocumentLayoutCache.frames`。`place_node_body` 一律以 `Point::ZERO` 为本容器的坐标系原点放置子节点，`place_node_head` 写出的盒子就是相对参照系的偏移。
- 跨参照系的读取只有几处：定位子节点的包含块内边距盒在包含块自己的坐标系里；静态位置经 `content_origin_in` 沿参照系链换到包含块的坐标系；触发式菜单的菜单项放在页面坐标里，菜单自身的页面原点由 `origin_in` 求出；行内上下文求部件时用盒子自己坐标系里的边框盒。
- 静态位置的逃逸绝对定位会随包含块与容器之间的盒子移动：`place_remote_overlays` 每遍对未放置容器复查，比较「静态位置换到包含块坐标系后是否变了」，O(逃逸条目数 × 深度)。
- `LayoutResult` 在本节点坐标里存（边框盒在原点）。结果不抄录子节点的盒子，子节点的放置与片段由 `UiWorld::layout_child_placements` / `layout_child_fragments` 询问时从子节点读（2026-11-11 之前结果里存着每个子节点的本地盒子三份）；`overflow` 和 `scroll_extent` 只并入参照系是本节点的子节点，`fixed`、逃逸的绝对定位、菜单项在放到页面上时才并入。
- `ContentBoundsIndex`（`world/scroll_bounds.rs`）每条按自己节点的原点存，携带给包含块的范围按落地的包含块的坐标存。落地点可能变的两种情况（`position` 在是否建立包含块之间变化、子树换父节点）让子树里携带范围的条目重读子节点（`reland`）。父节点刷新时只读移动了的子节点，叶子原地写入，上面按层合并一次（`MaxTree::rederive`）。
- 粘性定位从本地盒子和本地结果算，`StickyAncestor::result` 是祖先自己坐标里的结果。

### 3.2 平移一棵子树

父节点重新排布时只写直接子节点的偏移：只挪了位置的容器在父节点那里 `subtree_unchanged` 不成立，`place_node_head` 写下它的新偏移后，`keeps_moved_subtree` 判定它的主体读的东西都没变（自己不是种子、不在测量前沿里、书写模式与滚动没变，进受影响闭包只因父节点或同一格式化上下文里的兄弟把它挪了；下面没有受影响的节点；边框盒尺寸与用过的内边距同上一遍；不参与动态布局的行、不继承网格轨道、不是模态或触发式菜单、不在逃逸定位的复查表里、没有参照系不是它的子节点），就不进 `place_node_body`，一个子节点也不碰；它的结果不变，发布时 `geometry_eq` 判为相同。行内格式化上下文（段落）没有容器计划，以前每挪一次都要重新排一遍行盒，现在同样只写偏移。测试与 `layout-verify` 构建里，保住子树的盒子下面每个盒子都要与全量布局逐位相同。提交只标写回的盒子本身（`settle_layout_moves`）：原点变了的根记 `DirtyMask::MOVED`，进 `SystemWork::layout_moved`；后代一个不标。

发布时（`publish_layout_results`）：

- 结果投影出新几何（`layout_result_projects_new_geometry`：自身盒子、内边距盒、内容盒、基线、溢出、滚动、裁剪、参与方式、自己的片段与部件，）时标 INPUT | RENDER | ACCESSIBILITY；变化超出滚动范围、溢出与基线（`layout_result_reshapes`）时另标 `DirtyMask::RESHAPED`，进 `SystemWork::layout_reshaped`，无障碍据此判断裁剪盒是否还按原来的盒子裁剪。
- 子节点只在父级盒子里挪动（固定高度的列里首行加高）时父级的结果不变：结果不记子节点放置，`layout_result_geometry_current` 只核对被覆盖的子节点仍是本节点的、参照系是本节点、盒子仍在 `overflow` 之内，成立就沿用上一份，不重建、不标脏。子节点伸出或缩回时 `overflow` 变，按上一条投影。结果重建了却投影不出自己的新东西时仍**只标 RENDER**：提取给它的记录带着结果，否则 Scene 会一直留着旧的那份。

行盒记忆（「这个盒子有没有流内行盒」，`holds_line_box_in`，存在 `DocumentLayoutCache::line_boxes`）跨帧保留，按种子的改动类别清：`may_move_line_boxes`（`layout_engine.rs`）对只改尺寸、间距、对齐、滚动、变换或指针输入、原因也不涉及流的种子答「不清」；依赖边带进前沿、自己没变的节点也不清。结构性种子（拓扑、可见性、原因未知，`moves_line_boxes_below`）照旧沿所有受影响节点往上清到根，因为祖先派生的隐藏会让整棵子树的答案变。

### 3.3 改变换

变换不是布局输入：样式里的 `transform` 变了不标布局脏，行盒记忆也不清。提交按 `NodeSpace::of_style` 前后两份调用 `keeps_space`：成立时 `mark_transform_kept` 只标 `TRANSFORM | TRANSFORM_KEPT | INPUT | RENDER`（INPUT 不带 `INPUT_ELSEWHERE`），列在 `SystemWork::transform_kept`；不成立时标 `TRANSFORM | INPUT | RENDER`，列在 `SystemWork::transform`。两种都只标盒子自己，子孙由各子系统按下面的规则处理。层叠变了照旧整棵标 INPUT | RENDER。

### 3.4 仍整片处理的

- 结构性种子清行盒记忆时沿所有受影响节点清到根（见上）。
- 子树换父节点、`position` 在是否建立包含块之间切换时，`ContentBoundsIndex` 里携带范围的条目重读子节点。
- 滚动容器里每个移动的行仍各写一次内容范围的叶子（见 [已知限制](#_10-已知限制)）。

## 4. 命中索引

### 4.1 条目

每个条目（`HitEntry`）在**自己边框盒的坐标**里：`layout` 是 `(0, 0, 宽, 高)`，裁剪、路径裁剪、菜单、变换的轴心都按这个坐标存。

- `origin`：盒子离命中父节点原点的偏移，经父节点交给子节点的变换的线性部分换算（`linear_offset`）。来源：参照系是父节点且命中父节点就是树上的父节点时直接用 `local_layout_box`；`Page` 参照系的命中根也直接用本地值；其余（包含块在别处的绝对定位、逃逸条目挂在裁剪父节点上）用 `layout_box_from` / `layout_origin` 求差，这类条目是钉住的。
- `IndexedHit::shift`：滚动容器上的滚动平移；`HitEntry::scroll` 是索引让子节点处在的滚动偏移。
- 查询（`open_hit`，`query.rs`）先减 `origin` 进入条目自己的空间，交出本地坐标的条目再过一次逆变换（`transformed_point`，奇异时子节点不可达），再减 `shift` 进入子节点的空间。
- 父节点边界树里的包围盒是「自身 ∪ 子节点（加 `shift`）」再加 `origin`（`entry_bounds`；交出本地坐标的条目用 `turned_bounds`，子节点包围盒过自己的变换取外接框）。冷建与增量刷新同一个公式，逐位一致。
- 父节点的变换带透视时偏移不能在外层做：偏移并进条目自己的变换，`origin` 为零。
- **粘性偏移**（`sticky_translation`）和布局偏移一样是条目自身变换之前的平移，父节点没有透视时加进 `origin`，不进 `transform`：贴住或滑动只改这一个条目的 `origin`，子条目相对它存放，不继承它。模态框 body 的自身裁剪（`self_clips`）按父节点空间存，因此存的变换减去换算后的粘性偏移，查询减掉 `origin` 后裁剪仍在框架排出的位置。父节点有透视时粘性偏移连同布局偏移一起进条目的变换。
- **交出本地坐标**（`HitEntry::local_children`）：自身 2D 变换有缩放、旋转、斜切或翻转（`hit_frames_itself`）、一路没有透视、子节点不在 3D 上下文里的条目，交给子节点的只有「减滚动」（`scroll_seed`），子节点在它的本地布局坐标里建，不含它和它上面的任何变换。于是转过的子树里一切都是正立的。跟着改的读取：`inherited_shift`、`child_shift` 往上累加滚动到最近的这种条目为止；补丁与原地重建的种子统一走 `live_child_seed`；从窗口进出条目的地方（`pointer_layout_position`、`layout_pointer_position`、`menu_region_contains`、`escaped_hit_at`）走 `entry_space`。
- **钉住的条目**（`HitEntry::pinned`，集合 `HitIndex::pinned`）：读了自身偏移以外的几何——跨参照系求差的偏移、#201 逃逸的桥接变换（`clip_bridge_transform_from`）、组件几何（Select 菜单、悬停卡连接线、模态框的 surface / body 裁剪）、粘性定位。打补丁时脏节点下面的钉住条目即使没标脏也重建（`pinned_hit_entries_under`，O(钉住数 × 深度)）。

合成层叠加时的查询（`visit_presentation_hits`）按页面坐标现算变换，不读条目的 `transform` 和包围盒。

### 4.2 平移一棵子树

只重建根条目（`rebuilt_hit_entry_in_place`，`in_place.rs`，原地替换，`origin` 变了计入 `hit_test_entries_moved`），再按深度刷新祖先包围盒；后代条目一个不碰，钉住的除外。根条目不原地改 `origin`：同一次提交里可能有兄弟增删，顺序要从重建里拿。

原地重建的条件是「交给子节点的东西没变」：新条目从父条目现在的种子（`live_child_seed`：父条目的变换加上面的 `shift` 再减父节点现在的滚动）单独建，减去上面的 `shift` 回到索引存它的空间，然后要求

- 条目没有逃逸、透视为零，不交出本地坐标的条目线性部分是恒等；
- 命中父节点不变，`clip_escape` 为空，父条目没有透视，节点没有自己的 3D（`space.depth()`）；
- 新条目只建出一条、没有延后的；变换（不交出本地坐标时）、`HitEntry::scroll`、子节点列表（同一个 `Arc`）、z-index 与旧条目相同，没有变成 `fixed`；交出本地坐标的条目前后都还是这种条目。

裁剪、菜单、painter 只属于条目自己，可以随便变。`IndexedHit::shift` 原样保留，子节点仍按建它们时的滚动加 `shift` 查询。

### 4.3 改尺寸、改变换、滚动

- 转过的盒子自己被撑高或平移：变换原点（默认中心）跟着动，但子孙在它的本地坐标里，只重建它自己的条目。
- 改变换（`TRANSFORM` 或 `TRANSFORM_KEPT` 只标盒子自己）：前后都交出本地坐标时只重建它自己，脏节点下面的钉住条目照旧重建；其余情况（纯平移、变成或不再是这种条目、透视、3D）原地重建不成立，走 `patch_hit_subtree` 整棵修补它的分支。
- 滚动：`shift_hit_test_scroll` 只改 `shift` 并刷新祖先包围盒，O(深度)；在交出本地坐标的条目上按恒等线性部分换算。驱动在同一帧既有滚动又有别的脏节点时丢掉滚动增量、交给补丁处理；这时滚动容器的 `scroll` 与现在的值不同，走子树补丁。快路径（`hit_test_work_is_scroll_only`）要求 INPUT 脏节点全是本帧滚过的容器，且都不在 `non_scroll_hit_dirty` 里：这个集合由 `mark` / `mark_moved` / `mark_ancestors` 记下滚动以外的 INPUT 标记（滚动只走 `mark_scroll_compatible`），容器自己改了盒子又同帧滚动时据此走补丁；它在条目按现在几何建好时删去——整树重建删掉本文档的、按范围重建删掉传入的脏节点与钉住条目、滚动里重建的粘性条目删掉自己——取走工作后又被标 INPUT 的节点保留到下一帧。粘性盒子（`sticky_nodes.affected`）随滚动改自己的偏移：粘性偏移在 `origin` 里，下面的条目在它的坐标里，所以每个粘性盒子只按 `rebuilt_hit_entry_in_place` 原地重建自己这一条（贴住与否都一样），全部建好后一次替换、祖先包围盒只刷新一次；只有下面有钉住条目、或父节点有透视（偏移在变换里）时才整枝 `patch_hit_subtree`，由外向内。纯滚动下条目上面在索引空间里什么都没变，按现在的滚动建出、再减去 `shift` 的变换与存着的只差舍入，于是保留存着的变换（`scrolled` 参数），裁剪的变换平移同样的量，原地重建不因末位差退回整枝。所以一次滚动的命中成本是每个粘性盒子一个条目，与它装着多少盒子、滚动容器里有多少盒子都无关。

### 4.4 仍整棵修补的

透视与 3D 上下文下的分支、变换在「交出本地坐标」与「不交」之间切换、`HitEntry::scroll` 与现在的滚动不一致、子节点列表或层叠变了，都走 `patch_hit_subtree`。原因：这些条目的子孙变换里含着它们的变换（透视并不进仿射，3D 上下文要把变换往下传），一变就都变。

## 5. 提取与 Scene

### 5.1 记录与组件几何

- `ExtractedNode` 带 `local_layout` 与 `layout_frame`（与 `UiWorld::local_layout_box` / `layout_frame` 相同）；`layout` 是提取那一刻的页面盒子，只作参考。`layout_result` 是本地结果，共享同一个 `Arc`。`ExtractedMove` 同样带 `local_layout` / `layout_frame` / `layout_result`，只给平移的根发。
- `ExtractedNode::component_geometry` 在节点自己的坐标里。`UiWorld::local_component_geometry` 用同一份 `derive_component_geometry` 求它：把 `geometry_frame` 设成这个节点，`record_layout_origin` 沿链求和时走到它就停（它的原点记 0）；链上碰不到它或读了视口就记 `geometry_read_page`，调试构建断言。对外的 `component_geometry` 不设 frame，仍是页面坐标。
- **读页面的 visual**（`READS_PAGE`）：只剩打开的 MenuSurface（菜单项参照系是页面）。它在页面坐标里推导、记录在页面上，它或上面的盒子一动，`settle_layout_moves` 从 `page_bound` 里找出它并标 RENDER，整节点重新提取。关着的 MenuSurface 够不到菜单项（`closed_menu_hides`），关着的 Select 推导时不读视口，NativeMarkdown 从本地内容盒往下排，都按本地坐标推导，平移时沿用记录。
- **打开的 Select**（`MENU_ROOM`）：菜单在字段本地坐标里推导。读页面只有一处：`select_menu_room` 给出的空间（视口与裁剪链，存在 visual 的 `menu_room` 里）。`local_select_menu_room` 把它减去字段的页面原点，之后只经过 `select_menu_surface`（`select.rs`）决定表面盒子，其余都由这个盒子和字段自己的状态决定。所以表面盒子逐位相同，整份记录就逐位相同。平移时字段是平移根，`extract_nodes_reusing` 沿用记录前比一次（`keeps_select_menu`：渲染器持有记录的表面与 `local_select_menu_surface` 现算的逐位比较）；字段在平移根下面，`settle_layout_moves` 只标 RENDER 并记 `render_room_epoch`（`mark_keeping_render_reason`，不算「别的原因」），提取时同样比一次。换边、截短或空间本身变了（滚动、裁剪、窗口变了，`stale_select_menu_rooms` 改 visual）才重新提取。

### 5.2 图元与绘制

- 图元以 `UiScene::record_box` 为 `bounds` 建（本地记录是 `(0, 0, 宽, 高)`，页面记录是提取时的页面盒子），`bounds`、批量矩形、笔画点、`Path` / `Chart` 的 `origin`、图层裁剪、节点自己的裁剪都在节点自己的坐标里；`transform` 是页面上的变换（祖先的变换、滚动、节点自己绕现在位置的变换），不含原点。
- 节点现在的页面原点按参照系链累加（`placed_origin`，`placement.rs`，按 `placement_epoch` 记忆；第一次平移之前直接取 `layout`，`placement_live`）。`paint_origin` 是图元落点：本地记录取 `placed_origin`，页面记录取 0。
- `draw_primitive`（`attributes.rs`）给出 `SceneDraw { origin, transform, clips }`：几何先加 `origin`，再过 `transform`；`clips` 全在页面上，继承的由 `fold_ancestor` 从祖先现在的页面盒子（`placed_layout`）现算，节点自己的加上 `origin`（缓存在 `placed_clips`）。冷场景走同一个公式。保留投影直接用建图元时记下的原点（`RetainedPrimitive::origin`，调试构建断言与现算相同）；重定基的原点和继承裁剪记在 `DrawAttributes`。

### 5.3 平移一棵子树

- 平移的根只换本地盒子、参照系和结果，抬 `attribute_epoch` 与 `placement_epoch`，图元不动；尺寸或结果变了才重建。整节点重新提取、但只挪了位置且结果相同的（`moved_in_place`）同样不重建。后代不在 delta 里。
- **读了自己位置的节点**，分两个集合（`UiScene` 上的 `offset_bound` / `origin_bound`）：
  - `offset_bound`：只读了自己在父节点坐标里的偏移。对齐父节点文字的小图标（`icon_y_aligned_to_adjacent_text` 答 `PlacementRead::Offset`）只在它自己是平移根时重建，O(平移根数)。
  - `origin_bound`：读了页面位置，它或上面的盒子平移时整节点重建，O(集合大小 × 深度)。成员见 [已知限制](#_10-已知限制)。
- **剔除索引**：平移的根在可见性索引里整棵子树平移（`translate_subtree`，与滚动同一条路，嵌套的根只补自己多出来的那一段），根自己的图元重算；参照系不是父节点或粘性的（`unfollowing`：逃逸的绝对定位、`fixed`、菜单项、`sticky`）在平移根下面时重算自己和子树。所有平移先做、再做任何重算。平移量先过根继承的变换的线性部分；继承的变换带透视时整棵重算（`refresh_subtree`）。叶子边界按图元自己的不透明度组（`primitive_opacity_groups`，与 painter 同一套规则）判断是否未知。

### 5.4 改变换与改子节点放置

- 改变换只重新提取盒子自己，子孙的图元按 `current · inverse(base)` 重定基。`DrawAttributes` 的 `rebased` 记下建图元时的投影和现在的投影，图元与节点自己的裁剪直接画成现在的投影，不再经过逆矩阵，所以从转过改回纯平移时线性部分是精确的 1，painter 照常吸附。剔除索引在祖先投影变了时对那些根 `refresh_subtree`。
- 子节点在父级盒子里挪动不改父级的结果，父级不重新提取（见 3.2）；改了 `overflow` 的照常投影，结果重建而投影不出新东西的标 RENDER，节点重新提取，Scene 持有的结果跟着换。

### 5.5 公开语义

`ScenePrimitive` 的几何在节点自己的坐标里（读页面的 visual 除外）；页面上的位置用 `UiScene::primitive_layout_bounds` / `place_rect`，或 `SceneDraw::placed_bounds` / `place`。`node_bounds` 与 `node_layout_bounds` 相同，都是现在的页面盒子。

## 6. painter

`crates/nana-ui/src/scene_paint/`。painter 对每个图元：

1. **先放到原点**：几何加 `SceneDraw::origin`（`placed_bounds`、`place`、笔画点进复用的暂存区、`Path` / `Chart` / 图层蒙版的 `origin`）；
2. **再变换**：之后与对页面坐标图元完全一样；
3. **再吸附**：`paint_transform`（`scene_paint/clip.rs`）只把 `transform` 的纯平移吸附到整设备像素（#223 的滚动语义），文字的整像素与亚像素相位从放好的页面位置取。

布局的平移因此不是一个要吸附的平移：挪了小数像素的标签按新位置重新求相位，挪了整数像素的只换运行的 `origin`、实例照旧沿用。不需要每节点变换缓冲：painter 每帧只遍历 `visible_operations` 裁剪后的图元，每个图元多一次加法和一次原点读取。

## 7. 无障碍

### 7.1 语义

`AccessibilityNode::bounds` 相对父节点的**子节点空间**，`transform: Option<PaintTransform>` 对应 AccessKit `Node` 的 `transform`；节点自己的盒子在自己的空间里是 `(0, 0, width, height)`。`AccessibilityNode::scroll` 是节点滚动内容的量（滚动偏移乘以它空间的 `unit`）：子节点空间是它自己的空间平移 `-scroll`（`children_to_parent`），所以滚动只改滚动容器这一个节点，它装着的盒子 bounds 不变。不滚动的节点、子节点从视口盒放置的节点（3D、模态框、动画叠加下面，`unit` 为 `None`，滚动仍在子节点的 bounds 里）以及视口投影里 `scroll` 都是 `[0, 0]`。AccessKit 宿主把 `to_parent · translate(-scroll)` 设成节点的 `transform`、把本地盒子平移 `scroll` 设成 `bounds`（自己的盒子仍在原处），与 Chromium「滚动容器带滚动偏移、子节点相对它」是同一种表示。`clip` 仍是视口坐标。自动化与测试要视口坐标用 `project_accessibility_in_viewport` / `project_accessibility_nodes_in_viewport`，或用 `AccessibilityPlacements` 沿链累积。

每个节点的空间（`AccessibleSpace`，见 2.4）：

- **按偏移放置**（`offset_placed`，`local_placement`，`frames.rs` 的 `offset_bounds_in_parent`）：`places_by_offset()`、父节点空间让它正立（`unit` 有值）、全局没有动画叠加和裁剪 visual 时，bounds 由本地盒子算出：`(a·x, d·y, a·w, d·h)`，`(a, d)` 是父节点空间的缩放；父节点的滚动在父节点自己的 `scroll` 里，不进子节点。自己的变换只是平移时并进偏移、`transform` 为 `None`；正缩放发出 `diag(a·a', d·d')`；旋转、斜切、翻转发出 `diag(a, d) · L`。后两种叫自带框架（`framed`）：它的空间就是它的本地布局坐标，下面按未乘系数的偏移放置。全都与页面位置无关。
- **从视口来、自带框架**（`frames_from_viewport`，`viewport_frame_in_parent`）：不按偏移放置的盒子（页面根、`fixed`、`sticky`、逃逸的绝对定位）在 2D 链上、父节点空间到视口的映射可逆，而且被转过或缩放过时，`to_parent = V_P⁻¹ · M · translate(origin)`（`M` 是 `accessibility_ancestor_transform` 的整条链，`V_P` 是 `accessible_space_to_viewport`）。平移来自视口，线性部分交给子孙，下面按偏移放置的盒子与页面位置无关。
- **其余从视口外接框来**：3D、模态框的 surface、动画叠加或裁剪 visual 下的一切，以及父节点空间不可逆时。父节点空间有框架时发出它的逆的线性部分（`turned_space_inverse`），让它的空间与视口对齐。3D 上下文只退回外接框，不发透视：平台只有 2D 仿射。

### 7.2 平移一棵子树

写回时本地偏移变了的盒子标 ACCESSIBILITY；随父节点平移的后代不标。只因布局移动而标的 INPUT 不带 `INPUT_ELSEWHERE`，进 `SystemWork::accessibility_moved`。`project_accessibility_delta`（`delta.rs`）对每个平移根：

- **从钉子找**：`moved_root_hands_offsets` 成立时，根不整棵走，而是 `reach_moved_pins`（`pins.rs`）对 `space_pins` 里每个钉子往上找最近的平移根。路上碰到参照系是页面的盒子就停（它没跟着动）；参照系是某个块的盒子记下那个块，到根之前经过了它才算跟着根动；仍交偏移的钉子（缩放、`sticky`、从随根平移的块量起的绝对定位）不挡路。找到的钉子从它起按 `reach_moved_subtree` 的规则走：没跟着动的进受影响集合，裁剪的整棵重投影，不按偏移放置的逐个 `keeps_relative_bounds`。O(钉子数 × 深度)。
- **逐个走**：`moved_root_hands_offsets` 不成立（根的空间不正立、根是模态框、有动画叠加、长度动画或裁剪 visual）时 `reach_moved_subtree` 从根往下逐个精确判定，停在别的平移根处。
- 平移根自己的相对偏移确实变了，要重投影。布局标了 INPUT、却没进受影响集合的盒子（相对偏移没变），再问一次 `keeps_relative_bounds`，保不住的重投影。

**滚动**：滚动只标滚动容器的 INPUT，并记 `DirtyMask::INPUT_SCROLLED`（不带 `INPUT_ELSEWHERE`），drain 时进 `SystemWork::accessibility_scrolled`；同帧还被别的东西标了 INPUT、被布局移动或改形、改了变换的不算。`project_accessibility_delta` 投影容器自己（`scroll` 与滚动轴变了）；`moved_root_hands_offsets` 成立时，它装着的东西当作「内容平移了」交给 `reach_moved_pins`，以滚过的容器为根和停止点：滚动带不走的（`fixed`、包含块在容器外的绝对定位）、从视口盒放置的（`sticky`、量自别的块的绝对定位；父节点是滚动容器（裁剪溢出，或滚动偏移不为零）时 `keeps_relative_bounds` 不再按「两边的视口盒都是布局盒」保留——只看偏移不为零的话，滚回原点那一帧贴住过的 `sticky` 会被漏掉）与裁剪的整棵（子孙的 `clip` 在视口坐标里，嵌套的滚动容器也是）重投影，其余一个不碰。另一个滚过的容器里的滚动容器整棵重投影（它的裁剪在视口上动了）；`moved_root_hands_offsets` 不成立时整棵重投影，与以前相同。代价 O(钉子数 × 深度) 加容器自己的子节点列表。

**裁剪盒**：裁剪的种子原则上整棵重投影（视口坐标的 `clip` 从它来），但不在 `layout_reshaped` 里（原地不动、只是滚动范围变了的滚动容器）时按原来的盒子裁剪，只投影它自己。

**原地改尺寸**（596930d22 起）：盒子原地变了尺寸、不在 `layout_moved` 里时：

- 自带变换的盒子（2D 变换、`transform_3d` 或 `perspective`）一律当遍历根（`resized_roots`，与 `layout_moved` 合成 `walk_roots`）：变换原点随尺寸动，它的空间像平移过一样动了。是 2D 变换且 `moved_root_hands_offsets` 成立的交给 `reach_moved_pins`（下面只有钉子重投影，例如两层以下的 `fixed`）；其余（动画叠加、裁剪 visual、3D、模态框）交给 `reach_moved_subtree`，下面逐个精确判定，不只是直接子节点。
- 没有自带变换的盒子，一路有不交偏移的盒子或自己的空间不正立时，重投影没跟着移动的直接子节点（外接框原点随尺寸变）；否则什么也不做。

### 7.3 改变换

`SystemWork::transform_kept` 里的盒子，还要满足：本帧没被布局平移、自己不裁剪、`moved_root_hands_offsets`、`style_frames_its_space`（样式自带框架的盒子空间确实自带框架；3D 上下文下面的缩放盒子父节点空间不可逆，空间是视口的缩放，改系数会改子孙）。满足就只投影它自己，再 `reach_turned_pins`：每个钉子往上走，先碰到这种根就把钉子整棵重投影，先碰到另一个钉子就交给那个钉子。转动带着它下面的一切，所以与平移不同、不看参照系。不满足的、以及 `SystemWork::transform` 里的盒子整棵重投影。

### 7.4 投影本身的快路径

- 子节点筛选：父节点交给子节点的映射和页面原点每个父节点只取一次（`HandedMap`），按偏移放置、没有自己变换的子节点只读记录和两个角（`HandedMap::finite_under`）；父节点那条链的三个答案（隐藏、对齐、交偏移）每个被投影的节点只问一次（`ChildChain`）。父节点是文档时走完整判定（`0 + x` 与 `x` 在 `-0` 上不同）。
- 变换链在只有布局样式和记录的页面上，对流内、相对定位、没有自己变换的盒子直接取父节点的映射（`plain_transform_step`）。
- `axis_aligned` 只看链上每个盒子（滚动、变换、`fixed` / `sticky`、3D、父节点是打开的触发式菜单），全局只留动画叠加和裁剪 visual。
- 投影前按受影响节点数给 `ProjectionMemo` 留容量。

## 8. 代码地图

`crates/nana-ui-runtime/src/`：

| 文件 | 职责 |
| --- | --- |
| `node_space.rs` | `NodeSpace` 的位与派生答案 |
| `store.rs` | `NodeRecord::space`、`note_node_space`、`space_pins` / `page_bound` 索引、`check_node_spaces` |
| `layout_result.rs` | `LayoutFrame`、本地 `LayoutResult` |
| `world.rs` | `layout_box`、`layout_origin`、`record_layout_origin`、`layout_box_from`、`local_component_geometry`、`publish_layout_results`、`layout_result_projects_new_geometry`、`layout_result_reshapes` |
| `world/mutation.rs` | `write_local_layout`、`reframe_layouts`、`settle_layout_moves`、`mark_transform_kept` |
| `world/extraction.rs` | `extract_nodes_reusing`、`keeps_select_menu`、`check_records_kept_under_moves` |
| `world/geometry.rs` | `derive_component_geometry`、`local_select_menu_room`、`local_select_menu_surface` |
| `world/scroll_bounds.rs` | `ContentBoundsIndex`、`MaxTree` |
| `layout_engine.rs`、`layout_engine/placement.rs` | `may_move_line_boxes`；`origin_in`、`content_origin_in`、`place_remote_overlays` |
| `layout_engine/verify.rs` | 布局的重算比对与 `retained_line_boxes_are_fresh` |

`crates/nana-ui-runtime/src/world/accessibility/`（根文件 `accessibility.rs`：`ProjectionMemo` 与公开投影入口）：

| 文件 | 职责 |
| --- | --- |
| `space.rs` | `AccessibleSpace`、`AccessibleTransform`、`HandedMap`、`ChildChain`；空间分类与交给子节点的变换 |
| `bounds.rs` | 视口包围盒、祖先变换链（`accessibility_ancestor_transform`、`plain_transform_step`）与裁剪 |
| `project.rs` | 单节点全量投影 |
| `virtual_nodes.rs` | 菜单项、树行、Select 选项等虚拟节点 |
| `frames.rs` | 相对父空间的 bounds 与视口框架（`offset_bounds_in_parent`、`turned_space_inverse`、`viewport_frame_in_parent`） |
| `delta.rs` | 增量事务 `project_accessibility_delta`、`moved_root_hands_offsets`、`style_frames_its_space` |
| `pins.rs` | `reach_moved_pins`、`reach_turned_pins`、`reach_moved_subtree`、`keeps_relative_bounds` |
| `verify.rs` | 测试与 `layout-verify` 下的冷对照 |
| `relative_tests.rs`、`viewport_tests.rs` | 相对 bounds、钉子、视口框架的测试 |

`crates/nana-ui-runtime/src/world/hit_test/`（根文件 `hit_test.rs`：模块与给 `world` 的重导出）：

| 文件 | 职责 |
| --- | --- |
| `clip.rs` | 圆角与路径裁剪判定 |
| `index.rs` | 条目、索引结构、包围盒（`entry_bounds`、`turned_bounds`）与增删改（`insert_tree`） |
| `bounds.rs` | 兄弟节点的有序包围盒树 |
| `space.rs` | 条目空间、`shift`、子节点种子与窗口 / 布局坐标变换（`entry_space`） |
| `query.rs` | 索引遍历与 `open_hit` |
| `candidates.rs` | paint 键、浮层与菜单、候选与最上层命中 |
| `scroll.rs` | 滚动 `shift` |
| `build.rs` | 建条目与森林、整树重建 |
| `patch.rs` | 子树修补与撤下 |
| `in_place.rs` | 原地重建（`rebuilt_hit_entry_in_place`）与按范围重建（`rebuild_hit_test_scoped`） |
| `verify.rs` | 测试 / `layout-verify` 下的平移条目校验，`debug_assertions` 下的资格审计 |
| `moved_turned_tests.rs`、`moved_scroll_tests.rs`、`build_tests.rs`、`presentation_tests.rs`、`scroll_invalidation_tests.rs` | 测试 |

`crates/nana-ui-scene/src/scene/`（根文件 `scene.rs`：模块、重导出与 `UiScene` / `SceneNodes` 结构和只读访问）：

| 文件 | 职责 |
| --- | --- |
| `geometry.rs` | `SceneRect`、`AffineTransform`、`ClipRegion` |
| `slots.rs` | `PrimitiveId` 与槽位命名空间 |
| `paint_order.rs`、`order.rs` | 组前缀、画序键与根层；画序投影 |
| `kinds.rs` | `ScenePrimitive` 与图元种类 |
| `placement.rs` | 记录现在站在哪：`placed_origin`、`paint_origin`、`place_rect`、`primitive_layout_bounds` |
| `attributes.rs` | 共享投影属性：`SceneDraw`、`DrawAttributes`、`draw_primitive` |
| `ancestors.rs` | 祖先折叠的变换、不透明度与裁剪链及祖先判定 |
| `groups.rs` | 不透明度 / 滤镜 / 边缘淡出 / 内阴影组 |
| `delta.rs` | delta 应用：删除、平移、重定基与重新提取，剔除索引的平移与重算 |
| `retained.rs` | 按节点增删查保留图元 |
| `audit.rs` | 保留与平移图元的审计 |
| `visibility.rs` | 剔除用的有序包围盒层级（`translate_subtree`、`refresh_subtree`、`bounds_match`） |
| `primitives.rs`、`primitives/` | 按 visual 建图元（`offset_bound` / `origin_bound` 在这里登记） |
| `component_text.rs` | 组件文字图元，图标对齐文字 |
| `text_effects.rs`、`text_primitives.rs` | 文字效果与字形呈现；span 背景、编辑器标记、行内对象 |
| `visual_quads.rs` | 方块、图片、对比环、焦点环、描边与批量图元 |
| `presentation.rs` | 行内对象动画与图表 / 字形呈现是否在播 |
| `custom_paint.rs` | `Painter` 录制转成图元 |
| `composition.rs`、`compositor.rs`、`sticky.rs` | 合成投影、合成层、粘性 |

## 9. 校验与测试

### 9.1 冷对照与审计

| 守卫 | 构建 | 比什么 |
| --- | --- | --- |
| `layout_engine::verify`（`retained_matches_full_layout`） | 测试、`layout-verify` | 每遍保留布局后从零再排一遍，盒子逐位相等 |
| `retained_line_boxes_are_fresh` | 测试、`layout-verify`（`skip_layout_verify` 下也跑） | 每遍布局后留下的每条行盒记忆与新走一遍相同；读记忆时另有逐次比对（4 096 个节点以内） |
| `check_scroll_content_extent` | 测试、`layout-verify` | 每次读内容范围与从零建的索引逐位相同 |
| `assert_matches_cold`（`reflow_oracle.rs`） | 测试 | 布局、命中几何（含 `entry_offset`）与冷重建一致 |
| `check_node_spaces` | 测试、`layout-verify` | 分类与两个索引（见 2.3） |
| `check_moved_hit_entries` | 测试、`layout-verify` | 补丁打完后，被原地替换的条目及其下面所有条目逐字段（含 `origin`、`pinned`、`scroll`、`local_children`、包围盒）比对全量重建；上面有 `shift` 的换到现在的空间、在舍入量级（`max(1e-4, 1e-6·|v|)`）内比 |
| `audit_hit_eligibility` | `debug_assertions` | 指针资格 |
| `hit_frames_itself` 断言（`build.rs`） | 测试、`layout-verify` | 分类答案与矩阵判定逐位一致 |
| `check_records_kept_under_moves` | 测试、`layout-verify`（1 024 个节点以内） | 本批里盒子相对渲染器所持记录变了的节点，它下面没重新提取的记录逐个冷提取比对 |
| `geometry_read_page` 断言 | 调试 | 本地推导的组件几何没读页面 |
| `audit_moved_primitives` | 调试（512 个节点以内） | 每个带平移的 delta 之后克隆、全部重建，比对平移根及其下面的图元、放置原点、纯平移变换、裁剪矩形 |
| `audit_retained_projection` | 调试 | 保留投影与现算一致；剔除索引被平移过时比 `placed_mismatch`（叶子加上欠着的平移，按盒子量级数 ulp，`bounds_match`），并查内部节点罩住子节点 |
| `check_kept_accessibility` | 测试、`layout-verify`（4 096 个节点以内） | 没投影的节点逐个与冷投影比对（`ShippedAccessibility` 记下交出过的） |
| `check_moved_pins` | 测试、`layout-verify` | 从钉子找出的节点与子树，与按旧规则整棵走的结果相同；钉住的盒子都在索引里 |
| `check_offset_bounds` | 测试、`layout-verify` | 按偏移放置的盒子四角过父节点空间的映射，落在布局盒四角过整条变换链的位置上 |
| `check_viewport_frame` | 测试、`layout-verify` | 自带框架的视口盒子同上 |
| `offset_placed`、`moved_root_hands_offsets`、`plain_transform_step`、`HandedMap` 的断言 | 测试、`layout-verify` | 快路径与慢路径逐位相同 |

### 9.2 主要测试

孪生页测试每帧把增量世界与只做全量重建的孪生页逐位比对：

- `crates/nana-ui-scene/tests/moved_extraction.rs`：平移、同帧改颜色 / 文字 / 主题，记录与冷提取、图元与冷场景逐位比；`moved_controls_that_read_little_of_the_page_keep_what_they_built`、`an_open_select_moved_keeps_its_menu_until_it_flips`、`a_moved_subtree_that_reads_the_page_paints_like_a_cold_scene`。
- `crates/nana-ui-scene/tests/turned_container.rs`：容器连续改转角、缩放、原点，夹着平移、滚动、插入；命中网格、无障碍树、场景与孪生页比；`rows_moved_under_a_turn_with_a_popover_open_keep_a_cold_projection`、`rows_grown_under_a_3d_turn_keep_a_cold_projection`。
- `crates/nana-ui-scene/tests/viewport_frames.rs`：页面根、`fixed`、`sticky`、逃逸的绝对定位连续转与缩放，`AccessibilityPlacements` 核对外接框。
- `crates/nana-ui-scene/tests/child_placement_records.rs`：子节点在父级里挪动之后，Scene 的每条记录与新提取比。
- `crates/nana-ui-scene/tests/select_popup_hit.rs`：图元放到页面上（`place_rect`）再与页面几何比。
- `crates/nana-ui-runtime/src/world/hit_test/moved_turned_tests.rs`：九种转过的祖先下平移，60 × 36 指针网格与孪生页及合成层遍历（`visit_presentation_roots`）比；`boxes_moved_under_a_planar_3d_turn_answer_as_a_fresh_index`。`moved_scroll_tests.rs`：滚动列表里的平移；`stuck_sticky_boxes_are_built_again_alone`：带一行格子的粘性页眉与 body 为粘性的模态框，不整齐步长滚动，每步只重建两个粘性条目，指针网格与冷建一致，贴住的 body 只在框架排出的 body 盒子里接指针。
- `crates/nana-ui-runtime/src/world/accessibility/relative_tests.rs`：不整齐步长、滚动、缩放、转过、钉子、Select、原地改尺寸；`placed_corners_are_the_turned_corners_of_a_cold_projection` 是等价性测试；`a_scroll_projects_the_scroller_and_its_pins_alone`：滚动列表里的粘性页眉、`fixed`、绝对定位与嵌套滚动容器，不整齐步长连续滚动、两个容器同帧滚动、滚动夹着平移、最后滚回原点，每帧增量与全量投影逐位相同，只投影容器与钉子。
- `crates/nana-ui-runtime/src/world/line_box_memo.rs`：行盒记忆按种子类别清。
- `crates/nana-ui/src/scene_paint/tests.rs` 的 `rows_a_reflow_moves_by_a_fraction_paint_what_a_cold_scene_does`：1、1.1、1.2、1.75 倍下行一帧挪 0.37 px，逐像素对比冷场景。

### 9.3 改动时必须跑的

```bash
cargo fmt --all -- --check
cargo test -p nana-ui-runtime --lib
cargo test -p nana-ui-scene --features nana-ui-runtime/layout-verify,rich-text
cargo test -p nana-ui --features hosted,gpu --lib rows_a_reflow_moves_by_a_fraction
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
```

- 运行时的单元测试本身就是 `cfg(test)`，冷对照都在跑；Scene 的集成测试只有带 `nana-ui-runtime/layout-verify` 时才在里面跑运行时的冷对照；`rich-text` 打开 Markdown 那几页。
- `nana-ui` 的 `scene_paint` 在 `gpu` 后面，不带 feature 时匹配 0 条也显示通过。
- 快照预期零像素变化（`gallery-sidebar-collapsed-dark` 是已知抖动）。
- 成本：`perf/micro/stack-depth` 下 `PRINT_STAGES=1 cargo run --release -- edits 2000 height`，以及 `width`、`transform`、`scale` 和 `CONTROLS=1`、`MIXED=1`、`SCROLLED=1`、`ROTATED=1`、`VISIBLE=1`、`OPEN_SELECT=1` 各页；与基线提交交替 8 轮、每轮交换先后读最小值，指令数取 `RUNS=240` 与 `RUNS=40` 两次 `/usr/bin/time -l` 之差除以 200。`PRINT_COUNTERS=1` 打印最后一次编辑的工作计数。结果写进 [资源预算](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md#cpu-时间)。

### 9.4 变异实验

每加一条保留规则都要证明有测试守着它：临时改坏这条规则（例如让判定一律答「保留」、去掉一处标记、把求和顺序换掉、让快路径恒成立），跑上面的测试，确认至少一条变红，再改回来。没变红的，要么补测试，要么在代码注释里写明这条守卫是防御性的、为什么构造不出。规则只影响代价（不影响输出）时，变异实验看不出来，用基准或工作计数量。

## 10. 已知限制

下面这些有意仍按节点或整棵处理，每条都在当前代码里核实过。

- **3D 与透视**（`transform_3d`、`perspective`、`preserve-3d`）：命中不交出本地坐标、不原地重建（`rebuilt_hit_entry_in_place` 拒绝透视与 `space.depth()`），平移与改变换整棵修补；无障碍退回外接框，平移根下面逐个判定；剔除索引在继承的变换带透视时整棵重算。原因：透视并不进仿射，3D 上下文要把变换往下传，平台的无障碍 `transform` 只有 2D 仿射。
- **模态框、动画叠加、长度动画、裁剪 visual**：无障碍的平移根与原地改尺寸的转过盒子都退回逐个判定（`moved_root_hands_offsets` 不成立）。原因：叠加能变换任何盒子，裁剪 visual 裁剪它下面的每个盒子，模态框的无障碍盒是 surface。
- **打开的 MenuSurface**：仍按页面推导（`READS_PAGE`），它或上面的盒子平移时整节点重新提取。原因：表面是菜单项的外接框，菜单项的参照系是页面、由布局在每次平移时重新放置（贴视口翻转），本地表面是「菜单项页面盒子 − 触发器原点」，两边各挪同一个量后相减在末位不同。一次只开一个菜单。
- **Painter 非恒等局部变换**（`origin_bound`，`custom_paint.rs`）：局部变换绕记录在页面上的角点（`about_origin`），平移量是页面坐标的函数。要改成本地，painter 得在加原点之前先过局部变换，合成矩阵的舍入顺序不同，转过的 painter 内容在末位乃至抗锯齿上与现在不同。只有非恒等局部变换的 painter 节点进集合。
- **参照系不是父节点的图标**（`PlacementRead::Page`）：按两边的页面原点求差。要与平移无关得沿两边的参照系链求到共同祖先再相减，图标的 `y` 会在末位变。文字宿主下的绝对定位图标、宿主是打开的 MenuSurface 时才出现。
- **图表悬停指示线**：吸附到页面半像素是要的行为（不吸附线会发虚），只在指针悬停时存在。
- **逐字形呈现的行内对象**：`present_inline_objects` 把绕页面中心的缩放写进图元变换，与 painter 同一个结构（变换在加原点之后），只在呈现动画进行时存在。
- **转场视口里的文字**：视口是页面上的裁剪与文字框求交，裁剪不一定跟着平移；只在入场、退场转场时存在，结束后回到普通本地记录。
- **打开的 Select 的无障碍**：仍是钉子（`VIRTUAL`），平移时字段、列表框和可见选项重投影。原因：列表框和选项的 bounds 是页面上的组件几何与页面上的空间求交、再过视口映射到字段空间得来的，平移后末位就变；不钉住得把这条投影改成从本地几何和本地空间算（输出在末位不同），再给钉子加「菜单盒子没变」的跳过条件。打开的菜单一次只有一个，节点数是个位到十几个。
- **从视口来、自带框架的盒子跟着父节点平移**（转过空间里的 `sticky`、从移动的行量起的绝对定位、自己转的 `fixed`）：它自己重投影（`keeps_relative_bounds` 对 `framed` 答「不保留」），bounds 是页面位置过映射得来，末位随平移变；它下面按偏移放置的盒子不动。
- **改变换时仍整棵的**：自带框架与只是平移之间切换（空间换了）；盒子自己裁剪（子孙的 `clip` 在视口坐标里）；3D 上下文下面的缩放（`style_frames_its_space` 不成立）；命中上不交出本地坐标的条目整棵修补分支；剔除索引对祖先投影变了的子树 `refresh_subtree`，转过以后边界不是平移能得到的（成本没有单独量过）。
- **滚动容器里的平移剩下 O(移动行数) 的常数**：内容范围每个移动的行一次叶子写入（惰性区间平移做不到逐位：叶子是 `fl(子节点范围 + 偏移)`，「旧叶子 + 位移」在末位不同）；无障碍每个移动的行自己的视口盒判定、裁剪查表和变换记忆（子节点「有没有视口盒」要看子节点自己的页面盒子过映射是否有限，父节点一次判定不了）。
- **滚动的剩余成本**：滚动容器自己要重新投影，它的子节点列表随投影重建一次（O(直接子节点)，21 001 盒的页上约 0.2 M 条指令）；每个粘性盒子在命中索引里重建自己一个条目、在无障碍里投影自己一个节点（不管贴没贴住，都要算一遍粘性偏移才知道变没变；节点的子节点列表要随节点一起发，见上一条）；嵌套在滚过的容器里的滚动容器、以及滚动容器里任何裁剪的盒子，无障碍上整棵重投影（`clip` 在视口坐标里）。都跟着钉子走，每个钉子的成本不随它装着的盒子增长，也不随盒子总数增长。
- **平移的根本身**：它的相对偏移确实变了，无障碍、命中、提取都要处理每个平移根，这是下限。

## 11. 新增功能时的检查清单

新增一种布局特性（新的定位方式、参照系、裁剪方式）或一种 visual 时：

1. **分类**：它是否由节点自己的样式、参照系、种类或 visual 决定一种新的空间关系？是就在 `NodeSpace` 加一位，在 `of_style` / `of_visual` / `of` 里置位。只依赖上面链的答案不要存成位，放进 2.4 那类现推的答案里。
2. **维护点**：决定这一位的字段如果在 `note_node_space` 现有的调用点（插入、`set_visual`、`note_layout_anchor_change`、`write_local_layout` 换参照系、`reframe_layouts`）之外被写，补一个调用点。`check_node_spaces` 会在漏掉时报 stale。
3. **派生答案**：逐个过一遍 2.2 的表——它是否让 `places_by_offset`、`hands_offsets`、`pins_accessibility`、`keeps_space`、`aligned_by_style`、`hit_frames_itself`、`page_bound` 的答案变？尤其：
   - 它让盒子读了自身偏移以外的几何 → 命中要钉住（`HitEntry::pinned`），无障碍要进 `pins_accessibility`；
   - 它的 visual 读页面 → `geometry_reads_page` 或 `MENU_ROOM`，进 `page_bound`；只读自己的偏移 → `offset_bound`；读页面位置 → `origin_bound`；
   - 它改变子节点的坐标空间 → 命中的 `local_children` / 原地重建条件、无障碍的 `AccessibleSpace`、Scene 剔除的平移量都要看一遍。
4. **布局**：它的字段在 `may_move_line_boxes` 里有没有归类；跨参照系的读取是否都走同一种求和顺序；结果里新加的字段是否进 `layout_result_projects_new_geometry` / `layout_result_reshapes`。
5. **冷对照测试**：在孪生页测试里加一页（`moved_extraction.rs`、`turned_container.rs`、`viewport_frames.rs` 或 `relative_tests.rs` 里对应的一组），用不整齐的步长（1.37 px）平移、夹着滚动和同帧改动，逐帧比对；加 `layout-verify` 跑 9.3 的命令；对新加的每条保留规则做变异实验。
6. **限制**：保留按节点处理的，写进第 10 节并说明原因。

## 12. 演进

| 提交 | 内容 |
| --- | --- |
| 3d2f6f02e | 设计与分阶段计划 |
| 35045d816 | 阶段 1：布局权威改相对，对外 API 靠推导 |
| 715cc0331 | 阶段 2：命中条目本地化，平移只重建根条目 |
| 3e9c83b80 | 阶段 3：只发平移的根，提取、Scene、无障碍不再逐个处理后代 |
| 6838427fb | 阶段 4：图元与组件几何本地化，painter 先放到原点再吸附 |
| 7fa3b81b4、f47021db4、2f0479886、21f8058fa | 收尾：无障碍从钉子找；读页面的节点改本地；无障碍末位；端到端基准 |
| 40b898c12、dd3179b0f、917a1112e、16a42ab4f、66cb370c0 | 收尾：无障碍滚动与 Select；滚动容器里的命中原地重建、剔除修正与子节点筛选；内容范围 |
| a2539876a、1e9322016、54e3425e7、29f4c3e4d | 收尾：无障碍自带变换；命中交出本地坐标；改变换只标盒子自己 |
| d8eabffea、02c09ef2a、6e21c8579、1ab1855eb | 收尾：行盒记忆按类别清；视口框架与缩放；只改子节点放置也标 RENDER；打开的 Select 本地推导 |
| be0a703c8 | 节点空间分类 `NodeSpace` 与 `space_pins` / `page_bound` 索引 |
| 596930d22 | 自带变换的盒子原地改尺寸一律当遍历根 |
| e443a2a1e、e41729572、138c79333 | 拆分 `world/accessibility/`、`world/hit_test/`、`nana-ui-scene/src/scene/` |
| a2b52d862 | 滚动只改滚动容器：`AccessibilityNode::scroll` 与 `SystemWork::accessibility_scrolled`，滚过的容器只投影自己与钉子；命中里粘性条目原地重建 |
| 2572617f5 | 接回合并 3fab98a65 丢掉的 `non_scroll_hit_dirty` 判定与清空：滚动容器同帧改盒子不再走滚动快路径 |
| 36c731159、62a76afe6 | 滚回原点时贴住过的粘性节点重新投影；粘性偏移进命中条目的 `origin`：贴住的粘性盒子只重建自己一个条目，一次滚动的命中成本不随粘性分支里的盒子数增长 |

各阶段的基准见 [资源预算 · CPU 时间](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md#cpu-时间)。逐项的做法与变异实验记录在各提交的提交信息里，以及当时版本的本文（`git show <提交>:docs/reference/relative-geometry.md`）。
