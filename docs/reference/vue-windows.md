# 窗口

JavaScript 入口和 Rust 宿主的接法在 [Vue](vue.md)。概念对照在 [从 Vue 过来](../guide/from-vue.md)。

## 多窗口与 JavaScript 隔离

`Nana.windows.create({ isolation })` 选择新窗口的 JavaScript 上下文。取值是 `"shared"`（默认）或 `"isolated"`。

拼错的取值直接报错。不会悄悄退回共享。

**共享（默认）。** 新窗口和打开它的脚本在同一个全局环境里。下面这些状态跨窗口：JavaScript 全局和模块状态（Vue / Pinia store、组件注册）、`Nana.host.on` 监听和宿主事件队列、`setTimeout` / `setInterval` / `requestAnimationFrame` 的回调表与编号、`fetch` / `WebSocket` 的 JS 对象、`localStorage`、宿主资源句柄、`Nana.dialogs` provider，以及 `Nana.components.onError` 监听。

仍然每扇窗一份的是：文档、`window` 和 `document` 对象、`sessionStorage`、定时器与网络请求在宿主侧的调度、焦点和 IME。

**隔离。** 新窗口得到自己的 JavaScript 上下文。同一份应用脚本在里面**重新执行一遍**。全局变量、模块状态、定时器、`localStorage` 都和其他窗口互不可见。

`localStorage` 和 `Nana.storage` 只在内存里。窗口关闭即释放。

脚本用 `Nana.windows.current()` 判断自己身在哪扇窗：

```js
const current = Nana.windows.current(); // { id, isolation, params, ... }
createApp(current.params?.view === "settings" ? Settings : Main).mount();
```

打开方通过 `params` 传入初始数据。数据须可 JSON 序列化。循环引用在 `create()` 时抛 `TypeError`：

```js
const settings = await Nana.windows.create({ isolation: "isolated", params: { view: "settings" } });
settings.focus();
```

`params` 只送给隔离窗口。

窗口种类用 `tag`。共享窗口和隔离窗口都能用。它是字符串。它出现在 `create()`、`current()`、`list()` 返回的句柄上（`handle.tag`，未设置为 `null`）。它原样成为 `WindowDescriptor::tag`。Rust 侧从 `RuntimeProgramContext::window_tag()` 读回。见 [窗口](window.md)。

主窗口的 `tag` 取自传给 `VueRuntimeProgram::run` 的描述符。脚本首次执行时，`Nana.windows.current().tag` 就已经可读。

```js
const tracking = await Nana.windows.create({ tag: "tracking", role: "tool" });
tracking.mount(tracking.tag === "tracking" ? TrackingPanel : Main);
```

`shadow` 选桌面阴影。`"auto"` 是默认。透明窗口得到跟随根卡片的阴影。`"none"` 不要阴影。也可以是样式对象 `{ color: [r, g, b, a], offset: [x, y], blur, spread, source: "windowShape" }`。缺省字段取默认值。

拼错的关键字、未知字段或非法数值直接报错。实际结果以宿主为准。见 [窗口](window.md) 的 WindowShadow 一节。

打开方拿到的句柄可以控制窗口。例如 `focus`、`close`、`setBounds`、`ready`、`closed`。但不能 `mount`。窗口内容由它自己的上下文挂载。句柄上的 `window`、`document`、`root` 为 `null`。

在隔离窗口里再打开的共享窗口，属于这个隔离上下文。`Nana.windows.list()` 只列出当前上下文里的窗口。一个隔离上下文在它最后一扇窗口关闭、`window-closed` 送达并完成卸载之后销毁。

**两种模式都跨窗的**有这些：GPU Device 和 Queue，以及 WebGPU、Canvas、SVG、媒体、视频运行时，原生组件和宿主纹理注册表，应用样式表，动画时钟，诊断输出，你注册的宿主命令（它们的 Rust 状态），以及按窗口 id 生效的窗口控制。

所有上下文还共用同一个 V8 堆、同一个线程和同一个微任务队列。隔离的是状态。**不是**性能，也**不是**故障。一个窗口里的死循环仍会卡住所有窗口。

窗口控制也不是安全边界。同一份脚本，同一个进程。

隔离窗口需要以源码形式加载的应用脚本。V8 snapshot 形式的脚本，以及不支持多上下文的引擎，会让 `create()` 以 `WindowOpenError` 失败。

JS 的 `windowSetFullscreen` 和 `windowSetAlwaysOnTop` 接口不变。它们仍是布尔。`windowGeometry().fullscreen` 和窗口的 `alwaysOnTop` 现在是宿主观察到的值。请求入队时不再乐观写入。要等 `WindowEvent::ModeChanged` 回流。调用后立刻读几何，可能仍是旧值。

窗口化对照 `examples/vue-hosted-acceptance`。`examples/vue-counter` 是引擎探针，含无头点击。它不是应用模板。

