# Hot Reload

How quickly you see a UI change again depends on what you changed. Hot reload lives in `nana-ui-dev`. That crate refuses to compile without `debug_assertions`. `[profile.dist]` inherits `release`, so if a product dependency pulls it in, the build fails. There is no escape hatch.

| What changed | What happens | Magnitude |
| --- | --- | --- |
| Only static text or static `<style>` properties in a `.vue` file | The text table or dev style patch sheet is replaced; nodes, focus and state are untouched | No recompile, no restart |
| Rust source | Recompile, hand over window geometry and state, then re-exec | 1.5–3.6 s, mostly linking |

macOS / Apple Silicon, default dev profile, warmed-up incremental build, `cargo build -p component-gallery`: the application crate (18k lines) takes 1.5–1.6 s, including relinking the 131 MB debug binary; after touching `nana-ui-runtime` (127k lines), rebuilding the application takes 3.6 s, which also recompiles `nana-ui-scene` and `nana-ui`. Linking is slower on Windows. Measure once on your own machine with `cargo build -p <your-package> --timings`. The 0.4–1.5 s for recreating the window and GPU has not been re-measured.

`[profile.dev] debug = "line-tables-only"` was tried and not adopted: 1.53–1.77 s on the same workload against a baseline of 1.49–1.58 s, with the binary going from 131.6 MB to 125.5 MB. On macOS `split-debuginfo` defaults to `unpacked`, so narrowing debug info barely affects linking, while it costs you variables in the debugger.

## Rust applications

Rust cannot swap code inside a running process. `TypeId` is not stable across compilations, `RuntimeProgram` is not object-safe, event closures point into running code, and on macOS `dlclose` cannot unload images with Objective-C metadata or TLS. The honest approach is to re-exec. A complete example is in `crates/nana-ui-dev/examples/l3-dev-entry.rs`.

```rust
nana_ui_dev::run_with_restart::<MyProgram>(settings, Path::new("target/nana-dev-handoff"))?;
```

In `initialize`:

```rust
let watcher = nana_ui_dev::watch_and_rebuild(
    &DevConfig::new_rust("src"),
    RebuildCommand::cargo_package("my-app"),
    context,
)?;
```

The message type must implement `From<DevSignal>`. On `Rebuilt`, hand over the geometry and a piece of state the framework does not interpret, then exit. Unix uses `exec`; Windows spawns first and then exits. `BuildFailed` carries the diagnostic text.

```rust
nana_ui_dev::request_relaunch(DevHandoff { /* geometry */ }.with_state(&self.session));
```

The next `initialize` takes it back. When the type does not match, `state_as` returns `None`. `restored_handoff()` is consumed only once and can be called from `initialize` under `run_with_restart`.

```rust
let session = nana_ui_dev::restored_handoff()
    .and_then(|handoff| handoff.state_as::<Session>())
    .unwrap_or_default();
```

Changing only static text or static `<style>` declarations in a `.vue` file needs no restart. The build script turns on hot mode in debug, and the dev entry uses `watch_templates` from the `templates` feature (with `From<DevSignal> + From<TemplateText> + From<TemplateStyle>`, or `watch_template_events`). The `runtime` string must match the one passed to `Compiler::new`. If the shape changes (structure, bindings, script, text spanning lines, or adding `transition` / `@keyframes` / combinators / new interactive pseudo-classes), or the change is not in a `.vue` file, it rebuilds as usual. Interpolated text and attribute values are not in the replacement table.

```rust
let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
nana_ui_sfc::Compiler::new("::nana_ui::runtime").hot(debug).build("views")?;

let watcher = nana_ui_dev::watch_templates(
    &DevConfig::new_rust("src"),
    "views",
    "::nana_ui::runtime",
    RebuildCommand::cargo_package("my-app"),
    context,
)?;
```

In `update`, call `apply()` on `TemplateText` and `TemplateStyle`. Nodes, focus, scroll and state are untouched; the new text or styles appear on the next frame.

## Not yet covered

L3 has no default "reload data file" hook. To swap the tree in-process, call `AppContext::remove_view(root)` first and then `mount_view_root`. Mounting twice in a row stacks two trees.

To inspect the result headlessly, add `headless` to the dev bin:

```bash
cargo build -p my-app --bin dev --features dev,headless
```

The same JSON can query the accessibility tree, take screenshots and click.

Steps, measurement conditions and why hot swapping is not possible are written up in [Hot reload](../../reference/hot-reload.md).

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/store">
    <p class="next-step-link">Store</p>
    <p class="next-step-caption">How nested state that reload cannot keep is updated per field day to day.</p>
  </a>
  <a class="next-step" href="/guide/scaling/sfc">
    <p class="next-step-link">.vue dialect</p>
    <p class="next-step-caption">Hot mode is turned on in the build script.</p>
  </a>
  <a class="next-step" href="/guide/scaling/packaging">
    <p class="next-step-link">Packaging</p>
    <p class="next-step-caption">The dev watcher never ships in dist output.</p>
  </a>
</div>
