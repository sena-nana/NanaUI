# 保留期文本收尾：矩阵、门禁与前后 A/B（#98）

机器：Apple M4（Metal），macOS，release。本机常年有别的会话在编译，负载 5–15，
所以**判定用计数**，毫秒只用来确认「结构上的收益没有拿更多 CPU 换」。

## 文件

| 文件 | 内容 |
| --- | --- |
| `text-paint-matrix.json` | `nana-text-paint-benchmark` 全矩阵：14 种 workload × 1 / 100 / 1k / 10k 标签，动画类 workload 另跑 60 / 120 / 240 帧 |
| `text-paint-ab.json` | 会话开始时（`04a55ecbd`）与收尾后的同一批格子，交替两种跑序各 5 轮，取 min |
| `gpu-scene-text-*.json` | #8 的四道文本门禁的真机报告，全部 `ok` |

## 怎么跑的

```bash
cargo run --release --locked -p nana-ui --features gpu \
    --bin nana-text-paint-benchmark -- --output text-paint-matrix.json

for id in gpu-scene-text-retained gpu-scene-text-paint-color \
          gpu-scene-text-compositor-opacity gpu-scene-text-compositor-transform; do
  python3 perf/runners/nana/run.py --scenario "$id" --output "$id.json"
done
```

A/B 的「前」是用 `git archive 04a55ecbd` 导出、独立 target 目录构建的同一个基准，
没有在共享工作区里切换文件。每格的量是 `flush p50 + batch p50 + upload p50`。

## 计数（每帧，一万标签，60 帧那一档）

| workload | 重建 | 新栅格化 | instance 字节 | run/presentation 字节 | skipped / considered |
| --- | ---: | ---: | ---: | ---: | --- |
| `static`（旁边一个标签换文本） | 1 | 0 | 192 | 0 | 10 000 / 10 001 |
| `color` | 0 | 0 | 0 | 480 000（每个标签的 run 行，48 B） | 全部 |
| `opacity` | 0 | 0 | 0 | 0（淡入淡出在 opacity group 上） | 全部 |
| `transform`（整个文档在转） | 0 | 0 | 523 736 | 100 | 转进转出视口的标签让 arena 重排 |
| `transform-panel` | 0 | 0 | 0 | 128 | 9 998 / 10 001 |
| `table-scroll`（整像素滚动） | 0 | 0 | 0 | **96**（前 478 810） | 9 974 / 10 001（其余滚出了视口） |
| `table` | 1 | 0 | 192 | 0 | 10 000 / 10 001 |
| `paragraphs`（1 250 段换行段落） | 1 | 0 | 192 | 0 | 1 250 / 1 251 |
| `multi-window`（两个窗口共享 Device） | 2 | 0 | 384 | 0 | 20 000 / 20 002，字形只栅格化一次 |
| `mutate-1pct` | 100 | 0 | 24 000 | 0 | 9 901 |
| `mutate-random-1pct` | 97 | 0 | 1 136 190 | 0 | 9 904，见下 |
| `atlas-pressure` | 100 | 400 | 235 454 | 0 | 9 901，陈旧句柄 0 |
| `zoom`（1× → 2× 场景缩放） | 249 | 1.3 | 304 987 | 26 472 | 5 021，每经过一档可见标签各重建一次 |

读法：

- `color` / `opacity` / `transform-panel` / `table-scroll` 这四种纯呈现变化，**一个字形都没有重新解析**。改色写的是每个标签自己的 run 行（颜色本来就在那里），其余三种只动共享的一两行。
- 60 / 120 / 240 帧三档每帧的重建、栅格化计数相同：没有哪一项按时间摊。唯一有差别的是整片转的 `transform` 的 instance 字节（52–58 万），那是转进转出视口的标签触发的重排，随采样窗口里转到的角度变。
- `mutate-random-1pct` 的上传大是块长大搬家之后整 arena 重排——那是「多一次 draw」
  与「重写 1024 个槽」之间有意的平衡，见文档「这一阶段没做的」。一千标签跑 960 帧，
  「长大过的块不轻易缩回」这一条把它从 52.0 降到 40.5 KB/帧。
- `zoom` 的重建来自档位切换：60 帧里经过 √2、2 两档再从 2× 跳回 1×，每次切换可见标签各重建一次；另有缩小时转回视口的标签。它不随帧率变（三档相同），也就是不是按帧在重栅格化——新栅格化每帧 1.3 个，其余都是 raster cache 命中。

## A/B（`flush + batch + upload`，ms，每格 10 个样本取 min）

| workload | 1k 前 | 1k 后 | 10k 前 | 10k 后 |
| --- | ---: | ---: | ---: | ---: |
| static | 0.149 | 0.145 | 2.409 | 2.402 |
| static-unique | 0.150 | 0.149 | 2.526 | 2.459 |
| color | 0.855 | 0.864 | 11.976 | 11.562 |
| opacity | 0.279 | 0.274 | 8.064 | 8.186 |
| transform | 0.658 | 0.669 | 14.767 | 13.148 |
| transform-panel | 0.205 | 0.205 | 4.935 | 4.924 |
| mutate-1pct | 0.194 | 0.196 | 3.155 | 2.968 |

所有格子都在 −11% 到 +1.7% 之间：这一轮修掉的是正确性、内存与门禁，结构性的收益
落在上面的计数里（滚动、进出恒等变换、多窗口），原有格子没有拿更多 CPU 去换。
