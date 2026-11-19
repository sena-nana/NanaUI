# Coming from Vue

The guide's chapter names follow Vue. The runtime is not Vue: there is no virtual DOM, no WebView, and the tree is built once at mount. The table below is a lookup index. How to write signals, conditionals and lists is covered in the corresponding chapters. The full mapping and measurements are in [Declarative views](../reference/reactive-view.md#mapping-from-vue).

## Mapping

| Vue | NanaUI | Habit to change |
| --- | --- | --- |
| `ref(0)` | `signal(0)` | `ref` is a Rust keyword. A signal is a `Copy` id; its value lives on the thread that created it. Using it on another thread panics |
| `reactive({ … })` | `store` with `#[derive(Store)]` | Tracked per field, not a deep proxy. See [Store](scaling/store.md) |
| `computed(() => …)` | `computed(move \|\| …)` | Calling `.get()` while constructing a view writes a constant snapshot into the node |
| `watch` / `watchEffect` | `watch_effect` | Returns an `Effect`, with no old and new values. See [Watchers](essentials/watch.md) |
| `{{ x }}`, `:label="x"` | `"{x}"`, `.label(x)` | A bare signal creates a dependency. Inside expressions, `.get()` / `.with()` cannot be omitted |
| `@click` | `@activate` / `.on_activate` | A button is not a DOM click. Use `.on` for other events |
| `v-if` / `v-else` | `when` / `.otherwise` | The old branch is torn down only when the condition changes. See [Conditional rendering](essentials/conditional.md) |
| `v-show` | `.visible` | The node stays; only hidden is toggled |
| `v-for` + `:key` | `each` | Rows are retained by key. See [Lists](essentials/list.md) |
| `v-model` | `.model` / `v-model` | Writes back to a signal |
| `provide` / `inject` | `provide` / `use_context` | Readable only while building views. See [Provide](components/provide.md) |
| Template ref | Handle | `node_ref`, `entity_ref`. See [Handles](essentials/refs.md) |
| `onMounted` / `onUnmounted` | `on_mount` / `on_cleanup` | See [Mount and unmount](essentials/lifecycle.md) |
| `nextTick` | `flush_reactive` | Writes are queued first. Flushed at the end of input events and the start of each frame |
| props / emits | Function parameters / callbacks | See [Props](components/props.md) and [Events](components/events.md) |
| `<KeepAlive>`, `<Teleport>`, `<Transition>`, `<Suspense>` | Same-named features | See Built-ins in the guide |
| Vue Router, Pinia | Your application's own structures and `store` | The framework does not own routing, region content or persistence |
| SFC | `.vue` dialect, with Rust scripts | See [.vue dialect](scaling/sfc.md) |

`each` and `when` default to boxless fragments inside a host parent, emitting sibling nodes directly just like Vue's `v-for` / `v-if`; they materialize a wrapper `Stack` only when given explicit container options (`.gap`, `.horizontal`, `.grow`, `.class`, `.key`, etc.) or at a host-less single-root slot/mount boundary. Component functions are not lazy; ownership is divided only at three scope boundaries: mount, row and branch.

## What is not here

- A virtual DOM and tree diffing. A binding changes only the field it writes.
- WebView and a JS runtime. You cannot drop `@vue/runtime-dom` output in and ship it as a desktop app.
- Browser CORS, cookies, Service Workers and IndexedDB. Remote resources use an allowlist; see [Fetch](scaling/fetch.md).
- `v-html`.

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/quick-start">
    <p class="next-step-link">Quick start</p>
    <p class="next-step-caption">Write your first piece of UI.</p>
  </a>
  <a class="next-step" href="/guide/essentials/reactivity">
    <p class="next-step-link">Reactivity</p>
    <p class="next-step-caption">How to read and write signals.</p>
  </a>
  <a class="next-step" href="/guide/scaling/sfc">
    <p class="next-step-link">.vue dialect</p>
    <p class="next-step-caption">Template, script and style in one file; the script is Rust.</p>
  </a>
</div>
