# Declarative views

The learning path is [Writing views](../guide/essentials/view.md), [Reactivity](../guide/essentials/reactivity.md), and [Coming from Vue](../guide/from-vue.md). This page is the contract.

`nana_ui::runtime::view` lets you write a view as one expression. The dynamic parts are signals or closures. The tree is built once. After that, each binding updates only the node field it writes. There is no whole-tree render. There is no diff. It writes the same `UiWorld` and the same assembly key table as the imperative `create_component` / `mount`. Everything still ends in `create` / `insert` / `project` / `commit`.

**Switches.** The view layer is compiled in by default. It needs no feature. The template macros `view!`, `css!`, and `stylesheet!` are behind `view-macro`. That is a separate proc-macro crate. If you do not use it, it does not take part in the build. Cause tracing is behind `reactive-trace`. Examples: `crates/nana-ui/examples/reactive-counter.rs` (written with `view!`). `examples/reactive-sfc` (written as `.vue` files).

**Two first-class authoring styles. One path.** The `view!` template and the Rust function style are both official. Their capabilities match: `view!` expands to the very calls of the Rust style (`column().gap(8)`, `.class(..)`, `each(..)`, …). Neither has its own implementation. Every template construct has a Rust method of the same name. See the mapping in [`view!` and the Rust style](#view-and-the-rust-style). `tests/view_macro.rs` writes the same page three ways: as a template, with tuple children, and with `.with` blocks. After mounting they are identical node by node. They stay identical after signals change. Transition, KeepAlive, Suspense, Teleport, ErrorBoundary, virtual lists, and named slots each have the same kind of side-by-side check. Styles are likewise written once as `<style>` and once as `stylesheet!`. The layout is the same. The template stays close to Vue. It suits larger stretches of UI. The Rust style is plain Rust. You can write `for` / `if`. You get full completion and type checking. A misspelled class name is a compile error. `.vue` files are [advanced usage](#advanced-the-vue-dialect).

## Authoring

The switch in the sidebar toggles between the two styles. Mounting is still a host call, visible in both.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn counter() -> impl IntoView {
    let count = signal(0u64);
    view! {
        <Row gap=8>
            <Text>"Count: {count}"</Text>
            <Button @activate={count.update(|c| *c += 1)}>"Increment"</Button>
        </Row>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;
use nana_ui::runtime::text;

fn counter() -> impl IntoView {
    let count = signal(0u64);                                   // component-local state
    row().gap(8).children((
        text!("Count: {count}"),                                 // interpolation; signals read here become dependencies
        button("Increment").on_activate(move || count.update(|c| *c += 1)),
    ))
}
```

:::

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn todos(list: Signal<Vec<Todo>>, draft: Signal<String>) -> impl IntoView {
    view! {
        <Column gap=8>
            <TextInput placeholder="New task" v-model={draft} />
            <Button disabled={draft.with(|d| d.is_empty())}>"Add"</Button>
            <Text v-for={t in list} key={t.id}>{t.title}</Text>
            <Text v-if={list.with(Vec::is_empty)}>"No tasks yet"</Text>
            <Text v-else>"Has tasks"</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn todos(list: Signal<Vec<Todo>>, draft: Signal<String>) -> impl IntoView {
    column().gap(8).with(|c| {
        c.add(text_input().placeholder("New task").model(draft));   // v-model
        c.add(button("Add").disabled(move || draft.with(|d| d.is_empty())));
        c.add(list.each(|t| t.id, |t| text(t.title)));           // v-for + :key
        c.add((move || list.with(Vec::is_empty))
            .then_show(|| text("No tasks yet"))
            .otherwise(|| text("Has tasks")));                     // v-if / v-else
    })
}
```

:::

```rust
let view = cx.mount_view(parent, counter)?;                    // or mount_view_root(document, …)
```

- **Children**: a fixed handful goes in a tuple. `.children((a, b))`. The group of children is boxed once. An element's size therefore does not depend on how many nodes sit below it. (A static tree is one value: with children inlined, a view the size of a settings page is about 70 KB. Every builder method and every level of building moves the whole block once. A debug build overflows the 1 MB main-thread stack. `tests/view_stack.rs` builds such a page on a thread with a 1 MB stack.) When you need `for`, `if`, or `let`, use a block. `.with(|c| { c.add(a); … })`. Each child is boxed once. The block runs once, at tree building. The `for` / `if` inside it decide the structure at that moment. They do not follow the data. To follow the data, use `each` / `when`.
- **Working with hand-written code**: message-driven applications (the program keeps control handles; events send messages to `RuntimeProgram`) can also build their UI with views. `entity_ref::<C>()` together with `.entity_ref(r)` records the `Entity<C>` an element builds. When the mount closure returns `with_refs(view, handles)`, the mount returns `(MountedView, resolved handles)` directly. (`EntityRef<C>` becomes `Entity<C>`. `NodeRef` becomes a node id. Tuples, arrays, and `Vec` resolve item by item. If an element that carries a handle was not built, the mount reports `InvalidInput` and leaves nothing behind.) After that, use `update_component` as usual. `El<C>` only accepts `EntityRef<C>`. A wrong type does not compile. One element can fill several handles at once: `.entity_ref(e).node_ref(n)` (for example, a `NodeRef` for a teleport or `on_mount` next to the program's `Entity`). Each gets this node. None overwrites another. Handles are signals. Create them inside the mount closure so that this mount owns them. A handler registered with `.on_cx::<E>(|component, event, cx| …)` receives the component and a `ViewContext`. It can modify the component in place, `cx.emit(..)`, and `cx.dispatch_program(..)`. In a template, writing `on:E={…}` as a three-argument closure gives you this. To build something first and hand it to someone else for placement (slots that composite controls take by id), use `mount_view_detached`. For a view with a single root, `view.root::<C>()` gives it to you directly.
- **Starting from data**: `list.each(key, row)` is `each(list, key, row)`. `cond.then_show(|| v).otherwise(|| w)` is `when(cond, || v).otherwise(|| w)`. (It is not called `show`: Vue's `v-show` keeps the node. Its counterpart is `.visible(..)`.)

- **Properties** accept three kinds of values: constants, `Signal<T>` / `Computed<T>`, and `Fn() -> T` closures. A constant is written when the node is built. It costs nothing afterward. A signal is bound directly. It stores just one "signal id + field write function" record. There is no closure. A closure is boxed once.
- **Control properties** have setters generated field by field from the control table by `view/controls.rs` (`button(..).disabled(..)`, `slider(..).value(..)`). Each setter corresponds to a `FieldWrite`, reachable by name: `view::fields::<element fn>::<field>` (`fields::text::value` is the one bound by `.value(..)` on `text(..)`). `fields::Visible` is `.visible(..)`. `fields::HiddenWhenEmpty<W>` wraps any field: when the value is empty (empty string, `None`, empty list), the node leaves layout, painting, and hit testing at the same time, for example `text("").prop::<_, HiddenWhenEmpty<fields::text::value>>(title)`. Both change in the same write, so an empty value never shows first. Some fields carry rules: a slider's `value` is clamped to its range (no value yet, or a non-finite value, lands on the minimum); when a number field's `value` changes to a new value the caret goes to the end, while echoing the same value leaves the caret alone; `Tabs`' `selected` changes together with the tab strip's focus; `ColorField`'s `value` is written through its builder, so hue, saturation, and value follow; `Thumbnail`'s `resource` is ready as soon as it is non-empty; `EmptyState`'s `message` also accepts `String`, and an empty string means none. When an `Arc<str>` field is bound from a `String` in a store, wrap it in `fields::FromString<W>` (`FromOptionalString<W>` for optional fields): it compares first and does not allocate when equal, and it stacks with `HiddenWhenEmpty`, for example `HiddenWhenEmpty<FromString<fields::labeled_value::value>>`. `ActionMenu` is not a template tag (controls in templates are leaves, and its items are children); its fields are in `fields::action_menu`: `label` (empty closes it and hides the trigger), `accessible_name`, `open`. `ReorderList`'s `items` is in `fields::reorder_list` (`controls` feature). `Dialog`, `ConfirmDialog`, and `Drawer` are not template tags either; `widget(..)` has `.open(..)` and a two-way `.model(signal)` that open and close them under an `OverlayHost` (see [Dialog](../components/dialog.md#declarative-open-and-close)). Any control can use `widget(component).bind(|c| …)` and `.on::<E>(|e| …)`. For a custom field, write a `FieldWrite` and use `.prop::<T, W>(..)`.
- **key** is written only on `each` elements and on nodes that must be found by path (`resolve_assembly_path`). Static structure is built only once. Nodes without a key are named by position (`#v0`, `#v1`, …). Branches newly built after a `when` / `dynamic` switch, branches that `keep_alive` shows again, and rows that `each` adds after mounting are all registered under their container exactly like those built at mount: if the branch root in `dynamic(..).key("a")` has `key("b")`, then `a/b` finds the branch currently shown both before and after a switch (the same holds in `each_virtual` rows). `each` rows can be found by key only if they have one. (Positional names of unkeyed rows are counted separately each time they are built. Registering them would let them overwrite each other.)
- **`mount_view` takes a closure.** The view expression is evaluated in the mount scope. Signals created in component functions belong to this mount. They are reclaimed together on `unmount`, when the root node is destroyed, or when the `AppContext` is dropped. Each `each` row and each `when` branch has its own child scope. When a row is removed or a branch is switched away, only its own signals and effects are reclaimed.
- Signals created outside any scope (for example, global state at application startup, or a signal created inside an event handler) belong to no mount. They live until the thread ends. Signals each row of list data needs should be created in the row view, or by the mount that owns them.
- When `each` has duplicate keys, only the first is kept. Later duplicates are skipped.
- A value read directly with `count.get()` during view construction is a constant. It does not follow changes. To follow changes, pass the signal itself or a closure. This is the same rule as reading `.value` in Vue's `setup`.

## Store: nested state tracked per field

When anything inside a `Signal<App>` changes, every place that reads it reruns. `store(value)` keeps the value in one place. But each path is tracked on its own:

```rust
#[derive(Clone, Store)]            // the Store derive is behind the view-macro feature
struct Todo { id: u64, title: String, done: bool }
#[derive(Store)]
struct App { todos: Vec<Todo>, filter: String }

let app = store(App { todos, filter: String::new() });
let todos = app.todos().keyed(|t| t.id);            // the list locates rows by key
```

:::api

```rust view
use nana_ui::runtime::view;

todos.each(|t| view! {
    <Row gap=4>
        <Text>{t.title()}</Text>
        <Checkbox checked={t.done()}></Checkbox>
    </Row>
});
```

```rust rust
use nana_ui::runtime::view::{checkbox, row, text};

todos.each(|t| row().gap(4).children((text(t.title()), checkbox("").checked(t.done()))));
```

:::

```rust
todos.at(&7).done().set(true);     // only row 7's checkbox updates
app.todos().push(todo);            // the list gains a row; no existing row is reread
app.filter().set("done".into());   // affects only the places that read filter
```

- `#[derive(Store)]` generates a `<Name>StoreFields` trait. One accessor per field. It returns a `Subfield`. Wherever you use the accessors, import this trait. Only structs with named fields and no generics are supported.
- Path handles (`Store`, `Subfield`, `Item`) are `Copy` ids, like signals. They can be used directly as property values: `text(t.title())`, `.checked(t.done())`. They can also be bound two-way directly: `.model(t.done())`, `v-model={t.done()}` (`Writable<T>`; the write goes back through the path's `set`. It does nothing when the row has been deleted). The read and write methods `get` / `with` / `try_with` / `set` / `update` come from the `StorePath` trait.
- Each path has two triggers. Both are created only on the first tracked read. A path that has never been read occupies no signal. Reading a value (`get` / `with`) tracks deep. Iterating a list (`keyed(..).each` / `items()`, `len()`) tracks only shallow. Writing a path fires both triggers of the path itself and of every existing path below it, plus the deep trigger of each path above it.
- The list's own `push`, `insert`, `retain`, `swap`, and `sort_by_key` do not change the contents of any item. So they fire only the list itself and the deep triggers above it. They do not fire the rows. A whole-list `set` / `update` fires every row. Row bindings rerun and compare by field. If the value did not change, it stops there.
- Rows are located by a 64-bit hash of the key. After a reorder they still point at the same item. Two keys in the same list must not have the same hash. After a row is deleted, its triggers are released on the next lookup by key. `get` on a deleted row panics. `try_with` returns `None`.
- **Time travel**: `store_with_history(value, limit)` remembers earlier values. `undo()` / `redo()` / `travel(±n)` move back and forth. `can_undo()` / `can_redo()` / `steps()` (which row each step wrote) are tracked. They can be bound to buttons directly. All writes within one event handling (between two flushes) count as one step. Starting a step copies the whole value (requires `T: Clone`; the cost is proportional to the size of the value). Undo and redo write the whole value back. So only bindings whose value really changed update. Rows are kept by key. Writing after an undo discards the steps that could be redone.
- In `.vue`, `let x = store(…)` and props of type `Store` / `Subfield` / `Item` are treated as stores: bindings that use them are always tracked at runtime. They are never folded into constants. Writing the name alone (for example `:checked="done"`, where `done` is a `Subfield` prop) binds it directly.

## Async: tasks, `resource`, and `suspense`

A signal can only be used on the thread that created it. So async code that writes signals also runs on the UI thread:

```rust
let user = resource(move || id.get(), |id| spawn_blocking(move || load_user(id)));
```

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::text;

view! {
    <Suspense fallback={text("Loading…")}>
        <Text>{move || user.with(|u| u.map_or(String::new(), |u| u.name.clone()))}</Text>
    </Suspense>
}
```

```rust rust
use nana_ui::runtime::view::{suspense, text};

suspense(
    || text("Loading…"),
    move || text(move || user.with(|u| u.map_or(String::new(), |u| u.name.clone()))),
)
```

:::

- `spawn_local(future)`: runs a future on the UI thread's executor (no `Send` required; it can read and write signals directly). Waking its waker on any thread wakes the event loop through the host's wake hook. Call `poll_tasks()` on the UI thread. It is also polled once at the start of every frame (`take_system_work`). Tasks created while building a view belong to that view's scope. They are dropped when the view is unmounted. The returned `Task` can be `abort()`ed.
- `spawn_blocking(f)`: runs blocking work on a new thread. Returns a future of its result.
- `resource(source, fetch)`: `source` is tracked like an effect. Every change calls `fetch(source)` and drops the previous fetch if it has not finished. `get()` / `with()` read the latest result (`None` before the first completion; the old value is kept while reloading). `loading()` says whether it is loading. `refetch()` reloads with the current source. `set(v)` changes the value directly (optimistic update).
- `suspense(fallback, content)`: `content` is built right away but hidden until every `resource` created inside it has finished its first load. Meanwhile `fallback` is shown. Later reloads do not switch back to `fallback`. In templates write `<Suspense fallback={..}>`. In `.vue` use `<template #fallback>`.
- Host wiring: the `nana-ui` window host calls `set_task_wake` at startup, polls tasks while handling host work, and requests a redraw for windows with pending bindings. Without a host (tests, embedding), call `poll_tasks()` yourself.

## Stylesheets

A stylesheet is CSS. It uses the CSS subset in [Layout](layout.md). It is handled by the engine `nana-ui-css`. L3 views parse and match it **at build time**. The runtime only gets the result.

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <style>
        .todos { opacity: 1; transition: opacity 120ms ease-out; }
        .todos.empty { opacity: 0.6; font-size: "1.1em"; }
    </style>
    <Column class="todos" class:empty={list.with(Vec::is_empty)} gap=8>…</Column>
}
```

```rust rust
use nana_ui::runtime::view::column;

stylesheet! {
    mod todo_styles;
    .todos { opacity: 1; transition: opacity 120ms ease-out; }
    .todos.empty { opacity: 0.6; font-size: "1.1em"; }
}
column()
    .gap(8)
    .class(todo_styles::todos)
    .class_when(todo_styles::empty, move || list.with(Vec::is_empty))
```

:::

- **One path**: a template's `<style>` compiles into one `Sheet`. `class="a"` and `class:a={condition}` expand to `.class(..)` and `.class_when(..)`. This is exactly the same as `stylesheet!` plus hand-written calls. `stylesheet!` generates a `Class` constant for each class (`-` becomes `_`; `.todo-item` is `todo_styles::todo_item`). A misspelled class name is a compile error. The constants of `mod x;` are only for the parent module. `pub mod x;` is public. The classes of one element come from the same sheet.

- **CSS in `view!` is Rust tokens**: `<style>` goes first in the template. Write CSS directly inside it. Whitespace is restored from the tokens' positions in the source. So `.a.b` (compound) and `.a .b` (descendant) are not confused. Values that Rust lexing cannot express go in double quotes. At compile time the quotes are removed and the text is spliced in as is: `em` / `ex` units (`"1.5em"`; rustc reads `1em` as a float with a missing exponent and reports `expected at least one digit in exponent` before macro expansion), and hex colors where a digit is followed by `e` (`"#9ecafe"`). Single-quoted CSS strings use double quotes instead. `//` can only appear inside quotes. Strings inside `url("…")` and inside selectors keep their quotes.
- **Checks**: every warning lands on the piece of CSS it is about: unsupported selectors and at-rules land on the selector. Declarations with no matching Style Model field land on the declaration. Classes no rule uses land on the element's `class` attribute. In `view!` that is the token (where the squiggle shows in the editor). In `.vue` it is the file's line and column.
- `.vue`'s `<style>` is raw CSS text. It is not subject to the lexing limits above. Both styles compile to the same patches.

- **Containers of structural blocks**: the rows or branches of `each`, `when`, `dynamic`, and `each_virtual` are built in a container (`each` / `when` / `dynamic` use a column `Stack::column(0)`; `each_virtual` uses its `ScrollView`; with `.within(..)` it is the list itself). `each(..).container(element)` builds the rows directly into that element: it is the container, and the element's bindings, events, refs, and key work as before, while the list's own `gap`, `horizontal`, styles, and key are no longer used. Write it this way for components that take rows as their own children, for example a `ReorderList` with custom-painted rows (it takes each row's box from its own children): `each(ids, |id| id.clone(), row).container(widget(ReorderList::new(items).live_rows(true)))`. All children of that element are managed by the list; it cannot carry children of its own, and the component cannot have slots or assembly. This container accepts `.class(..)`, `.class_when(..)`, and `.css(..)` like an element: `each(items, key, row).class(s::strip)`. The style is composed onto the layout the container is built with. Conditional classes are bindings as usual. In templates, write it on the wrapping block: `<Block class="strip">` wrapping a `v-for` element or a `v-if` chain. It does only this. `class` / `class:name` on `<Virtual>`, `<Transition>`, `<TransitionGroup>`, and `<KeepAlive>` likewise lands on the container of the list or chain they wrap. The `class` on a `v-for` / `v-if` element itself still goes to each row and each branch. In a chain (`v-if` / `v-else-if` / `v-else`) only the outermost container carries these classes. A `teleport` anchor (the `Stack::column(0)` in which the content is built at the declaration site and stays when there is no target) also accepts `.class` / `.class_when` / `.css`. In templates write it on `<Teleport class="fill">`. For example, to let content that was not teleported fill the height. Containers also accept `.visible(..)` (`v-show`): `each_virtual(..).visible(shown)` hides the list's `ScrollView` (with `.within(..)`, the list itself) without tearing it down. Rows, scroll position, and reading position are kept. In templates write it on `<Virtual v-show={shown}>`.
- **Syntax**: `class="a b"` is fixed classes. `class:name="condition"` is a class present only while the condition is true (a Rust expression cannot express Vue's `{ active: x }` object syntax, so Svelte's form is used). The function API uses `.css(css! { padding: 12px; opacity: 0.8 })` to write a block of declarations for a single element. The rules are the same as `<style>`.
- **Compilation**: class selectors like `.a` and `.a.b`. The cascade is ordered by `!important`, specificity, and source order. Each rule's patch is the set of Style Model fields its declarations write and the values written (by field path, such as `align_items`, `paint.outline.width`). It is embedded in the program as JSON data. Which fields are written comes from `nana-ui-css`'s `written_layout`: the declarations are applied both to the default layout and to a witness layout in which no field has its default value. A field is written if either side changed and the witness side ends up at the same value as the default side. So writing back a default value (`align-items: flex-start`, `position: static`) is also in the patch, while fields whose new value depends on the old one (physical margins re-derived from the existing logical margins after `direction` changes) do not count. Patches replace fields wholesale. Enums and `Option` are not merged key by key with the element's existing value. `var()` is evaluated at build time from the stylesheet's own custom properties. The stylesheet becomes a table of "required classes → patch", ordered by the cascade.
- **Runtime**: does not parse CSS. There is no CSS code in `nana-ui-runtime` either. When an element first appears with a given set of "fixed classes + conditional classes", the stylesheet picks from the table the rules this set of classes might match (comparing class numbers). After that, the same set of classes reuses this directly. It then composes once from "base layout + currently active conditional classes". The result is one shared layout. All instances with the same set of classes (including each row of a `v-for`) hold only a reference to it. The conditions of conditional classes are ordinary bindings. When they change, a different composed result is swapped in.
- **`@container`**: class rules inside an `@container [name] (condition)` block are compiled into the table too, ordered in the cascade by source position. Each carries its query, embedded in the program as data: which container to read (the nearest one with that name; without a name, the nearest one qualified for that axis), which axis to read, and the interval on that axis where it holds. `max-width: 480px` includes 480, so the interval's upper bound is `480f32.next_up()`. Composition skips these rules. Under the same "base layout + active conditional classes", the boundaries they use become breakpoints, cut into at most 17 buckets. Each bucket is composed once in cascade order, so ordinary rules written later still win. The fields written beyond the composition without container rules are that bucket's variant. The result is one `ResponsiveRule` (`ResponsiveContainer::Nearest { name }`), handed to the Runtime in the commit that builds the node. When conditional classes change which rules apply, a different one is swapped in; the same set of classes gets the same cached `Arc`. The Runtime measures container sizes; the view layer does not. Containers are declared with `container-type`, `container-name`, or the `container` shorthand, which are ordinary fields. At any moment an element follows one axis of one container, with at most 16 breakpoints. When the container rules a class brings in ask about two containers or two axes, or exceed 16 breakpoints, the template and `.vue` warn on that element; at runtime none of the container rules under that set of classes apply, and a `runtime.view.container_query_unsupported` diagnostic is recorded. Switching buckets does not play `transition`. `css!` is just a block of declarations; an `@container` inside it warns and is ignored: write it in `<style>` or `stylesheet!`.
- **`transition`**: compiles into an implicit animation (`El::animate`). When a binding changes `opacity`, `transform`, `width`, `height`, or `background`, it plays on the compositor track from the currently displayed value to the new value. The logical style takes the new value directly. Only `transition` on an element's fixed classes is honored.
- **Not compiled, and warned about**: other selectors (tag, id, combinators, attributes), `:hover` / `:focus` / `:active`, `@media`, `@keyframes` and `animation`, `@font-face`, pseudo-elements, declarations with no matching Style Model field, classes on an element that no rule uses, bound `:class`, `@container` queries the engine does not evaluate (`style()`, `scroll-state()`, `aspect-ratio`, `orientation`, units other than px, two axes in one query, nested `@container`), and `:hover`, `transition`, `@media`, and pseudo-elements inside `@container`.
- **Theme colors**: stylesheet colors are evaluated against the light theme at build time. So colors that change with the theme are written as semantic roles: every element has `.foreground(..)`, `.background(..)`, `.border(..)` (`SemanticColorRole`) and `.radius(..)` (`RadiusTier`). They can be bound. In templates they are attributes of the same name. For example `<Text foreground={SemanticColorRole::Muted}>`.
- **Writing back defaults**: a class overrides the values the element was built with. That includes writing back defaults: `Stack::row` centers by default. Writing `align-items: flex-start` in a class returns it to top alignment. `column()`'s height follows its content. Writing `height: auto` in a class makes it `auto`.
- **Known trade-off**: colors are evaluated against the light theme at build time. For colors that follow theme switches, use the components' built-in semantic colors.

## Clamp notification

A `Text` with a line limit (`line-clamp`) knows after layout whether it cut off lines. Text that listens for `TextClamped` receives `TextClamped { clamped }` when this changes (it is decided only in the post-layout text layout; the measurement before that has no width to cut at). An "Expand" button can then appear only when there really is more:

:::api

```rust view
use nana_ui::runtime::view;

let clamped = signal(false);
view! {
    <Column>
        <Text class={s::body} on:TextClamped={move |e: &TextClamped| clamped.set(e.clamped)}>{body}</Text>
        <Button v-show={clamped}>"Expand"</Button>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{button, column, text};

let clamped = signal(false);
column().children((
    text(body).class(s::body).on(move |e: &TextClamped| clamped.set(e.clamped)),
    button("Expand").visible(clamped),
))
```

:::

## Container size

To choose an arrangement by the width a container actually gets (two columns side by side if they fit, otherwise stacked), listen for its `SizeChanged { width, height }`: it is sent after the layout commit when the node's box **changed size** (first layout, window resize, a sibling grew and squeezed it). It is sent only to the node that listens for it. A layout that does not change the size sends nothing. Like `ScrollViewportChanged`, it arrives after layout. The signal change takes effect on the next frame. You do not need to read the layout box every frame.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::dynamic;

let wide = signal(false);
view! {
    <Column on:SizeChanged={move |e: &SizeChanged| wide.set(e.width >= 720.0)}>
        {dynamic(wide, |wide| if *wide { two_columns() } else { stacked() })}
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{column, dynamic};

let wide = signal(false);
column()
    .on(move |e: &SizeChanged| wide.set(e.width >= 720.0))
    .children(dynamic(wide, |wide| if *wide { two_columns() } else { stacked() }))
```

:::

For arrangements that only need wrapping (side by side when they fit), prefer `flex-wrap` with `flex-basis` (see [Layout](layout.md)). No signal is needed.

## Localized text

`t(..)` builds a text node that says a message. The message is resolved in the locale of the scope the node is in. `.locale(..)` makes an element a locale scope. For message catalogs, fallback, formatting, and the cost of switching, see the "Localized text and locale scopes" section of [Text engine](text-engine.md).

:::api

```rust view
use nana_ui::runtime::view;

let count = signal(3u64);
view! {
    <Column>
        <T id="title" />
        <T id="files" count={count.get()} />
        <Column locale="ar">
            <T id="title" />
        </Column>
    </Column>
}
```

```rust rust
use nana_ui::runtime::LocalizedText;
use nana_ui::runtime::view::{column, t};

let count = signal(3u64);
column().children((
    t(LocalizedText::new("title")),
    t(move || LocalizedText::new("files").arg("count", count.get())),
    column().locale("ar").children(t(LocalizedText::new("title"))),
))
```

:::

- **`t(message)`** accepts a `LocalizedText` constant, a signal or `Computed` of one, or a closure that builds one. A constant is written when the node is built. A closure recomputes when a signal it read changes. When the computed message (key and arguments) equals the one displayed, nothing is sent. On a locale switch the world re-resolves directly. No binding runs. The field it binds is `fields::Localized`.
- **`<T>` in templates**: `id` is the message key. All other attributes are message arguments. `key`, `ref`, `labelled_by`, `locale`, `class`, `decorative`, and the theme color attributes (`foreground` and so on) still go to the node. When everything is a literal, it is a constant: `<T id="title"/>` is `t(LocalizedText::new("title"))`. When there is an expression, the whole message is one binding. Arguments are values: in `view!` a signal is written `count={count.get()}`. Paths and fields are cloned by reference, so the value is still usable elsewhere. `<T>` has no children.
- **`.locale(..)`** (in templates `locale="ar"`, `locale={sig}`): the element becomes a locale scope. Localized text below it (including itself) resolves in this locale and is laid out in its language. The subtree is arranged in its direction. `None` returns to the enclosing scope. A constant (`Locale`, or a language tag such as `"ar"`) is set in the commit that builds the node, before children are inserted. It creates no effect. The first value of a signal or closure is set the same way. After that it follows. A value equal to the scope's current locale is not committed. Literal text is unaffected. `lang` is still only the layout language of literal text. Templates check the spelling of language tags at build time.
- The Runtime keeps only one structural binding per node. So when `.labelled_by(..)` and `.locale(..)` both follow signals, they share it: when either changes, both are reread. That costs only one extra comparison.
- `locale` can only be written on elements. `<Block>`, `<Transition>`, `<TransitionGroup>`, `<KeepAlive>`, `<Virtual>`, `<Suspense>`, `<Teleport>`, and `<ErrorBoundary>` are not elements. Writing `locale` on them is a compile error. Write it on an element inside, or wrap them in a `<Column locale="ar">`. `locale` on a component tag is an ordinary prop.

## Enter, leave, and move animations

`when(..)` and `each(..)` (including a `Store`'s `keyed(..).each`) can take `.transition(t)`. They correspond to Vue's `<Transition>` and `<TransitionGroup>`:

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Transition name="fade" duration="150">
        <Block v-if={open}>{panel()}</Block>
    </Transition>
    <TransitionGroup name="slide-up" duration="180" move="200">
        <Block v-for={t in items} key={t.id}>{row(t)}</Block>
    </TransitionGroup>
}
```

```rust rust
use nana_ui::runtime::view::{each, when};

when(open, panel).transition(Transition::fade(ms(150)));
each(items, |t| t.id, row).transition(Transition::slide(0.0, 12.0, ms(180)).moves(ms(200)));
```

:::

- **Enter**: new rows and new branches play from the given opacity and transform to the node's own values. Rows that already exist at mount do not play.
- **Leave**: removed rows or branches are not destroyed right away. Their scope is reclaimed immediately (they no longer follow the data). The node stays in place. It takes no part in hit testing. Focus moves away. It is destroyed only after the leave animation finishes.
- **Move** (`.moves(duration)`): rows whose position changed slide over from where they were (FLIP). Insertions, deletions, and reorders all count. So does a leaving row finally being destroyed and the rows after it closing the gap.
- Showing and hiding a resident node (a card, the contents of a tool window) uses the same `Transition`, handed to `PresenceLifecycle`; see [Runtime and Scene](runtime-scene.md).
- Presets: `Transition::fade`, `slide(dx, dy, duration)`, `scale(ratio, duration)`. For custom ones use `Transition::new().enter(Presence::new(duration).opacity(0.0).translate(0.0, 8.0)).leave(..)`. `.ease(e)` sets one easing for everything; `Presence::ease_transform(e)` gives translation its own curve. `without_enter()` / `without_leave()` keep only one half.
- Everything goes through the node's compositor track. Nothing is written back to the logical style. The end of a leave is driven by the completion event of `advance_animations`. Moves start after the layout stage and before extraction in the same frame. So nothing flashes to the new position first.
- The old and new branches of `when` exist at the same time: the old branch leaves in place. The new branch enters right after it. With `.moves`, the new branch slides up smoothly as the old one disappears. Vue's `mode="out-in"` (leave first, then enter) is not available yet.
- Virtual lists (`each_virtual`) are not supported: rows scrolled out of the viewport must be reclaimed immediately anyway.
- **Reduced motion**: when `AppContext::reduced_motion()` is true, enter, leave, and implicit property transitions (`El::animate`, stylesheet `transition`) do not play: values land on their end state immediately, leaving rows are destroyed on the next advance, and moves do not slide. Logical values are committed as usual, and the `Finished` they would have reported is still delivered with the next advance, so code waiting on it proceeds as usual. The managed runtime calls `set_system_reduced_motion` to sync when it builds a window document and when the system setting changes; an application's own setting uses `set_reduced_motion(bool)` to override the system, and `follow_system_reduced_motion()` returns to following the system. Animations an application starts itself with `queue.node(id, now).transition()` or `Timeline` do not see this switch; write their duration as `cx.motion_duration(duration)`: under reduced motion it is zero, the animation settles immediately, and `Finished` is still delivered. Among framework controls, an indeterminate `Progress` stops sweeping and shows a full faint fill instead; when the preference changes, progress bars on screen stop or resume sweeping immediately.
- Templates: `<Transition name="fade" duration="150">` wraps a `v-if` / `v-else-if` / `v-else` chain. `<TransitionGroup duration="150" move="200">` wraps a `v-for` element (the two tags are interchangeable). `name` can be `fade`, `slide-up`, `slide-down`, `slide-left`, `slide-right`, or `scale`. The default is `fade`. Durations are in milliseconds. `:transition="value"` passes a `Transition` directly.

## Slots of composite controls

Composite controls such as `DesktopShell` and `SidebarSection` receive application content by node id. A view uses slots to hand them a piece of view. There is no need to build nodes first and then fetch their ids:

In templates, write `<template #name>`. It expands to the same method calls.

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Widget of={DesktopShell::from_model(model).title("Gallery")}>
        <template #title-trailing><Row gap=6>…</Row></template>
        <template #navigation>{sidebar()}</template>
        <template #primary>{page()}</template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{row, widget};

widget(DesktopShell::from_model(model).title("Gallery"))
    .title_trailing(row().gap(6).children((search, theme)))   // named slot; underneath it is .slot(..)
    .navigation(sidebar())
    .primary(page())
```

:::

- **`.slot(view, |c, id| c.xxx(id))`**: the control places the slot itself (the shell's regions, the title bar's three columns). The slot view is built first and not inserted anywhere. Then its root node id is written into the component. Finally the component itself is built.
- When a `.slot` root has a key, it is not registered as a keyed child of any node (where it goes is up to the control, and `resolve_assembly_path` cannot find it), but `assembly_path` still counts this key into the path and then continues with the parent it actually sits under: the path of the input in `settings_row("Name").control(text_input().key("name"))` ends with `/name`, so automation and tests can identify it by path.
- **`.child_slot(view, write)`**: the slot is this node's own child (a section's header, a list item's icon). It is inserted before `.children(..)`, in slot declaration order. The slot root keeps the key written in the view and is registered under this node: after `list_item(..).leading(icon.key("icon"))`, `resolve_assembly_path(item, "icon")` finds it. Without a key it is named by position.
- A slot must have exactly one root node. Otherwise the mount fails and returns `InvalidInput`. Nothing is committed that time. The slot and the component are built in the same commit. Signals created in a slot belong to the same mount scope. They are reclaimed together on unmount.
- **Assembly**: controls that need assembly register their `assemble_*` in `TypeBehavior::slot_assembler`. The view layer runs assembly automatically after building such a node and after a binding changes it. So views do not call `assemble_desktop_shell`. Composite controls built by hand with `create_component` still have the caller call `assemble_*`: assembling on every write would dirty an idle world. See `TypeBehavior::assembler`. Within one view, a composite control is assembled only after its slots and children have been assembled. So its patches to the content (for example, `DesktopShell`'s corner radius on the primary region content) are not overwritten by the content's own assembly. Leaf composite controls whose children are determined only by their own properties (`Chip`, `ColorField`, `PathField`, `FileTab`, `DiffView`, `MediaTransportBar`) register `TypeBehavior::assembler`: it already runs after every write. The view layer also runs it once when building such a node. So they need not also register a `slot_assembler`.
- **Initial focus**: `.initial_focus_on(entity_ref)` on `Dialog`, `Drawer`, and `ConfirmDialog` focuses a control in a slot when it opens. For example `.confirm(button("Quit").entity_ref(accept)).initial_focus_on(accept)`. The slot is built before the panel, so its id is readable here. A target that has not been built yet leaves the panel's default focus.
- **`detached(view)`**: builds nodes but places them nowhere. Get ids with `entity_ref` / `with_refs`. Hand them to interfaces that take content by id (`set_modal_slots`, `set_form_field_control`). Put it in the same tuple as the other roots. One mount, one commit: `with_refs((detached(body.entity_ref(b)), widget(dialog).entity_ref(d)), (b, d))`.
- A slot's nodes do not change for the whole lifetime of the view: when the content inside must change, use `when` / `each` / `dynamic` inside the slot. Whether a region is shown or hidden is decided by `WorkspaceModel`.
- Currently registered for assembly: `DesktopShell`, `AppShell`, `AppTitleBar`, `Workspace`, `Dock`, `SplitPane`, `PaneSection`, `GraphCanvas`, `DatePicker`, `NativeMarkdown`, `ConfirmDialog` (`.title_icon` / `.body` / `.footer` / `.close_action` / `.cancel` / `.secondary` / `.confirm`; cancel and confirm buttons that are not given are generated from `cancel_label` / `confirm_label`), `PaneChrome` (`.tabs` / `.action(action, view)` / `.body`; it builds its header row itself; when `.header(view)` supplies its own header, it only sets styles and leaves the children inside alone), `SettingsRow`, `SegmentedControl` (and its `SegmentedOption`), `SidebarSection`, plus the settings page's `AppearanceSection`, `AboutSection`, `SettingsSidebar`, `SettingsCollapsibleCard` (`.summary` / `.details` / `.accessory`), and `SettingsPage` (`.content`). The `assembler` of leaf composite controls (`Chip`, `ColorField`, and so on) also runs when the view is built. Named slot methods: `SplitPane`'s `.first` / `.second`, `PaneSection`'s `.header` / `.tabs` / `.body` (`view/panes.rs`). `DesktopShell`'s regions, `AppShell`'s `.title_bar` / `.body` / `.overlay`, `Workspace`'s `.region(id, view)`, `AppTitleBar`'s `.leading` / `.center` / `.trailing` (`view/shell.rs`; the title bar also accepts `.visible(..)` and `.class(..)`; hiding the window title bar in fullscreen is `.visible(false)`). `SidebarSection`'s `.tools`, `SidebarFrame`'s `.top` / `.body` / `.footer` (`view/sidebar.rs`). Settings rows' `.control` / `.label_mark` / `.label_accessory` (`view/settings.rs`). `ListItem`'s `.leading` / `.after_label` / `.content` / `.trailing` (plus unrestricted `.children(..)` between leading and trailing) and `SidebarRow`'s `.leading` / `.content` / `.trailing`. The modal panels `Dialog` and `Drawer`'s `.title_icon` / `.title_content` / `.body` / `.footer` / `.close_action` (`.title_content` stands in the title row in place of the title text; the title is still the accessible name). `FormField`'s `.control`. And `MediaTransportBar`'s `.leading` / `.trailing` / `.secondary` / `.settings`: they put the application's controls into its own button groups, second row, and settings menu. `Popover` and `ActionMenu`'s `.trigger`: what the trigger shows (icon, text, count). The popover is still the same control. `.children(..)` are the panel's items (`view/composites.rs`). Every composite control that takes application content by id now has named slots. `ReorderItem::tools` is not a slot. It is a hit-exclusion area of one row in the list data. Get the id with `entity_ref` and write it in `.bind`.
- **Dock panels**: panels correspond to ids in the layout. Their number is decided by data. So they do not use named slots, but child keys instead: `widget(Dock::new(layout)).children((files().key("files"), preview().key("preview")))`. In templates write `<Widget of={dock}><FilesPanel key="files"/>…</Widget>`. During assembly each panel id looks for the child with the same key as its content. When a binding replaces the layout, they are matched again by key. Panel nodes are not rebuilt. Children whose key matches no panel are parked.
- **Sidebar sections**: `widget(SidebarSection::new("Resources").count(2).collapsible(true)).tools(…).children(rows)`. The section builds its own header (expand button, title, count) and body. The application's rows (including `each` lists) are moved into the body. `tools` sit in the header and show on hover. The resulting structure is identical node by node to a section assembled by hand. `SidebarFrame`'s `.body(..)` automatically puts the content into its vertical scroll area. Write slots in the order top, body, footer.
- **Segmented choice**: `segmented().children((segmented_option("Face capture").selected(..).on_select(..), …))` (`view/selection.rs`). Options are the control's children. `label`, `disabled`, and `selected` can be bound. The control registers options and the selected one from its children and their `selected` flags (`assemble_segmented_control`). A change to any option's binding also re-registers them. When the user selects an option, the control first sends `SegmentedSelectionRequested`, then sends `SegmentedOptionChosen` on that option. `.on_select` listens for the latter. Options to be hidden stay as children and are disabled at the same time.
- **Settings rows**: `settings_row(label).hint(..).divided(..).control(switch(""))`. The row builds its own label and hint (`assemble_settings_row`). The structure is identical node by node to a row built by `mount_settings_leaf_row`. `label`, `hint`, `divided`, `stacked`, `first_in_group`, and `last_in_group` can all be bound. `.visible(..)` controls visibility. Which row is first or last in a group is computed by the application from its own data and then bound. The row label is also the control's accessible name: when the control has no name of its own (`switch("")`, sliders, dropdown selects), the screen reader reads the row label. `.label_accessory(view)` puts something like a status pill after the label on the same line. See [Controls](components.md).

## `view!` and the Rust style

The `view-macro` feature provides `view!` (`nana_ui::runtime::view!`). It is the counterpart of Vue's template syntax. It expands to the calls of the Rust style. It adds no runtime concepts. The macro lives in the separate proc-macro crate `nana-ui-view-macros`. Without the feature it does not take part in the build.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn todos() -> impl IntoView {
    let draft = signal(String::new());
    let list: Signal<Vec<Todo>> = signal(Vec::new());
    let add = move || { /* … */ };
    view! {
        <Column gap=8>
            <TextInput placeholder="New task" v-model={draft} />
            <Button disabled={draft.with(|d| d.trim().is_empty())} @activate={add}>"Add"</Button>
            <TodoRow v-for={todo in list} key={todo.id} todo={todo} list={list} />
            <Text v-if={list.with(Vec::is_empty)}>"No tasks yet"</Text>
            <Text v-else>{format!("{} items", list.with(Vec::len))}</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn todos() -> impl IntoView {
    let draft = signal(String::new());
    let list: Signal<Vec<Todo>> = signal(Vec::new());
    let add = move || { /* … */ };
    column().gap(8).children((
        text_input().placeholder("New task").model(draft),
        button("Add")
            .disabled(move || draft.with(|d| d.trim().is_empty()))
            .on_activate(add),
        list.each(|todo| todo.id, |todo| todo_row(todo, list)),
        (move || list.with(Vec::is_empty))
            .then_show(|| text("No tasks yet"))
            .otherwise(|| text(move || format!("{} items", list.with(Vec::len)))),
    ))
}
```

:::

| Template | Rust style (that is, the expansion) |
| --- | --- |
| `<Column gap=8>…</Column>` / `<Row>` | `column().gap(8_f32).children((…))` / `row()…`; numeric literals get a type suffix, expressions are passed as is |
| `<Text>"Count: {count}"</Text>` | `text!("Count: {count}")`; a string without `{…}` is `text("…")` |
| `<T id="files" count={n.get()} />` | `t(move \|\| LocalizedText::new("files").arg("count", n.get()))`; when everything is a literal it is a constant: `<T id="title"/>` is `t(LocalizedText::new("title"))` |
| `<Button>"Add one"</Button>`, `<Checkbox>` | `button("Add one")`, `checkbox(…)` |
| `<Slider min=0 max=1 step=0.05/>`, `<TextInput/>` | `slider(0_f64, 1_f64, 0.05_f64)`, `text_input()` |
| `<Widget of={component}>…</Widget>` | `widget(component).children((…))` |
| `<TodoRow list={list} todo={t}/>` (other tags) | `todo_row::Props::__new().list(list).todo(t).__build()`, that is, `todo_row(t, list)`: the tag name is converted to snake_case and resolves to the function of that name marked `#[component]`; attributes go to the parameters of the same name in any order, children go to `children`, an attribute written by name only is `true`, and parameters with `#[prop(default)]` / `#[prop(default = expr)]` may be omitted (see "Component props" below) |
| `<kit::EmptyState title={t}/>` (tag with a path) | `kit::empty_state::Props::__new().title(t).__build()`: a tag with a path always calls a function, even if its last segment matches a built-in tag. Without a path, built-in tags win: the macro cannot see functions in scope, so a custom component with the same name is shadowed by the built-in control, without an error if the attributes happen to fit. When a custom component shares a name with a built-in tag, write it with a path (`self::` within the same module) |
| `name="x"`, `name=3`, `name={x}`, `name={a.b}` | passed as is: a constant, or the signal itself |
| `name={other expression}` | `move \|\| expression`: recomputed when a signal it read changes |
| `name={\|\| …}` | the closure is passed as is |
| `@activate={expression}` | `.on_activate(move \|\| { expression; })`; `@activate={add}` passes the function value directly |
| `on:RangeChanged={\|e: &RangeChanged\| …}` | `.on::<RangeChanged>(…)`; written as a three-argument `\|component, event, cx\|` closure it is `.on_cx::<…>(…)` |
| `v-if` / `v-else-if` / `v-else` (sibling nodes) | `when(…).otherwise(…)`, with `v-else-if` nested inside `otherwise`; by hand you can also write `cond.then_show(…)` |
| `v-for={pat in items} key={…}` | `each(items, move \|item\| { let pat = item; key }, move \|pat\| element)`; by hand you can also write `items.each(key, row)` |
| `<Block class="a">` wrapping a `v-for` element or a `v-if` chain | `each(..).class(s::a)` / `when(..).class(s::a)`: the class goes to the structural block's container |
| `v-show={x}`, `v-model={sig}`, `key="x"` | `.visible(x)`, `.model(sig)`, `.key("x")` |
| `locale="ar"`, `locale={sig}` | `.locale("ar")`, `.locale(sig)`: the element becomes a locale scope |
| `<style>…</style>`, `class="a"`, `class:a={c}` | `stylesheet! { mod s; … }`, `.class(s::a)`, `.class_when(s::a, c)` |
| `<template #navigation>…</template>` (named slot) | `.navigation(…)`: the element's method of the same name; `#title-trailing` is `.title_trailing(…)`; `#default` is ordinary children |

With more than 12 children, the macro nests them into several levels of tuples. Mistakes are pointed out precisely. For example: an unclosed tag, `</Row>` closing `<Column>`, `v-else` without a preceding `v-if`, `v-for` without `key`, an unknown directive, `v-if` and `v-for` on the same element. `tests/view_macro.rs` writes the same page once as a template and once as hand-written function calls. The retained trees after mounting are identical node by node. They are still identical after a signal changes and flushes.

### Component props

A component in a template is a function marked `#[component]` (`nana_ui::runtime::view::component`). The macro cannot see the function's definition when it expands, so `#[component]` generates next to the function a builder that takes arguments by name, placed in a hidden module with the same name as the function (`todo_row::Props`). When you `use` the function, the module comes along. The function itself is unchanged; in Rust you pass all arguments in order as usual.

```rust
#[component]
fn switch_row(
    label: &'static str,
    #[prop(default = "")] hint: &'static str,
    checked: impl IntoProp<bool>,
    #[prop(default = false)] disabled: impl IntoProp<bool>,
    on: fn(bool) -> Action,
) -> impl IntoView { /* … */ }

view! { <SwitchRow on={Action::Mute} label="Mute" checked={muted} /> }
```

- Each attribute's setter takes the parameter's own type (an `impl Trait` parameter takes values that satisfy the same bounds), so type inference for literals and closures works as in a direct call.
- Defaults are written next to the function, and names resolve in the function's module. `#[prop(default)]` is `Default::default()`. The default of an `impl Trait` parameter is written as an expression, for example `#[prop(default = || true)] shown: impl Fn() -> bool`. Attributes whose type uses the function's own generic parameters cannot have defaults.
- A missing required attribute reports `` `<SwitchRow>` needs `on` ``, pointing at the tag. An attribute that does not exist is `no method named …`, pointing at the attribute. Writing the same attribute twice, or giving `children` both as an attribute and as child nodes, are template errors. When a function without `#[component]` is used as a tag, the compiler says it cannot find the module of that name.
- Parameters must take ownership or be `'static` values: the template hands them over once when it builds. A parameter cannot be named `key` (`key` on a component tag lands on the first root node), and cannot start with `__`.
- Views compiled in the same `.vue` batch still have their arguments ordered by prop name by the `.vue` compiler, which calls the function directly; Rust function components in `.vue` templates need `#[component]` just like in `view!`.

Inside the macro only three kinds can be told apart statically: constants (literals), direct values or signals (paths, fields), and closures (other expressions). The "static dependencies" kind requires knowing which identifiers are signals. Only the [`.vue` dialect](#advanced-the-vue-dialect) compiler, which sees the whole script, can do that.

## Advanced: the `.vue` dialect

For everyday views, use `view!` or the Rust style. `.vue` files suit two cases: migrating from Vue and converting file by file side by side, or wanting the template, script, and styles in separate files. The code it compiles to goes through the same code generator and the same Rust calls as `view!`. It has no extra capabilities. What it adds is that the compiler sees the whole script, which enables the dependency analysis and constant folding below. The cost is that the script and template expressions get no rust-analyzer support.

Views can also be written as `.vue` files: the template is Vue syntax. Rust goes in `<script setup lang="rust">`. `nana-ui-sfc` compiles them into ordinary Rust functions at build time. The template part shares one code generator, `nana-ui-view-codegen`, with `view!`.

```vue
<!-- views/TodoList.vue -->
<script setup lang="rust">
let draft = signal(String::new());
let list: Signal<Vec<Todo>> = signal(Vec::new());
let add = move || { /* … */ };
</script>

<template>
  <Column :gap="8">
    <TextInput placeholder="New task" v-model="draft" />
    <Button :disabled="draft.with(|d| d.trim().is_empty())" @activate="add">Add</Button>
    <TodoItem v-for="todo in list" :key="todo.id" :todo="todo.clone()"
              @remove="list.update(|l| l.retain(|t| t.id != todo.id))" />
    <Text v-if="list.with(Vec::is_empty)">No tasks yet</Text>
    <Text v-else>{{ list.with(Vec::len) }} items</Text>
  </Column>
</template>
```

```rust
// build.rs
fn main() {
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime").build("views") {
        panic!("{error}"); // views/TodoList.vue:20:34: …
    }
}

// src/lib.rs
pub mod views {
    use crate::model::Todo;
    include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
}
```

- `TodoList.vue` compiles to `pub fn todo_list(…) -> impl IntoView`. `defineProps!(todo: Todo, on_remove: impl Fn() + Send + 'static)` declares the parameters.
- Attribute syntax matches Vue: `name="x"` is a string. `:name="expression"` is a Rust expression. `{{ expression }}` interpolates with `Display`.
- Components compiled in the same batch match arguments by prop name, not by order: `@remove` corresponds to the `on_remove` callback. Children correspond to the last prop named `children`. Missing and extra arguments are compile errors.
- Named slots: the child component declares a view parameter such as `header: impl IntoView`. The template places it with `<slot name="header"/>` (`<slot/>` places `children`). The parent writes `<template #header>…</template>`. `#default` equals the remaining children. Slot content is a view passed by value. It can be placed only once. There is no fallback content.
- `ref="name"` writes the element's node id into `let name = node_ref();` in the script. `on_mount(move |cx| …)` runs after the view enters the tree. Use it to focus or read layout. `labelled-by="name"` makes the control use that node's text as its accessible name (`aria-labelledby`).
- `key` on a component lands on the component's first root node (`keyed`). Tags not in this batch call the `#[component]` Rust function of the same name under the `view!` rules (arguments by attribute name; see [Component props](#component-props)).
- `<T id="files" :count="n" />` is `t(..)`. Arguments are classified like `{{ }}` interpolation: a lone signal is read as `n.get()` and declared as a dependency. A message that reads only constants is written as `Fixed(..)` and written once. `count="3"` is a string argument; for a number write `:count="3"`. `locale="ar"` is checked at build time. `:locale="sig"` is a binding like any other attribute.
- A built-in tag always builds the built-in one. A view in the same batch with the same name (for example `T.vue`) is still a function Rust can call (`views::t(..)`), but writing its tag in a template is a compile error; it is not silently replaced by the built-in.
- `<style>` (with or without `scoped`, it is the same: it always applies only to this component) is compiled at build time. See "Stylesheets" above. Templates still follow Rust's ownership rules. For example, when the same value is passed to a component and also used by an event closure, write `todo.clone()`.
- The generated code is formatted with prettyplease and written to `$OUT_DIR/nana_views.rs`, together with a versioned `$OUT_DIR/nana_views.map.json`. rustc's native JSON diagnostics still point at the generated file; feed the diagnostics line by line to `nana-sfc-remap <nana_views.map.json>` and the primary/child spans point back to the `.vue` file, line, and column. Errors in the template and script themselves (syntax, mismatched tags, missing `key`, missing arguments, computed cycles) are reported at build time, with file, line, and column.

Hot reload during development: in the build script write `Compiler::new(..).hot(debug)`. When you change only static text in a `.vue` file, `nana-ui-dev`'s `watch_templates` sends the new text into the running window. No rebuild. Any other change rebuilds as usual. See `docs/reference/hot-reload.md`.

### What the compiler can see

The compiler sees the whole component. So it knows which names are signals and how each signal is used. Dependency analysis scans tokens in the script and template:

- Calling `get` / `with` counts as a read. `set` / `update` count as writes. Any other use (passed out as an argument, captured by value by a closure, calling some other method) counts as "passed out".
- Calls whose insides it cannot see (free functions, macros other than `format!`, calling `get` on an unknown receiver) make the expression "dynamic".

The scan cannot see variable shadowing. So every uncertain case is counted toward "do not optimize".

| Analysis result | What the compiler does |
| --- | --- |
| The signal is never written and never passed out | Folded into `constant(…)`: the `Copy` handle and read API are unchanged, but reads create no dependency, and as a property it is written only once |
| A computed reads only folded constants | Also folded: `constant((f)())`, computed only once; folding can chain |
| A binding reads only constants or plain values | Wrapped in `Fixed(value)`, written once when the node is built, no effect created |
| A binding reads only some signals through code the compiler can see (static dependencies) | Wrapped in `__checked(location, [deps…], closure)`: in debug builds every run checks "signals actually read ⊆ declared dependencies"; a mismatch reports the `runtime.reactive.static_deps_mismatch` fault and is counted in `ReactiveStats::static_deps_mismatches`; in release builds the closure is returned as is |
| Anything else | Dynamic: tracked at runtime |
| A cycle between computeds | Compile error |
| A signal is written but nobody reads it / a `watch_effect` writes a signal it reads | Warning via `cargo:warning` |

Each component's signals (reads, writes, passed out, folded or not) and each binding's location, classification, and dependencies are all written to `$OUT_DIR/nana_views.deps.md`.

Static dependencies are currently **only verified in debug. They are not used to skip runtime tracking**: rerunning a node whose "value did not change" costs about 70 ns in total. Dependency tracking is only part of that. Release correctness is not staked on a token scan. The tests of `examples/reactive-sfc` assert that the number of verification failures is 0.

## Control-level tree shaking: `BuiltinComponents::Typed`

`AppContext::new()` registers all 91 built-in controls. Each registration carries a binder that "constructs this control from tag and attributes" and a reprojection function. So whether or not the program uses them, the code of every control is linked in. Programs that create controls only from Rust types (`mount`, declarative views, `.vue`) do not need these binders. They can switch to the lean mode:

```rust
impl ApplicationState for App {
    const BUILTINS: BuiltinComponents = BuiltinComponents::Typed;
    // …
}
// Without a host: AppContext::typed(), RuntimeDocument::typed(document)
```

- Built-in controls register only their **identity** (type name, tag, TypeId). Nodes still carry the component type stamp as usual. Type names can still be resolved from tags.
- Each type's behavior is written in `ComponentView::BEHAVIOR` (`TypeBehavior`: activation, assembler, activation at a point, dismissing popups, maintenance after writes, input routing hooks `TypeHooks`). It is installed when the first node of that type is created. Every creation path goes through this step (`stamp_component_type`, `install_view` for composite controls, `finish_semantic_binding` for binding by tag). For types the program never created, none of this code is referenced anywhere. Input routing calls subsystems through `hooked!`. When a hook is not installed, the result is the same as when the document has no such node.
- The central paths (pointer and key dispatch, commit, the frame loop's animation step, focus and hover, accessibility actions) name no method that belongs to one control. They write `via!(self.method(..))`, which asks the table of routed calls (`TypeHooks::calls`, a `RoutedCalls`); each entry is named after the `AppContext` method it stands for. A type calls its own function in `framework/routes.rs` from `BEHAVIOR.hooks`, which fills the entries it needs with `route!`. An entry not filled answers `Absent` (`Ok(false)`, `None`, `Ok(())`, an empty list). That is what the method itself answers for a document without such a node. A new central call into one control takes three steps: an entry in `routed_calls!`, the method in that type's function in `routes.rs`, and `via!` at the call. A host aligns its title bars with the maximized state through `AppContext::sync_title_bars_maximized` and no longer names `AppTitleBar` itself.
- Two tests in `routes.rs` hold the table: every entry has a type that brings it (or that feature is dead for every program); and a form of text, an input and a button brings none.
- Measured (2026-11-11, `examples/reactive-sfc`, fat LTO, `codegen-units = 1`, `panic = "abort"`, stripped, aarch64 macOS): the minimal window 16.52 MiB → 15.80 MiB. Code whose symbols name a control type the program never creates 550 KiB → 61 KiB. What is left is mostly the chart pipeline in the painter (15.6 KiB) and the tooltip (4.5 KiB: any control can carry one, so it cannot be cut away).
- The mode must be fixed at compile time: `ApplicationState::BUILTINS` is an associated constant. The host chooses by the constant. The other branch is removed in release. Choosing at runtime would link both modes in.
- In the lean mode, constructing a built-in control by tag (`bind_semantic`) returns `FrameworkError::InvalidComponentType`. Hosts that construct controls by tag keep using the full mode.

Measured (release, after strip):

| Program | Full | Lean | Saved |
| --- | --- | --- | --- |
| Minimal headless program (a column of text and a button) | 5.40 MB | 2.28 MB | 3.12 MB (-58%) |
| Managed window application `examples/reactive-sfc` | 35.48 MB | 33.42 MB | 2.06 MB (-5.8%) |

The size of a managed application comes mostly from GPU, text, and built-in fonts. Controls are only a small part.

Verification: `NANA_TEST_TYPED_BUILTINS=1 cargo test -p nana-ui-runtime` makes `AppContext::new()` in this crate's unit tests return a lean context (this exists only under `cfg(test)`). Tests that need construction by tag are skipped. When a creation path forgets to install behavior, the corresponding test fails in this mode.

## Semantics

| Stage | Behavior |
| --- | --- |
| Mount | The whole declarative tree is built in one commit; attaching it under a parent takes one more insert commit. The root node is inserted without a key and does not affect the parent's own keyed children |
| Writing a signal | `set` / `update` only queue the effects subscribed to it; they do not touch the tree. `computed` is only marked dirty and recomputed the next time it is read. `set` does not compare old and new values; `set_if_changed` (`T: PartialEq`) neither writes nor queues when equal, and `try_set_if_changed` also skips signals already reclaimed |
| Equality cutoff | A write marks only direct readers dirty, and further downstream ones "to check". A computed or effect to check first brings its computed dependencies up to date, and reruns only if one of them computed a **different value** (`computed` requires `T: PartialEq`). So for `computed(\|\| n.get() % 2)` going 1 → 3, not one binding that reads it runs |
| flush | `AppContext::flush_reactive`. It runs in rounds: first `watch_effect`, then the structural updates of `each` / `when`, and finally it stages each node that needs changes in this round once and merges them into **one** commit. Input routing calls it at the end of every event (so `InputRouteOutcome::invalidated_work` reflects binding changes), and `take_system_work` calls it at the start of every frame |
| Node bindings | All dynamic fields of one node share one effect. When any input changes, it first compares field by field against the retained view (`FieldWrite::differs`); if all are equal it stops there: no copy, no projection, no commit. Only when at least one field differs does it copy once, write, and project once |
| `each` | Reconciles by key: kept rows are not rebuilt, and node ids and control interaction state stay the same; removed rows reclaim their scope and destroy their nodes; new rows are built in one detached build. Reordering moves only the nodes outside the longest increasing subsequence: inserting or deleting one row moves no existing node, and a full reversal moves n−1. Field changes within a row should use in-row signals or a `Store`, so they do not trigger a list recompute |
| `each_virtual` | `each_virtual(items, key, row_height, row)`: rows go in a vertical `ScrollView`, and only the rows the viewport (plus overscan) covers are built; underneath is the Runtime's existing retained virtual list. Rows scrolled away are reclaimed together with their scope; rows holding focus or an IME composition are kept. Data changes, scrolling (`ScrollChanged`), and viewport size changes (`ScrollViewportChanged`, sent after layout) all move the window. 50,000 rows: mount plus layout plus first window 2.4 ms; `each` takes 175 ms and about 2.3 KB resident per row. Row height is fixed by default; `.measured()` sizes rows by content, and the given row height is only an estimate: after new rows appear, the next layout (`ScrollLaidOut`) measures their real height and places them again, keeping the row at the top of the viewport still; when a measured row's content later grows or shrinks (expanding, rewrapping), it is reread after the next layout too, and the rows below move along. In templates write `v-for` plus `v-virtual="row_height"`, and `v-virtual.measured` for content sizing; to set the scroll area's size, wrap the `v-for` element in `<Virtual row-height="24" height="400" measured>` (there are also `width`, `grow`, `overscan`, and `:scroll` to pass in a `ScrollView` you configured). The function API counterparts are `.height()`, `.width()`, `.grow()`, `.scroll_view()`. When the list is inside the page's main scroll (a comment section, a section under a page header), use `.within(page NodeRef)`: no separate scroll area is built, the list sits at its current position, and the window is the stretch of the ancestor `ScrollView`'s viewport the list covers; when content above the list grows or shrinks, the window moves after the next layout. `.grid(min_column_width, gap)` arranges items in as many columns as fit the list width; the given row height is the height of one grid row (excluding the gap), columns split the width equally, and when the last row is not full the empty columns stay aligned; when the width changes, items are rearranged for the new column count. With content sizing, data changes (for example, appending a page) keep the measured row heights and do not fall back to the estimate. In templates: `<Virtual within={page} grid=220 gap=12>`. When the ref given to `.within(ref)` later points to another scroll area (the list was moved into another layout), the list releases the old area and listens to the new one. **Reading position**: when data changes, the item at the top of the viewport stays where it was (scroll anchoring): items inserted or deleted above it (loading older chat history upward) do not push away what is being read, and items appended below change nothing; when the `ScrollView` has `follow_end` on, it follows to the end as usual. **List handle**: `let rows = virtual_list_ref::<K>()`, `each_virtual(..).list_ref(rows.clone())` (template `<Virtual list-ref={rows.clone()}>`): `rows.item(&key)` gives any item's `offset` and `extent` in the list, built or not (measured values for measured items, estimates otherwise; a grid item gets its row's); `rows.first_visible()` gives the item at the top of the viewport and how far the top is into it; store it and that is the reading position, and `rows.scroll_to_inset(&key, inset)` returns there; `rows.scroll_to(&key, VirtualAlignment::Start / Center / End / Nearest)` scrolls an item into the viewport (jump to the latest message), building the rows around it; with content sizing, an item aligned to the end or center aligns once more after its real height is measured. `rows.row(&key)` is the node that holds the built item's view: every row (every cell in a grid) is its own key scope, keyed nodes inside a row are found by path from here, `cx.resolve_assembly_path(rows.row(&7)?, "title")`, and different rows using the same key do not interfere. Readings reflect the list's most recent sync (every scroll, size change, and data change syncs it) |
| `when` | Acts only when the condition changes: the old branch reclaims its scope and is destroyed, and the new branch is built and then inserted. `.visible(sig)` instead keeps the node and only toggles `layout.hidden` (the `v-show` counterpart) |
| Keep-alive | `when(..).keep_alive()`: the branch not shown is kept together with its nodes and state, and reappears as it was when switched back (Vue `<KeepAlive>`). Kept branches are moved into a hidden `Stack` inside the container (no layout, painting, or hit testing; focus moves away) and are destroyed together with the container. `dynamic(key, render)` shows one view by key (Vue `<component :is>`); `.keep_alive()` keeps the views of earlier keys, and `.max(n)` keeps at most n, dropping the least recently shown first. In templates `<KeepAlive>` wraps a `v-if` chain and can nest with `<Transition>` either way; a kept-alive branch does not play a leave when switched away, and plays an enter when switched back |
| Teleport | `teleport(to, content)`: `content` is built at the declaration site, belongs to the declaration site's scope, and is destroyed with it, but is attached under `to` in the tree (Vue `<Teleport>`), so layout, painting, hit testing, focus order, and accessibility all follow `to`'s position. `to` can be a `NodeRef`, a node id, or a closure that picks a node, and the content moves when it changes; when it is `None` or the target is absent, the content stays in place. Underneath it is `place_assembled`. The anchor accepts `.class` / `.class_when` / `.css` (in templates `<Teleport to={layer} class="fill">`). In templates write `<Teleport :to="layer">` |
| Errors | Errors are values: a `Result<V, E>` (`E: Display`) is itself a view; `Err(e)` builds nothing and reports `e` to the nearest enclosing `error_boundary(fallback, content)`; `report_error(e)` reports manually. While a boundary has any error, it hides the content and shows `fallback(error list)`. Errors follow the scope that reported them: when the failed branch or row is dropped because of a data change, the error is withdrawn and the content comes back as it was. Without a boundary, the `runtime.view.error_unhandled` fault is reported. A panic is a bug and is not caught. In templates write `<ErrorBoundary :fallback='\|errors\| …'>` |
| Reclamation | When a node is destroyed (by whatever path), the cleanup stage of `commit_mutations` reclaims its bindings, structural effects, and the scopes anchored on it |
| Context | `provide(value)` / `use_context::<T>()`: the value hangs on the current scope, and lower scopes (including rows and branches built later) read it along the parent chain, nearest provider first. It can only be read while building views; event handlers do not run in any scope |
| Frame output | Every time the host finishes handling an update (input, program message, timer), it checks each window for pending bindings and requests a redraw if there are any, so changing a signal in `RuntimeProgram::update` also produces a frame |
| Threads | One signal runtime per thread. Handles are `Copy` ids and can go into `Send` event handlers; using them on another thread panics. The `AppContext` must stay on the thread that created the views |

## Mapping from Vue

| Vue | L3 views |
| --- | --- |
| `ref(0)` | `signal(0)` (`ref` is a Rust keyword) |
| `computed(() => …)` | `computed(move \|\| …)` |
| `reactive({ … })` | `store(value)` + `#[derive(Store)]`, tracked per field |
| `watchEffect` | `watch_effect(move \|\| …)` |
| `{{ x }}`, `:label="x"` | `text!("{x}")`, `.label(x)`, or `.bind(move \|c\| …)` |
| vue-i18n's `$t('files', { count })`, `<i18n-t keypath>` | `<T id="files" :count="n" />`; in Rust `t(move \|\| LocalizedText::new("files").arg("count", n.get()))`. A subtree's locale uses `locale="ar"` / `.locale(..)` |
| `@click` | `.on_activate(move \|\| …)`; other events use `.on(move \|e: &E\| …)` |
| `v-if` / `v-else` | `when(cond, \|\| a).otherwise(\|\| b)` |
| `v-show` | `.visible(sig)` |
| `onErrorCaptured` / `<ErrorBoundary>` | `error_boundary(fallback, content)`, with views returning `Result`; in templates `<ErrorBoundary :fallback>` |
| `<Teleport to>` | `teleport(to, content)`; in templates `<Teleport :to>` |
| `<style scoped>`, `:class` | `<style>` is compiled at build time; `class="a"`, `class:a="condition"`; the function API uses `.css(css! { … })` |
| `<KeepAlive>` / `<component :is>` | `when(..).keep_alive()` / `dynamic(key, render).keep_alive().max(n)`; in templates `<KeepAlive>` |
| `<Transition>` / `<TransitionGroup>` | `when(..).transition(t)` / `each(..).transition(t.moves(..))`; tags of the same name in templates |
| `v-for` + `:key` | `each(items, key, row)` |
| Long lists (virtual scrolling) | `each_virtual(items, key, row_height, row)`; in templates `v-virtual="row_height"` / `v-virtual.measured`, or `<Virtual row-height height …>` wrapping the `v-for` element |
| `v-model` | `.model(sig)` (text inputs, sliders, checkboxes) |
| props / emits | function parameters / `impl Fn(T)` callback parameters |
| slot / named slot | `impl IntoView` parameters; in `.vue`, `<slot name="x"/>` and `<template #x>` |
| template `ref` | `node_ref()` + `.node_ref(r)`; in `.vue`, `ref="r"` |
| `aria-labelledby` | `.labelled_by(r)`; template `labelled_by={r}`, in `.vue` `labelled-by="r"` |
| `onMounted` | `on_mount(move \|cx\| …)` |
| `provide` / `inject` | `provide(value)` / `use_context::<T>()` |
| `onUnmounted` | `on_cleanup` |
| async `setup` + `<Suspense>` | `resource(source, fetch)` + `suspense(fallback, content)`; in templates `<Suspense>` |
| `defineProps` | `defineProps!(name: Type, …)` (`.vue`) |
| `defineEmits` + `emit('done')` | a callback prop `on_done: impl Fn() + …`, with the parent writing `@done="…"` (`.vue`) |

The same UI written both ways:

```vue
<script setup>
const count = ref(0)
const draft = ref('')
const todos = ref([])
</script>
<template>
  <div class="column">
    <span>Count: {{ count }}</span>
    <NanaButton @click="count++">Increment</NanaButton>
    <NanaInput v-model="draft" placeholder="New task" />
    <span v-for="t in todos" :key="t.id">{{ t.title }}</span>
    <span v-if="todos.length === 0">No tasks yet</span>
  </div>
</template>
```

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn page() -> impl IntoView {
    let count = signal(0u64);
    let draft = signal(String::new());
    let todos = signal(Vec::<Todo>::new());
    view! {
        <Column>
            <Text>"Count: {count}"</Text>
            <Button @activate={count.update(|c| *c += 1)}>"Increment"</Button>
            <TextInput v-model={draft} placeholder="New task" />
            <Text v-for={t in todos} key={t.id}>{t.title}</Text>
            <Text v-if={todos.with(Vec::is_empty)}>"No tasks yet"</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn page() -> impl IntoView {
    let count = signal(0u64);
    let draft = signal(String::new());
    let todos = signal(Vec::<Todo>::new());
    column().children((
        text!("Count: {count}"),
        button("Increment").on_activate(move || count.update(|c| *c += 1)),
        text_input().model(draft).placeholder("New task"),
        todos.each(|t| t.id, |t| text(t.title)),
        (move || todos.with(Vec::is_empty)).then_show(|| text("No tasks yet")),
    ))
}
```

:::

Apart from script logic (JS has to become Rust), the template, reactivity, and stylesheets all line up line by line. In L3, stylesheets are compiled at build time; see "Stylesheets" for the syntax. Three differences from Vue templates remain:

- `each` and `when` each bring a container `Stack`, whereas Vue's `v-for` / `v-if` produce sibling nodes directly. Style the container with classes (`<Block class>` / `.class(..)`). There is no need to wrap another sized box around it. It is not a pass-through like `display: contents`: in the Runtime, children of `contents` take the parent container out of the layout cache, so the whole column of rows would be remeasured every time. Long lists cannot afford that. The container is also where children are reordered when `each` rows are added or removed and when `when` branches switch.
- `.get()` / `.with()` in expressions cannot be dropped, because stable Rust cannot implement `Fn` for signals. Only a lone signal can omit them. For example `"{count}"`, `disabled={busy}`.
- Component functions are not lazy. Ownership is divided only at three kinds of scope boundary: mounts, rows, and branches.

## Cost

Command: `cargo run --release -p nana-ui-runtime --features benchmark --bin nana-reactive-benchmark` (`NANA_BENCH=updates` runs only the update scenarios). Each style is run alternately for 15 rounds; the minimum is read. It measures the authoring layer plus commit, not layout or painting. The control group is the style without views: `create_detached_component` + `append_child` to build one node at a time (one commit per call), `update_component` to change fields, and a keyed `mount` rewriting the whole list. Measured on this machine in the early hours of 2026-09-30, load about 4.

| Scenario | Imperative | View |
| --- | --- | --- |
| Mount 1,000 texts | 1.49 ms | constants 1.30 ms / each bound to a signal 1.41 ms |
| Mount 5,000 texts | 7.72 ms | constants 7.30 ms / all bound 7.96 ms |
| Mount 1,000 rows (row container plus button) | 4.44 ms | 3.82 ms |
| Change 1 of 5,000 | `update_component` 0.30 µs | `set` + flush 0.44 µs |
| Change 100 of 5,000 | `update_component` one by one 30.5 µs | 100 signals, one flush 29.9 µs / 1 signal bound to 100 nodes 23.9 µs |
| Change the label of 100 buttons | `update_component` one by one 37.7 µs | 38.8 µs |
| Insert then delete 1 row among 2,000 | whole `mount` rewrite 7.56 ms | `each` 0.057 ms |
| Change a field of 1 row among 2,000 | `update_component` 0.26 µs / whole `mount` rewrite 6.12 ms | in-row signal 0.39 µs |
| Bindings of 100 buttons rerun with unchanged values | — | 5.09 µs |

Mounting with views is over 10% faster than building nodes one by one: the whole tree is one commit, whereas building one by one commits once per node.

The cost of one field update is shared by both paths: copying the component, projection, and commit. On 2026-09-30 this shared path was squeezed item by item based on sampling (two binaries run alternately, 4 rounds each, minimum read: changing 100 buttons 64.9 → 38.8 µs, changing 100 texts 44.8 → 29.9 µs, mounting 12–16% faster):

- When projection compares styles, it compares the layout pointers first: `Arc` equality only compares pointers first when the content is `Eq`. The float-based `LayoutStyle` is not. So previously every comparison went field by field through the whole layout.
- Tables keyed by node, type, and document in `AppContext` and `UiWorld` switched to `IdHasher`. So did component animation ids. They used SipHash before.
- The node record stores "whether the layout depends on the viewport". After writing a visual or text, the dozens of lengths in the layout are no longer rescanned (debug builds recompute and compare on every read).
- Commit validation: no set is built when the node is not the focus target of any overlay. Accessibility state is not staged when there is no overlay host.
- Projection for `Button`, `Text`, and `Chip` compares against the world first and copies text only if it changed. `Button`'s text, visual, and accessible name share one `Arc<str>`.

Whole-list operations on large lists (5,000 rows, each with a row container, a text, and a button; before and after binaries run alternately 4 times each, 10 rounds per run, minimum taken):

| Operation | Before | After |
| --- | --- | --- |
| Mount | 49.5 ms | 39.7 ms |
| Clear | 30.3 ms | 22.8 ms |
| Put 5,000 rows into an empty list at once | 152.7 ms | 49.8 ms |

These numbers come from sampling the mount process: two thirds of the time is in the world's commit, a quarter in building the view. Four changes were made accordingly:

- Every time a subtree was moved out of the document, the accumulated pending-removal list was sorted and deduplicated in full. Putting in n rows was O(n² log n). Now it is sorted only once, when system work is handed out.
- Removing a scope from its parent scope's child list needed a linear search. Deleting n rows one by one was O(n²). Now each scope remembers its position, and removal is one swap-remove.
- A commit's staging table hashed node ids with SipHash. It now uses the Runtime's existing integer hash.
- Comparing whether two layouts are "identical except for transform and cursor" copied both first and then compared. Now identical layouts are equal directly. If fields such as the transform are equal, they are compared directly. Only otherwise is one copy made. `depends_on_viewport` skips the three groups of logical margins that are still the process defaults.

Items decided against on the same evidence:

- **Row template cloning**: building views is only a quarter of mount time, and component construction and projection still happen for every row. Cloning the static part would save less than a tenth.
- **Mounting across frames**: mounting 5,000 rows takes about 40 ms, which is two or three frames. But the right approach for long lists is `each_virtual` (50,000 rows in 2.4 ms). Splitting across frames would only show the user a half-mounted list.
- **Making static text `Cow<'static, str>`**: a text node is 1,804 B resident. An empty `Stack` is 1,778 B. The text itself is only about 26 B. Not worth changing every control's API.
- A separate control registration list is not needed either: `BuiltinComponents::Typed` already registers by the types actually used.

A single-point update costs about 0.15 µs more than a hand-written `update_component`. That is the cost of signal bookkeeping. When many nodes change at once, there is only one commit, which is actually faster.

The two second-stage optimizations were each A/B tested (before and after binaries run alternately 3 times each, minimum read):

- Field-level comparison: when bindings rerun with unchanged values, 21.1 µs → 7.2 µs. When values really change it is roughly even. "1 signal bound to 100 nodes" is about 4% slower, because a changed direct binding reads the signal cell twice.
- `reconcile_child_order` switched to the longest increasing subsequence: `each` inserting plus deleting one row, 1.85 ms → 0.165 ms. It is also the reorder path shared by `mount` and workspace assembly.

Memory (`tests/reactive_view_alloc.rs`, a per-thread counting allocator):

- Among 5,000 nodes, mounting constant text allocates about 11.8 KB cumulatively per node, in 25 allocations (this is allocation traffic; frees are not subtracted). A node bound directly to a signal costs about 945 B and 4 allocations more. That includes the amortized share of hash table and Vec growth.
- Resident memory (allocations minus frees, `tests/node_memory.rs`): text and `Stack` about 1.8 KB. `Button` about 2.4 KB. `TextInput` about 3.7 KB. The roughly 2 KB `LayoutStyle` is no longer one per node: its large field groups (`paint`, the three groups of logical margins, grid placement) are copy-on-write `Shared`. Default values are shared process-wide. Layouts with equal content share one allocation at component creation (`ComponentView::share_layouts`) and when written into the world (the recent-layout cache). Before this, a `Button` held three 4.8 KB layouts (the component's own, the copy made during projection, and the one resolved by radius tier), 11.7 KB resident.
- Adding another direct binding to the same node costs just 1 more allocation: the new signal's subscription table.
- Rerunning an effect with unchanged dependencies: 0 allocations. A flush in which bindings rerun but field values did not change: also 0 allocations.

### `.vue` versus hand-written functions

A compiled `.vue` file is just `fn … -> impl IntoView`. It calls the same API as hand-written functions. The two differ only where compiler-generated code differs from human-written code: constant folding, build-time compiled stylesheets, and static text in hot-reload mode.

Measured with `examples/reactive-sfc`'s `sfc-benchmark`. The same UI is written three times:

- `.vue` in `bench/`.
- Idiomatic hand-written: static values passed as values. Single signals bound directly.
- Naive hand-written: every signal read inside a closure. Like `counter_by_hand` in `tests/views.rs`.

`tests/views.rs` asserts that the trees each version builds (nodes, text, layout) are identical.

Command: `cargo run --release -p reactive-sfc --bin sfc-benchmark`. Conditions: this machine on 2026-09-29, load 3–5. Each scenario run alternately for 15 rounds; the whole program run 3 times; times take the minimum. Memory is per-thread resident memory (allocations minus frees) over 2,000 rows. Before measuring, n rows are built once to grow the thread-level signal table.

Scenarios:

- A: each row is a `Row` containing a text and a button. The text comes from a prop and a signal that is never written.
- B: A plus a `<style scoped>` setting `padding` / `flex-grow`, plus a conditional class `class:active` setting `opacity`. The hand-written versions use builders (`padding_xy`, `with_layout`) and `css!` respectively; the function API can use `.class_when` directly.
- C: a `v-for` list. Each row shows an in-row signal. There is no naive version: `.vue` compiles `{{ title }}` into a closure, which is the naive style already. The idiomatic hand-written version binds `text(title)` directly. Both have one effect per row.

| Scenario | `.vue` | Idiomatic hand-written | Naive hand-written |
| --- | --- | --- | --- |
| A mount 1,000 rows | 6.86 ms | 6.78 ms | 7.08 ms |
| A mount 5,000 rows | 40.7 ms | 41.0 ms | 42.8 ms |
| A resident / effects per row | 4,763 B / 0 | 4,695 B / 0 | 5,522 B / 2 |
| B mount 5,000 rows | 42.6 ms | builder 43.6 ms / `css!` 41.6 ms | — |
| B switch the selected row among 5,000 | 459 µs | builder 412 µs / `css!` 405 µs | — |
| B resident per row | 5,089 B | 5,025 B | — |
| C mount 2,000 rows | 15.1 ms | 15.1 ms | — |
| C insert or delete 1 row | 120 µs | 120 µs | — |
| C change a field of 1 row | 0.60 µs | 0.60 µs | — |
| C resident / effects per row | 5,926 B / 1 | 5,950 B / 1 | — |

Conclusions:

- Mount and update time: `.vue` is within 3% of idiomatic hand-written code, that is, within noise.
- Constant folding: the naive style also binds values that never change. That costs 2 more effects and about 830 B resident per row, and mounting is about 5% slower. `.vue` gets the idiomatic result automatically through folding.
- `constant(..)` still occupies a signal slot. So `.vue` uses about 68 B more per row than idiomatic hand-written code.
- Conditional classes: `.vue` is about 11% slower than hand-written code when switching. Each row rerun computes a class mask and locks `StyleSite` for a table lookup. A hand-written `FieldWrite` compares one field directly.
- Hot reload: debug builds default to `hot(true)`. Every purely static text node becomes an effect that reads the replacement table. Measuring that generated code separately in release: B and C cost 1 more effect and about 315–330 B resident per row, and C mounts about 3% slower.
- Compilation: the build script compiles 9 `.vue` files (`views/` once, `bench/` twice) in about 0.25 s.

This measurement found and fixed a problem: for elements with conditional classes, the binding kept its own copy of the base layout instead of sharing it with other instances. So each row used about 2.2 KB more (one `LayoutStyle`). Each rerun's cache lookup also could not find the same pointer and had to compare the whole layout field by field. After the fix, `StyleSite::shared_base` lets all instances at the same site share one base layout.

| Item | Before fix | After fix |
| --- | --- | --- |
| B resident per row | 7,241 B | 5,089 B |
| Switch the selected row among 5,000 | 817 µs | 459 µs |
| Switch the selected row among 1,000 | 134 µs | 83 µs |

`tests/view_memory.rs` asserts that the compiled version's resident memory per row exceeds the hand-written version's by no more than 512 B. Reverting the fix makes this test fail.

## Tracing and inspection

- `cx.view_bindings(node)` returns the element's declaration location, plus each bound field and its declaration location. Always available.
- With `reactive-trace` on, `cx.why_updated(node)` returns why this node was last changed: which `set` / `update` on which line wrote which signal (and on which line the signal was created). Records contain only ids and `&'static Location`. They are written into a preallocated per-thread ring buffer. Recording does not allocate, format, or do I/O.
- `cx.inspect(node)` returns how the node looks in the view layer: which control in the control table it is, the current value of every bindable field (`Debug` text), and on which line the binding that drives each field is declared. `cx.set_field(node, field, text)` writes one field the way a binding would (strings as is, numbers and `true` / `false` parsed, empty text clears optional fields). A bound field returns to the bound value the next time the binding runs. The read and write functions are generated from the control table. New controls get support automatically.
- devtools: `nana-ui-devtools`'s runtime session supports `{"cmd":"inspect", …target}` and `{"cmd":"set_field", …target, "field":"label", "value":"Save as"}`. The `inspect` reply carries the control, fields, and binding locations. With `reactive-trace` on, it also carries `causes`: on which line the signal that last changed it was written, and on which line the signal was created.
- Diagnostics: the `runtime.reactive.*` metrics (flush count, signal writes, effects run, nodes changed, commit count, flush time), plus two faults:
  - `runtime.reactive.did_not_settle`: reported when effects trigger each other for more than 64 rounds. The remaining queue is dropped.
  - `runtime.reactive.disposed_access`: reported when a signal is read or written after its scope was reclaimed. It also panics.

## Known limitations

- When a field really changes, the whole component is still copied and fully projected once, and then the world compares by field and marks dirty. Sampling shows that the expensive part of projection is comparing and copying unchanged fields. That has been removed on the shared path and in `Button`, `Text`, and `Chip`. Projection of other controls still copies first and compares afterward. They are changed one by one as needed (see `Button::project` for how).
- `.bind(|c| …)` cannot tell which field it changed. So it is always treated as "changed" and goes through the copy path.
- Each closure binding is boxed separately. Only `view!`, which sees the whole template, has the chance to merge the closures of one node into one.
- The only built-in controls recognized by name are those in the `nana-ui-view-schema` control table: `Text`, `Button`, `Checkbox`, `Switch`, `Slider`, `RangeSpan`, `TextInput`, `TextArea`, `NumberInput`, `Select`, `ListItem`, `Progress`, `Spinner`, `Divider`, `Thumbnail`, `Avatar`, `Texture` (`GpuTextureView`), `IconButton`, `Chip`, `StatusBadge`, `EmptyState` (`#action` slot), `LabeledValue`, `Tabs`, `TreeView`, `ColorField`, `ActionMenuItem`. Plus `T` (localized text), `Column`, `Row`, `Widget`. Other controls use `widget(C)` with `.bind` / `.on`.

**Accessibility check**: when compiling a template, controls a screen reader cannot name get a warning. Compilation does not fail. There are two rules: `Button`, `Checkbox`, `Switch`, and `ListItem` without text (empty children or an empty `label`). `TextInput`, `TextArea`, `NumberInput`, `Slider`, and `Progress` without `label` (their accessible name comes only from `label`; placeholder text does not count). Controls with `labelled_by` are named by that text and are not reported. `.vue` warnings are printed with line and column via `cargo:warning`. `view!` has no warning interface on stable. The warning appears at the macro call site as "use of a deprecated constant", with the explanation in the deprecation note.

The control table is the single source: each entry states the template tag, the element function and its parameters, the bindable fields and their types, the field and event `v-model` maps to, and the event methods. The Runtime generates setters, `model`, and event methods from it. The `view!` and `.vue` compilers learn from it which tags are built in and which attributes they accept. A misspelled attribute name, a nonexistent event, or `v-model` on a control without one are all reported with line and column when the template is compiled. Numeric fields accept literals like `max="100"` and generate constants with a type suffix. Events without data (`@activate`) accept `|| …`. Events with data (`@change`, `@input`, `@submit`) accept `|e| …`. The type of `e` is inferred. No annotation needed. Statement-style handlers in templates work in both forms. The compiler generates the corresponding closure.

## Non-goals

- No whole-tree render. No virtual DOM diff either.
- `view!` is only syntactic sugar that expands to function calls. It introduces no new runtime semantics.
