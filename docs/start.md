# 开始

先跑起来，再写一棵最小的界面。细节和内部机制后面再查。

## 先看成品

在仓库根目录：

```bash
# 控件、侧栏、深色/浅色
cargo run -p component-gallery

# 界面和实时画面画在同一个窗口里
cargo run -p nana-ui --example hosted-gpu-demo --features bundled-fonts,gpu
```

Gallery 只是示例，不是你的产品骨架。有 Vue 旧代码要迁时，再看 `examples/vue-counter` 和 [Vue](vue.md)。

## 这套框架怎么理解

你写的是一棵界面树。`Text`、`Button`、侧栏、对话框，以及一块实时画面，都是树上的节点：一起排版，一起裁剪，点到谁就是谁。

应用打开窗口、创建图形设备。NanaUI 画进这个窗口，不另开一套设备，也不把画面拷到 CPU 再贴回去。

业务状态、配置文件、每个区域里放什么、角色这一帧画什么，都是你的。NanaUI 不替你存设置，也不内置登录或任何产品业务。

## 写一棵界面

新应用用 `nana_ui::runtime`。先建一份文档，往上挂控件，再接到点击：

```rust
use nana_ui::runtime::{
    Activate, Button, DocumentId, List, RuntimeDocument, Text,
};

let document_id = DocumentId::new(1).unwrap();
let mut document = RuntimeDocument::new(document_id);
let cx = document.context_mut();

let root = cx.create_component(document_id, List::new())?;
let title = cx.create_component(document_id, Text::new("你好"))?;
let start = cx.create_component(document_id, Button::new("开始"))?;
cx.append_child(root, title)?;
cx.append_child(root, start)?;
cx.on(start, move |_button, _event: &Activate, _cx| {
    // 改你自己的状态
})?;
```

然后实现 `RuntimeProgram`：按窗口交出这份文档，在 `update` 里消化消息。最后 `run_runtime` 打开窗口、注入图形设备并开始画。完整的窗口、多窗口和事件循环，对照 `crates/nana-ui/examples/window-chrome-multi-window.rs` 和 `examples/runtime-host-fixture`。

crate 根上那些控件再导出是旧名字，新代码不要当第二套 API 用。

## 接下来做什么

- 各种控件怎么拼、怎么自己加一种：[控件](components.md)
- 把 Live2D、着色器、预览视口放进这棵树：[实时画面](gpu.md)
- 标题栏、图标、系统模糊、再开一扇窗：[窗口](window.md)
- 颜色和尺寸：[视觉](look.md)

查函数签名和扩展方式时用 [应用 API](application-api.md)。想知道树如何更新、一帧如何画出来，再读 [Runtime 与 Scene](runtime-scene.md)。
