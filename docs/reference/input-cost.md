# 单次指针事件的成本与脏帧分析

本文记录 Rust L3 在指针高频移动（hover）下的实测开销、组件类型索引的收敛过程，以及当事件处理器触发真实视图更新时的脏帧成本。原始性能档案见 [`archive/docs-notes/performance-data/vue-hover-2026-09-08/`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/vue-hover-2026-09-08) 与 [`archive/docs-notes/runtime-dirty-frame.md`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/runtime-dirty-frame.md)。

## 核心结论

1. **指针事件派发本身为 $O(1)$ 常数开销，不随树规模增长。** 在 250 至 2,000 行的列表上，Rust L3 单次指针移动恒为 **`0.0003 ms`**。
2. **当局部修改的欠账本身为常数时，Runtime 增量布局与绘制同样保持常数耗时**（502 至 8,002 节点均在 `0.008–0.011 ms`）。改动首行尺寸会让下面每一行移位，这部分 $O(N)$ 是应付的工作。

## 实测数据（macOS / Apple Silicon，`--release`，60 次预热 + 400 次移动，P50 毫秒）

测量对象是一列定高 `24 px` 的行，指针逐行下移并回绕，每次移动后 flush 一帧。

| 行数 | Rust L3 |
| ---: | ---: |
| 250 | 0.0003 |
| 500 | 0.0003 |
| 1,000 | 0.0003 |
| 2,000 | 0.0003 |

### 脏帧的两处 $O(N)$ 修复

消除「布局脏集向无障碍投影广播至文档根」以及「布局引擎对每个兄弟节点重复解析 effective style」两处 $O(N)$ 瓶颈后，纯常数局部改动的耗时从随节点数线性增长收敛为 $O(1)$：502 至 8,002 节点均为 **`0.008–0.011 ms`**。

## Rust L3 路径的组件类型索引（`0.0819 ms` → `0.0003 ms`）

早期基准曾把事件时间戳固定传为 `Duration::ZERO`，导致带 `8 ms` 节流的路径只执行第一次，掩盖了 L3 侧随节点数增长的扫描开销（真实 2,000 行耗时为 `0.0819 ms`）。将基准改为每事件推进固定 `16 ms` 步长后，分段计时抓出了四处「通过扫描全文档 `document_order` 查找特定组件」的线性热点：

- `active_runtime_overlays`（改用已有的 `UiWorld::overlay_hosts_by_document` 索引）；
- `split_handle_near`、`sync_split_handle_hover` 与 `clear_calendar_heatmap_hover`。

**根治方案**：在唯一权威 `UiWorld` 内维护 `nodes_by_component: HashMap<ComponentTypeId, HashSet<StableNodeId>>`，于 `SetComponentType` 应用与节点销毁（despawn）时同步更新（直接维护在唯一权威 `UiWorld` 内，所有写入 `SetComponentType` 的路径均自动同步，不依赖上层 `AppContext` 包装）。修复后，L3 单次指针事件在任何规模下均稳定在 **`0.0003 ms`**，且 `component-gallery` 的 559 张像素基线全部匹配。

## 测量命令

输入路由本身（不 flush、不布局、不抽取 Scene）：

```bash
cargo run --release -p nana-ui-devtools --features runtime-agent \
  --bin nana-input-benchmark -- --rounds 15 --events 4000
```

一次 `RuntimeDocument::flush` 随脏节点数与总节点数的变化：

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 2000 --samples 150 --warmup 30
```

`--position tail` 改动末尾行，下面没有行要移位；仍随文档增长的成本就是过度失效。比较两个构建时交替运行并读最小的 p50。
