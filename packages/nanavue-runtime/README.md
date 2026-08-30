# @nanaui/nanavue-runtime

NanaUI 的 Vue Custom Renderer。把 Vue 3 的一个子集写进同一棵原生树，不是 WebView。

产品说明：[Vue](../../docs/vue.md)。新应用请从 Rust 的 [开始](../../docs/start.md) 写起。

```js
import { createApp } from "@nanaui/nanavue-runtime";

createApp({ /* 根组件 */ }).mount();
```

语义控件用 [`@nanaui/nanavue-components`](../nanavue-components/README.md)。Rust 宿主用 `nana_ui_vue::prelude` 的 `VueRuntimeProgram::run` 加载你打出来的脚本，走同一套 `run_runtime`。

要新增进入布局和点击的控件：Rust `register_component`，Vue tag 为 `ComponentTypeId` 去掉 `nana.`。与 HTML 同语义用原生标签；不同语义换名。未识别的 tag 会报错，不会当成布局盒。只暴露 JS 命令时走 `NativeComponentRegistry`。两张表不是同一条 ABI。
