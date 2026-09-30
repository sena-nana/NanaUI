# MediaTransportBar

`MediaTransportBar` 是媒体播放条。

框架提供播放、点播进度或直播进度、音量弹出、设置菜单和全屏。

场景自己的控件放进槽里。

`MediaTransportBar::new()` 默认常规密度、贴在父级底边、音量 100、可以拖动进度。

`.density` 是 `MediaTransportDensity`：`Regular` 把时间读数放在进度上面，并可以开第二行；`Compact` 收成一行，设置和全屏默认隐藏；`Stacked` 的按钮和紧凑档一样，读数和进度单独占上面一整行。

`.placement` 是 `MediaTransportPlacement::Overlay` 或 `Inline`。

Overlay 沿父级底边绝对定位，只有铬接命中；Inline 参与父级排版，`max_width` 不用。

`.show_play(false)` 藏起播放钮，藏起的钮不占位、不能聚焦。

`.show_settings` 和 `.show_fullscreen` 为 `None` 时跟随密度。

`.playing`、`.position`、`.duration`、`.volume`、`.live`、`.muted`、`.seekable` 是播放态。

视图槽是 `.leading`、`.trailing`、`.secondary`、`.settings`。

`secondary` 没有可见子节点时第二行收起。

设置菜单里的剧场、独立窗口、停止播放放进 `.settings`。

`assemble_media_transport_bar` 是叶子装配：视图建好时，以及每次写入之后，都会重建槽并接线。

`update_component` 写完字段已经同步过播放态、进度和时间读数，播放 tick 不必再调一次 sync。

事件是 `MediaTransportEvent`：`PlayPause`、`Seek(秒)`、`SeekStarted`、`SeekEnded`、`Volume`（`0..=100`，含拖动预览）、`Fullscreen`、`MenuOpened`、`MenuClosed`。

`Seek` 只在抬起或键盘步进时提交。

菜单开合时调用 `AppContext::sync_overlay_visibility`，打开的菜单把条留住，关上后空闲计时从这一下重新开始。

条自己的 `hidden` 只表示传输不可用，空闲收起不写这个字段。

控件表里没有 `<MediaTransportBar>`。

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

时间读数的格式是 `media_clock`：`m:ss` 或 `h:mm:ss`。

直播用进度条上的计量，点播用可拖的范围；`seekable` 为假时范围还在，只是禁用。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
