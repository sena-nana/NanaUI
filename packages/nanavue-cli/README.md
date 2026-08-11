# nanavue-cli

Phase 6 packaging entry (stub JS package). **Runnable Release path today:**

See [`docs/release-artifacts.md`](../../docs/release-artifacts.md) and:

```bash
cargo run -p nana-js-quickjs --bin nana-qjs-compile --locked -- \
  --in <app.iife.js> --out <app.qbc> --compose-shim
```
