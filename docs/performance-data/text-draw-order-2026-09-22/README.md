# 画序索引表：draw 不再依赖 instance 存储顺序（#224）

机器：Apple M4（Metal），macOS，release。本机负载常年 3–15，另有别的项目的 GPU 基准在跑，
**判定用计数**；CPU 用 `/usr/bin/time -l` 的 instructions retired（不受负载影响），毫秒与
GPU 时间只引用负载低于 4 时交替跑、取 min 的数字。

## 文件

| 文件 | 内容 |
| --- | --- |
| `instructions.json` | 一万标签十格，前 / 紧区间（中间版本）/ 最终三份二进制交替各 3 次，120 帧整进程的 instructions retired，附最终版每格的计数 |
| `text-paint-ab-10k.json` | 一万标签 `static` / `mutate-random-1pct` / `mutate-1pct` / `transform`，前与最终交替 12 轮，CPU（`flush + batch + upload` p50）与 GPU（提交到完成 p50），附全部样本 |
| `gpu-scene-text-*.json` | #8 的四道文本门禁与 #99 的 constraint-resize，全部 `ok`；retained 那道新增 `text_index_upload_bytes ≤ 4096`，实测 0 |

「前」是 `git archive 81a61b7e8` 导出、独立 target 目录构建的同一个基准，只补了
`text_pipeline_draws` 计数和 GPU / encode 两项计时，没有在共享工作区里切换文件。

## 怎么跑的

```bash
cargo run --release --locked -p nana-ui --features gpu --bin nana-text-paint-benchmark -- \
    --frames 60 --labels 10000 --workload mutate-random-1pct   # 等

/usr/bin/time -l target/release/nana-text-paint-benchmark \
    --labels 10000 --workload mutate-1pct --frames 120         # instructions retired

for id in gpu-scene-text-retained gpu-scene-text-paint-color \
          gpu-scene-text-compositor-opacity gpu-scene-text-compositor-transform \
          gpu-scene-text-constraint-resize; do
  python3 perf/runners/nana/run.py --scenario "$id" --output "$id.json"
done
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked
```

`gpu_ms` 是每帧提交后等 GPU 做完的墙钟时间。本机 Metal 不支持 encoder 内的 timestamp
query，所以它包含提交本身，也受同时在跑的其它 GPU 负载影响：负载一上到 7–9，同样的工作
能差 ±50%。

## 计数（`mutate-random-1pct`，每帧）

| | 标签 | 帧 | instance | 索引 | draw |
| --- | ---: | ---: | ---: | ---: | ---: |
| 前（整 arena 按画序重排） | 10 000 | 60 | 1 136 190 | — | 86.6 |
| 紧区间（中间版本） | 10 000 | 60 | 92 642 | 179 076 | 89.9 |
| **最终** | 10 000 | 60 | **40 300** | **65 107** | **20.7** |
| 最终 | 10 000 | 240 | 56 520 | 45 151 | 26.5 |
| 最终 | 10 000 | 600 | 49 975 | 24 758 | 36.2 |
| 前 | 1 000 | 60 | 109 177 | — | 9.9 |
| 最终 | 1 000 | 60 | 9 363 | 8 208 | 6.3 |

`transform`（整片在转）一万标签：instance 523 736 → **0**，索引 87 289，draw 56.5 不变。
`static` / `color` / `opacity` / `transform-panel` / `mutate-1pct` / `table` / `paragraphs`：
与前相同，索引 0。

读法：

- **instance 只剩被重建段落自己的块**：一万标签 60 帧每个重建段落约 410 B。紧区间那一版
  还有 arena 放不下时的整理（约一半的字节）；还回来的空间改成与相邻空间合并、按最合适的
  大小切给下一块之后，这个窗口里 arena 一次都不用整理。
- **索引**：每 512 槽留 128 槽空隙，一段文字长大就挪进紧跟着的空隙，不再搬到表尾、拆两个
  draw。只在「区间换位置多半是因为段落长大」时才留——`transform` 的区间换位置是因为标签
  转进转出视口，它没有空隙，字节与 draw 与紧区间相同。
- 两项合计约为前的九分之一。

## CPU（instructions retired，一万标签，120 帧整进程，3 次取 min，相对前）

| workload | 紧区间 | 最终 |
| --- | ---: | ---: |
| static | +0.0% | +1.2% |
| static-unique | +0.4% | +1.1% |
| color | +0.0% | +0.2% |
| opacity | +0.5% | +0.7% |
| transform | −0.3% | +0.1% |
| transform-panel | +0.4% | +0.8% |
| mutate-1pct | +0.6% | **−4.7%** |
| mutate-random-1pct | +0.1% | **−2.3%** |
| table | +0.4% | +1.1% |
| paragraphs | +0.2% | +2.1% |

同一台机器上 instructions 的跑间波动约 1%。

有重建的格子变便宜，是因为 instance 块与索引区间不再各自 `queue.write_buffer`——每次调用在
Metal 上约五万条指令（把同样的字节拆成两次写、每帧多一百次调用，指令数多 0.63e9 / 128 帧，
由此量出），一帧重建一百个标签就是一百次。现在一帧的写全部排进每个 target 自己的 staging
ring（一到两次 `write_buffer`），再在 encoder 里逐块 `copy_buffer_to_buffer`。每帧只写一小块
的格子因此多一条拷贝命令，约四十万条指令（约 1%）。

低负载时交替 12 轮（`text-paint-ab-10k.json`）：`mutate-random-1pct` CPU −6.1%、
`mutate-1pct` −7.5%；GPU 四格在 −2.6% 到 +12% 之间，其中 `static` 的 +12% 单独复测为
2.345 → 2.311 ms（−1.5%），属于噪声。

## 试过、没有采用的

都在一万标签 `mutate-random-1pct` 上，同一轮里交替 8 轮取中位（四倍余量那行来自另一轮，
同轮两倍余量是 3.28 ms）：

| 做法 | 索引 B/帧 | draw | GPU 中位 |
| --- | ---: | ---: | ---: |
| 紧区间 | 179 076 | 90 | 2.79 ms |
| 每个区间长大时留到两倍 | 262 707 | 89 | 3.02 ms |
| 拆碎因长大而起时，整理给每个区间留两倍 | 21 144 | 1 | 3.32 ms |
| 同上，只留到最近长到的那一档 | 20 926 | 1 | 3.27 ms |
| 四倍余量 | 35 241 | 1 | 3.89 ms |
| 前（整 arena 按画序重排） | — | 87 | 2.88 ms |

每个空着的索引槽都是顶点阶段照样要跑的一个四角 quad（本机约 2 ns 一槽）；每段留余量的
做法让索引槽多到两倍半，成片空隙只多两成。

空隙的尺寸也扫过（60 帧 / 600 帧）：

| 每 N 槽 | 空隙 | 索引 B/帧 | draw |
| ---: | ---: | --- | --- |
| 128 | 1/8 | 66 724 / 26 507 | 50.7 / 67.4 |
| 128 | 1/4 | 45 917 / 15 163 | 38.9 / 45.5 |
| 256 | 1/8 | 61 176 / 22 539 | 37.4 / 38.7 |
| 256 | 1/4 | 49 904 / 17 833 | 27.4 / 51.5 |
| 512 | 1/8 | 76 316 / 27 896 | 27.5 / 33.9 |
| **512** | **1/4** | 64 233 / 25 258 | **13.7 / 24.1** |

（这张表是空隙对每次整理都生效时量的；最终只在文字长度在变时生效，第一次整理不留，
所以 60 帧那格是 20.7 个 draw。）break 预算也扫过：每 256 槽一个 draw 让索引字节减半，draw
翻三倍，GPU 慢约 2%；每 64 槽一个更差。
