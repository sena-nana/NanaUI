# Examples

These are programs in the repository that already run. Dialogs, settings pages and sidebars are in the Gallery; there is no separate tutorial for them.

Run them from the repository root. `nana-ui` examples need the features they declare.

| What to look at | Command | Where |
| --- | --- | --- |
| Control catalog, including dialogs, settings and sidebars | `cargo run -p component-gallery` | `examples/component-gallery` |
| Minimal host | `cargo run -p runtime-host-fixture` | `examples/runtime-host-fixture` |
| Realtime application (60 Hz fixed update, instanced GPU playfield, UI overlay) | `cargo run -p realtime-fixture` | `examples/realtime-fixture` |
| Counter (`view!`) | `cargo run -p nana-ui --example reactive-counter --features hosted,bundled-fonts,view-macro` | `crates/nana-ui/examples/reactive-counter.rs` |
| Counter (application shell) | `cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts` | `crates/nana-ui/examples/application-counter.rs` |
| To-do list (`.vue`) | `cargo run -p reactive-sfc` | `examples/reactive-sfc` |
| Task workbench (`#[derive(Store)]` + undo/redo + virtual list + validation + `ActionRegistry`) | `cargo run -p task-workbench` | `examples/task-workbench` |
| Live GPU content (`GpuView`) | `cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts` | `crates/nana-ui/examples/gpu-view-demo.rs` |
| Live WGSL shader viewport | `cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop,runtime-wgsl` | `crates/nana-ui/examples/hosted-gpu-demo.rs` |
| Multiple windows | `cargo run -p nana-ui --example window-chrome-multi-window --features hosted,bundled-fonts` | `crates/nana-ui/examples/window-chrome-multi-window.rs` |

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/quick-start">
    <p class="next-step-link">Quick start</p>
    <p class="next-step-caption">Dependencies and your first piece of UI.</p>
  </a>
  <a class="next-step" href="/components/">
    <p class="next-step-link">Components</p>
    <p class="next-step-caption">Browse existing controls by family.</p>
  </a>
</div>
