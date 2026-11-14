# nana-ui-view-codegen

把模板元素树展开成 `nana-ui-runtime` 声明式视图调用的代码生成器，`view!` 和 `.vue` 编译器共用它。应用不需要直接依赖它。

内置标签和属性来自 [`nana-ui-view-schema`](../nana-ui-view-schema) 的控件表，拼错的标签或属性是带位置的模板错误；不认识的标签按同名组件函数调用。`<style>`、`stylesheet!` 和 `css!` 也在这里经 [`nana-ui-css`](../nana-ui-css) 编译成样式数据。模板文本的解析分别在 [`nana-ui-view-macros`](../nana-ui-view-macros) 和 [`nana-ui-sfc`](../nana-ui-sfc)。

## 测试

```bash
cargo test -p nana-ui-view-codegen
cargo test -p nana-ui-runtime --features view-macro --test contracts view_macro
```

更多内容见 [作者工具链](../../docs/learn/toolchain.md)。
