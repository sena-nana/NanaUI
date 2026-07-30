# NanaUI 设计语言

视觉状态可通过 `ui-snapshots` 在离屏 WGPU renderer 中复测，当前覆盖工作区和组件画廊的 dark/light、Surface、Context Menu 与 Dialog；验收范围和限制见 `visual-validation.md`。

NanaUI 采用与 LiliaUI 相同的视觉层级：深色/浅色主题、`background → surface → active` 的表面阶梯、弱分隔线、蓝色主操作色和紧凑的工程工具排版。

第一阶段的语义令牌位于 `crates/nana-ui/src/theme.rs`：面板使用 `surface`，输入/非激活项使用 `subtle`，hover/pressed/selected 使用 `hover`/`active`，文本按 `text`/`muted`/`faint` 分级；`ThemeMetrics` 同时定义 density、radius 和 motion。所有可见按钮都连接到状态模型的真实消息，不放置路线图式入口。

共享控件样式位于 `crates/nana-ui/src/widgets.rs`。输入框采用 13px 文本、12px 圆角和 30px 级高度，hover 使用 `border-strong`，focused 使用 2px `accent`，invalid 使用 `danger`；复选框为 16px、4px 圆角，开关为 30px 级宽度，滑轨为 4px，进度条为 6px。disabled、loading、invalid 和 selection 只通过语义令牌表达，深色与浅色主题保持相同的尺寸和状态层级。

按钮内容由 Iced `Button` 的布局阶段统一执行水平、垂直居中，而不是依靠字体
baseline 或各页面的偶然 padding。菜单、列表和侧栏等需要左对齐的复合按钮会
显式声明左对齐；图标按钮、文本操作和“图标 + 文本”操作共享居中合同。

卡片布局使用等分网格：同一行的卡片具有相同宽度和高度，相邻行共享相同外边距
与间距；内容多少不再改变卡片边缘。Controls 的三列卡片和下方两列卡片、
Surface 普通/交互卡片以及 Feedback 状态/操作卡均遵循该合同。

窗口材质、透明 Surface 和平台原生外观属于后续 `nana-window` 边界；控件不会直接调用平台 API。
