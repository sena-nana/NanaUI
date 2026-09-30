# ImageViewer

`ImageViewer` 是整窗的大图浮层。

它画遮罩、表面、舞台和说明行。

图片由你给。

遮罩用主题的 `EffectTokens::media_scrim`，浅色主题下也是深色，图片总是放在深色上看；表面和说明行仍随主题。

这个类型在 Cargo feature `image-viewer` 后面。

`components` 会打开它。

`ImageViewer::new(content)` 的内容是 `ImageViewerContent`：`None`、`Child(id)`、`HostTexture(槽名)` 或 `CustomRender`。

便捷构造是 `ImageViewerContent::host_texture(slot)` 和 `ImageViewerContent::child(id)`。

`.name` 和 `.metadata` 出现在舞台下面的说明行。

`.intrinsic_size(width, height)` 在两边都大于 0 时记下原图像素，用来把图放进舞台。

`.gallery(index, count)` 的 `index` 从 0 起。

多于一张时，舞台底部出现「上一张」「下一张」和位置「3 / 9」；到头的那一侧按钮禁用。

只有一张或没有 `gallery` 时不显示导航。

关闭钮由 `assemble_image_viewer` 建成 `IconButton`，可聚焦，默认名字是「关闭」，用 `.close_label` 替换。

上一张、下一张的默认名字是「上一张」「下一张」。

这些控件总排在其他子节点之后，铺满画面的 `Child` 也盖不住它们。

内容晚于控件放进来时，查看器会重新装配，把控件挪回末尾。

它是叶子复合件：视图建好时，以及每次写入之后，都会跑 `assemble_image_viewer`。

手工 `create_component` 的查看器自己调一次。

事件是 `ImageViewerEvent`：`Close`、`Outside`、`Interaction`、`Previous`、`Next`。

换图由你做：收到 `Previous` 或 `Next` 后更换内容，并把新位置写回 `gallery`。

缩放和平移是否复位也由你决定。

焦点在查看器或它的控件上时，左右方向键发出同样的请求；到头时方向键不消费。

控件表里没有 `<ImageViewer>`。

Vue 的 `nana-image-viewer` 只做投影，不建控件，关闭由宿主负责。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ImageViewer, ImageViewerContent};

view! {
    <Widget
        of={ImageViewer::new(ImageViewerContent::host_texture("shot"))
            .name("截图")
            .gallery(2, 9)
            .close_label("关闭")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ImageViewer, ImageViewerContent};

widget(
    ImageViewer::new(ImageViewerContent::host_texture("shot"))
        .name("截图")
        .gallery(2, 9)
        .close_label("关闭"),
)
```

:::

`ImageViewer::geometry` 的 `close` 给出关闭钮的位置，给你的内容留边。

Escape 走共享的浮层关闭。

打开和收起用宿主上的 `activate_overlay` 与 `dismiss_overlay`，查看器要是 `OverlayHost` 的直接子节点。

[总览](index.md) · [控件合同](../reference/components.md)
