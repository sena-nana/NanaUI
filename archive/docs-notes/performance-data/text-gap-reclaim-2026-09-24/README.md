# 画序空隙在文字停止变长后回收（#230）

机器：Apple M4（Metal），macOS，release。**判定用计数**；CPU 用 `/usr/bin/time -l` 的
instructions retired；GPU 时间只引用负载低于 4 时交替跑、取 min 的数字（本机 Metal 没有 encoder
内的 timestamp，`gpu_ms` 是提交到完成的墙钟）。

## 文件

| 文件 | 内容 |
| --- | --- |
| `text-paint-settle-gapped.json` / `-reclaimed.json` | 一万标签 `mutate-random-settle`（240 帧），回收关掉 / 打开 |
| `gpu-ab-10k.json` | 同一 workload 600 帧，两份二进制交替 10 轮，每轮的负载与 `gpu_ms` / `batch_ms` / `gpu_upload_ms` |
| `text-paint-counters-before.json` / `-after.json` | 一万标签八个既有格子，改动前（`git archive 5f4cc03bc`、独立 target 目录）与改动后 |
| `instructions.json` | 一万标签五格，前后交替 3 次，120 帧整进程的 instructions retired |
| `gpu-scene-text-*.json` | #8 的文本门禁与 #223 / #99 的两道，全部 `ok` |

「回收关掉」是同一份源码把 `SETTLE_FRAMES` 设成 `u32::MAX` 构建的二进制。

## 怎么跑的

```bash
cargo run --release --locked -p nana-ui --features gpu --bin nana-text-paint-benchmark -- \
    --labels 10000 --workload mutate-random-settle            # 默认 240 帧
cargo run --release --locked -p nana-ui --features gpu --bin nana-text-paint-benchmark -- \
    --labels 10000 --workload mutate-random-settle --frames 600   # GPU A/B，负载 < 4 时交替

/usr/bin/time -l target/release/nana-text-paint-benchmark \
    --labels 10000 --workload mutate-random-1pct --frames 120     # instructions retired
```

`mutate-random-settle`：前 68 帧（8 帧预热 + 60 帧）与 `mutate-random-1pct` 相同——每帧随机
1% 的标签换随机长度的文本，画序进入留空隙的布局——这些帧都算预热；之后只动角落的 ticker，
与 `static` 相同。回收打开时，停止变化后第 60 个要画的帧整理一次，落在采样窗口的第 59 帧。

## 计数（一万标签，`mutate-random-settle`）

| | 索引上传 | 跨空隙的 quad / 帧 | draw / 帧 |
| --- | ---: | ---: | ---: |
| 回收关掉 | 0 | 152 085 | 15 |
| 回收打开，整理前 59 帧 | 0 | 152 085 | 15 |
| 回收打开，整理那一帧 | 544 200 B | 136 045 | 1 |
| 回收打开，之后 | 0 | 136 045 | 1 |

整理只重写索引表（136 050 槽 × 4 B），instance 上传在两边都只有 ticker 自己的 192 B/帧。
quad 少的 16 040 个是空隙与区间缩回留下的余量；draw 从 15 降到 1，是因为留空隙的布局在 churn
期间攒下、又没到预算的 break 也一起消失了。

`static` / `color` / `opacity` / `transform` / `mutate-1pct` / `mutate-random-1pct` / `table` /
`paragraphs` 的 instance、索引、draw、重建、resolve、presentation 计数与改动前逐项相同：它们要么
从不留空隙，要么每帧都有区间在长大。

## GPU（一万标签，`mutate-random-settle`，600 帧，交替 10 轮，负载 3.1–3.9）

| | min | 中位 |
| --- | ---: | ---: |
| 留空隙 | 2.863 ms | 2.912 ms |
| 回收 | **2.724 ms** | **2.830 ms** |

−4.9%（min）/ −2.8%（中位）；10 轮里回收那份有 9 轮低于留空隙那份的最小值。回收那份的 600 帧里
有 59 帧还在整理之前。整理那一帧在 `batch_ms` / `gpu_upload_ms` 的 max 里看不出尖峰。

N 的取法：一次回收整理加上文字恢复变长时再留空隙的那次整理，成本都在帧间噪声里；空隙每帧约
0.1 ms GPU，几帧就抵得上。取 60 帧（60 Hz 下一秒）是它的数倍，间歇刷新的文字留得住空隙。

## CPU（instructions retired，一万标签，120 帧整进程，3 次取 min，相对改动前）

| workload | 改动前 | 改动后 | |
| --- | ---: | ---: | ---: |
| static | 5 768 997 116 | 5 766 622 238 | −0.04% |
| mutate-random-1pct | 6 751 395 437 | 6 722 855 424 | −0.42% |
| mutate-1pct | 6 355 951 357 | 6 344 668 131 | −0.18% |
| transform | 55 047 431 623 | 55 026 516 344 | −0.04% |
| table | 7 017 918 749 | 7 008 115 057 | −0.14% |

同一台机器上 instructions 的跑间波动约 1%，都在噪声内。

## 画廊

`ui-snapshots` 620 张逐字节不变。
