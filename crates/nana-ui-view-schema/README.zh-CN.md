# nana-ui-view-schema

[English](README.md) · 简体中文

声明式视图认识的内置控件表，只导出一个宏 `for_each_control!`。应用不需要直接依赖它。

表里每一项写明一个控件的模板标签、元素函数和参数、可绑定字段、`v-model` 与事件。`nana-ui-runtime` 把它展开成类型化的 setter 和事件方法，[`nana-ui-view-codegen`](../nana-ui-view-codegen) 把同一张表展开成模板接受的标签与属性，所以在表里加一个控件，两边同时得到它。控件本身的实现在 `nana-ui-runtime`；应用自定义的组件不需要登记在这里。

本 crate 没有自己的测试，表的内容由两侧的展开检验：

```bash
cargo test -p nana-ui-runtime --features view-macro --test contracts view_macro
```

更多内容见 [作者工具链](../../docs/learn/toolchain.md)。
