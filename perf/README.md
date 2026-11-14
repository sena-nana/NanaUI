# Performance harness

Shared Scenario schema and thin runners. #8 DoD is Nana work-counter /
catalog / hotspot + CI fail-closed. Cross-toolkit same-batch numbers are
[#12](https://github.com/sena-nana/NanaUI/issues/12) observation, not #8
pass/fail.

```text
perf/
├── contract.py              # schema helpers + extractors
├── schema/                  # JSON Schema for Scenario and run reports
├── scenarios/               # shared workload definitions
├── runners/{nana,iced,gpui}/
├── micro/                   # reserved; not #8 DoD (no micro suite)
├── baselines/               # reserved; do not invent a history database
└── reports/                 # generated runner output (gitignored)
```

## NanaUI 内部优化行

Layout Foundation 的结构门禁登记在 `catalog.json.nana_layout_gates`，引用 Runtime / Scene 的真实 cargo fixtures。它们复用 `WorkCounters`，由 `--check-schema` 检查源测试和 counter 名称；CI 运行对应测试。#199 的 1k context 切换、#200 的 scroll/sticky、#205 的组件实体/parts，#206 的组件常驻内存（`node_memory.rs`：布局后的 Text/Button/ListItem 在 1k 与 10k 下每节点常驻字节有上限且不随规模上涨；它量的是分配减释放，没有 WorkCounters 字段），以及既有 #214/#259/#261/#262/#263 共用此目录。它们不冒充跨框架 timing scenario。

Runtime 和 framework benchmark 报告同时导出 `layout_context_transitions`、intrinsic cache hits/misses、`intrinsic_generation_bumps`、`baseline_queries`、`cross_context_measure_hits/misses`。报告未测量的字段继续缺省，不填成 0。状态和资源身份、实体/parts 数量由真实组件测试直接断言；这与固定机器的 CPU / RSS 前后比例证据分别记录。

`catalog.json` 的 `nana_gpu_scale_ids` 列出四个 `GpuScene` / `UiOnly` 场景，量的是 GPU
节点规模与框架层 draw call：`gpu-scene-shader-nodes-256`、
`gpu-scene-shader-nodes-256-independent`、`gpu-scene-ui-dense-2k`、
`gpu-scene-host-textures-64`。它们**不在** `harness_ids` 里，不是 #8 DoD，也没有
Iced / GPUI 对照——那两个 runner 对 `GpuScene` 一律 unsupported，摆进跨框架表格只会
重新引入本文件开头警告的不可比性。规模写在 `params.node_repeat`，runner 必须原样回显，
否则 extractor 拒绝该报告。基线与判据见
[`archive/docs-notes/gpu-node-scale.md`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/gpu-node-scale.md)。

## Windows 合成器静止期

`windows-composition-steady` 量的是宿主对 OS 合成器做了多少无效功：窗口几何静止之后，
`IDCompositionDevice::Commit`、visual tree 改动、native-content region 提取和原生 chrome
重写都必须停在 0，不管 GPU 还在以多高帧率出帧。六条 invariant 全是 `eq 0`。

它**不在** `harness_ids` 里，不是 #8 §8.1 目录 id，也不是 weekly DoD：数字只有 Windows +
DX12 真机跑得出来，没有合成 target 的机器**不要**写报告——宁可没有，也不要一份全零的假报告。
产出入口是 `cargo run -p nana-ui --features "hosted bundled-fonts wgpu-interop" --example native-content-probe`，
它把静止期增量写到 `target/performance/windows-composition-steady.json`。判据与实现见
[`docs/reference/window.md`](../docs/reference/window.md)。

## Issue #98 保留期文本

`catalog.json` 的 `nana_text_ids` 是 #98 的三类文本门禁，加 #223 的亚像素平移门禁和 #99 的 constraint-only 门禁，都是一千个文本节点：

| id | 每帧动的是什么 | 判据 |
| --- | --- | --- |
| `gpu-scene-text-retained` | 其中一个标签换文本 | 静止稳态：只重建那一个，不栅格化、不传 atlas、不重传别人的 instance |
| `gpu-scene-text-paint-color` | 每个标签换前景色 | paint-only：不塑形、不排版、不栅格化、不传 atlas，也不重建 / 重传 instance |
| `gpu-scene-text-compositor-opacity` | 装标签的列表淡入淡出 | compositor-only：同上 |
| `gpu-scene-text-compositor-transform` | 装标签的列表转 −1.5° / 0° / +1.5° | compositor-only：同上，且每三帧进出一次恒等变换 |
| `gpu-scene-text-compositor-slide` | 装标签的列表每帧用 CSS translate 横移 0.37 px，模拟触控板横向滚动（#223） | 同 paint-color，instance 上传也为 0：画笔把纯平移吸附到设备像素，标签的亚像素相位不变。走 Runtime 真实滚动的数字见 text-paint 基准的 `table-scroll-x` |
| `gpu-scene-text-constraint-resize` | 每个标签在两档宽度间交替（#99） | constraint-only：不塑形、新 layout 不重新塑形、画笔不自排、不栅格化、不传 atlas；`text_layout_lookups ≥ 900` 防空转 |

它们都**不在** `harness_ids` 里，理由和 motion 那组一样——判据是这一帧让文本路径重做了
什么（`text_counters.*`），不是公共 CI 的 GPU timing。`text_counters` 里 Runtime 与画笔
两侧都有：`text_nodes_shaped` / `text_layouts_created` 是 Runtime 这一帧重新测量的，
`paint_shape_cache_misses` 是画笔拿不到 Runtime layout 时自己排的。

场景必须真的在动：`params.text_ticker`（换文本）或 `params.text_animation`
（`color` / `opacity` / `transform` / `slide` / `resize`）二选一。不动的话 painter 直接复用上一帧的批次，那一帧
什么都没做，counter 全是 0，门禁永远不会红。extractor 核对报告里回显的这两个参数，跑了
不动的场景会被拒绝。ticker 的文字是定宽的（`tick 0007`）：`tick 9 → tick 10` 会把同一行
后面的标签挪一个数字宽，那是重排的成本，不是保留期的，不该由采样窗口碰没碰上它来决定门禁红绿。

```bash
for id in gpu-scene-text-retained gpu-scene-text-paint-color \
          gpu-scene-text-compositor-opacity gpu-scene-text-compositor-transform \
          gpu-scene-text-compositor-slide gpu-scene-text-constraint-resize; do
  python3 perf/runners/nana/run.py --scenario "$id" --output "target/performance/issue98/$id.json"
done
# 标签数 × 动画频率的完整报告（不是门禁，是可复现数字）
cargo run --release --locked -p nana-ui --features gpu --bin nana-text-paint-benchmark -- --output target/performance/issue98/text-paint.json
```

`--evaluate-invariants` 判定这六个 id（缺 `text_counters` 是 skipped，不是 ok）。PR CI 用
`perf/fixtures/nana-gpu-scene-text-*.json`（真机 `nana-gpu-scene-benchmark` 原样输出）判定，
每周的 macOS 任务现跑；重录 fixture 就是对每个 id 跑一次
`cargo run --release --locked -p nana-ui --features gpu --bin nana-gpu-scene-benchmark -- --scenario perf/scenarios/<id>.json --output perf/fixtures/nana-<id>.json`。

判据与前后数字见 [`docs/reference/text-engine.md`](../docs/reference/text-engine.md) 的「保留期文本」一节。

## Issue #87 compositor motion

`catalog.json` 的 `nana_motion_ids` 列出 compositor-only 结构门禁与 1/100/1k/10k
scale、retarget、churn。它们**不在** `harness_ids`：验收是 work counter（UiWorld /
layout / style / extract / CPU sample 必须为 0），不是公共 CI GPU timing。
`#8` 的 `animation.json` 稀疏门禁（`animations_considered` /
`animation_deadlines_scanned`）继续独立存在。

```bash
python3 perf/contract.py --self-test
python3 perf/runners/nana/run.py --print-plan --scenario compositor-steady
python3 perf/runners/nana/run.py --scenario compositor-steady --output target/performance/issue87/nana-compositor-steady.json
python3 perf/runners/nana/run.py --scenario compositor-tracks-1k --output target/performance/issue87/nana-compositor-tracks-1k.json
python3 perf/runners/nana/run.py --scenario compositor-retarget --output target/performance/issue87/nana-compositor-retarget.json
cargo run --release --locked -p nana-ui-scene --features benchmark --bin nana-scene-benchmark -- --compositor --output target/performance/issue87/compositor.json
```

`--compositor` 一次 dump 全部 scale（transform / opacity / mixed）以及 retarget / churn。
`motion_descriptors_uploaded` 故意省略：该 binary 不 encode/submit。GPU descriptor
不每帧重传由 `SceneWgpuPainter::last_motion_work()` 的测试覆盖，不是这条门禁。

Inspector 入口：`UiWorld::inspect_motion()`（track / class / evaluator / base vs
presentation / CPU fallback / deadline / GPU handle / layout-paint-extract 影响），
`UiScene::annotate_motion_inspector` 补 layer 与 promotion 原因。打印合同：

```text
Node #123 transform
Class: Compositor
Evaluator: GPU
Layer: #7
Runtime samples/frame: 0
```


## Issue #89 文本迁移基准

`nana-dirty-frame-benchmark --shape layout --position head` 的 2k / 4k / 8k 三格是
Issue #33 的原始 workload，Epic #88 把它保留为 `nana-text` 的迁移基准。

它**不在** `catalog.json` 的任何 id 列表里，**没有时间门禁**，是人工前后对比：接进
scenario 合同需要新的 `kind`、extractor 和签入 fixture，等 `nana-text` 真有引擎可测再做。
CI 只保证它还在、还能编译、网格还没被收窄（`ci.yml` 的
`cargo check -p nana-ui-scene --features benchmark` 与
`the_migration_grid_still_spans_two_four_and_eight_thousand_nodes`）。

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 1000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 2000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 4000 --samples 150 --warmup 30
```

基线数字与「每阶段重跑并贴回」的规则在 [文本引擎](../docs/reference/text-engine.md#33-迁移基准)。
`nana-text` 自己的结构化 correctness 门禁是 `cargo test -p nana-text --all-targets`，
与本目录的 work-counter 合同是两回事。

## Issue #101 theme / style 基线

`catalog.json` 的 `nana_theme_ids` 是 Issue #101 §4 的 theme/style 工作量基线：static idle、
1/100/1k/10k 控件、单节点 hover / focus、light↔dark、accent-only、density、以及大树头部的
style mutation。和 motion / text 那两组一样，它们**不在** `harness_ids` 里——判据是一次主题
变化让保留期流水线重做了什么（`work_counters.style_nodes_*` / `theme_reads` /
`*_from_style`），不是公共 CI 的时序。

```bash
python3 perf/runners/nana/run.py --print-plan --scenario theme-palette-switch
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-theme-benchmark -- --output target/performance/issue101/theme.json
python3 perf/runners/nana/run.py --scenario theme-palette-switch --from-report target/performance/issue101/theme.json
# 不想重跑 benchmark 时，用归档的那份报告重放（归档在 tag docs-archive 里）：
git show docs-archive:archive/docs-notes/performance-data/theme-audit-2026-09-19/theme-work-counters.json > target/theme-work-counters.json
python3 perf/runners/nana/run.py --scenario theme-density --from-report target/theme-work-counters.json
```

这组没有单独的 `perf/fixtures/` 副本：归档报告只留一份（带日期、带机器），extractor 与门禁本身由
`perf/contract.py --self-test` 的 `theme_baseline_tests` 覆盖——合成报告能测到真实报告测不到的
负例（workload 不匹配、scale 不回显、`considered != resolved + skipped`、counter 缺失）。

报告里 theme counter 与 `#8` frame counter 落在同一个 `work_counters` 对象：名字不冲突，
放在一起才能用 `style_processed` 去核对 `style_nodes_considered`。`frame_work` 取的是测量
窗口里那次 drain 自己的计数，不是 `last_work_counters`——空帧不会替换后者，否则 idle 那行
会报上一帧的数字。

这些数字是 **Phase 0 基线**，描述现状（包括 Issue #100 要收窄的地方），不是目标值。改动
后重跑并贴回 [`docs/reference/theme.md`](../docs/reference/theme.md) 的「性能基线」一节。

## Invoke

From the repository root:

```bash
python3 perf/contract.py --check-schema
python3 perf/contract.py --self-test
python3 perf/contract.py --evaluate-invariants path/to/nana-text-table.json path/to/nana-virtual-list-10k.json
python3 perf/contract.py --evaluate-invariants target/performance/issue8
python3 perf/contract.py --evaluate-relative \
  perf/fixtures/iced-scenario-static-tree-100.json \
  perf/fixtures/gpui-scenario-static-tree-100.json

python3 perf/runners/nana/run.py --list
python3 perf/runners/nana/run.py --print-plan --scenario static-tree-100
python3 perf/runners/nana/run.py --print-plan --scenario gpu-scene-ui
python3 perf/runners/nana/run.py --scenario gpu-scene-ui-live2d   # expected exit 2
python3 perf/runners/nana/run.py --scenario static-tree-100 --output /tmp/nana-static-tree-100.json
python3 perf/runners/nana/run.py --all --output-dir perf/reports

python3 perf/runners/iced/run.py --scenario static-tree-100 --from-report perf/fixtures/iced-scenario-static-tree-100.json
python3 perf/runners/iced/run.py --scenario mutation-paint-only --from-report perf/fixtures/iced-scenario-mutation-paint-only.json
python3 perf/runners/iced/run.py --scenario hover --from-report perf/fixtures/iced-scenario-hover.json
python3 perf/runners/iced/run.py --scenario virtual-list-10k --from-report perf/fixtures/iced-scenario-virtual-list-10k.json
python3 perf/runners/iced/run.py --scenario text-table --from-report perf/fixtures/iced-scenario-text-table.json
python3 perf/runners/iced/run.py --scenario dock-workspace --output /tmp/iced-dock-workspace.json     # expected exit 2
python3 perf/runners/iced/run.py --scenario text-editor --output /tmp/iced-text-editor.json           # expected exit 2
python3 perf/runners/iced/run.py --scenario gpu-scene-ui --output /tmp/iced-gpu-scene-ui.json          # expected exit 2
python3 perf/runners/iced/run.py --scenario static-tree-50k --output /tmp/iced-static-tree-50k.json   # expected exit 2 (incomparable)
python3 perf/runners/gpui/run.py --scenario static-tree-100 --from-report perf/fixtures/gpui-scenario-static-tree-100.json
python3 perf/runners/gpui/run.py --scenario gpu-scene-ui --output /tmp/gpui-gpu-scene-ui.json  # expected exit 2
```

Iced/GPUI observation is fixture-only (`--from-report`). Live compile of
`engine/iced` / `engine/gpui-scenario-bench` is gone. Unwired kinds: exit **2**.
``--evaluate-relative`` is #12 observation, not multiplier CI.

`--from-report` maps an already-produced `nana-*-benchmark` /
`nana-gpu-scene-benchmark` / `scenario-bench` / Gallery `ui-benchmark` JSON
without invoking cargo. Use it in tests and when another job already ran the
binary. Iced Dock/TextEditor stay exit 2 even if a JSON says `status: ok`; do
not keep fake-ok files under `perf/fixtures/`.

`overscan_rows`: catalog Table (and list/tree) overscan is **8 rows**. Iced
copies that catalog param; Nana writes `mounted − visible`. Compare windows via
`list_overscan_px` / `table_overscan_y_px`. `window_ms` is index arithmetic
(Fenwick lookup may round to 0); judged work is `materialize_ms` +
`live_ui_entities`, not `window_ms`.

`--evaluate-invariants` judges Nana runner envelopes. Exit codes,
`invariants/` completeness vs `weekly/`, and gated ids are described
below in §8.1.

## Issue #242 presentation output

`catalog.json` 的 `nana_output_ids` 是八个独立工作量门禁；不进入跨框架
`harness_ids`。`output-direct` 来自真实 Window/Surface probe；其余七项来自同一设备上
真实的保留目标、Scene painter、consumer lease 与 final transfer。初始化与 30 次预热不计入
240 次测量窗口，报告同时给出调用区间的 elapsed wall time p50/p95/p99；这些时间没有公共 CI 阈值。
`output-recovery` 的时间包含非阻塞 GPU 完成轮询；其余 headless 时间计生产调用，
conversion 不包含 consumer final transfer 的耗时。nested 另外报告 parent 调用耗时；
direct 计真实窗口 prepare 至 presented 的区间。

```bash
cargo run --release --locked -p nana-ui --features gpu --bin nana-output-benchmark -- --output target/performance/output/live.json
for workload in static multi nested conversion slow recovery incompatible; do
  python3 perf/runners/nana/run.py --scenario "output-${workload}" --from-report target/performance/output/live.json --output "target/performance/output/gates/output-${workload}.json"
done
python3 perf/contract.py --evaluate-invariants target/performance/output/gates
cargo run --release --locked -p nana-ui --example presentation-output-probe --features hosted,bundled-fonts -- --output target/performance/output/window.json
python3 perf/runners/nana/run.py --scenario output-direct --from-report target/performance/output/window.json --output target/performance/output/direct.json
python3 perf/contract.py --evaluate-invariants target/performance/output/direct.json
```

静态输出与 active parent 内的静态 child 要求 producer render/submit/copy/recreate 为 0；
三消费方要求每次内容修订只 render 一次；两个不兼容组（`output-incompatible`）每帧各 render 一次、
共一次 submit，并报告共享 painter 的字形/图标/URL 纹素字节；BGRA typed/untyped 转换只增加显式 final transfer；
慢消费者填满池后必须零录制地 defer，释放后恢复；resize 验证旧代际被拒绝与有界目标重建。
真实窗口 probe 还验证 attach/detach、不兼容目标、resize 与隐藏静态输出。

`evidence` 明确区分 `actual-gpu`、`actual-window-gpu`、`mock-contract`、`unavailable`。
extractor 拒绝把 mock、不可用平台、缺失计数或不足 240 次的工作量当作 GPU 通过；没有
adapter 时 executable/runner 返回 2。XR、native-host、Web 与真实 device-loss 需要各自
平台 probe；CPU fallback 当前没有实现，不会默认选择或写入虚构的零拷贝证据。
`--self-test` 对每条 invariant 注入失败、删除计数与伪造 evidence，验证门禁确实能拒绝。

headless 的 `measurement_gpu_policy` 仅累计测量窗口，`gpu_policy` 明确包含 setup/预热；嵌套 child
与 parent 分别观测。静态 producer 的 painter 为 `skipped`、240 帧且 `work: null`，
不会用虚构的全零 draw/上传计数替代未发生的 encode。真实 encode 必须导出 draw、batch、
upload 与 GPU buffer reallocation；后者不代表进程 malloc。计时必须有 240 次样本，
分位数有限、非负且有序；测量期意外 Deferred 直接使正常场景失败，不吞掉重试工作。
GPU timestamp、进程级 malloc、driver VRAM 当前未测，报告为 null；CPU 调用时间不能
解释为 GPU 执行时间。slow/recovery 的内部 painter 未公开指标，限制在报告中单独说明。
direct 使用真实 Window 的 painter observation，并要求 240 次 encode、有 draw/batch、五项
指标均存在；direct 的 device-policy 仍为 null，不表示零上传或零等待。

direct 的 elapsed wall time 包含该区间内的 native acquire/present、调度和等待，不能视为
主线程执行时间。定位时可以保留逐帧 wall、主线程 CPU clock、两者差值和实际 prepare cadence：

```bash
cargo run --release --locked -p nana-ui --example presentation-output-probe --features hosted,bundled-fonts -- --direct-only --timing-samples --warmup-frames 240 --measured-frames 1200 --output target/performance/output/direct-diagnostic.json
```

`--timing-samples` 使用预分配的有界存储；它不安装 diagnostics，也不增加 GPU 完成轮询。
主线程 CPU clock 当前仅在 macOS 可用，其他平台为 null。wall 与 thread CPU 差值可能为负，
且不能直接解释为 GPU 等待。样本包含预热标记和实际 present/prepare 序号；同一进程内
样本存在相关性，不能将全部帧当成独立重复实验。QoS 字段仅观察请求值，不证明实际 CPU
核心、性能层级或频率。维护示例的 Rust heap observer 始终开启；计数仍只代表
Rust System 分配请求，并非 foreign malloc、驱动显存或 GPU 等待。

R5 的 kind4/P/E、Mach endpoint、native helper、focus/category 干预与 allocator on/off
实验已从维护示例移至[归档研究源码与冻结工具](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/issue242-r5-2026-10-15/research-fixture-source/README.md)。
`--execution-samples`、`--focus-schedule`、`--task-category-policy`、`--allocator-observation`
在维护示例中会明确报迁移错误，不能静默执行另一种工作负载。旧实验仅使用已记录 hash 的
冻结二进制或其精确源码快照；原 raw、构建记录和不完整/失败证据继续保留。
研究时延、后台负载和平台证据不能转移为精简后的新二进制性能结论。

正式 `output-direct` extractor 拒绝任何非 null 的逐帧诊断、stage profile、producer/consumer
trace，也拒绝 direct-only 和 allocator-off 报告。默认完整六阶段、allocator-on 报告才能
进入原有门禁；历史报告缺失或 null 的可选字段保持兼容。诊断实验必须单独保存，不能混入
默认时延对照；比较时记录原始样本、二进制哈希、运行顺序和后台干扰。

The two Issue #242 fixtures install a binary-only `System` allocator wrapper. `measurement_rust_heap` reports 240 producer-call intervals (nested parent separately); the Window interval is prepare through presented. Relaxed atomic counters record requested alloc/realloc calls and bytes plus dealloc calls/bytes across all Rust threads in that process. Reading counters does not allocate, and report aggregation/serialization occurs outside each interval. Observer overhead is included in CPU timings. These are not live/peak heap or process-wide malloc measurements: foreign/native driver allocations and VRAM remain unobserved. Failed requests are counted as requests. Neither framework allocator behavior nor GPU work changes.

静态复用的零 producer GPU 工作不代表零 CPU 分配。当前 `PresentationDriver::render_scene`
为返回 outcomes 创建 `Vec::with_capacity(self.targets.len())`，全 Reused 也会请求分配；
本机诊断观测为每次 producer 调用一次 Rust allocation。未来若要求 CPU allocation-free，
可以另提供 caller-owned outcomes 或迭代接口，不以改写观测数据替代优化。

最初 output 改造前未保留相同配置的 Release 耗时 baseline，因此不宣称相对改造前的时延改善。
历史默认门禁与受控调查的配置、结果和保留证据见[交付记录](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/issue242-delivery.md)。
原始 trace、冻结 executable、失败记录和完整研究工具保留在原工作区，未作为代码提交上传。
direct 时延信号与 P/E 推荐触发原因由 [#354](https://github.com/sena-nana/NanaUI/issues/354) 继续跟踪；
GPU duration/driver VRAM 未观测时保持 unavailable，不能由 CPU 或分配计数替代。

### External transport work

`catalog.json` 的 `nana_transport_ids` 另维护 9 个 transport 门禁，不能把原来七项
presentation 工作量重命名为 XR/Web 验收。六个 portable workload 真正通过
`OverlayHostAdapter` 向独立的消费目标执行 GPU transfer：overlay 静态、变化、两个独立
transport、饱和后恢复；`web-*` 这里只是同一通用 transport 的 mock foreign-retirement
静态与变化场景，不代表已安装 Electron bridge。实际 Electron native import 由
`examples/web-output-bridge` 单独验证。
foreign receipt 明确使用 mock，证据为 `actual-gpu-transfer-mock-foreign-retirement`，
`platform_conformance` 必须为 false；这些门禁通过也不代表 native handle import、
SteamVR 或 Chromium 已运行。producer 与 consumer 的 render/pass/submit/bytes、CPU 调用
及 Rust heap 分开记录，consumer 的复用/饱和不能记成 producer GPU 工作。

```bash
cargo run --release --locked -p nana-ui --features gpu --bin nana-output-benchmark -- --transport-only --output target/performance/output-transports.json
for workload in overlay-static overlay-changing overlay-multi overlay-slow web-static web-changing; do
  python3 perf/runners/nana/run.py --scenario "transport-${workload}" --from-report target/performance/output-transports.json --output "target/performance/transport-gates/${workload}.json"
done
python3 perf/contract.py --evaluate-invariants target/performance/transport-gates
```

这些场景同样先预热 30 次，再测 240 次；全部 producer/consumer 输出工作项都有精确
预算，静态/饱和工作还检查实际上传和等待策略计数。mock transport 调用发生在本机 GPU
transfer 完成后，foreign 生命周期不能从本机 submission 完成推断。GPU 完成轮询位于
CPU 调用计时之外。`--transport-only` 独立运行，不改变原七个工作量的默认执行顺序。

`PresentationDriver::work()` 保留累计兼容语义：同一次 producer 调用之后执行
`present_to`，返回值会包含实际 consumer transfer。判断静态 producer 是否零 GPU 工作时，
先在 consumer 调用前保存该值，或使用按 owner 区分的 `output_traces()`；不能把后续消费
提交算成 producer 工作，也不能从一个 encoder 的 pass 推断调用者已执行 submission。

三个真实平台场景只接受各自 reference 的实际报告，不自动换成 portable mock：

| 场景 | 生产与消费证据 |
| --- | --- |
| `transport-native-host` | Windows `window-output-probe --perf-output ...`：30 个 native token 预热，再进行 240 次 D3D11 导入目标 copy；source Window 继续运行，实际多出的源帧/导出 copy 仍如实计数。最后观察真实 release fence 完成。perf 模式没有像素回读，普通功能 probe 的 test-only 像素验收独立运行。 |
| `transport-openvr-native` | `openvr-output-probe` 的 `shared_atomic` 模式：真实 SteamVR + DXGI shared handle；先完成 30 次实际 native + consumer 更新，再测量 240 次原子更新与 240 次静态零工作检查。1 个稳定 handle 的首次 compositor 绑定和 target allocation 单列 setup；稳态 SDK call 为 skipped（0 样本、null 分位数）。持续引用该 allocation 不代表资源已退休。 |
| `transport-web-native` | 专用 Electron bridge：真实 NT handle import、WebGPU draw/submit、`allReferencesReleased` 后回收。30 次实际导入、显示与 foreign release 预热后再测 240 次，源端还须观察真实 release fence 完成才开始计数。270 次总 proof、30 次 warmup proof 与 240 次 measured proof 分列；producer call CPU 与 renderer import/encode/submit CPU 分开。Chromium 内部 copy、GPU 执行时间和显存仍未观测。 |

```bash
# Windows native-host: functional pixel verification remains a separate run.
cargo run --release --locked -p nana-ui --example window-output-probe --features hosted,bundled-fonts,native-export -- --perf-output target/performance/native-host.json
python3 perf/runners/nana/run.py --scenario transport-native-host --from-report target/performance/native-host.json --output target/performance/native-host-gate.json
python3 perf/contract.py --evaluate-invariants target/performance/native-host-gate.json
# OpenVR/Electron: feed the actual reference report from the corresponding host.
python3 perf/runners/nana/run.py --scenario transport-openvr-native --from-report target/performance/openvr.json --output target/performance/openvr-gate.json
python3 perf/runners/nana/run.py --scenario transport-web-native --from-report target/performance/electron.json --output target/performance/web-gate.json
```

平台不可用仍为 `unsupported`，不是 invariant-ok；平台标签必须真实为 Windows，
fixture 版本、设备代际、固定 BGRA extent 和预热范围必须符合维护中的 reference。
成功报告缺少必要工作计数、完成证明、consumer CPU 样本、有效计时，或出现 mock 升级为
平台证据时，runner 返回 `error`，不能把坏报告降级为平台缺失。actual reference 的 CPU 时间按各自
报告 scope 解读，不与 portable 稳态或 GPU execution time 混为同一指标。

## 平滑滚轮 CPU 采样诊断

```sh
CARGO_INCREMENTAL=0 RUSTC_WRAPPER= CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p nana-ui-runtime --test perf_contracts smooth_scroll_cost --no-default-features --locked -- --ignored --nocapture
```

这是独立的 Runtime 微测量，不属于共享场景门禁。每种组合先预热 32 次，再测 512 次上下交替的 180ms 滚动；覆盖隐藏/自动隐藏滚动条、有/无事件监听器。报告采样数、线程内分配/重分配次数及每次完整过渡的 CPU 时间中位数和 p95；计时包含启用分配计数器的成本。测量包含动画 deadline 查询、采样和 system-work drain，不含起始滚轮输入、实际样式/布局执行、场景抽取、命中索引更新或 GPU 呈现。

比较时必须使用同机、同构建配置和相同采样数，交替运行两个版本；调试构建绝对时间不能代表发布构建性能。该探针不能证明原生窗口帧率、输入延迟或耗电改善，真实长列表和订阅回调的业务开销需另测。

本轮改动的同机交替测量和边界见 [2026-10-16 采样开销记录](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/smooth-scroll-cost-2026-10-16.md)。

Inline plain-label 固定机器成本使用 `crates/nana-ui-scene/tests/scene_perf/inline_cost.rs`。保留前后 executable，交错运行各至少三次；`NANA_INLINE_COST_OUTPUT` 写原始报告。manifest 的 `machine` 标明固定机器，`before` / `after` 是互不重复的报告相对路径数组。`python3 perf/contract.py --evaluate-inline-cost <manifest>` 复用现有 contract CLI，执行 idle p95 ≤1.05、retained live bytes ≤1.08、无新增 idle 分配及单实体门禁。它不推断 dirty-frame 成本，也不把进程 RSS 当作 live allocator bytes。

同一 fixture 的 `list_item_fixed_machine_cost` 单独覆盖 1k/10k label/note/detail 行，默认 ignored，使用 `--exact list_item_fixed_machine_cost --ignored --nocapture` 运行。它与 plain-label fixture 不并发执行，避免同一进程的全局 allocator 计数互相污染。对应 manifest 设置 `workload_family: "list_item"`；默认 `plain_label` 仍要求原有四项，行系列要求两项完整 workload，均沿用同一组比例门槛。

`labeled_value_fixed_machine_cost` 和 `card_fixed_machine_cost` 也以 `--ignored --exact <name> --nocapture` 单独运行，manifest 的 `workload_family` 分别为 `labeled_value` / `card`，各要求 1k/10k 两项。Card workload 保留 title/spinner 部件并冻结动画时钟，测量的是 idle-flush 和 retained bytes，不能代表 spinner 活动帧成本。
