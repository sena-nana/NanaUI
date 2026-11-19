# Creating an app

Behind one window there are three things: you hold one `RuntimeDocument`, you implement `RuntimeProgram`, and you hand it to `run_runtime`. The dependencies and the first piece of UI are in [Quick start](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/quick-start.md). This chapter explains what each of those parts does and gives a whole program that opens.

A desktop entry point must enable `hosted`. Without it there is no `run_runtime`. For how to write the dependency, see Quick start.

## RuntimeDocument

`RuntimeDocument` is one retained document plus the scene extracted from it. The host borrows it per window; it is never split into another tree.

```rust
use nana_ui::runtime::{DocumentId, RuntimeDocument};

let document_id = DocumentId::allocate();
let mut document = RuntimeDocument::new(document_id);
```

`DocumentId::allocate` returns an id that is unique within the process every time, and each document in the application takes one. Tests and fixtures that need a hard-coded number use `DocumentId::new`, which does not accept `0` and returns `None` for it. `RuntimeDocument::new` registers built-in control behavior on demand so controls that were never created are not linked in.

`document()` reads back this id. `context()` / `context_mut()` hand out the `AppContext`. Views mount on that context:

```rust
document
    .context_mut()
    .mount_view_root(document_id, page)
    .unwrap();
```

`page` is a closure evaluated in the mount scope that returns `impl IntoView`. The result becomes the document root. To mount below an existing node use `mount_view`. For how the two styles write this tree, see [View syntax](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/view.md).

## How the host finds this document

`run_runtime` does not keep your struct fields. Every frame it calls `with_document` or `with_document_mut` with a `WindowId` and borrows the matching `RuntimeDocument` inside the closure.

```rust
fn with_document<R>(
    &self,
    id: WindowId,
    f: impl FnOnce(&RuntimeDocument) -> R,
) -> Result<Option<R>, nana_ui::DocumentAccessError> {
    Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
}
```

Returning `Ok(None)` means this window has no document. A reference taken inside the closure cannot be carried out of it. Changing the document uses `with_document_mut`, with the same signature but handing out `&mut RuntimeDocument`. The complete program below has only the primary window, so it matches that one tree with `WindowId::PRIMARY`. With several windows, hand out the tree you stored for each id.

## RuntimeProgram

The application implements this trait and gives it to the host. `Message` carries host messages across windows, GPU work and persistence; it is not a bus for every click. For changing a control or a signal on click, see [Events](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/events.md).

| Method | What it does |
| --- | --- |
| `initialize` | Called once when the host can mount, lay out, dispatch and draw. Returns `(program, messages to send into update right away)`. Build only the state the first screen needs. |
| `with_document` / `with_document_mut` | Hands the `RuntimeDocument` to the host by `WindowId`. |
| `update` | Handles one `Message`. Keep it cheap. |
| `theme` | Returns the currently registered `Arc<CompiledTheme>`. |
| `window_event` | Window lifecycle. The default implementation is below. |

`initialize` receives `&RuntimeProgramContext<Message>`. It exposes host capabilities such as `window_id` and `dispatch`. Native window handles never cross this boundary.

`update` returns `RuntimeProgramUpdate`. When there is nothing to do, use `RuntimeProgramUpdate::default()`. To redraw one window, use `RuntimeProgramUpdate::redraw(id)`.

The host never closes a window by itself. Both the system close and the title-bar close button only emit `WindowEvent::CloseRequested`. The default `window_event` immediately returns `WindowCommand::Close(id)` and closes that one window. To exit the whole process, return `RuntimeProgramUpdate::exit()`. When saving comes first, return the default update, then close once saving finishes.

With `ApplicationState`, this answer comes from `close_requested`. The default closes this window. To ask a question first, or to keep it in the tray, return an update without `Close(id)` and close later from `update` once the user decides. `window_event` still receives the request, for observation only.

## run_runtime

```rust
fn main() -> Result<(), nana_ui::HostedRunError> {
    nana_ui::run_runtime::<App>(nana_ui::WindowDescriptor::new("NanaUI"))
}
```

`run_runtime` opens the window described by `WindowDescriptor`, creates `App`, and then runs the event loop. It does not set application paths and does not turn on diagnostics. For directories and diagnostics, use `NanaApplication::builder`, see [Application API](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/application-api.md).

## theme

```rust
fn theme(&self) -> std::sync::Arc<nana_ui::CompiledTheme> {
    nana_ui::builtin_theme_arc(nana_ui::ThemeAppearance::Dark)
}
```

Light and Dark are the two preset themes; an application can register its own `ThemeId` through `ThemeDefinition` and `ThemeRegistry`. Write semantic roles for colors that should follow the theme on controls; do not hard-code a light palette in the style, see [Classes and styles](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/class-and-style.md).

## When you do not implement the trait by hand

An ordinary Rust application can implement `ApplicationState` and use `RuntimeApplication<State>` to implement `RuntimeProgram`. After `initialize` it calls your `build`, and `ApplicationWindow.document` on the window is that `RuntimeDocument`. `with_document` looks it up in its window table by `WindowId`. `theme` returns the built-in Dark by default.

A host that manages its own windows and documents still writes `RuntimeProgram` directly. The full method table is in [Application API](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/application-api.md).

::: warning
Do not `mount_view_root` the whole page again inside a click handler. The tree is built once at mount; after that, change a signal or change that one control.
:::

## The complete program

The window shell does not depend on which authoring style you use. The example below uses the function style, with the click going through `on_cx`. The UI itself is the column from Quick start.

```rust
use std::convert::Infallible;

use nana_ui::runtime::view::{button, column, text};
use nana_ui::runtime::{Activate, DocumentId, RuntimeDocument};
use nana_ui::{
    RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate, ThemeAppearance, WindowDescriptor,
    run_runtime,
};
use nana_ui_platform::{WindowEvent, WindowId};

struct App {
    document: RuntimeDocument,
}

impl App {
    fn mount() -> Self {
        let document_id = DocumentId::allocate();
        let mut document = RuntimeDocument::new(document_id);
        let cx = document.context_mut();

        cx.mount_view_root(document_id, || {
            column().gap(12).children((
                text("Hello"),
                button("Start").on_cx(|_button, _event: &Activate, _cx| {
                    // Change your own state. To open a window or switch GPU: cx.dispatch_program(msg)
                }),
            ))
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

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
    }

    fn update(
        &mut self,
        _message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn theme(&self) -> std::sync::Arc<nana_ui::CompiledTheme> {
        nana_ui::builtin_theme_arc(ThemeAppearance::Dark)
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
    run_runtime::<App>(WindowDescriptor::new("NanaUI"))
}
```

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/view">
    <p class="next-step-link">View syntax</p>
    <p class="next-step-caption">See how the two styles of the same UI expand.</p>
  </a>
  <a class="next-step" href="/guide/examples">
    <p class="next-step-link">Examples</p>
    <p class="next-step-caption">The Gallery, a counter, a todo list and live content.</p>
  </a>
</div>
