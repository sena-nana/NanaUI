# MediaTransportBar

`MediaTransportBar` 是媒体播放条。框架提供播放、点播进度或直播进度、音量弹出、设置菜单和全屏。场景自己的控件放进槽里。

控件表里没有 `<MediaTransportBar>`。

## 基本用法

`MediaTransportBar::new()` 默认常规密度、贴在父级底边、音量 100、可以拖动进度。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{MediaTransportBar, MediaTransportDensity, MediaTransportPlacement};

view! {
    <Widget
        of={MediaTransportBar::new()
            .density(MediaTransportDensity::Regular)
            .placement(MediaTransportPlacement::Overlay)
            .show_play(true)}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{MediaTransportBar, MediaTransportDensity, MediaTransportPlacement};

widget(
    MediaTransportBar::new()
        .density(MediaTransportDensity::Regular)
        .placement(MediaTransportPlacement::Overlay)
        .show_play(true),
)
```

:::

## 密度

`.density` 是 `MediaTransportDensity`：`Regular` 把合并时间读数放在进度上面，并可以开第二行；`Compact` 收成一行，设置和全屏默认隐藏；`Stacked` 的按钮和紧凑档一样，当前时间、进度和总时长按顺序排在上面一整行，时间读数不会挤压进度范围；`Mini` 是小窗播放器。

### Mini

`MediaTransportBar::new().mini()`（`Mini` 加 `Inline`）给小窗用，布局全归条自己：

- 进度是条顶边上一条 2px 的细轨（`RangeField::rail`），外面套一条以它为中心、16px 高的透明命中带。命中带伸到条上方的内容（画面）上，在那里按下、拖动就能定位；伸进按钮行的部分让给按钮。直播时细轨换成直播进度计量。
- 下面一行依次是播放、静音钮、没有圆点的音量细轨、`leading` 槽，然后是合并的时间读数和 `trailing` 槽。没有音量弹出层；设置和全屏默认隐藏，可以用 `show_settings` / `show_fullscreen` 打开。
- 静音钮按下发 `MediaTransportEvent::Mute(想要的静音状态)`，也就是 `!muted`。图标在 `muted` 或音量为 0 时换成静音图标，名字说的是按下会做的事（`mute_label` / `unmute_label`）。
- 画面上的播放覆盖层仍是应用自己的；不想要条上的播放钮就 `show_play(false)`。

```rust
widget(MediaTransportBar::new().mini().show_play(false).icons(icons))
    .leading(extra_buttons)   // 音量细轨之后
    .trailing(window_actions) // 时间读数之后
    .bind(move |bar| playback.with(|p| p.paint(bar)))
    .on(move |event: &MediaTransportEvent| match event {
        MediaTransportEvent::Mute(muted) => set_muted(*muted),
        _ => {}
    });
```

应用不要再移动条内部的节点、改它们的尺寸或透明度：`density` 在 `Mini` 和其他档之间切换时，条会把进度、时间和音量放回各自的位置。

## 位置

`.placement` 是 `MediaTransportPlacement::Overlay` 或 `Inline`。

Overlay 沿父级底边绝对定位，只有铬接命中；Inline 参与父级排版，`max_width` 不用。

## 显隐

`.show_play(false)` 藏起播放钮，藏起的钮不占位、不能聚焦。

`.show_settings` 和 `.show_fullscreen` 为 `None` 时跟随密度。

## 播放态

`.playing`、`.position`、`.duration`、`.volume`、`.live`、`.muted`、`.seekable` 是播放态。

`update_component` 写完字段已经同步过播放态、进度和时间读数，播放 tick 不必再调一次 sync。

条自己的 `hidden` 只表示传输不可用，空闲收起不写这个字段。

## 装配

`assemble_media_transport_bar` 是叶子装配：视图建好时，以及每次写入之后，都会重建槽并接线。

`secondary` 没有可见子节点时第二行收起。

设置菜单里的剧场、独立窗口、停止播放放进 `.settings`。

## 菜单与空闲

菜单开合时调用 `AppContext::sync_overlay_visibility`，打开的菜单把条留住，关上后空闲计时从这一下重新开始。

### 自动收起

`.auto_hide(true)` 让运行时自己驱动空闲收起，宿主不用转发输入、不用排定时器：

- 播放中（`playing` 且没有 `disabled`）最后一次活动之后 `OVERLAY_IDLE`（3 秒）收起。活动是指针在条的父节点（画面）上移动、按下或抬起，或者文档里任意一次按键。
- 指针离开窗口时立刻收起，暂停时也一样，直到指针回来。
- 打开的菜单、拖动（定位、音量）和条里的键盘焦点把条留住；点按留下的焦点不算，否则点过一次按钮条就再也不会收起。
- 每次显隐翻转，条发 `OverlayVisibilityChanged { visible }`。画面上的其他外壳（标题栏）跟着它走。

这样的条不要再调 `sync_overlay_visibility` / `reveal_overlay`：那两个用宿主的时钟，和运行时的时钟混在一起没有意义。手动驱动的条也会发 `OverlayVisibilityChanged`；`conceal_overlay` 是手动驱动时的「指针离开」。

只放视频的窗口这样写：

```rust
let chrome = signal(true);
view! {
    <Widget of={Stack::fill_column(0.0)}>
        <Widget of={GpuTextureView::new("").contain()} />
        <Widget
            of={MediaTransportBar::new().compact_overlay().auto_hide(true)}
            on:OverlayVisibilityChanged={move |e: &OverlayVisibilityChanged| {
                chrome.set_if_changed(e.visible);
            }}
        />
        <Widget of={AppTitleBar::new("").over_media(true)} v-show={chrome.get() && !fullscreen.get()} />
    </Widget>
}
```

条放在画面的同一个父节点下，指针在整扇窗口上都算「在画面上」。标题栏用 [`over_media(true)`](app-title-bar.md#叠在媒体上)，浅色主题下按钮和标题在画面上也看得清。窗口的宽高比交给 [`content_aspect_ratio`](../reference/window.md#内容宽高比)。

## 时间读数

时间读数的格式是 `media_clock`：`m:ss` 或 `h:mm:ss`。

`Stacked` 将当前时间和总时长分别放在进度范围两端；拖动时当前时间显示预览值。`Regular` 与 `Compact` 保留合并的 `当前 / 总时长` 读数。直播没有可用的端点总时长，因此仍只显示直播进度计量。

直播用进度条上的计量，点播用可拖的范围；`seekable` 为假时范围还在，只是禁用。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.density` | `MediaTransportDensity` | `Regular` 合并读数在进度上面，并可以开第二行；`Compact` 收成一行，设置和全屏默认隐藏；`Stacked` 当前时间、进度和总时长在上面一整行；`Mini` 顶边细轨加一行控件，见上文。`new()` 默认常规密度 |
| `.mini()` | — | `Mini` 加 `Inline` |
| `.placement` | `MediaTransportPlacement` | `Overlay` 或 `Inline`。默认贴在父级底边 |
| `.show_play` | — | `show_play(false)` 藏起播放钮，藏起的钮不占位、不能聚焦 |
| `.show_settings` | — | 为 `None` 时跟随密度 |
| `.show_fullscreen` | — | 为 `None` 时跟随密度 |
| `.playing` | — | 播放态 |
| `.position` | — | 播放态 |
| `.duration` | — | 播放态 |
| `.volume` | — | 播放态。默认音量 100 |
| `.live` | — | 播放态 |
| `.muted` | — | 播放态 |
| `.seekable` | — | 播放态。为假时范围还在，只是禁用 |
| `.hidden` | — | 只表示传输不可用，空闲收起不写这个字段 |

## 事件

事件是 `MediaTransportEvent`：`PlayPause`、`Seek(秒)`、`SeekStarted`、`SeekEnded`、`Volume`（`0..=100`，含拖动预览）、`Fullscreen`、`Mute(bool)`（只有 `Mini` 的静音钮发）、`MenuOpened`、`MenuClosed`。

`Seek` 只在抬起或键盘步进时提交。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `MediaTransportEvent::PlayPause` | — | `MediaTransportEvent` |
| `MediaTransportEvent::Seek` | 秒 | 只在抬起或键盘步进时提交 |
| `MediaTransportEvent::SeekStarted` | — | `MediaTransportEvent` |
| `MediaTransportEvent::SeekEnded` | — | `MediaTransportEvent` |
| `MediaTransportEvent::Volume` | `0..=100` | 含拖动预览 |
| `MediaTransportEvent::Fullscreen` | — | `MediaTransportEvent` |
| `MediaTransportEvent::Mute` | `bool` | `Mini` 的静音钮：想要的静音状态，即 `!muted` |
| `MediaTransportEvent::MenuOpened` | — | 菜单打开时调用 `AppContext::sync_overlay_visibility`，打开的菜单把条留住 |
| `MediaTransportEvent::MenuClosed` | — | 关上后空闲计时从这一下重新开始 |

## 插槽

视图槽是 `.leading`、`.trailing`、`.secondary`、`.settings`。

| 插槽 | 说明 |
| --- | --- |
| `.leading` | 视图槽 |
| `.trailing` | 视图槽 |
| `.secondary` | 没有可见子节点时第二行收起 |
| `.settings` | 设置菜单里的剧场、独立窗口、停止播放 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
