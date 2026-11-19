# Quick Start

This page walks you through your first piece of UI. A complete program that opens a window is in [Creating an application](essentials/application.md). Reading the [Introduction](introduction.md) first makes this easier to follow.

::: tip Prerequisites
Rust 1.98+ is required. The repository is currently referenced by path or git and is not yet published to crates.io.
:::

## Add the dependency

`nana-ui` has `default = []`: without `hosted` there is no `run_runtime`, and without `gpu` there is no painter. Enabling `hosted` implies `gpu` and pulls only the platform's primary graphics backend (`metal` on Apple, `dx12` on Windows, `vulkan` on Unix), loading precompiled shader archives by default; add `wgpu-backends` if you need all cross-platform backends, or enable `full`. See the [Migration Guide](migration-layout-and-features.md) for opt-in features. A minimal desktop application needs at least this:

```toml
[dependencies]
nana-ui = { path = "../NanaUI/crates/nana-ui", features = ["hosted", "bundled-fonts", "view-macro"] }
```

`hosted` brings in `gpu`, winit and AccessKit. `view-macro` compiles `view!`, `css!` and `stylesheet!`. If you only write function calls, you can drop it. For more control families and `[profile.dist]` builds, see [Application API](../reference/application-api.md).

## See the finished product

From the repository root:

```bash
cargo run -p component-gallery
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop,runtime-wgsl
```

The Gallery is the control catalog, while `gpu-view-demo` and `hosted-gpu-demo` mount live GPU nodes in the UI tree (`hosted-gpu-demo` uses custom WGSL shaders). The minimal host reference is `examples/runtime-host-fixture`.

For compact release binaries, build with the workspace distribution profile (`cargo build --profile dist`).

## Your first UI

An application does three things: builds a `RuntimeDocument`, implements `RuntimeProgram`, and calls `run_runtime`. The UI itself is written in the mount closure.

The two snippets below are the same UI and expand to the same mount. `start` is a function with no parameters. The toggle at the top of the sidebar decides which one you see.

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=12>
        <Text>"Hello"</Text>
        <Button @activate={start}>"Start"</Button>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{button, column, text};

column().gap(12).children((
    text("Hello"),
    button("Start").on_activate(start),
))
```

:::

To open a window or swap a texture on click, use `.on_cx(|_button, _event: &Activate, cx| cx.dispatch_program(msg))`. In a template, write `on:Activate={…}` as a three-argument closure; it expands to that same method. The message is handled in `update`.

To put this view in a window you need a `RuntimeDocument` and a `RuntimeProgram`. The complete program that compiles and runs is in [Creating an application](essentials/application.md#the-complete-program).

With `bundled-fonts` on, the host registers Noto Sans SC and makes it the UI's default font. Turn the feature off and it falls back to system fonts. Controls are imported from `nana_ui::runtime`.

## Where state lives

| Thing | Where it lives |
| --- | --- |
| Whether a button is loading, an input's current value | The control, or a signal in the view |
| Which document is open, sign-in state, setting values | Your application's own structures |
| Sidebar width, Region collapse | `WorkspaceModel`; see [Workspace](../reference/workspace.md) |
| Window position and size | `WindowDescriptor::persist_key`; see [Window](../reference/window.md) |
| This frame's live GPU content | Your GPU resources plus `HostTextureRegistry`; see [Live GPU content](../reference/gpu.md) |

`RuntimeProgram::Message` carries host messages that cross windows, GPU work and persistence.

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/application">
    <p class="next-step-link">Creating an application</p>
    <p class="next-step-caption">Put the view above into a window that opens.</p>
  </a>
  <a class="next-step" href="/guide/essentials/view">
    <p class="next-step-link">View syntax</p>
    <p class="next-step-caption">How the two styles of the same UI expand.</p>
  </a>
  <a class="next-step" href="/components/">
    <p class="next-step-link">Components</p>
    <p class="next-step-caption">Browse existing controls by family.</p>
  </a>
  <a class="next-step" href="/architecture/">
    <p class="next-step-link">Architecture</p>
    <p class="next-step-caption">From this tree to one frame in a window.</p>
  </a>
</div>
