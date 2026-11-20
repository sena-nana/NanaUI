# ColorField

`ColorField` 是色块加十六进制。它是叶子复合件：你写属性时，`assemble_color_field` 自己建出色块、hex 输入和 HSV 选择器，不用再记一次装配。选择器是现有控件：饱和度/明度用 `XYPad`，色相用 `RangeField`，包在它自己的 `Popover` 里，挂在色块下方。

## 外框

整个字段只有一层框，做法同 Ant Design 的 ColorPicker 和 Mantine 的 ColorInput。

- 外框用文本框的底色和边框，高度正好是一个控件高（`.size` 决定）。
- 色块在框内，上、下、左三边各留 3px。边长是控件高减去边框和两边留白，所以 Small 是 20，Medium 是 24。圆角是外框圆角（`RadiusTier::Sm`）按色块边长与外框高度的比例缩小，所以色块是外框的等比缩小版，换主题圆角时跟着变。色块的触发器用 `Popover::compact_trigger`，大小正好等于色块，没有按钮的内边距。
- hex 输入没有自己的底色、边框和圆角，竖向填满框内，右侧保留文本框的内边距。
- 焦点在字段里（hex 或色块）时，外框画焦点边框，跟 `:focus-within` 一样。`.invalid` 时外框画 Danger 边框，`.disabled` 时外框换成 Subtle 底。

## 选择器的样子

选择器画出它取值的颜色。饱和度/明度是一块画面垫（`XYPadSurface::Picture`，高度是大号垫子的两倍）：底色是当前色相的纯色，上面一层白色从左往右淡出（饱和度），再上面一层黑色从上往下渐入（明度），就是标准的 HSV 方块；色相变了，方块跟着换底色。色相是一条不带读数的光谱条（`RangeField::track_gradient(hue_spectrum())`，0 到 360 度：红、黄、绿、青、蓝、品红、回到红）。两处都用对比色圆环标出当前值，没有十字轴，也没有强调色填充。色相条仍是读屏 `Slider`，可以用键盘步进。当宿主支持屏幕取色且未关闭 `.eyedropper` 时，色相条左侧摆一颗吸管 `IconButton`（`[ 吸管 | 色相条 ]`）；点一下会立即关上弹出层并启动平台原生屏幕取色器，把拾取到的 sRGB 颜色写回字段、保留原本的透明度（按 Esc 或右键取消时不改变颜色）。

### 屏幕取色可用性

`ColorField` 颜色吸管（eyedropper）的可用性取决于宿主平台与会话的屏幕读取权限，宿主通过 `color_sampler_support()` 查询：当返回 `ColorSamplerSupport::Native` 时，吸管可见并可调用；返回 `ColorSamplerSupport::Unsupported`（例如无 Portal v2+ 的 Wayland 或不支持屏幕采样的平台）时自动隐藏；若运行中因权限或会话原因失败，通过 `ColorSamplerResult::Failed` 返回错误，同时保持字段当前颜色值不变。各平台底层行为与限制见 [窗口管理与原生边界](../reference/window.md)。

控件表里没有 `<ColorField>`。

## 基本用法

宿主拿不到窗口句柄。

`ColorField::new(value)` 的 `value` 是 `0..=1` 的 RGBA。

构造时会收成合法颜色，并算出 `hue`、`sat`、`val`。

`.size`、`.disabled`、`.invalid` 跟字段控件一样。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::ColorField;

view! {
    <Widget of={ColorField::new([0.29, 0.57, 0.85, 1.0]).label("强调色")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::ColorField;

widget(ColorField::new([0.29, 0.57, 0.85, 1.0]).label("强调色"))
```

:::

## 颜色值

RGBA 四个分量都在 `0..=1`。

hex 框的文本变化会写回 `value`。

同一颜色再次写入时，打开状态和 HSV 光标留在拖动中的位置；颜色变了才按新的 RGB 重算。

同一颜色再写一次时，HSV 光标留在拖动中的位置；颜色变了才按新的 RGB 重算。

## 开合

`.opened` 在禁用时强制关上。

开合写在字段自己的 `opened` 上。

不要去改装配出来的那个 `Popover`，下一次装配会把它盖掉。

色块点一下翻转 `opened`。

禁用时点色块没有反应，选择器也关着。

## 装配

`create_component` 建出来的字段同样会在写入后跑 `assemble_color_field`。

叶子复合件的装配挂在写入上：你改 `value`、`label` 或 `opened`，`assemble_color_field` 会重建色块、hex 和选择器。

这和壳层不一样，壳层要等槽位放好再显式 `assemble_*`。

子节点身份由运行时持有。

## 无障碍

默认无障碍名是「颜色」，用 `.label` 换成你的文案。

默认无障碍名是「颜色」。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | — | `ColorField::new(value)` 的 `value` 是 `0..=1` 的 RGBA。构造时会收成合法颜色，并算出 `hue`、`sat`、`val`。hex 框的文本变化会写回 `value` |
| `.label` | — | 默认无障碍名是「颜色」，用 `.label` 换成你的文案 |
| `.size` | — | 跟字段控件一样 |
| `.disabled` | — | 跟字段控件一样。禁用时 `.opened` 强制关上，点色块没有反应，选择器也关着 |
| `.invalid` | — | 跟字段控件一样 |
| `.opened` | — | 开合写在字段自己的 `opened` 上。色块点一下翻转它。禁用时强制关上 |
| `.eyedropper` | `bool` | 默认 `true`；宿主支持屏幕取色时在色相条左侧显示吸管按钮，在不支持的平台（如 iOS 或无 Portal 的 Wayland）自动隐藏 |

## 事件

提交发 `ColorChanged { value }`，拖动中发 `ColorInput { value }`；在 hex 框按回车会解析提交有效色或把无效草稿恢复为规范 hex，并发出 `ColorSubmitted { value }`（失焦同样把无法解析的 hex 草稿恢复为当前颜色的规范 hex）。

提交读 `ColorChanged`，确认动作（`on_submit`）读 `ColorSubmitted`，拖动中的预览读 `ColorInput`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `ColorChanged` | `{ value }` | 提交发 `ColorChanged { value }` |
| `ColorSubmitted` | `{ value }` | 在 hex 框按回车确认后发出（有效 hex 先同步并发 `ColorChanged`，无效草稿回退到规范 hex） |
| `ColorInput` | `{ value }` | 拖动中发 `ColorInput { value }`。拖动中的预览读 `ColorInput` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 子节点身份由运行时持有。不要去改装配出来的那个 `Popover`，下一次装配会把它盖掉 |

## 参见

[总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
