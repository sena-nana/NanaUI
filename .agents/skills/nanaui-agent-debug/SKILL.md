---
name: nanaui-agent-debug
description: Headless NanaUI Agent session for visual verification and UI operation without a window. Use when debugging a product Vue/JS or Runtime app, taking offscreen screenshots, dumping a11y/semantic trees, clicking widgets, or verifying layout boxes. Do not put CPU readback on product Surface present.
---

# NanaUI Agent headless debug

Use `nana-ui-devtools` only. Product `run_runtime` present must not CPU-read back.

| App | Call |
|---|---|
| Vue/JS | `VueAgentSession::new(engine, artifact, w, h)` |
| L3 Runtime | `RuntimeAgentSession::new(document, w, h)` |
| JS file | `nana-agent-session --js <app.js> --stdio` |

```bash
printf '%s\n' '{"cmd":"a11y"}' '{"cmd":"click","agent_id":"increment"}' \
  '{"cmd":"screenshot","path":"target/agent-session.png"}' \
| cargo run -p nana-ui-devtools --features agent-bin --bin nana-agent-session -- \
  --js path/to/app.js --width 800 --height 600 --stdio

cargo test -p nana-ui-devtools --features agent --lib
```

Commands: `screenshot` `{path}`, `a11y`, `semantic`, `click` (`agent_id` \| `node` \| `x`+`y`), `type` `{text}`, `pump`.

Click via `data-agent-id`. After each step: content a11y boxes **> 8px**, **open the PNG** (flat clear is fail), then semantic dump if state changed.

`nana-ui` must not depend on `nana-ui-devtools`.
