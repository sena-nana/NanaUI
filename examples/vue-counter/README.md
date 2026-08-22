# vue-counter / vue-todo

Vue Custom Renderer → Rust `NanaTreeDocument` / `MessageBridge` → Runtime / UiScene → `SceneWgpuPainter`。
Blitz / paint-stub / paint-vello / CustomContent 已移除。

可见 UI 经 `createWidget` / 语义降维 → `MessageBridge` → `UiWorld`。

## Commands

```bash
# Headless Counter (V8, legacy DOM probe tree)
cargo run -p vue-counter -- counter

# Semantic message bridge (createWidget → BridgeEvent → Runtime props)
cargo run -p vue-counter -- counter --semantic --clicks=2

# Headless Todo
cargo run -p vue-counter -- todo

# Simulate clicks on legacy probe (hit-test → onClick)
cargo run -p vue-counter -- counter --clicks=3

# Release path: compose-shim → V8Snapshot (host-free snapshot; full Vue IIFE stays SourceUtf8)
cargo run -p vue-counter --release -- counter --clicks=2 --bytecode

# Windowed NanaUI driven by semantic snapshot
cargo run -p vue-counter --features windowed -- --window
```

Default engine is V8 (`engine-v8`).
