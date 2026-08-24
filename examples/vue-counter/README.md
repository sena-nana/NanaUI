# vue-counter

引擎探针：Vue Custom Renderer → Runtime / UiScene。用来验收 host op、语义桥和无头点击，**不是**应用模板。

窗口化 Vue 应用对照 `examples/vue-hosted-acceptance`。产品说明见 [Vue](../../docs/vue.md)。

```bash
# 无头 Counter
cargo run -p vue-counter -- counter

# 语义桥
cargo run -p vue-counter -- counter --semantic --clicks=2

# 无头 Todo
cargo run -p vue-counter -- todo

# 窗口（同一套 Scene host）
cargo run -p vue-counter --features windowed -- --window
```

默认引擎是 V8（`engine-v8`）。
