# NanaUI 设计语言

视觉状态可通过 `ui-snapshots` 在离屏 WGPU renderer 中复测，当前覆盖工作区和组件画廊的 dark/light、Surface、Context Menu 与 Dialog；验收范围和限制见 `visual-validation.md`。

NanaUI 采用与 LiliaUI 相同的视觉层级：深色/浅色主题、`background → surface → active` 的表面阶梯、弱分隔线、蓝色主操作色和紧凑的工程工具排版。

第一阶段的语义令牌位于 `crates/nana-ui/src/theme.rs`：面板使用 `surface`，输入/非激活项使用 `subtle`，hover/pressed/selected 使用 `hover`/`active`，文本按 `text`/`muted`/`faint` 分级；`UI_METRICS` 是所有组件共用的 `ThemeMetrics`，统一定义组件高度、面板内距、radius 和 motion。运行时 `ThemeTokens` 可组合当前颜色与宿主提供的 metrics；外观设置用 `AppearanceSettings` 只暴露标准圆角，内部以 4px 级差派生微型、控件、卡片和页面圆角。默认标准值 10px 对应 2/6/10/14px，修改后立即作用于公共控件。所有可见按钮都连接到状态模型的真实消息，不放置路线图式入口。

单行交互组件只使用 `ControlSize` 的三档外框高度：

| 档位 | 高度 | 默认用途 |
| --- | ---: | --- |
| `Small` | 28px | 导航、菜单、列表和紧凑图标操作 |
| `Medium` | 32px | 表单、按钮、选择器和常规操作 |
| `Large` | 36px | 调用方显式选择的强调场景 |

按钮、输入、选择器、复选框、开关、范围控件、分段控件、Tab、列表、菜单和侧栏
均从当前 `ThemeTokens.metrics` 解析该档位，不再拥有独立行高。带 hint 的开关可以按
多行内容自然增高；Textarea、XYPad、卡片、Dialog、日历、标题栏和 Workspace
Region 等内容或结构尺寸不套用单行档位，但其中的单行操作仍遵循三档合同。

字体使用与 LiliaUI `fonts.css` 同源的 Noto Sans SC 400/500/600/700。资源转换为
Iced 可读取的 TTF 后由 `ui_font_sources()` 统一注册；标题使用 0.2px 字距，
分区标题与 Card 标题使用 0.5px 字距。平台字体仅作为未注册资源时的降级路径，
不作为验收基线。

`UI_BASE_TEXT_SIZE` 将标准正文与中号控件统一为 13px；NanaUI 应用在启动时调用
`ui_font_defaults()`，手动构造的 Renderer 同样使用该常量。统一排版合同不等于
所有文本使用同一字号：小号控件、辅助文本、标题和展示文字继续保留各自的语义
层级。

共享控件样式位于 `crates/nana-ui/src/widgets.rs`。输入框采用 13px 文本、默认
6px 控件圆角和 32px 中档高度，hover 使用 `border-strong`，focused 使用 2px
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
得到 3px 有效内缩；外层严格使用所选 `ControlSize`，内部选项高度由外层减去
上下内缩得到。内层圆角同步使用“外层圆角 − 3px”，使两层圆弧保持同心。普通
Tab、列表项和其他选中按钮仍使用各自的控件圆角。

侧栏采用统一的 28px 单行导航与分区标题，列表行与分区标题共用完整宽度和
“控件圆角”（默认 6px）、
14px 层级缩进和固定
Footer；Frame 内距为 10/8/10/12px，正文独立滚动，顶部与 Footer 不随列表
移动。Footer 设置入口使用 28px 小档图标按钮；普通列表与设置分类共用同一个侧栏
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

应用标题栏固定为 36px 三列结构：左右区域等分、标题保持窗口几何居中，leading /
trailing 内边距为 6px，窗口按钮为 28px、间距 2px。空白区域移动超过 4px 后
发起宿主拖拽；按钮等交互子项不会触发拖拽。macOS 为左侧原生交通灯增加 78px
leading inset，Windows/Linux 显示自绘最小化、最大化/还原和关闭按钮；关闭按钮
复用 danger 状态。

窗口材质、透明 Surface 和平台原生外观仍属于 `nana-window` 边界；标题栏只发出
语义动作，不直接读取原生窗口句柄或调用平台 API。
