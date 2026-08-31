# 视觉

NanaUI 的默认外观是给桌面产品用的：深色和浅色、紧凑、弱分隔。强调色只出现在主操作和「值本身就是强调」的控件上（主按钮、打开的开关、有值的滑杆）。

深色 / 浅色只换颜色，不换尺寸，也不换状态怎么分层。由 `RuntimeProgram::theme_mode` 返回 `ThemeMode::Dark` 或 `Light`。

## 尺寸

单行控件三档高度，经 `ControlSize` 生效：

| 档位 | 高度 | 用在 |
| --- | ---: | --- |
| 小 | 28px | 导航、菜单、列表、紧凑图标 |
| 中 | 32px | 表单、按钮、选择、常规操作 |
| 大 | 36px | 需要强调、由你显式选的场合 |

标准正文 13px（`UI_BASE_TEXT_SIZE`）。标题栏 36px，窗口按钮 28px，侧栏导航行 28px。多行内容按内容长，里面的单行操作仍走这三档。

## 颜色

面板是一层表面，输入和未激活项更弱一层，悬停、按下、选中再往上走。文字分主、次、更弱。

输入框聚焦时加一圈中性描边，不改底色。错误优先用危险色。不要用任意业务色去改框架 token；主题色走 `ThemeMode` 和 `AppearanceSettings`。

卡片默认没有描边。需要抬起来用阴影。选中卡片用柔和选中底，不用强调色包边。Vue CSS 的单层 `box-shadow`（outset 与 inset）/ `text-shadow`（仅 outset）已映射到绘制；inset 走内阴影 SDF，有子节点时 dest 合成组 overlay，不是把 outset 画进盒子里冒充。

颜色来自共享的 `ThemeTokens` / `SemanticPalette`（`nana_ui::theme`），不是每个控件一份样式表。

## 字体

`bundled-fonts` 开启时，宿主注册 Noto Sans SC 四档字重，并设为 sans-serif 默认。这是界面字体。未启用该 feature 时回落系统字体，快照和设计对照不能拿系统字体当真。

应用也可以关掉捆绑字体、用 `register_host_font_bytes` / `register_host_font_file` 把自有字体载入同一套 FontSystem（与捆绑 Noto 并列）。未注册仍回落捆绑或系统字体。

字距走 cosmic-text shaping（tracking，不是事后平移）。`font-feature-settings` / `font-kerning` 进 shaper；`font-variation-settings` 只兑现 `wght` / `wdth`。`word-break: break-all|break-word` 与 `line-break: anywhere` 改 wrap；`keep-all` / `strict` / `loose` 与竖排不支持，声明被跳过。`@font-face` 在 stylesheet 解析时只收集规则；`url(...)` 的加载与 FontSystem 注册发生在 `inject_stylesheet`（宿主适配器，`scene-view`），不在 CSS parse。CSS `font-family`（及 weight/style）会映射到刚载入的 face，坏 src 丢掉该 face，不用系统字体顶替。

## 圆角

`AppearanceSettings` 暴露四级圆角：微型 / 控件 / 卡片 / 页面，默认 2 / 6 / 10 / 14。`standard_radius` 仍是 md（10）的别名，只改这一档不会重算另外三档。遗留 JSON 若只有 `standard_radius`，仍按旧规则一次推导 ±4 / ±8。

主区域贴着展开的侧栏时，挨着侧栏的那两个角收成直角，另一侧保持页面圆角。这由工作区表面统一画，不靠页面自己设相同圆角。

## 运动与浮层

Runtime 侧栏折叠 240ms，分组展开 160ms，ease-out。只有动画在跑时才要帧；反向从当前进度接着走。Vue 路径的 `@keyframes` 做成绘制动画（不是布局条件）。

菜单默认和触发它的控件起始边对齐，靠近窗口边缘时由控件收回来，应用不要算坐标。

可拖的 Tab 和列表：按下后移动超过 4px 才算拖；点击仍然只是选中。命令面板宽 680px，打开后输入框必须拿到焦点。
