# nana-text

[English](README.md) · 简体中文

NanaUI 的文本引擎，也是框架里唯一测量和排版文字的地方。

它负责字体注册、匹配和缺字回退，用 OpenType 整形，处理双向文本、断行、省略号和竖排，最后产出不可变的 `TextLayout`。界面树里的文本节点通过 `TextLayoutStore` 保留布局，判断是否过期只比较 `TextRevision` 和 `FontGeneration` 两个整数。光标、选区、命中测试和 IME 组合都建立在 `TextLayout` 之上。

排版用到的枚举（如 `LineBreakSpec`、`WritingModeSpec`）借自 [`nana-ui-core`](../nana-ui-core)。图集上传和绘制在 [`nana-ui`](../nana-ui)，本 crate 只提供字形落点和可选的 CPU 光栅化。

## 示例

```rust
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    NativeTextEngine, TextConstraints, TextEngine, TextKind, TextSource, TextStyle,
    TextWorkCounters,
};
use nana_ui_core::TextWrapBreak;

let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
fonts
    .register_file(
        "crates/nana-ui-core/assets/fonts/NotoSansSC-Variable.ttf",
        &FaceDescriptor::default(),
    )
    .unwrap();
let mut engine = NativeTextEngine::new(fonts);

let layout = engine.layout(
    TextKind::Paragraph,
    &TextSource::new("文本由 nana-text 测量和排版"),
    &TextStyle { font_size_px: 16.0, ..TextStyle::default() },
    &TextConstraints {
        max_width_px: Some(120.0),
        wrap: Some(TextWrapBreak::Word),
        ..TextConstraints::default()
    },
    &mut TextWorkCounters::default(),
);
```

## Features

- `raster`：在 CPU 上把字形光栅化成位图（`nana_text::raster`）。`nana-ui` 的 `gpu` feature 会打开它，自己合成文字的宿主也可以单独使用。
- `svg`：打开 `raster` 并启用 OpenType `SVG ` 表字形的 CPU 光栅化（经由 `nana-svg-raster`，不在默认特性中；`nana-ui` 的 `svg` feature 会打开它）。

## 测试

```bash
cargo test -p nana-text --features raster
```

测试只使用仓库里签入的字体。其余命令见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [文本引擎（学习）](../../docs/learn/text-engine.md) 和 [文本引擎（参考）](../../docs/reference/text-engine.md)。
