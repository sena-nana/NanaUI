# 创建应用

一扇窗口背后是三件事：你持有一棵 `RuntimeDocument`，实现 `RuntimeProgram`，然后把它交给 `run_runtime`。能跑起来的整段程序在 [快速开始](../quick-start.md)。这一章只说明这几处各自做什么。

桌面入口要打开 `hosted`。不打开就没有 `run_runtime`。依赖怎么写，见快速开始。

## RuntimeDocument

`RuntimeDocument` 是一棵保留文档，外加从它抽出来的场景。宿主按窗口来借它，不把它拆成另一棵树。

```rust
use nana_ui::runtime::{DocumentId, RuntimeDocument};

let document_id = DocumentId::new(1).expect("document id");
let mut document = RuntimeDocument::new(document_id);
```

`DocumentId::new` 不接受 `0`，那时返回 `None`。`RuntimeDocument::new` 用一份完整的内置控件表建上下文。只从 Rust 类型建控件时，可以改用 `RuntimeDocument::typed`，没创建过的控件不会被链接进来。按标签构造的路径仍然用 `new`。

`document()` 读回这份 id。`context()` / `context_mut()` 拿出 `AppContext`。视图挂在这个上下文上：

```rust
document
    .context_mut()
    .mount_view_root(document_id, page)
    .unwrap();
```

`page` 是一个闭包，在挂载作用域里求值，返回 `impl IntoView`。根作为文档根。挂到某个已有节点下面用 `mount_view`。两种写法怎么写这棵树，见 [视图写法](view.md)。

## 宿主怎么找到这棵文档

`run_runtime` 不保存你的结构体字段。每一帧它按 `WindowId` 调用 `with_document` 或 `with_document_mut`，在闭包里借用对应的 `RuntimeDocument`。

```rust
fn with_document<R>(
    &self,
    id: WindowId,
    f: impl FnOnce(&RuntimeDocument) -> R,
) -> Result<Option<R>, nana_ui::DocumentAccessError> {
    Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
}
```

返回 `Ok(None)` 表示这扇窗口没有文档。闭包里拿到的引用不能带出闭包。改文档用 `with_document_mut`，签名一样，只是交出 `&mut RuntimeDocument`。快速开始里只有主窗口，所以用 `WindowId::PRIMARY` 对上那一棵。多窗口时按 id 交出你自己保存的那一棵。

## RuntimeProgram

应用实现这个 trait，再交给宿主。`Message` 用来传递跨窗口、GPU 和持久化这类宿主消息，不是每一次点击的总线。点击改控件或信号，见 [事件](events.md)。

| 方法 | 做什么 |
| --- | --- |
| `initialize` | 宿主可以挂载、布局、派发和绘制时调用一次。返回 `(程序, 要马上送进 update 的消息)`。只建第一屏需要的状态。 |
| `with_document` / `with_document_mut` | 按 `WindowId` 把 `RuntimeDocument` 交给宿主。 |
| `update` | 处理一条 `Message`。保持便宜。 |
| `theme_mode` | 这一帧用深色还是浅色。 |
| `window_event` | 窗口生命周期。默认实现见下文。 |

`initialize` 收到 `&RuntimeProgramContext<Message>`。上面有 `window_id`、`dispatch` 这些宿主能力。原生窗口句柄不穿过这条边界。

`update` 返回 `RuntimeProgramUpdate`。什么都不用做就用 `RuntimeProgramUpdate::default()`。要重绘某一扇用 `RuntimeProgramUpdate::redraw(id)`。

宿主不会自己关窗。系统或标题栏关闭按钮都只发 `WindowEvent::CloseRequested`。默认的 `window_event` 立刻回 `WindowCommand::Close(id)`，关掉这一扇。要退出整个进程，回 `RuntimeProgramUpdate::exit()`。需要先保存时，先回默认更新，保存完成后再关。

## run_runtime

```rust
fn main() -> Result<(), nana_ui::HostedRunError> {
    nana_ui::run_runtime::<App>(nana_ui::WindowDescriptor::new("NanaUI"))
}
```

`run_runtime` 打开 `WindowDescriptor` 描述的那扇窗口，创建 `App`，然后跑事件循环。它不设应用路径，也不开诊断。要目录和诊断，用 `NanaApplication::builder`，见 [应用 API](../../reference/application-api.md)。

## theme_mode

```rust
fn theme_mode(&self) -> nana_ui::ThemeMode {
    nana_ui::ThemeMode::Dark
}
```

只有 `ThemeMode::Dark` 和 `ThemeMode::Light`。深色和浅色只换颜色，不换尺寸，也不换状态怎么分层。控件上要跟着主题走的颜色写语义角色，不要在样式里写死一份亮色，见 [类与样式](class-and-style.md)。

## 不必亲手实现 trait 的时候

普通 Rust 应用可以实现 `ApplicationState`，用 `RuntimeApplication<State>` 去实现 `RuntimeProgram`。`initialize` 之后它会调用你的 `build`，窗口上的 `ApplicationWindow.document` 就是那棵 `RuntimeDocument`。`with_document` 按 `WindowId` 从它的窗口表里取。`theme_mode` 默认是 `ThemeMode::Dark`。

自己管理窗口和文档的宿主仍然直接写 `RuntimeProgram`。方法全表在 [应用 API](../../reference/application-api.md)。

::: warning
点击处理里不要把整页再 `mount_view_root` 一遍。树在挂载时建一次，之后改信号或改那一个控件。
:::

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/view">
    <p class="next-step-link">视图写法</p>
    <p class="next-step-caption">同一个界面，两种写法怎么展开。</p>
  </a>
  <a class="next-step" href="/guide/quick-start">
    <p class="next-step-link">快速开始</p>
    <p class="next-step-caption">把这些方法接成能打开的第一扇窗口。</p>
  </a>
</div>
