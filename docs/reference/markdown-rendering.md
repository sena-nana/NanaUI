# Markdown、图片查看器与富内容绘制

本文说明 `NativeMarkdown`、`ImageViewer` 以及文本输入长度/尺寸调整在 Runtime 与 Scene 之间的协作合同。

## NativeMarkdown：排版、公式、图表与图片

`NativeMarkdown` 在 `nana-ui-runtime` 内完成解析与排版。`UiWorld::markdown_layout` 直接使用当前宿主的文本整形器（text shaper）与节点当前的内容盒尺寸，因此文本选区、指针命中激活与 Scene 绘制消费的是同一份几何结果。Scene 将解析后的内容展开为普通的文本、Quad、矢量路径与 SVG 图元并合入现有场景树——既不创建独立的 Markdown 窗口或渲染器，也不申请额外的 GPU 设备。

- **行内样式与装饰**：`strong`、`emphasis`、`strike`（删除线）、链接下划线以及行内 `code` 表面均直接生成对应的绘制命令。
- **数学公式（TeX）**：行内公式在排版阶段占据其渲染后的实际宽度，独立公式块占据其实际高度，使用 RaTeX 生成的 SVG 字形轮廓。
- **Mermaid 图表**：由原生图表引擎 `nana-ui-diagram` 解析与布局，按当前主题录成矢量路径与普通文字，随明暗主题自动换色，任意缩放比都清晰；图表占据其实际高度，比栏宽时等比缩小。每个围栏在一次排版里只布局一次，解析与布局结果按源码和测宽身份缓存，滚动与选区不触发重排。源文本仍保留供框选与复制，`accTitle` / `accDescr` 进入无障碍名称与描述。详见 [原生图表引擎](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagram-engine.md)。
- **失败回退与体积上限**：语法无效的公式回退显示源文本；无法识别或语法错误的 Mermaid 图表显示为 `mermaid` 代码块。数学公式源码超过 `16,384` 字节时在构造缓存键之前被直接拒绝；Mermaid 的上限见 [原生图表引擎](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagram-engine.md#失败与上限)。
- **异步图片解析（`resolve_image`）**：图片的异步拉取与资源缓存归应用所有。下载或解码完成后，应用调用 `resolve_image(original_source, resolved_url, width, height)`（`width` 与 `height` 必须是非零固有尺寸），文档中该图片的所有出现位置（包括表格单元格内以及空 `alt` 文本的图片）都会同步更新。绘制使用解析后的 `resolved_url`，而 `RichTextEvent::ImageActivated` 事件仍保留原始 `original_source` 与 `alt` 文本；共享同一 URL 的普通超链接仍触发 `LinkActivated`。重复提交相同的图片解析结果是安全的空操作。

## ImageViewer：固有尺寸与舞台缩放

`ImageViewer::intrinsic_size(width, height)` 根据图片的固有像素尺寸决定初始呈现：小于舞台可用区域的图片保持自然尺寸居中显示，大图则按宽高比缩放收进舞台（stage）内。

用户的缩放与平移操作作用在适配后的尺寸之上。宿主纹理绘制在背景遮罩之上、控制栏之下，并严格裁剪在舞台矩形内。指针交互复用 Runtime 布局坐标换算与指针捕获生命周期，按 `Escape` 键走统一的浮层关闭路径。

## TextInput 字数上限与 TextArea 纵向拖拽缩放

- **`TextInput::max_length`**：按 **UTF-16 码元（code units）** 计数。用户输入、剪贴板粘贴、IME 组字提交以及编程式替换走同一条准入路径：超出上限的编辑会被整体拒绝，不改文字、不改选区、也不污染撤销历史。若初始传入的字符串已经超过 `max_length`，控件不会静默截断它——用户仍可移动光标和删除字符，但在长度降回上限以下之前不能继续增长。
- **`TextArea::resize_vertical(true)`**：允许用户拖拽文本框底边调整高度，并在后续声明式视图重投影时保留用户拖出的高度（同时遵守显式的像素高度约束，且不会误发文本变更事件）。指针取消（cancel）、丢失捕获或节点停放（park）会自动结束本次拖拽。注意：仅当 `TextArea` 设置了显式像素高度且未附带 `min-height` / `max-height` 约束时才支持 `content-box` 纵向缩放；不满足条件时不显示拖拽手柄。

## 验证命令

涉及 Markdown 排版、图片查看器图层顺序及输入框尺寸/长度约束的改动，可通过以下聚焦测试与无头探针验证：

```bash
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-runtime --all-features --lib markdown
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-diagram
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-scene --features rich-text --lib markdown
CARGO_BUILD_JOBS=2 cargo test -p nana-ui --features components --test ui_contracts -- text_input_max_length text_area_resize scroll_workspace_surface
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-scene --features components --test scene_contracts image_viewer_content_order
CARGO_BUILD_JOBS=2 cargo run -p nana-ui-devtools --features runtime-agent,nana-ui/components --example restored-content-probe -- target/restored-content-probe
CARGO_BUILD_JOBS=2 cargo run -p nana-ui-devtools --features runtime-agent,nana-ui/components --example diagram-probe -- target/diagram-probe
```

`restored-content-probe` 会通过共享 Scene 路径渲染真实的 SVG、数学公式、Markdown 图片与宿主纹理（覆盖浅色/深色主题及 1×/2× 缩放），并通过 `RuntimeAgentSession` 注入指针拖拽验证高度变化与前后帧像素。`diagram-probe` 把每个 Mermaid 样例按同样的主题与缩放矩阵渲染成 PNG。

## 参见

- [原生图表引擎](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagram-engine.md) · [NativeMarkdown 组件](https://github.com/sena-nana/NanaUI/blob/main/docs/components/native-markdown.md) · [ImageViewer 组件](https://github.com/sena-nana/NanaUI/blob/main/docs/components/image-viewer.md) · [TextInput 组件](https://github.com/sena-nana/NanaUI/blob/main/docs/components/text-input.md) · [TextArea 组件](https://github.com/sena-nana/NanaUI/blob/main/docs/components/text-area.md)
