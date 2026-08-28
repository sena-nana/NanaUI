# 开始

先看 [框架如何运行](how-it-works.md)。这篇把第一扇窗口写出来。

## 依赖

`nana-ui` 默认 feature 为空：不启用 `hosted` 就没有 `run_runtime`，不启用 `gpu` 就没有 painter。桌面应用最少：

```toml
[dependencies]
nana-ui = { path = "../NanaUI/crates/nana-ui", features = ["hosted", "bundled-fonts"] }
```

`hosted` 会带上 `gpu`、winit 和 AccessKit。更多控件族见 [应用 API](application-api.md) 的 feature 表。Rust 1.92+。

仓库本身用 path / git 消费，尚未作为 crates.io 包发布。

## 先看成品

在仓库根目录：

```bash
cargo run -p component-gallery
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
```

Gallery 是控件目录，不是你的产品骨架。最小宿主对照 `examples/runtime-host-fixture`；多窗口对照 `crates/nana-ui/examples/window-chrome-multi-window.rs`。

## 第一扇窗口

应用做三件事：建一棵 `RuntimeDocument`，实现 `RuntimeProgram`，调用 `run_runtime`。

```rust
use std::convert::Infallible;

use nana_ui::runtime::{Activate, Button, DocumentId, RuntimeDocument, Text};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, RuntimeWindowSettings, ThemeMode,
    run_runtime,
};
use nana_ui_platform::{WindowEvent, WindowId};

struct App {
    document: RuntimeDocument,
}

impl App {
    fn mount() -> Self {
        let document_id = DocumentId::new(1).expect("document id");
        let mut document = RuntimeDocument::new(document_id);
        let cx = document.context_mut();

        cx.build(document_id, |ui| {
            ui.column(12.0, |ui| {
                ui.child("title", Text::new("你好"));
                let start = ui.child("start", Button::new("开始"));
                ui.on(start, move |_button, _event: &Activate, _cx| {
                    // 改你自己的状态。需要开窗或换 GPU 时：cx.dispatch_program(msg)
                });
            });
        })
        .unwrap();

        Self { document }
    }
}

impl RuntimeProgram for App {
    type Message = ();
    type Error = Infallible;

    fn initialize(
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        Ok((Self::mount(), Vec::new()))
    }

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument> {
        (id == WindowId::PRIMARY).then_some(&self.document)
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument> {
        (id == WindowId::PRIMARY).then_some(&mut self.document)
    }

    fn update(
        &mut self,
        _message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Dark
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::CloseRequested { .. } => RuntimeProgramUpdate::exit(),
            _ => RuntimeProgramUpdate::default(),
        }
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<App>(RuntimeWindowSettings::new("NanaUI"))
}
```

`bundled-fonts` 开启时，宿主会注册 Noto Sans SC 并设为界面默认字体。关掉则回落到系统字体，不能当设计稿。

新代码从 `nana_ui::runtime` 引入控件。crate 根上的同名再导出是兼容面，不要当第二套 API。

## 状态放哪

| 东西 | 放哪 |
| --- | --- |
| 按钮是否 loading、输入框当前值 | 对应控件（`update_component`）或你的 view state |
| 打开了哪个文档、登录态、设置值 | 应用自己的结构，NanaUI 不替你存盘 |
| 侧栏宽度、Region 折叠 | `WorkspaceModel`，见 [工作区](workspace.md) |
| 这一帧的实时画面 | 你的 GPU 资源 + `HostTextureRegistry`，见 [实时画面](gpu.md) |

`RuntimeProgram::Message` 是跨窗口 / GPU / 持久化的宿主消息，不是每个点击的总线。

## 接下来

- 拼控件、加一种自己的：[控件](components.md)
- 桌面壳：[工作区](workspace.md)
- 把视口挂上树：[实时画面](gpu.md)
- 标题栏与系统模糊：[窗口](window.md)
- 深色 / 浅色与尺寸：[视觉](look.md)
- 组成式建树（`build` / `mount` / 何时不该重建）：[L3 组成式建树](l3-authoring.md)
