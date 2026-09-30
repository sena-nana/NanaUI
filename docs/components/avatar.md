# Avatar

`avatar(size)` 创建一个圆形头像。`size` 是 `f32`，边长按逻辑像素。模板里的标签是 `<Avatar>`。内部是 `Avatar::new("").size(size)`，资源一开始是空的。

字段是 `resource: Arc<str>`、`generation: u64`、`version: u64`、`size: f32`、`label: Arc<str>`。没有事件，也没有 `model`。默认不参与命中，不可聚焦。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Avatar size=32 resource={photo} label="头像" />
}
```

```rust rust
use nana_ui::runtime::view::avatar;

avatar(32_f32).resource(photo).label("头像")
```

:::

模板里的 `size=32` 展开成 `32_f32`。手写时参数是 `f32`，整数字面量要带后缀。`size` 也是字段，之后还可以再绑。

采样是 Cover，裁成圆。空 `resource`、宿主清空、加载失败都走 Subtle 占位，不自绘字母。加载失败由宿主把 `resource` 清成空。这不改缺槽拒绝帧的约定。

`generation` 表示换了一张采样视图，`version` 表示同一张里的内容失效。宿主纹理晚到时，推进这两个数，画面才会从占位换成实图。组件上对应 `replace_view` 和 `invalidate_content`。

默认只采样第 0 层。纹理带 mip 链时，用组件上的 `.sampling(ImageSampling::Mipmap)` 切到三线性。这不是字段。

有 `label` 时，无障碍是有名字的 Image。名字为空则是 Image，且没有 name。和 `Thumbnail` 的区别：Cover、圆形、固定边长，不是随宽高比的方框。

非有限或非正的 `size` 会退回默认边长，也就是中等控件高度。圆角是边长的一半，溢出裁掉，所以总是圆的。

要让头像接到点击，用组件上的 `pointer_events(true)`。默认为假，而且仍然不可聚焦。这不是字段。

`generation` 占 revision 的高 32 位，`version` 占低 32 位。和封面一样，同一帧里同一个槽的 revision 必须一致。

[总览](index.md) 和 [控件合同](../reference/components.md)
