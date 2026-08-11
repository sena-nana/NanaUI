# vue-counter / vue-todo

Vue Custom Renderer → Rust `NanaTreeDocument` / `MessageBridge` → **NanaUI (Iced)**。
Blitz / paint-stub / paint-vello / CustomContent 已移除。

可见 UI 经 `createWidget` / 语义降维 → `MessageBridge` → Iced 控件。

## Commands

```bash
# Headless Counter (QuickJS, legacy DOM probe tree)
cargo run -p vue-counter -- counter

# Semantic message bridge (createWidget → BridgeEvent → Iced props)
cargo run -p vue-counter -- counter --semantic --clicks=2

# Headless Todo
cargo run -p vue-counter -- todo

# Simulate clicks on legacy probe (hit-test → onClick)
cargo run -p vue-counter -- counter --clicks=3

# Release path: compose-shim → QuickJsBytecode (not SourceUtf8)
cargo run -p vue-counter --release -- counter --clicks=2 --bytecode

# Windowed NanaUI driven by semantic snapshot
cargo run -p vue-counter --features windowed -- --window
```

Engines are mutually exclusive: do not enable `engine-quickjs` and `engine-v8` together.
