# NativeMarkdown drawing and image contract

`NativeMarkdown` 在 Runtime 里解析，也在 Runtime 里排版。

`UiWorld::markdown_layout` 用当前的 text shaper，也用当前的内容盒。选区、指针激活和 Scene 绘制都消费同一份几何。

Scene 把普通文字、quad 和 SVG 图元发进已有的树。你不会得到单独的 Markdown 窗口，也不会得到单独的 renderer，更不会得到产品自己的 GPU 设备。

行内的 strong、emphasis、strike、链接下划线和 code 表面会进入绘制命令。

行内公式先占住它渲染出来的宽度。独立公式和 Mermaid 先占住渲染出来的高度。

TeX 用 RaTeX 的 SVG 字形轮廓。Mermaid 的 SVG 文字走现有的捆绑 UI 字体，经普通的 SVG 栅格路径。源文本仍然可以选，也可以复制。

公式无效时退回源文本。图无效时仍然是代码文本。

数学源超过 16,384 字节，或 Mermaid 源超过 128,000 字节时，会在构造缓存键之前被拒绝。被拒绝的超长输入不会进入那个有界的 200 条缓存。

异步加载图片，以及资源身份，由你的应用负责。

加载完成后调用 `resolve_image(original_source, resolved_url, width, height)`。宽和高必须是非零的固有尺寸。每一次出现都会更新，包括表格单元格，也包括空的 alt 文本。

绘制用解析后的 URL。原始 source 和 alt 文本留在 `RichTextEvent::ImageActivated` 里。

共用同一个 URL 的普通链接仍然是 `LinkActivated`。把同一份解析再提交一次，是空操作。

图片的宽高比、绘制、命中区域和选区共用同一份布局。

`ImageViewer::intrinsic_size(width, height)` 让小图保持自然尺寸。大图则收进可用的舞台。

缩放和平移作用在适配后的尺寸上。宿主纹理画在背景之上、控件之下，并裁到舞台里。

指针坐标用共享的 Runtime 布局换算，也用共享的捕获生命周期。Escape 走公共的浮层关闭路径。

`TextInput::max_length` 按 UTF-16 码元计数。

用户替换、粘贴、IME 和高级替换走同一条准入路径。被拒绝的编辑不改文字，不改选区，也不改历史。

事先已经超限的值不会被悄悄截断。它仍然可以编辑，但当前长度不会再增加。

`TextArea::resize_vertical(true)` 在普通视图投影里保留你拉过的高度。它尊重显式的像素约束，也不发出文字变更。

指针取消、丢失捕获和停放，都会清掉这次缩放。

content-box 缩放只在你写了显式像素高度、并且没有 min/max 高度约束时才支持。不支持的情况不露出拖柄。

## Recovery and verification

这些能力从原任务 `01a066c5-4b6a-7b81-958c-1825eb3330e7` 和它记录的子任务改动里收回，再适配到当前的 Runtime / Scene 接口。

原始记录和抽取出处留在 `../nanaui-restoration-recovery-20260910/markdown/`。历史验收不能代替你在当前树上做的验收。

Focused validation commands:

```sh
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-runtime --all-features --lib markdown
CARGO_BUILD_JOBS=2 cargo test -p nana-ui --features components --test text_input_max_length --test text_area_resize --test scroll_workspace_surface
CARGO_BUILD_JOBS=2 cargo test -p nana-ui-scene --features components --test image_viewer_content_order
CARGO_BUILD_JOBS=2 cargo run -p nana-ui-devtools --features runtime-agent,nana-ui/components --example restored-content-probe -- target/restored-content-probe
```

探针经共享 Scene 路径画出真实的 SVG、公式和图片，也画出一张上传的宿主纹理。浅色和深色都会画。

它把一次普通指针拖动送进 Runtime 输入路由（`RuntimeAgentSession`），检查得到的高度，并截下前后帧。

你必须看像素。进程成功本身不够当验收。

## Current recovery evidence (2026-09-10)

最终探针产出 16 张 PNG：浅色/深色 × 逻辑 760×680 的 1× 和 380×720 的 2× × Markdown/缩放的前后，以及 ImageViewer 的自然尺寸和 3× 缩放。16 张都打开看过。

公式、图和图片内容都在。图片宽高比保持住。窄查看器的说明文字打省略号。缩放后的内容裁在舞台里。

指针拉高在每一种情况下都增加 55 个逻辑像素。链接下划线和删除线用现有的公共 Scene 装饰辅助，并通过前景像素断言。

持久截图、命令日志和 SHA-256 清单在 `../nanaui-restoration-recovery-20260910/markdown/evidence/`。临时工作输出在 `/tmp/nanaui-consumer-upgrade-20260909/restoration-content-captures-after-shaping/`。

2× 截图也露出了现有 URL-SVG 栅格路径在固有分辨率上的发软。几何检查和存在性检查不声称已经做了按缩放的矢量栅格化。

Code 的本地 all-targets 检查通过。全部 557 个桌面库测试也通过。它正式的 native agent / 性能入口仍被锁住的桌面会话挡住。

框架最终的完整 Runtime / Scene / 输入门禁另行协调。早先收回的、绕过现代命中索引的测试夹具，已改成真实布局和指针坐标，而不是放宽输入检查。

聚焦的输入集成各自独立跑过，并且通过：`scroll_workspace_surface`（1）、`text_area_resize`（3）、`text_input_max_length`（5）。

Workspace 夹具现在在布局前使用 `assemble_workspace`。同一棵装配好的树，加上一个结构 `new` 槽，作为负对照，复现了 ScrollView 指针语义的丢失。

显式 `borrowed` 合同保留活的输入和无障碍。它通过了悬停、滚动条拖动、标签刷新和表面保持检查。默认的结构槽保持不变。借用合同见 `application-api.md`。

共享 Painter 做完 Auto-to-Advanced shaping 修复之后，同一组 16 张图重新跑成功。每张 PNG 又看过一遍。

持久证据清单现在描述的是这次最终运行。更早的图留在 `markdown/before-text-shaping-fix/`，供你对照。
