# 示例

这些是仓库里已经能跑的程序。对话框、设置页和侧栏在 Gallery 里，不另附一份教程。

在仓库根目录运行。`nana-ui` 的 example 要带上它声明的 feature。

| 要看什么 | 命令 | 在哪 |
| --- | --- | --- |
| 控件目录，含对话框、设置、侧栏 | `cargo run -p component-gallery` | `examples/component-gallery` |
| 最小宿主 | `cargo run -p runtime-host-fixture` | `examples/runtime-host-fixture` |
| 实时应用（60 Hz 固定步更新、实例化 GPU playfield、界面叠层） | `cargo run -p realtime-fixture` | `examples/realtime-fixture` |
| 计数器（`view!`） | `cargo run -p nana-ui --example reactive-counter --features hosted,bundled-fonts,view-macro` | `crates/nana-ui/examples/reactive-counter.rs` |
| 计数器（应用壳） | `cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts` | `crates/nana-ui/examples/application-counter.rs` |
| 待办（`.vue`） | `cargo run -p reactive-sfc` | `examples/reactive-sfc` |
| 任务工作台（`#[derive(Store)]` + 历史撤回 + 虚拟列表 + 校验 + `ActionRegistry`） | `cargo run -p task-workbench` | `examples/task-workbench` |
| 实时画面（`GpuView`） | `cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts` | `crates/nana-ui/examples/gpu-view-demo.rs` |
| 实时 WGSL 着色器视口 | `cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop,runtime-wgsl` | `crates/nana-ui/examples/hosted-gpu-demo.rs` |
| 在树中混排 egui 面板 | `cargo run -p nana-ui --example egui-in-nanaui --features hosted,bundled-fonts,wgpu-interop` | `crates/nana-ui/examples/egui-in-nanaui.rs` |
| 多窗口 | `cargo run -p nana-ui --example window-chrome-multi-window --features hosted,bundled-fonts` | `crates/nana-ui/examples/window-chrome-multi-window.rs` |

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/quick-start">
    <p class="next-step-link">快速开始</p>
    <p class="next-step-caption">依赖和第一段界面。</p>
  </a>
  <a class="next-step" href="/components/">
    <p class="next-step-link">组件</p>
    <p class="next-step-caption">按族查已经有的控件。</p>
  </a>
</div>
