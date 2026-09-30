# 示例

这些是仓库里已经能跑的程序。对话框、设置页和侧栏在 Gallery 里，不另附一份教程。

在仓库根目录运行。`nana-ui` 的 example 要带上它声明的 feature。

| 要看什么 | 命令 | 在哪 |
| --- | --- | --- |
| 控件目录，含对话框、设置、侧栏 | `cargo run -p component-gallery` | `examples/component-gallery` |
| 最小宿主 | `cargo run -p runtime-host-fixture` | `examples/runtime-host-fixture` |
| 计数器（`view!`） | `cargo run -p nana-ui --example reactive-counter --features hosted,bundled-fonts,view-macro` | `crates/nana-ui/examples/reactive-counter.rs` |
| 计数器（应用壳） | `cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts` | `crates/nana-ui/examples/application-counter.rs` |
| 待办（`.vue`） | `cargo run -p reactive-sfc` | `examples/reactive-sfc` |
| 实时画面 | `cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts` | `crates/nana-ui/examples/gpu-view-demo.rs` |
| 多窗口 | `cargo run -p nana-ui --example window-chrome-multi-window --features hosted,bundled-fonts` | `crates/nana-ui/examples/window-chrome-multi-window.rs` |

`examples/vue-counter` 是引擎探针，含无头点击。窗口化的 Vue 对照是 `examples/vue-hosted-acceptance`。两者都不是应用模板。接法见 [Vue](../reference/vue.md)。

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
