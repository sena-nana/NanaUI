# Thumbnail

`thumbnail()` 创建一个宿主纹理的封面槽。模板里的标签是 `<Thumbnail>`。它没有构造参数，内部是 `Thumbnail::new("")`。空的 `resource` 是空态，不采样。

字段是 `resource: Arc<str>`、`generation: u64`、`version: u64`、`aspect: f32`、`label: Arc<str>`。没有事件，也没有 `model`。指针默认不参与命中，命中归父行。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Thumbnail resource={cover} aspect=1.5 label="封面" />
}
```

```rust rust
use nana_ui::runtime::view::thumbnail;

thumbnail().resource(cover).aspect(1.5_f32).label("封面")
```

:::

模板里的 `aspect=1.5` 展开成 `1.5_f32`。手写时字段是 `f32`，字面量要带后缀。没写 `aspect` 时按 1。盒子是控件高度乘这个比。显式样式的宽高、约束和圆角优先。圆角都没写时才取 `Xs`。

默认适配是 `ContentFit::Contain`。封面裁切用组件上的 `Thumbnail::fit(ContentFit::Cover)`，不是字段。空、加载、就绪、不可用共用这一套布局尺寸。Loading 的转圈居中，边长是紧凑 `Spinner` 的两倍。

`generation` 在宿主换掉采样视图时推进，`version` 在同一视图里像素变了时推进。晚到的纹理靠这两个数让画面从占位换成实图。`Thumbnail::replace_view` 和 `invalidate_content` 做这件事。同一帧里，同一个槽报出的 revision 必须一致。

默认只采样第 0 层，宿主按 `painted_extent` 准备尺寸。有 mip 链时，组件上 `.sampling(ImageSampling::Mipmap)` 改成三线性。角标用 `Thumbnail::badge()`：右下、不命中、实底，裁在这个盒子的圆角里。

有 `label` 时无障碍是有名字的 Image。名字为空则没有 name。和 `Avatar` 的区别：这里默认是 Contain、方框随高度和宽高比，不是圆形固定边长。

状态是组件上的 `ThumbnailState`：`Empty`、`Loading`、`Ready`、`Unavailable`。不在字段表里。`resource` 去掉空白后非空，构造时就是 Ready，否则是 Empty。`loading()` 和 `unavailable()` 用来标另外两态。四态共用盒子，所以加载时布局不跳。

`generation` 占 revision 的高 32 位，`version` 占低 32 位。同一帧里，同一个槽报出的 revision 必须一致，否则这一帧被拒绝，什么都不画。

非有限或非正的 `aspect` 退回 1。封面不参与命中。角标是 `Thumbnail::badge()`，自己也关掉命中。

[总览](index.md) 和 [控件合同](../reference/components.md)
