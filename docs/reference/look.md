# 视觉与主题规范

NanaUI 的默认视觉面向桌面生产力软件设计：同时提供深色与浅色外观，采用紧凑的间距与克制的弱分隔。强调色（Accent）仅用于主操作或值本身带有强调语义的控件（如主按钮、开启的开关、带有效值的滑杆）。切换深浅色只替换调色板颜色，不改变控件尺寸，也不改变交互状态的分层方式。

宿主在 `RuntimeProgram::theme` 中返回当前激活的 `Arc<CompiledTheme>`。内置的 `Light`（`nana.light`）与 `Dark`（`nana.dark`）只是两套预制主题；应用可以通过 `ThemeDefinition` 在 `ThemeRegistry` 中注册任意自定义 `ThemeId`。所有内置控件只读取语义 token，不依赖具体的主题名称。

## 尺寸与字阶体系

### 控件高度与间距网格

单行交互控件提供三档标准高度，通过 `ControlSize` 生效：

| 档位 | 高度 | 典型场景 |
| --- | ---: | --- |
| 小（`Small`） | 28 px | 导航行、菜单项、密集列表、紧凑图标按钮 |
| 中（`Medium`） | 32 px | 表单输入、常规按钮、下拉选择、默认操作 |
| 大（`Large`） | 36 px | 需要突出强调、由应用显式指定的主操作 |

桌面壳层固定尺寸：标题栏高度为 `36 px`（`TITLE_BAR_HEIGHT`），窗口控制按钮为 `28 px`，侧栏导航行为 `28 px`。多行容器的高度随内容增长，但其内部的单行操作控件仍遵循上述三档高度。

间距标度集中在 `nana_ui::theme::space`，基于 **2 px 网格**定义（1、3、5、7 不单独设档，分别归入 `XXS`、`XS`、`SM`）。`1 px` 细描边使用 `HAIRLINE` 常量，不属于间距档位。

### 排版字阶与字重

产品字阶与字重定义在 `nana_ui::theme::type_scale`。标准正文尺寸为 `13 px`（即 `UI_BASE_TEXT_SIZE` / `type_scale::BODY`）：

| 字阶常量 | 字号 | 字重常量 | 数值 |
| --- | ---: | --- | ---: |
| `HINT` | 11 px | `REGULAR` | 400 |
| `META` | 12 px | `MEDIUM` | 500 |
| `BODY` | 13 px | `SEMIBOLD` | 600 |
| `SECTION` | 14 px | `BOLD` | 700 |
| `HEADING` | 16 px | | |
| `TITLE` | 18 px | | |
| `DISPLAY` | 20 px | | |

## 颜色、高对比度与阴影

界面表面按层级递进：面板是一层基础表面，输入框与未激活项退后一层，悬停（hover）、按下（pressed）与选中（selected）依次抬升；文本相应分为主要（primary）、次要（muted）与弱化（faint）三档。输入框聚焦时增加一圈中性描边而不改变底色；校验错误优先使用危险色（danger）。卡片默认不画描边，需要区分层级时使用阴影；选中态卡片使用柔和的选中底色，不用强调色包边。

CSS 的单层 `box-shadow`（支持 outset 与 inset）与 `text-shadow`（仅 outset）已接入渲染管线：inset 阴影使用内阴影 SDF 绘制；当容器带有子节点时，自动通过目标合成组（dest group）以 overlay 方式叠在内容之上，而不是把外阴影画进盒子里充数。

### 主题定义与高对比度覆盖

颜色统一来自共享的 `ThemeTokens` 与 `SemanticPalette`（位于 `nana_ui::theme`），而不是每个控件各自维护一套样式表。请不要把任意业务硬编码颜色写进框架 token：

- **自定义整套主题**：构造一份 `ThemeDefinition`（可从 `ThemeDefinition::NANA_DARK` 或 `NANA_LIGHT` 派生，只修改需要定制的档位，但必须填齐所有组件配方槽位），调用 `nana_ui::theme::install_theme_definition(&mut context, &definition)` 或交由宿主的 `ThemeRegistry` 注册。它会一次性替换颜色、尺寸、排版、动效时长、阴影与组件配方。
- **系统高对比度**：当操作系统开启高对比度模式时，原生宿主调用 `AppContext::set_high_contrast(true)`。此时已安装的深色主题自动改用 `SemanticPalette::high_contrast()`（黑底、白字、黄强调、青高亮），浅色主题改用 `SemanticPalette::high_contrast_light()`（白底、黑字、深蓝强调）；`muted` 与 `faint` 保持与正文可区分的灰度，避免禁用态文字看起来像可用状态。高对比度是渲染期的策略覆盖（policy overlay），不改写已安装主题本身——关闭高对比度即可原样恢复，期间新安装的主题也会自动套用高对比度覆盖。
- **公共导出面**：`nana_ui::theme` 导出了构成主题定义的全部 token 与配方类型，包括 `ThemeMetrics`、`SwitchMetrics`、`TreeMetrics`、`KeycapMetrics`、`TypographyTokens`、`MotionTokens`、`EffectTokens`、`SpacingTokens`、`BorderTokens`、`OpacityTokens`、`SurfaceTokens`、`AccentRamp`、`ComponentThemeRegistry`、各类 recipe draft（如 `ButtonVariantDraft`）、组件配方（如 `DialogRecipe`、`ToastRecipe` 及其 `ToastIndicator` / `ToastGlyphs` / `ToastProgressRecipe` / `ToastMotion`），以及 `ThemeId`、`ThemeSchemaVersion`、`ThemeGeneration`。节点级命名尺寸使用的 `RadiusTier`、`ControlHeight`、`ControlPadding`、`SurfacePadding`、`SquareSize` 与 `SemanticColorMix` 同样从这里导入，应用无需直接依赖 `nana-ui-core`。

> 如果你要修改框架自身的主题架构或审计组件硬编码，请先阅读维护者文档 [主题审计记录](theme.md)。

## 字体、语言与排版特性

启用 `bundled-fonts` feature 时，宿主会自动注册 Noto Sans SC，并将其设为默认的 `sans-serif` 界面字体。捆绑的是一个可变字体：`wght` 轴 400–700、默认实例 Regular，命名实例 Regular / Medium / SemiBold / Bold，四档界面字重都走真实字重轴，不合成粗体。字符集是简体中文常用字加 JIS X 0208 第一水准汉字与各消费者界面文案用到的字；文件由 `scripts/build-ui-font.py` 生成，来源、校验和与字符集见 `crates/nana-ui/assets/fonts/README.md`。关闭该 feature 则回退到系统字体（注意：视觉快照与设计基线核对必须使用捆绑字体，不能拿系统字体替代）。

### 宿主字体注册与热卸载

应用可以通过以下两种粒度将自有字体载入同一个 `FontSystem`，与捆绑的 Noto 字体并列使用（未命中的字形仍回退到捆绑字体或系统字体）：

1. **进程级常驻字体**：调用 `register_host_font_bytes` 或 `register_host_font_file`，注册后的字体保留至进程结束。
2. **可卸载的作用域字体（皮肤 / 主题包）**：使用 `HostFontScope`，其 `add_bytes`、`add_file`、`add_face` 方法各返回一份带字体族名的 `HostFontRegistration`；调用 `clear()` 或 drop 该 scope 即可一次性撤回其注册的全部字体（单份注册也可用 `register_host_font_source*` 配合 `unregister_host_font` 手动管理）。撤回字体会推进一次新的字体代际（`FontGeneration`）：凡是用过这些字体的文本会在下一次 `flush` 时自动重新测量，旧代际的字形位图不会再被采样。

### OpenType 整形、中日文标点挤压与竖排

- **字距与可变字体轴**：字距（tracking）与 `font-feature-settings`、`font-kerning` 直接进入 `nana-text` 的整形器（shaper），不是排版后的事后平移。`font-variation-settings` 会兑现字体中实际存在的变体轴：`wght` 并入 `font-weight`，`wdth` 与自定义轴（如 `BEVL`）进入同一份 `FontVariations`；字体本身没有的轴会被跳过，不会谎报或改写成 `wght`。
- **断行与中日文排版**：`word-break: break-all | break-word` 与 `line-break: anywhere` 会改变折行策略（不支持 `keep-all`，声明后会被跳过）；`line-break: strict | loose` 按中日文规则裁剪断点。当语言（`lang`）为中文（`zh`）或日文（`ja`）时，默认挤压行首、行尾及相邻标点；`text-spacing-trim: space-all` 可关闭挤压，`auto` 则让放不下的行先挤压标点再折行。`text-autospace`、`text-align: justify` + `text-justify`、`text-wrap-style: pretty` 可按需开启。
- **语言声明（`lang`）与 `locl` 字形替换**：应用级默认语言通过 `AppContext::set_default_language` 声明，局部子树通过 `MutationQueue::set_language` 声明；若未声明且文本引擎无兜底语言，则文本不携带语言标签，也不触发标点挤压。与浏览器的 HTML `lang` 一致，语言标签还会按每段文本的 script 选择字体内置的 OpenType 语言系统（如 `locl`）：例如捆绑的 Noto Sans SC 在无语言或 `en` / `ja` 下会把拉丁数字 `0–9` 和 `…` 替换成较矮的字形，而在声明 `zh-CN`（或 `zh-Hans`）后数字保持标准高度、直引号 `"` `'` 相应抬高（与 Chrome 渲染 `lang="zh-CN"` 的行为完全一致）。
- **竖排书写（`writing-mode`）**：在 `writing-mode: vertical-rl | vertical-lr` 下，文本按列自上而下排列，中日韩文字直立、拉丁字母侧卧，文本编辑器同样支持在竖排列中就地编辑。

## 组件专属度量与节点命名尺寸

单行控件的三档高度和内边距存放在 `ThemeMetrics` 中，而特定组件的专属几何则作为组合子结构挂在 `ThemeMetrics` 下——这样既避免把单个控件的内部数字和全局 `control_height` 混排，又能保证它们随已安装的主题一起切换：

- **`ThemeMetrics::switch`（`SwitchMetrics`）**：默认 `30×16 px` 轨道，配 `8 px` 标签间距。无标签的 `Switch` 会直接按已安装主题的轨道宽度定宽，不额外加内边距，因此放进设置行尾时无需手动计算宽度。
- **`ThemeMetrics::scrollbar`**：管理滚动条车道、静止与激活滑块厚度及内缩；其配色、动效和显隐策略则位于 `ScrollbarRecipe`。
- **`ThemeMetrics::tree`（`TreeMetrics`）**：管理 `TreeView` 的行间距（`row_gap`）、每层缩进（`indent`）、行内边距（`inset`）、展开三角方块（`disclosure`）与行图标方块（`icon`），默认常量 `TREE_METRICS` 为 `2 / 12 / 4 / 16 / 12`。
- **`ThemeMetrics::keycap`（`KeycapMetrics`）**：管理 `kbd` / `chord` 键帽的高度（`height`，也是单字母键帽的最小宽度）与底边厚度（`edge`），默认常量 `KEYCAP_METRICS` 为 `24 / 2`。
- **紧凑密度预设（`COMPACT_METRICS`）**：面向专业工具检查器与高密度面板（20 px 小号控件、24 px 中号控件，内边距收紧，树行无间距、每层缩进 10 px，键帽高 20 px），可通过 `ThemeDefinition::with_metrics(nana_ui::theme::COMPACT_METRICS)` 安装。

### 用命名档位代替硬编码数字

给节点设置尺寸与圆角时，应使用 `NodeStyle` 上的命名档位方法：`radius`、`corner_radii`、`control_height`、`control_padding_x`、`square`。**不要在构造节点时把 `UI_METRICS.radius_lg` 这类具体浮点数直接写进 `layout`**——那等于在构造期就把 token 展开成了死数字，后续无论安装什么主题都无法改变它。

使用命名档位时注意两条规则：

1. **`control_padding_x` 会覆盖节点自身的左右内边距**。若某个容器需要退出标准控件水平内边距（例如 `sidebar.rs`），需先将 `control_padding_x` 设为 `None`，再设置自定义 `padding`。
2. **`square` 只是未显式指定尺寸时的默认方盒**（写入 `min_width` / `min_height`）。只要节点在某一轴上显式设置了 `width` 或 `height`，方盒在该轴上的最小值就会自动让位，因此给图标按钮指定 `22×22` 时无需先清空 `square`。

当两个块拼接成一个整体时，用 `corner_radii` 按「左上、右上、右下、左下」分别指定四个角的档位（`None` 表示直角）：外侧保留圆角，**接缝处必须设为直角（`None`）**——哪怕给接缝处留了极小的 `Xs` 圆角，两段圆弧交界处也会露出缺口。

## 圆角档位与超椭圆（CornerShape）

`AppearanceSettings` 暴露四级基础圆角：微型（xs）、控件（sm）、卡片（md）、页面（lg），默认值分别为 `2`、`6`、`10`、`14`（`standard_radius` 仍是 `md = 10` 的兼容别名，只改它不会重算其余三档；仅含 `standard_radius` 的旧版 JSON 仍按 ±4 / ±8 兼容推导）。在此之上还有两档跟随页面档位派生的浮层与大表面圆角：

- **`RadiusTier::Xl`（默认 20 px）**：用于浮在页面之上的轨道表面，始终比卡片圆一档。
- **`RadiusTier::Xxl`（默认 24 px，`ThemeMetrics::radius_xxl`）**：用于媒体播放条、详情大卡片等独立大块表面（对应 CSS `--radius-2xl`，Paint 脚本中名为 `"xxl"`）。
- **`RadiusTier::Full`（`FULL_RADIUS`，不随主题）**：胶囊、圆点、头像。解析为一个大于任何盒子的值，绘制时按短边一半收紧，所以不必知道高度（Paint 脚本中名为 `"full"`）。

字号、字重和间距也可以只说档位：`Text::role(TypeRole::Body)` / `.weight(TextWeight::Semibold)`、`Stack::gap_step(SpacingStep::Md)` / `.padding_step(x, y)`（即 `NodeStyle::type_role` / `type_weight` / `gap_step` / `padding_step`）。它们和圆角档位一样在安装主题时解析，主题换了字号或间距刻度，节点跟着动；写死 `type_scale::BODY` 或 `space::MD` 则在构造时就花掉了。

应用可通过 `save_to_store` 与 `restore_from_store` 将外观设置持久化到独立的 `Settings` 命名空间。当主区域紧贴展开的侧栏时，靠近侧栏的那两个角会由工作区表面自动收成直角，另一侧保留页面圆角。

### 圆角形状：`CornerShape`

圆角的几何曲线由主题中的 `ThemeMetrics.corner_shape` 控制，通过 `ThemeDefinition::with_corner_shape` 配置并随 `set_theme_definition` 安装到文档：

- `CornerShape::Round`（默认）：标准圆弧。
- `CornerShape::SQUIRCLE`：对应 CSS `corner-shape: superellipse(2)`。
- `CornerShape::Superellipse(k)`：对应 `superellipse(k)`（`k` 取值范围为 `1..=4`）。

`corner_shape` 作用于该文档中由 GPU 绘制的每一个圆角：控件与卡片表面、描边、阴影、`overflow` 圆角裁剪、毛玻璃、HostTexture、`Painter` 中无裁剪的实心圆角矩形、全圆胶囊，以及 `clip-path: inset() round`。在 CPU 上三角化的圆角（如 `Painter` 在裁剪内、渐变或非等比缩放下绘制的圆角矩形，以及原生窗口阴影）仍保持标准圆弧。每份文档按各自安装的主题独立绘制，多窗口与离屏会话互不干扰；仅切换 `corner_shape` 只触发重绘，不触发重新布局。

## 动效与浮层交互细节

- **壳层过渡**：Runtime 侧栏折叠与分组展开时长均为 `260 ms`，缓动曲线为 `EaseInOutCubic`。只有动画进行中才请求帧，中途反向操作会从当前进度平滑接续。
- **Motion IR 分流**：`nana-ui-core::motion` 的 Motion IR 按属性分流。`opacity` 与 `transform` 走合成器 overlay；`color` 与 `filter` 走 Paint 类 CPU 插值；`width`、`height` 与逐轴的 `font-variation-settings` 走 Layout 类轨道（可变字体轴每帧生成真实的字形变体、重新整形并栅格化，绝不用几何缩放伪造）。
- **浮层与拖拽阈值**：菜单默认与触发控件的起始边对齐，靠近窗口边缘时由框架自动收回视口内，应用无需手算坐标。可拖拽的 Tab 与列表在按下后移动超过 `4 px` 才进入拖拽态，未越过阈值仍视为普通点击选中。命令面板（`CommandPalette`）标准宽度为 `680 px`，打开后输入框自动获取焦点。

### 通知操作按钮

`ToastRecipe::control_radius` 独立控制通知内部操作按钮与关闭按钮的圆角，默认 `RadiusTier::Sm`。主题切换会原地更新已有按钮。需要胶囊外观时可选择较大的圆角档位（如 `Xl`）；按钮大小仍由 `control_size` 决定，颜色与交互反馈沿用按钮种类。

## Runtime theme expressions

Compiled Rust views retain theme references through the shared Style Model. Use
`gap: theme(space-md)`, `font-size: theme(type-body)`,
`font-weight: theme(weight-semibold)`, `line-height: theme(line-tall)`,
`border-width: theme(border-hairline)` and `border-radius: theme(radius-md)`.
Spacing names are `xxs/xs/sm/md/lg/xl/xxl/xxxl/page-tight/page`; type names are
`hint/meta/body/section/heading/title/display`. Line roles are `line/tall`.
The existing radius and control tiers also accept `theme(radius-lg)` and
`theme(control-small)`.

Padding, margin, gap and border-width shorthands accept mixed literal and theme
values. Dimensions, physical edges, individual border widths and typography also
accept linear `calc()`, for example
`min-height: calc(2 * theme(line-tall) + theme(space-xs) + 4px)`.
Multiplication and division require a constant operand; percentages and viewport
units continue to use the ordinary CSS length path. Literal declarations clear
the corresponding theme binding, including conditional and container rules.

Rust uses `nana_ui::theme::{ThemeExpr, ThemeToken, ThemeProperty}` through
`NodeStyle::theme(property, expression)` or
`Arc::make_mut(&mut style.layout).theme.padding_left = Some(expression)`.
`ThemeExpr::token(ThemeToken::Space(SpacingStep::Md))` supports `scaled`, `plus`
and `offset` for derived geometry. The expressions resolve on style writes and
theme installs, after responsive composition, without string lookup during a
frame. Palette-only changes do not invalidate layout or text shaping.

For geometry derived outside layout, `view::theme_signal()` yields the installed
`Arc<CompiledTheme>` after mounting and follows subsequent installs until its
scope is disposed. Prefer ordinary theme expressions for styles. Virtual grids
can use `.grid_theme(min_width, SpacingStep::Xxxl)` so column grouping, row spacing
and virtual row extents follow the same installed spacing step.
