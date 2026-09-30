# QrCode

`QrCode` 把一段数据画成扫描用的二维码。

它不可聚焦，无障碍角色是图像。

默认名字是 “QR code”，空标签会回到这句。

`QrCode::encode(data, size)` 用载荷编码，深色模块为真，再交给绘制。

`QrCode::from_modules(modules, width, size)` 接收已经排好的方阵。

矩阵为空、边长为 0，或长度不是 `width * width` 时，返回 `QrCodeError::InvalidModules`。

尺寸不是有限数时返回 `NonFiniteSize`。

编码失败返回 `EncodeFailed`。

安静区每边 4 个模块，在绘制时加上，不写进矩阵。

`DEFAULT_SIZE` 是 224，`MIN_SIZE` 是 64，更小或非有限的尺寸会抬到 64。

`.label` 换成你的可访问名。

`.size` 再改边长。

模块像素和原点由 `module_geometry` 按布局盒计算：取宽高里较短的一边，除以模块数加 8，向下取整，至少 1，再在盒子里居中。

控件表里没有 `<QrCode>`。

二维码本身不接指针。

卡片空白也要归这张卡时，外层用 `Stack::column(...).hittable()`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::QrCode;

let code = QrCode::encode(b"nana://join", QrCode::DEFAULT_SIZE)?.label("加入");
view! {
    <Widget of={code} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::QrCode;

let code = QrCode::encode(b"nana://join", QrCode::DEFAULT_SIZE)?.label("加入");
widget(code)
```

:::

`encode` 的第一个参数是字节。

字符串先变成字节再传入。

编码结果是 `Result`，失败时用你自己的说明代替这块图。

[总览](index.md) · [控件合同](../reference/components.md)
