# 视图迁移补缺（三）：弹出层盖住页面、收窄的盒子放宽、媒体遮罩、句柄与键路径、虚拟列表显隐、播放条

LiliaBilibili 迁到声明式视图时发现的第三批框架缺口。说明见 [控件](components.md)、[声明式视图](reactive-view.md)、[布局](layout.md)、[文本引擎](text-engine.md)、[主题](theme.md)。

## 需要改的地方

- **`MediaTransportEvent` 多了 `MenuOpened`、`MenuClosed`。** 穷举匹配它的代码要加分支（收到时调一次 `sync_overlay_visibility`，见下）。
- **`MediaTransportBar` 多了公开字段 `show_play`。** 用结构体字面量构造它的代码要补上（用 `new()` 的不受影响）。
- **播放条的空闲收起不再写条的 `hidden`。** 条自己的 `hidden`（视图里 `.visible(..)` / `v-show`）从此只归应用；空闲策略的状态留在条里，两者任一要藏，投影出去的节点就是隐藏的。
  - 读 `bar.style.layout.hidden` 来判断「空闲时藏起来没有」的代码改读 `bar.shown()`（或 world 里投影出的节点样式）。
  - 每次 `sync_overlay_visibility` / `reveal_overlay` 之后再把 `hidden` 写回 `true` 来表达「传输不可用时整条不出现」的代码可以删掉：给条绑 `.visible(available)`（模板 `<Widget of={MediaTransportBar::new()} v-show={available}>`），或写一次 `hidden`。它不会再被空闲同步覆盖，也不会让绑定重跑时把空闲藏起的条显出来。
- **自定义主题的 `EffectTokens::media_scrim`**：画家在线性光里合成，黑色 α 留下背后 `1 - α` 的线性亮度。内置值从 0.9 改成 0.99（白底上约 `#1a1a1a`，也就是 CSS 里 0.9 的样子）；自定义主题里照 CSS 写的 0.9 在浅色页面上是中灰，要按同样的算法改。
- 应用里为绕开缺口写的代码可以删掉：
  - 给放着 `Popover` / `ActionMenu` 的面板、卡片抬 `z_index`，或把后面的内容往下压，好让弹出层的底不被盖住。
  - 在框架建的播放条菜单节点（设置 `ActionMenu`、音量 `Popover`）上听 `PopoverToggled` / `PopoverClosed` 来重新同步空闲收起：改听条上的 `MediaTransportEvent::MenuOpened` / `MenuClosed`。
  - 自己在条外藏播放钮、或在直播间没准备好时拦 `PlayPause`：用 `show_play(false)`。
  - 消息气泡这类按内容收窄的盒子，为了不「先按最窄排、放宽后卡在旧宽度」而在量到可用宽之前不设 `max-width` 的写法：上限从小变大时盒子现在会跟着变宽。
  - 同一个元素上要同时拿 `Entity` 和 `NodeRef` 时两次挂载或额外包一层：`.entity_ref(e).node_ref(n)` 两个都会填上。
  - `dynamic` / `when` 切换后、`keep_alive` 切回后、`each` 挂载后加进来的行里，按路径找不到节点时退回按树结构或 id 找的代码。
  - 为了显隐一个 `each_virtual` 列表而在外面包一层可见性容器：直接 `.visible(..)`（模板 `<Virtual v-show={..}>`）。

## 行为变化

- **打开的 `Popover` / `ActionMenu` 的卡面盖住页面后面的内容**，和 `HoverCard` 一样：卡面排在根层叠上下文里弹出内容的那一层（`MENU_OVERLAY_Z_INDEX`），后出现、`z_index` 更高的兄弟（说明框、评论卡、相关推荐、播放条）不再透过卡面显出来，`Top` / `Bottom` 都一样。卡面也在这一层接指针：落在条目之间、内边距上的按下留在菜单里，不再穿到底下的页面，也不再被当成「点外面」把菜单关掉，更不会当成按触发器把它开合；`HoverCard` 卡面空白处的按下同样不再激活它的触发器。组件库里暗色的 popover 画面因此变了：原来透过卡面看得见的「日历热力图」标题和热力图格子现在被卡面盖住。
- **折过行的文字按它不折行的宽度量内容宽**（不超过可用宽），所以 `fit-content` / `max-content` / 可收缩 flex 项这类按内容收窄的盒子，在上限放宽之后会变宽，文字再按新宽度重折；结果和一开始就按宽的上限排的一样。以前它们停在上次折行的宽度。组件库里富文本、工作区页标题栏上被挤成两行的「富文本」页签现在是一行。
- **图片查看器的遮罩在浅色主题下是近黑**（约 `#171717`，原来是中灰 `#5a5a5a`）；深色主题下几乎看不出差别。
- **`dynamic` / `when` 切换后新建的分支、`keep_alive` 重新显示的分支、`each` 挂载后加进来的（写了 key 的）行，都登记在容器下**，`resolve_assembly_path` 按路径找得到，和挂载时建的一样；在 `each_virtual` 的行里也一样。
- **一个元素上的 `.entity_ref(..)` 和 `.node_ref(..)` 不再互相覆盖**，每个都拿到这个节点。
- **播放条设置菜单被藏起时顺带关上**，走和点外面一样的路径，所以也发 `MenuClosed`。

## 新增

- `nana_ui_runtime::MENU_OVERLAY_Z_INDEX`：打开的弹出层（内容与卡面）所在的层级。
- 结构块（`each`、`when`、`dynamic`、`each_virtual`、teleport 锚点）容器上的 `.visible(..)`；模板 `<Virtual v-show={..}>`。
- `MediaTransportBar::shown()`、`MediaTransportBar::show_play` / `.show_play(bool)`（标记属性 `show-play`）、`MediaTransportEvent::MenuOpened` / `MenuClosed`。
