# 视觉

NanaUI 的默认外观是给桌面产品用的。它有深色和浅色，间距紧凑，分隔很弱。

强调色只出现在主操作上，或者值本身就是强调的控件上。主按钮、打开的开关、有值的滑杆属于这一类。

深色和浅色只换颜色。尺寸不换。状态怎么分层也不换。

宿主在 `RuntimeProgram::theme` 返回已注册的 `Arc<CompiledTheme>`。Light 和 Dark
只是两个预制主题；应用可以用 `ThemeDefinition` 注册任意 `ThemeId`，组件只依赖
语义 token，不依赖主题名称。

## 尺寸

单行控件有三档高度。它们经 `ControlSize` 生效：

| 档位 | 高度 | 用在 |
| --- | ---: | --- |
| 小 | 28px | 导航、菜单、列表、紧凑图标 |
| 中 | 32px | 表单、按钮、选择、常规操作 |
| 大 | 36px | 需要强调、由你显式选的场合 |

标准正文是 13px。它就是 `UI_BASE_TEXT_SIZE`，也就是 `type_scale::BODY`。

产品字号阶梯是 `nana_ui::theme::type_scale`。`HINT` 是 11，`META` 是 12，`BODY` 是 13，`SECTION` 是 14，`HEADING` 是 16，`TITLE` 是 18，`DISPLAY` 是 20。字重是 `REGULAR` 400、`MEDIUM` 500、`SEMIBOLD` 600、`BOLD` 700。

间距走 `nana_ui::theme::space`。这是 2px 网格。1、3、5、7 不新增档，接到 `XXS`、`XS`、`SM`。

1px 描边走 `HAIRLINE`。它不是 spacing 档。

标题栏是 36px。窗口按钮是 28px。侧栏导航行是 28px。多行内容按内容长。里面的单行操作仍走这三档。

## 颜色

面板是一层表面。输入和未激活项更弱一层。悬停、按下、选中再往上走。文字分主、次、更弱。

输入框聚焦时加一圈中性描边。底色不改。错误优先用危险色。

不要用任意业务色去改框架 token。主题色走已安装主题的 `SemanticPalette`，
外观设置只负责 policy overlay。

卡片默认没有描边。需要抬起来时用阴影。选中卡片用柔和的选中底，不用强调色包边。

Vue CSS 的单层 `box-shadow`（outset 与 inset）和 `text-shadow`（仅 outset）已经映射到绘制。inset 走内阴影 SDF。有子节点时用 dest 合成组 overlay。这不是把 outset 画进盒子里冒充。

颜色来自共享的 `ThemeTokens` 和 `SemanticPalette`（`nana_ui::theme`）。不是每个控件一份样式表。

间距标度是 `nana_ui::theme::space`。字号和字重是 `nana_ui::theme::type_scale`。你不要直接依赖 `nana-ui-core`，也不要复制字面量。

要整套换掉时，装一个 `ThemeDefinition`。换掉的不只是颜色和尺寸，还有排版、动效时长、阴影和组件配方。调用 `nana_ui::theme::install_theme_definition(&mut context, &definition)`。

可以从 `ThemeDefinition::NANA_DARK` 或 `NANA_LIGHT` 派生，只改你要改的那几档，
再交给宿主的 `ThemeRegistry` 注册。缺一个 recipe 槽位时主题不会安装。

`nana_ui::theme` 把构成一份定义的 token 结构体都导出了。其中包括 `ThemeMetrics`、`SwitchMetrics`、`TypographyTokens`、`MotionTokens`、`EffectTokens`、`SpacingTokens`、`BorderTokens`、`OpacityTokens`、`SurfaceTokens`、`AccentRamp`、`ComponentThemeRegistry`，以及 `ButtonVariantDraft` 一类的 recipe draft，还有 `ThemeId`、`ThemeSchemaVersion`、`ThemeGeneration`。节点级命名尺寸要用的 `RadiusTier`、`ControlHeight`、`ControlPadding`、`SurfacePadding`、`SquareSize` 和 `SemanticColorMix` 也在这里。

你从这里拿。不要直接依赖 `nana-ui-core`。

要改框架本身的主题架构，先读 [主题与样式](theme.md)。那篇是这套合同当前的完整清单和基线，包括哪些 token 装了却没人读。

## 字体

`bundled-fonts` 开启时，宿主注册 Noto Sans SC 的四档字重，并设为 sans-serif 默认。这是界面字体。

未启用该 feature 时回落系统字体。快照和设计对照不能拿系统字体当真。

你也可以关掉捆绑字体，用 `register_host_font_bytes` 或 `register_host_font_file` 把自有字体载入同一套 FontSystem。它们和捆绑的 Noto 并列。未注册的仍回落捆绑字体或系统字体。

字距走 `nana-text` 的 shaping。这是 tracking，不是事后平移。

`font-feature-settings` 和 `font-kerning` 进入 shaper。`font-variation-settings` 兑现已经声明、并且字体里存在的轴。`wght` 并进 `font-weight`。`wdth` 和自定义轴（例如 `BEVL`）走同一份 `FontVariations`。字体没有的轴会跳过，不会改写成 `wght`。

`word-break: break-all|break-word` 和 `line-break: anywhere` 会改 wrap。`keep-all`、`strict`、`loose` 不支持。你声明了也会被跳过。

`writing-mode: vertical-rl | vertical-lr` 下，文字按列排。CJK 直立，拉丁侧卧。编辑器也在列里编辑。

`@font-face` 在 stylesheet 解析时只收集规则。`url(...)` 的加载和字体注册发生在 `inject_stylesheet`。那是宿主适配器，`scene-view`，不在 CSS parse 里。

CSS `font-family`（以及 weight 和 style）会映射到刚载入的 face。坏的 src 丢掉该 face。不用系统字体顶替。

## 控件尺寸

单行控件的三档高度和内边距在 `ThemeMetrics` 里。switch 的轨道在 `ThemeMetrics::switch`（`SwitchMetrics`：30×16 轨道，加上 8 的标签间距）。scrollbar 的在 `ThemeMetrics::scrollbar`。

两者都是组合进来的子结构。理由一样：某控件专属的几个数字不该和 `control_height` 并排，但它们确实属于已经安装的主题。

节点级尺寸用命名档位：`NodeStyle::radius`、`corner_radii`、`control_height`、`control_padding_x`、`square`。不要把 `UI_METRICS.radius_lg` 这类数字写进 `layout`。那是在构造期把 token 花掉。之后装什么主题都推不动它。

`control_padding_x` 会盖掉节点自己的左右内边距。这是「用命名档位，而不是花掉数字」的代价。要退出，就把它设成 `None`，再自己写 padding。`sidebar.rs` 就是这么做的。

四个角不同档的形状，用 `corner_radii`。两块拼成一体时，外侧是圆角，接缝是直角。四个值按左上、右上、右下、左下。`None` 是直角。

接缝处不要给圆角。哪怕是 `Xs`，两段弧分开的地方都会露出缺口。

没有标签的 `Switch` 自己按安装的轨道宽度定宽，不加内边距。行尾开关不用再算宽度。

## 圆角

`AppearanceSettings` 暴露四级圆角：微型、控件、卡片、页面。默认是 2、6、10、14。

`RadiusTier::Xl`（默认 20）给浮在页面上的轨道用。它比卡片圆一档。它不是单独的设置，始终跟在页面档之上。

`standard_radius` 仍是 md（10）的别名。只改这一档，不会重算另外三档。

遗留 JSON 若只有 `standard_radius`，仍按旧规则一次推导 ±4 和 ±8。

你可以用 `save_to_store` 和 `restore_from_store` 把这份设置写进独立的 Settings namespace。旧的 `nana.appearance.*` 只迁移一次。

主区域贴着展开的侧栏时，挨着侧栏的那两个角收成直角。另一侧保持页面圆角。这由工作区表面统一画。不靠页面自己设相同的圆角。

## 运动与浮层

Runtime 侧栏折叠是 260ms。分组展开也是 260ms。缓动是 EaseInOutCubic。

只有动画在跑时才要帧。反向从当前进度接着走。

Vue 路径的 `@keyframes` 编译进同一条 Motion IR。`opacity` 和 `transform` 走 compositor overlay。`color` 和 `filter` 走 Paint-class CPU。它们都不是布局条件。

`width` 和 `height` 仍是 Layout-class。`font-variation-settings` 按轴编译成 Layout-class track。每一帧都是真实的字形变体，会重新 shaping，也会重新栅格。这不是缩放。

菜单默认和触发它的控件起始边对齐。靠近窗口边缘时，由控件自己收回来。你不要算坐标。

可拖的 Tab 和列表：按下后移动超过 4px 才算拖。点击仍然只是选中。

命令面板宽 680px。打开后，输入框必须拿到焦点。
