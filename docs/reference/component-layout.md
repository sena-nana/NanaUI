# 组件内容布局与迁移审计

组件保留语义、交互和 chrome。普通盒子排列交给 Layout Foundation；`ComponentGeometry` 是已发布部件的绘制投影。本文记录 #205 的迁移边界，未迁移行不能据此宣称验收完成。

## 当前产品路径

Button、Chip 和 ActionMenuItem 的旧 API 保持不变。组件内容声明为轻量 Flex parts，调用 `layout_engine::parts`；其主轴分配、收缩和对齐复用 retained Flex 的求解函数。部件没有 `StableNodeId`，不会产生 wrapper、组件生命周期或另一棵树。

`UiWorld` 在同一次布局发布中把 `TextContent`、`LeadingContent`、`TrailingContent`、`Hint`、`Spinner` 写入 `LayoutResult.parts`。文本片段使用实际的 `TextContent` 盒子。Scene 的组件几何只读取这些部件并附加颜色、文字、图标等绘制数据。发布期间的兼容查询调用同一个部件求解器。

纯文字 Button 和 Chip 沿用原来的两个部件（文本和内容），不增加 retained 节点或部件。loading、内容对齐、菜单 hint 与图标占位的变化失效布局输入；hover、disabled 颜色和 spinner 相位保留布局结果。极窄的内容盒会裁限部件范围，避免图标或 hint 产生负宽度。

已继续接入 [InlineContent](inline-content.md)：Text/Button/Chip/ActionMenuItem 接受 canonical 内容，普通 retained child 通过 nana-text object placement 混排，typed fragments 携带 source range；高级控件文字复用 Runtime retained layout，atomic Text 子项独立绘制。`StandardVisual` 的兼容 label/icon 字段仍在；ListItem 的 fallback label/note/detail 已继续迁移为 Foundation 的 TextContent/Note/Detail 部件，复用 Flex 分配与 RTL placement；leading/content/trailing 仍为普通 retained slots，读取已有 child placements。SidebarRow 与 FileTab 复用这一路径；纯 label 不额外测量 note/detail。其余组件和完整 chrome/content 拆分继续按下表迁移。

## mini-layout 审计

| 组件或几何 | 分类 | 当前状态与迁移路径 |
| --- | --- | --- |
| Button / Chip | Flex 内容 + layout parts | 已迁移文字、前后图标和 spinner 位置；绘制保留 chrome 与语义 |
| ActionMenuItem | Flex 内容 + layout parts | 已迁移 icon / label / hint；hint 更新复用原节点 |
| ListItem / SidebarRow / FileTab | 普通 slot composition + Inline parts | retained slots 使用已有 child placements；fallback label/note/detail 已迁移，隐藏 slot 不占文字预算 |
| FormField / LabeledValue | 普通 composition + 命名文本部件 | LabeledValue 已发布 Label/Value，action 使用普通 child，RTL/padding/path 消费同一结果；FormField 已发布 Label/Hint/LeadingContent，control 仍是普通 child，error/hint 槽和 RTL 消费同一结果 |
| Card / Panel / Menu surface | container / Overlay + header / body parts | Card 已发布 Header/Spinner，body 使用普通 children 与 padding；菜单 option row 使用共享 label/hint helper，ActionMenuItem rich span 仍走单一 text primitive |
| Select / Dropdown / ContextMenu option | 可复用 menu row composition | Select trigger 已发布 TextContent/Hint/TrailingContent，Geometry 不再重算 trigger fallback；SelectMenuGeometry 与 ComponentGeometry 提供按逻辑 index 的 row 查询，select/context option 的 hint 分割复用同一 helper 并使用 owner 的 chrome text measure；ContextMenu hint 作为独立行尾文本绘制；option `content` 只承载 fill-colour span；打开的 Select / Dropdown / SearchDropdown 菜单的无障碍选项（`ListBoxOption`）读同一份 SelectMenuGeometry 的行范围，不另算 |
| TextInput / 编辑器 caret、selection、IME | 文本布局与交互几何 | 继续由 `nana-text` 和 Runtime 文本布局提供；不作为普通 Flex 子项机械迁移 |
| 图表、图路径、画布、resize grip | 非 box visual / 组件语义交互几何 | 保留领域算法；外部盒子仍消费 LayoutResult |

## 验证入口

- `cargo test --locked -p nana-ui-runtime --lib issue205`：部件与绘制投影一致、hint/loading 更新、a11y、hover 零布局、1k/10k 单实体和 retained parts 数量。
- `cargo test --locked -p nana-ui-runtime --lib issue199`：1k 实际组件跨 context / 正常流与脱流切换，保留状态、焦点和资源身份。
- `cargo test --locked -p nana-ui-devtools --features runtime-agent --test issue205_component_pixels`：实际 GPU 的组件图像和 hover；设置 `NANA_COMPONENT_REQUIRE_GPU=1` 时无 GPU 必须失败。

Card 标题复用 Runtime 的 retained paragraph，具有独立的单行字形预算与字号/字重；这些文字样式不继承到 body。Header baseline 与所有 context 的 intrinsic baseline 共用同一部件，标题高度只由 band 保留一次。spinner 相位为 paint-only；窄标题的溢出字形裁剪到 Header，不能占用 spinner 间隔。

CPU p95 和 allocator/RSS 的历史比例需要固定机器上的前后报告。实体数和 parts 数量门禁只证明结构成本，不替代 #188 的 CPU ≤5% / memory ≤8% 比例验收。

参见 [布局结果](layout-result.md)、[布局](layout.md)、[组件](components.md)。
