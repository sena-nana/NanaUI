# 画笔取 Runtime 保留 layout 的 A/B（#99）

`TextPipeline::prepare` 改成直接画 `ScenePrimitiveKind::Text.layout` 指的那份
`TextLayout`，而不是按同一套约束再排一遍。这份数据是那次改动的前后对照。

- **A**：画笔自己排（`lay_out` + `ShapeCache`）。
- **B**：取 Runtime 的句柄。

## 怎么跑的

同一个二进制，临时用环境变量切 A/B（量完即删，产品代码里没有这个开关——release
不得暴露双 layout 来源）。**两种跑序各 3 轮交替**（A,B,B,A × 3），每格取
`batch_ms.p50` 的 **min**：这台机器的背景负载能让单次 p50 跳好几倍，B 侧就有一个
7.2ms 的离群样本，取 min 才不会把负载读成信号。

```bash
cargo run --release --locked -p nana-ui --features gpu \
    --bin nana-text-paint-benchmark -- --output <out>.json
```

## 结果

| workload | A batch p50 | B batch p50 | |
| --- | --- | --- | --- |
| `static-unique` 10k | 2.949 ms | 2.070 ms | −29.8% |
| `static` 10k | 2.373 ms | 1.995 ms | −15.9% |
| `mutate-1pct` 10k | 2.222 ms | 2.037 ms | −8.3% |
| `mutate-1pct` 1k | 0.144 ms | 0.128 ms | −10.8% |

`shape_cache_misses`：A 侧 churn 的 workload 上非 0，B 侧**全部为 0**——画笔在这些
workload 上一次都没有排过段落。

两点解释：

- `static` 两边 `shape_cache_misses` 都是 0，省下的是**每节点每帧一次 HashMap 查询**：
  旧路径要 `shape_cache.holds(hash)` 确认段落还在缓存里；句柄由场景持有，不需要问。
- `static-unique` 的 30% 是旧路径为 1 万条不重复段落维护缓存的代价，现在一条不存。

原始逐格数据见 `text-paint-ab.json`（含 flush_ms 与两侧计数器）。
