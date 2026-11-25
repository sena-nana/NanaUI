# Development Hot Reload Contract

This page describes the response latency of development hot reload (`nana-ui-dev`), its state handoff mechanism, and why Rust L3 uses process re-execution (re-exec) instead of dylib hot swapping. For quick setup steps, see [Hot Reload](../guide/scaling/hot-reload.md) in the guide.

## Reload mode and measured latency per layer

The top of `nana-ui-dev` carries a compile-time gate, `#[cfg(not(debug_assertions))] compile_error!(...)`. The distribution profile `[profile.dist]` inherits from `release`, so if a production build pulls in the crate by mistake, compilation fails outright. There is no way for the file watcher to end up in a shipped product.

| What you change | Reload action triggered | Measured magnitude |
| --- | --- | --- |
| **Rust `.vue` dialect: static text or static `<style>` properties only** | **Replace the static text table (`TemplateText::apply`) or dev style patch sheet (`TemplateStyle::apply`)**: no recompile, no restart; nodes and focus are preserved in place | `< 50 ms` |
| **Rust L3: edit `.rs` sources** | **Background incremental build → hand off window geometry and session state → `re-exec`**: the window flashes only once | `1.5–3.6 s` (dominated by linking) |

Measured with `cargo build -p component-gallery` on macOS (Apple Silicon, default `dev` profile, warm): changing only the application crate (18k lines) and relinking the 131 MB binary takes **`1.5–1.6 s`**; changing the lower-level `nana-ui-runtime` (127k lines, which also rebuilds `scene` and `nana-ui`) takes **`3.6 s`**. Linking takes longer on Windows. Measure locally with `cargo build -p <package> --timings` and enable `lld` for the dev profile.

## Rust L3 path: why process re-exec instead of dylib hot swapping

Rust L3 does not hot-swap code in process with `dlopen / dlclose`. Doing so without breaking the framework's core contracts runs into five hard obstacles:

1. **`TypeId` is not stable across compilations**: declarative views and keyed assembly depend on `TypeId::of::<C>()`. A recompiled dylib produces a different `TypeId` for the same type, so every keyed child mismatches and is rebuilt, and `remove_view` fails outright.
2. **`RuntimeProgram` is not object-safe**: `with_document` is a method taking a generic closure and cannot go into a vtable. Wrapping it across FFI would amount to adding the second C ABI that `AGENTS.md` strictly forbids.
3. **Event closures hold raw function pointers into the old code segment**: before a dylib can be unloaded safely, the whole tree must be torn down, which is no different from rebuilding it.
4. **Platform limits on unloading dynamic libraries**: on macOS, `dlclose` does not actually unload images that contain Objective-C metadata or thread-local storage (TLS).
5. **Split global statics**: a dylib that links `wgpu` again produces a second set of instances.

So the honest approach for Rust L3 is **background incremental build + state handoff + in-place `re-exec`** (for a full example see [`crates/nana-ui-dev/examples/l3-dev-entry.rs`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-dev/examples/l3-dev-entry.rs)):

```rust
// 1. Wrap the main entry point with run_with_restart
nana_ui_dev::run_with_restart::<MyProgram>(settings, Path::new("target/nana-dev-handoff"))?;

// 2. In initialize, restore the session state handed off by the previous run (falls back to Default on first launch or when deserialization fails after struct fields change)
let session = nana_ui_dev::restored_handoff()
    .and_then(|handoff| handoff.state_as::<Session>())
    .unwrap_or_default();
let watcher = nana_ui_dev::watch_and_rebuild(
    &DevConfig::new_rust("src"),
    RebuildCommand::cargo_package("my-app"),
    context,
)?;

// 3. In update, on DevSignal::Rebuilt, hand off window geometry and application session state, then exit and re-exec
Message::Dev(DevSignal::Rebuilt) => {
    nana_ui_dev::request_relaunch(DevHandoff { /* window geometry */ }.with_state(&self.session));
    RuntimeProgramUpdate { exit: true, ..Default::default() }
}
```

In addition, if you write views in the `.vue` dialect (Rust script) and only change static strings or static `<style>` declarations, enable `Compiler::new("::nana_ui::runtime").hot(debug)` in `build.rs` and use `watch_templates` (or `watch_template_events`, `templates` feature) in the dev entry point. Window text is replaced live through `TemplateText::apply()`, and static `<style>` changes recompile via `nana-ui-css` into a dev-only `HotStyleSheet` injected through `TemplateStyle::apply()` without recompiling or restarting the process (product builds keep static `StylePatch` function pointers). Adding `transition`, `@keyframes`, combinators, or new interactive pseudo-classes still triggers an incremental rebuild.

## Verifying without a window through a headless agent session

Enable the `headless` feature on the `dev` binary as well (it brings in the `runtime-agent` layer of `nana-ui-devtools`), and the same `dev` executable supports both a "normal desktop window" mode and a "headless JSON automation" mode. After each rebuild you no longer have to click your way back into deep pages by hand: pipe JSON commands in to click, inspect the accessibility tree, and take offscreen screenshots:

```bash
cargo build -p my-app --bin dev --features dev,headless

printf '%s\n' \
  '{"id":1,"cmd":"a11y"}' \
  '{"id":2,"cmd":"screenshot","path":"target/agent/after.png"}' \
  '{"id":3,"cmd":"click","agent_path":"open"}' \
  | ./target/debug/dev --stdio --width 320 --height 200 --theme dark
```
