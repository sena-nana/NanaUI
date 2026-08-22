# Phase 2：双 JS 引擎粗测（Vue runtime-core probe）

> 状态注记：历史证据。QuickJS 已从产品路径移除；当前单一引擎为 V8 150.4.0。

测量环境：

- macOS 26.5.2（25F84），Apple Silicon arm64；
- Rust/Cargo 1.97.0；
- `rquickjs = "0.12.2"`；`v8 = "150.4.0"`（rusty_v8 后继包名）；
- 同一预构建产物：`crates/nana-js-engine/fixtures/vue-runtime-probe/dist/vue-runtime-probe.iife.js`（约 194 KiB，含真实 `@vue/runtime-core`）；
- Release 二进制，每引擎 11 次独立冷启动 + 单次 `__nanaProbe.run`，取中位数；
- 无 Blitz paint / 完整 Custom Renderer（Phase 3）。

## 命令

```bash
cargo run --release -p vue-quickjs -- 11
cargo run --release -p vue-v8 -- 11
```

## 结果（中位数）

| 引擎 | 冷启动 initialize | invoke `__nanaProbe.run` | host 结果 |
| --- | ---: | ---: | --- |
| QuickJS | 6.781 ms | 0.177 ms | ok / count=2 / createElement=3 / insert=3 / increment=1 |
| V8 150.4.0 | 6.180 ms | 3.937 ms | 同上（一致） |

说明：

- 两引擎对同一 HostApiRegistry 回调序列与返回值一致，满足 Phase 2 正确性验证。
- 本探针体量很小；V8 invoke 中位数偏高可能含 isolate/context 稳态与 JSON 桥开销，**不能**外推为大型 Vue 树或动画场景结论。
- 包体与常驻内存未在本轮做完整对比（V8 静态库体积显著更大是已知成本）；后续可在目标平台补 RSS / 产物体积。
- 默认产物仍互斥：`vue-quickjs` 不链接 V8，`vue-v8` 不链接 QuickJS。

完整机器可读样本见同目录 JSON。
