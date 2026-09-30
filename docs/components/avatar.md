# Avatar

`avatar(size)` 创建一个圆形头像。`size` 是 `f32`，边长按逻辑像素。模板里的标签是 `<Avatar>`。

## 基本用法

内部是 `Avatar::new("").size(size)`，资源一开始是空的。

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

## 采样

采样是 Cover，裁成圆。空 `resource`、宿主清空、加载失败都走 Subtle 占位，不自绘字母。加载失败由宿主把 `resource` 清成空。这不改缺槽拒绝帧的约定。

默认只采样第 0 层。纹理带 mip 链时，用组件上的 `.sampling(ImageSampling::Mipmap)` 切到三线性。这不是字段。

## 换图

`generation` 表示换了一张采样视图，`version` 表示同一张里的内容失效。宿主纹理晚到时，推进这两个数，画面才会从占位换成实图。组件上对应 `replace_view` 和 `invalidate_content`。

`generation` 占 revision 的高 32 位，`version` 占低 32 位。和封面一样，同一帧里同一个槽的 revision 必须一致。

## 无障碍

有 `label` 时，无障碍是有名字的 Image。名字为空则是 Image，且没有 name。和 `Thumbnail` 的区别：Cover、圆形、固定边长，不是随宽高比的方框。

## 尺寸

非有限或非正的 `size` 会退回默认边长，也就是中等控件高度。圆角是边长的一半，溢出裁掉，所以总是圆的。

## 命中

默认不参与命中，不可聚焦。

要让头像接到点击，用组件上的 `pointer_events(true)`。默认为假，而且仍然不可聚焦。这不是字段。

## 属性

字段是 `resource: Arc<str>`、`generation: u64`、`version: u64`、`size: f32`、`label: Arc<str>`。没有事件，也没有 `model`。默认不参与命中，不可聚焦。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `resource` | `Arc<str>` | 资源一开始是空的。空 `resource`、宿主清空、加载失败都走 Subtle 占位 |
| `generation` | `u64` | 换了一张采样视图。占 revision 的高 32 位。组件上对应 `replace_view` |
| `version` | `u64` | 同一张里的内容失效。占 revision 的低 32 位。组件上对应 `invalidate_content` |
| `size` | `f32` | 边长，逻辑像素。也是字段，之后还可以再绑。非有限或非正会退回默认边长，也就是中等控件高度 |
| `label` | `Arc<str>` | 有名字时，无障碍是有名字的 Image。名字为空则是 Image，且没有 name |
| `.sampling` | — | 不是字段。默认只采样第 0 层。`.sampling(ImageSampling::Mipmap)` 切到三线性 |
| `pointer_events` | — | 不是字段。`pointer_events(true)` 才接到点击。默认为假，而且仍然不可聚焦 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 没有事件，也没有 `model` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
