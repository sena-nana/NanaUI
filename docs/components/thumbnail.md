# Thumbnail

`thumbnail()` 创建一个宿主纹理的封面槽。模板里的标签是 `<Thumbnail>`。它没有构造参数，内部是 `Thumbnail::new("")`。

## 基本用法

空的 `resource` 是空态，不采样。字段是 `resource: Arc<str>`、`generation: u64`、`version: u64`、`aspect: f32`、`label: Arc<str>`。没有事件，也没有 `model`。

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

模板里的 `aspect=1.5` 展开成 `1.5_f32`。手写时字段是 `f32`，字面量要带后缀。没写 `aspect` 时按 1。盒子是控件高度乘这个比。显式样式的宽高、约束和圆角优先。圆角都没写时才取 `Xs`。非有限或非正的 `aspect` 退回 1。

默认适配是 `ContentFit::Contain`。封面裁切用组件上的 `Thumbnail::fit(ContentFit::Cover)`，不是字段。空、加载、就绪、不可用共用这一套布局尺寸。

## 状态

状态是组件上的 `ThumbnailState`：`Empty`、`Loading`、`Ready`、`Unavailable`。不在字段表里。`resource` 去掉空白后非空，构造时就是 Ready，否则是 Empty。`loading()` 和 `unavailable()` 用来标另外两态。四态共用盒子，所以加载时布局不跳。

Loading 的转圈居中，边长是紧凑 `Spinner` 的两倍。

## 修订

`generation` 在宿主换掉采样视图时推进，`version` 在同一视图里像素变了时推进。晚到的纹理靠这两个数让画面从占位换成实图。`Thumbnail::replace_view` 和 `invalidate_content` 做这件事。同一帧里，同一个槽报出的 revision 必须一致。

`generation` 占 revision 的高 32 位，`version` 占低 32 位。同一帧里，同一个槽报出的 revision 必须一致，否则这一帧被拒绝，什么都不画。

默认只采样第 0 层，宿主按 `painted_extent` 准备尺寸。有 mip 链时，组件上 `.sampling(ImageSampling::Mipmap)` 改成三线性。

## 命中、角标和名字

指针默认不参与命中，命中归父行。封面不参与命中。

角标用 `Thumbnail::badge()`：右下、不命中、实底，裁在这个盒子的圆角里。角标是 `Thumbnail::badge()`，自己也关掉命中。

有 `label` 时无障碍是有名字的 Image。名字为空则没有 name。和 `Avatar` 的区别：这里默认是 Contain、方框随高度和宽高比，不是圆形固定边长。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `resource` | `Arc<str>` | 空的是空态，不采样。去掉空白后非空，构造时就是 Ready，否则是 Empty |
| `generation` | `u64` | 宿主换掉采样视图时推进。占 revision 的高 32 位 |
| `version` | `u64` | 同一视图里像素变了时推进。占低 32 位 |
| `aspect` | `f32` | 没写时按 1。模板 `aspect=1.5` 展开成 `1.5_f32`。非有限或非正退回 1。盒子是控件高度乘这个比 |
| `label` | `Arc<str>` | 有 `label` 时无障碍是有名字的 Image。名字为空则没有 name |

## 事件

没有事件，也没有 `model`。

## 插槽

没有插槽。画面来自 `resource` 指向的宿主纹理。

## 参见

[总览](index.md) · [控件](../reference/components.md)
