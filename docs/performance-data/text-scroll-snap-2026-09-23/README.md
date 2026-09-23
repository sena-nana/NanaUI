# #223：小数横向平移吸附到设备像素（2026-09-23）

macOS，Apple M4（Metal），`--release --locked`。`text_counters` / `counters` 是**每个采样帧**的
平均值，判据读 work counter。毫秒是在负载 3–5 的机器上测的，只作观察，不作判据；「无吸附」
两行与其余行不是同一次运行。

## text-paint-benchmark：`table-scroll-x`

Runtime 的真实横向滚动：一列 `overflow-x: scroll` 的容器里放一张比它宽 40 px 的表，每帧
`set_scroll_offset` 前进 0.37 px，滚过 40 px 回到 0。滚动走 Scene 的快速路径，后代只被重新定基，不会重建。
「无吸附」一列是把 `scene_paint` 的改动撤掉后，同一 workload 在同一台机器上跑的结果。

| 标签 | 缩放 | | 重建/帧 | resolve/帧 | instance 上传/帧 | presentation 上传/帧 | batch p50 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 10 000 | 1× | 无吸附 | 10 001 | 84 030 | 3.0 MB | 99 B | 8.9 ms |
| 10 000 | 1× | 吸附 | **0** | **0** | **0** | 59 B | 4.8 ms |
| 10 000 | 1.2× | 无吸附 | 10 001 | 84 030 | 3.0 MB | 480 KB | 9.1 ms |
| 10 000 | 1.2× | 吸附 | **0** | **0** | **0** | 72 B | 4.4 ms |
| 10 000 | 1.75× | 吸附 | **0** | **0** | **0** | 101 B | 4.8 ms |
| 10 000 | 2× | 吸附 | **0** | **0** | **0** | 120 B | 4.8 ms |
| 1 000 | 1× / 1.2× / 1.75× / 2× | 吸附 | **0** | **0** | **0** | 59 / 72 / 101 / 120 B | 0.18–0.21 ms |

presentation 上传不为 0，说明画面确实在动：吸附后的整像素变化的那一帧，重写一次所有
标签共用的那一行（160 B）。0.37 px/帧在 1× 下约 37% 的帧跨过一个整像素，1.2× 下约 45%，
1.75× 下约 63%，2× 下 75%。

文件：`text-paint-{1x,1.2x,1.75x}.json`（含 `table-scroll` 对照）、`text-paint-2x.json`，
`text-paint-{1x,1.2x}-without-snap.json`。

```bash
cargo run --release --locked -p nana-ui --features gpu --bin nana-text-paint-benchmark -- \
    --labels 10000 --labels 1000 --workload table-scroll-x --workload table-scroll \
    --scale 1.2 --output target/performance/issue223/text-paint-1.2x.json
```

## 门禁：`gpu-scene-text-compositor-slide`

一千个标签的列表每帧用 CSS translate 横移 0.37 px，模拟触控板横向滚动；门禁机器是 1×。判据同
paint-color（含 instance 上传为 0，不像 transform 那样豁免）。Runtime 真实滚动与非整数缩放的数字
见上一节。

| | 重建 | resolve | instance 上传 | 由 entry 回答 | 判定 |
| --- | --- | --- | --- | --- | --- |
| 吸附 | 0 | 0 | 0 | 1000 | ok |
| 无吸附（`-without-snap.json`） | 1000 | 6000 | 240 KB | 0 | error（3 条 invariant failed） |

其余五道文本门禁（retained / paint-color / compositor-opacity / compositor-transform /
constraint-resize）在同一次改动下真机重跑，全部 ok，报告与本目录同名。

## 画廊基线

`ui-snapshots` 620 张，这次改动**没有一张变化**：没有哪张图元的最终平移带小数。本机有
10 张 settings / appearance 页的基线不一致，撤掉本改动后是同样 10 张、同样的像素数，
与本 Issue 无关。
