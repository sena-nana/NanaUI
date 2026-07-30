# NanaUI 设计语言

视觉状态可通过 `ui-snapshots` 在离屏 WGPU renderer 中复测，当前覆盖工作区和组件画廊的 dark/light、Surface、Context Menu 与 Dialog；验收范围和限制见 `visual-validation.md`。

NanaUI 采用与 LiliaUI 相同的视觉层级：深色/浅色主题、`background → surface → active` 的表面阶梯、弱分隔线、蓝色主操作色和紧凑的工程工具排版。

第一阶段的语义令牌位于 `crates/nana-ui/src/theme.rs`：面板使用 `surface`，输入/非激活项使用 `subtle`，hover/pressed/selected 使用 `hover`/`active`，文本按 `text`/`muted`/`faint` 分级；`UI_METRICS` 是所有组件共用的 `ThemeMetrics`，统一定义 compact/standard/selection 高度、导航行与分区高度、面板内距、radius 和 motion。运行时 `ThemeTokens` 可组合当前颜色与宿主提供的 metrics；外观设置用 `AppearanceSettings` 只暴露标准圆角，内部以 4px 级差派生微型、控件、卡片和页面圆角。默认标准值 10px 对应 2/6/10/14px，修改后立即作用于公共控件。所有可见按钮都连接到状态模型的真实消息，不放置路线图式入口。

字体使用与 LiliaUI `fonts.css` 同源的 Noto Sans SC 400/500/600/700。资源转换为
Iced 可读取的 TTF 后由 `ui_font_sources()` 统一注册；标题使用 0.2px 字距，
分区标题与 Card 标题使用 0.5px 字距。平台字体仅作为未注册资源时的降级路径，
不作为验收基线。

共享控件样式位于 `crates/nana-ui/src/widgets.rs`。输入框采用 13px 文本、默认
6px 控件圆角和 30px 级高度，hover 使用 `border-strong`，focused 使用 2px
`accent`，invalid 使用 `danger`；复选框为 16px、4px 圆角，开关为 30px
级宽度，滑轨为 4px，进度条为 6px。disabled、loading、invalid 和 selection
只通过语义令牌表达，深色与浅色主题保持相同的尺寸和状态层级。

按钮内容由 Iced `Button` 的布局阶段统一执行水平、垂直居中，而不是依靠字体
baseline 或各页面的偶然 padding。菜单、列表和侧栏等需要左对齐的复合按钮会
显式声明左对齐；图标按钮、文本操作和“图标 + 文本”操作共享居中合同。

卡片布局使用等分网格：同一行的卡片具有相同宽度和高度，相邻行共享相同外边距
与间距；内容多少不再改变卡片边缘。Controls 的三列卡片和下方两列卡片、
Surface 普通/交互卡片以及 Feedback 状态/操作卡均遵循该合同。

分段控件的 1px 边框不参与 Iced 布局，因此组件以“1px 边框 + 2px 内容内距”
得到 3px 有效内缩：34px 外层与 28px 选项上下各留 3px，内层圆角同步使用
“外层圆角 − 3px”，使两层圆弧保持同心。普通 Tab、列表项和其他选中按钮仍使用
各自的控件圆角。

侧栏采用统一的 28px 单行导航与分区标题，列表行与分区标题共用完整宽度和
“控件圆角”（默认 6px）、
14px 层级缩进和固定
Footer；Frame 内距为 10/8/10/12px，正文独立滚动，顶部与 Footer 不随列表
移动。Footer 设置入口使用 26px 图标按钮；普通列表与设置分类共用同一个侧栏
Region，不并排叠加导航层。长标签使用末尾省略。设置返回行和分类直接复用
`SidebarRow`，统一为 28px 行高、同一控件圆角和相同的左右内距及交互状态。
设置页只呈现已接入的真实状态；主题和标准圆角可直接编辑并即时预览，四档圆角
始终保持内部级差，恢复默认操作会回到 `UI_METRICS`。设置行允许 `label + hint`，普通
侧栏行只保留单行主标签。

侧栏 Region 折叠使用 240ms ease-out 尺寸过渡；分组项使用 160ms ease-out
高度裁剪，并同步将 disclosure 箭头从向右旋转为向下。帧订阅只在动画进行时
存在，反向点击从当前进度继续，动画结束后不产生空闲重绘。

主区域沿用 LiliaUI 原有的 `edge-start` / `edge-end` 规则：对应侧存在展开的
侧栏 Region 时，主区域保留由标准圆角派生的页面圆角；主区域成为中间行首个或
最后一个展开轨道时，该侧上下两个圆角自动归零。圆角由工作区表面统一绘制并用
角部遮罩约束宿主内容，不依赖页面内容自行设置相同圆角。“外观 → 圆角”提供
默认开启的主区域圆角开关；关闭后主区域四角全部归零，重新开启后恢复贴边计算。

窗口材质、透明 Surface 和平台原生外观属于后续 `nana-window` 边界；控件不会直接调用平台 API。
