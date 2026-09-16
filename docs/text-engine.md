# 文本引擎骨架（nana-text）

给**改 NanaUI 文本的人**。写应用不需要看这篇：`nana-text` 现在不在产品路径上。

Epic #88 要把文本能力从 `cosmic-text` / `cryoglyph` fork 上迁走。#89 是其中的 Phase 0：
先把内部合同、reference backend 和 correctness corpus 固定下来，让后续每一阶段都能对着
同一份结构化基线比较，而不是在 shaping / layout / GPU 三层同时改动时失去可比性。

## 这是什么

```text
corpus/cases/TX-*.json          输入：文本 + 样式 + 约束 + 探针
        │
        ├── tests/reference/     cosmic-text 参照引擎（临时，仅 dev 依赖）
        │        ↓
        │   Nana IR TextLayout
        │        ↓
        └── corpus/golden/TX-*.layout.json   签入的基线
                 ↓
        parity::compare(expected, actual, &tol) -> Vec<LayoutDelta>
```

`compare` 是唯一的结构化 diff。Phase 0 比「参照引擎 vs golden」；后续阶段原生引擎接上
**同一个函数**，比「原生 vs golden」和「原生 vs 参照」。不换实现，不重写断言。

产品文本仍然走 `crates/nana-ui/src/nana_text.rs`（cosmic-text 后端）与
`crates/nana-ui/src/scene_paint/text.rs`（cryoglyph 绘制）。本阶段一行都没动它们。

## nana-text 自己拥有什么，什么留在成熟 crate 上

`nana-text` 拥有的是生命周期、IR、缓存合同与编排，**不重写标准重型算法**。

| 事项 | 归属 | 理由 |
| --- | --- | --- |
| 文本 IR（`TextKind` / `TextSource` / `TextStyle` / `TextConstraints` / `ShapedRun` / `LineBox` / `TextLayout`） | **nana-text** | 跨层长期 ABI，不能是第三方内部类型 |
| 稳定代际 ID 与失效规则（`FontId` / `ShapeRunId` / `TextLayoutId` / `TextRevision` / `FontGeneration`） | **nana-text** | 缓存正确性的权威 |
| 命中测试 / caret / 选区矩形 | **nana-text** | 纯函数，跑在 IR 之上；写不出来就说明 IR 缺字段 |
| 结构化 diff 与容差 | **nana-text** | 迁移验收合同，必须比 cosmic 活得久 |
| 排版词汇（变体轴 / kerning / line-break / word-break / direction / writing-mode / wrap-break / line-height / feature） | **nana-ui-core** | 已是后端中立令牌；重造一套只会在 UiWorld 接缝上长出一个有损转换器 |
| OpenType 表解析、字形轮廓、变体插值 | **成熟 crate**（skrifa / ttf-parser） | #89 非目标明确写了不重实现 |
| 复杂文字整形（GSUB/GPOS、Arabic joining、印度系重排） | **成熟 crate**（harfrust） | 同上 |
| Unicode 算法（BiDi、断行、字素簇、script 判定） | **成熟 crate**（unicode-bidi / unicode-linebreak / unicode-segmentation / unicode-script） | 同上 |
| 系统字体发现与 `@font-face` 注册 | **成熟 crate**（fontdb）+ 宿主 | nana-text 只发 `FontId` 与 `FontGeneration` |
| 字形栅格化、图集、GPU instance | 现有 cryoglyph 路径 | #89 非目标；由 Epic #88 的后续阶段接手 |

`nana-text` 只许 import 这些 `nana_ui_core` 项：`DirSpec`、`FontFeatureSetting`、
`FontKerningSpec`、`FontVariationSetting`、`LineBreakSpec`、`LineHeightSpec`、
`TextWrapBreak`、`WordBreakSpec`、`WritingModeSpec`。其余（`LayoutStyle`、`ComputedStyle`、
语义色、几何）是边界违规，见「边界如何被机器守住」。

两处例外自己定义：`TextRect`（`LogicalRect` 不能 `Serialize`，且构造时把宽高 clamp 到
`>= 0`，会把 RTL / 退化 bounds 的回归悄悄抹平）、`TextScale`（core 里没有文本用的分数设备缩放）。

## ID 与失效

句柄都是 `{ index, generation }`，`generation == 0` 是 NULL，照 `MotionHandle` 的约定。
槽位回收后以更高的 generation 重发，旧句柄因此被拒绝而不是别名到新内容。

| 句柄 | 签发者 | 何时失效 | 如何发现 |
| --- | --- | --- | --- |
| `FontId` | font db | `FontGeneration` bump，或槽位被回收重用 | 槽位代际不匹配 |
| `ShapeRunId` | shaper | source 字节、`TextStyle`、script/direction 分段、`FontGeneration` 任一变化 | 槽位代际不匹配 |
| `TextLayoutId` | layout | `TextRevision` / `TextStyle` / `TextConstraints` / `FontGeneration` 任一变化 | 槽位代际不匹配 |
| `TextRevision` | `TextSource` | 单调递增，每个 mutating 方法都 bump | `layout.revision != source.revision()` |
| `FontGeneration` | engine | 每次字体集合变更 bump | `layout.font_generation != engine.font_generation()` |

两条承重的不变量：

- `TextSource` 的字段是私有的。改字节的唯一入口是会 bump revision 的方法，没有绕过的路径——
  这才是 revision 值得拿来做缓存键的原因。
- `TextLayout` 自带产出时的 revision 与 generation，所以判断陈旧是 O(1)
  （`TextLayout::is_stale`），不需要像今天的 `ShapedLayoutKey` 那样重新指纹整串文本。

`TextRevision` **只在单个 `TextSource` 内可比**：两个不相干的 source 都可能停在 7。缓存键必须
配上调用方自己的节点身份。

## 语料与 golden

`crates/nana-text/corpus/cases/TX-*.json` 是输入，`corpus/golden/TX-*.layout.json` 是签入的
基线。id 前缀即类别：

| 前缀 | 覆盖 |
| --- | --- |
| `TX-L*` | Latin、kerning |
| `TX-C*` | 汉字、假名混排 |
| `TX-K*` | 谚文 |
| `TX-M*` | 组合记号 |
| `TX-G*` | 连字（`liga` 开 / 关） |
| `TX-B*` | Arabic 连写与纯 RTL、mixed BiDi（base ltr / base rtl） |
| `TX-Z*` | emoji、ZWJ 序列、变体选择符 |
| `TX-F*` | 多字体 fallback、缺字 |
| `TX-V*` | 可变字体 `wdth` 与自定义 `BEVL` 轴 |
| `TX-W*` | 单行、显式换行、word wrap、glyph wrap、max-lines + ellipsis |
| `TX-D*` | 分数 DPI（1.25 / 1.5） |
| `TX-E*` | IME preedit |

`status` 是 `pass` 或 `ignore`；`ignore` 必须同时填 `gap` 和 `gap_note`，否则
`corpus_is_wellformed.rs` 会红。`parity::CATEGORIES` 列出 #89 要求的每个类别，并断言每个类别
至少有一条 passing 用例——一个悄悄丢掉最后一条用例的需求，看起来和一个通过的需求一模一样。

重新录制：

```bash
NANA_TEXT_BLESS=1 cargo test -p nana-text --test text_parity_corpus
```

bless 只记录，不判定：它写完 golden 就返回，不做比较。重新 bless 必须是一次**可见、可评审的
diff**，永远不自动发生。这些 golden 是 hermetic 字体库下的纯 Rust 度量值，跨机器确定，和
[像素快照](pending-snapshot-bless.md)不同，本机可以放心 bless。

### 确定性靠 hermetic 字体库

产品路径用的是进程级、从系统字体播种的 `FontSystem`。照它录的 golden 只在一台机器上成立。
参照引擎因此从不碰它：每个用例新建一个空 `fontdb::Database`，只按用例声明的顺序装载它声明的
fixture，locale 固定 `en-US`。face id 因而确定，fallback 链就是用例的字体列表，缺字用例也白送
（只装 `nana-test-vf` 时，除 `A` 外一切都是 `.notdef`）。

### 字体 fixture

`crates/nana-text/fonts/`，全部 OFL，合计约 18 KB：

| fixture | 来源 | 覆盖 |
| --- | --- | --- |
| `noto-sans-sc` | `nana_ui_core::fonts::UI_FONT_REGULAR`，**不额外拷贝** | Latin、组合记号、f 连字、kerning、假名、汉字 |
| `nana-test-vf.ttf` | `crates/nana-ui/src/nana_text/fixtures/nana-wdth-bevl.ttf` 的副本，1 260 B | `wdth` + 自定义 `BEVL` 轴；cmap 只有 U+0041，兼作 fallback 与缺字 fixture |
| `noto-sans-arabic.ttf` | Noto Sans Arabic subset | Arabic 连写（`init`/`medi`/`fina`/`rlig`/`mark`） |
| `noto-sans-kr.ttf` | Noto Sans KR subset | 谚文音节 |
| `noto-emoji.ttf` | Noto Emoji（**单色轮廓**）subset | emoji 与 ZWJ 连字 |

用 `python3 scripts/build-text-corpus-fonts.py` 重新生成，`--check` 验证签入的文件与脚本
的产物一致。两端都钉死了才有意义：**输入**钉在 google/fonts 的某个 commit 上，且每个源文件
的 SHA-256 记在 `Fixture` 上（`main` 是会动的，Noto 会重新发版）；**输出**钉死 `head` 时间戳，
所以同样的输入必然产出同样的字节。

源文件对不上时脚本在下载处就报错，并明说这不是「签入的文件过期」——那是上游换了版本，要
显式 bump `GOOGLE_FONTS_REV` 与对应的 `sha256`，重新生成，并在同一个提交里重新 bless 用到
该字体的语料。否则会悄悄把 Arabic / 谚文 / emoji 语料挪到另一个字体版本上，golden 的变化
看起来和引擎回归一模一样。

彩色 emoji（`COLR` / `CBDT`）**永久**不在本 IR 范围内：IR 承载的是 glyph id 与 cluster，上色是
painter 的事。单色 Noto Emoji 的 `ccmp` 里带着 ZWJ 连字规则，足够验证 ZWJ cluster 合并——
`TX-Z01` 的 golden 里 `👩‍💻` 就是**一个** glyph，cluster 覆盖 0..11。

## 容差

```rust
pub const DEFAULT_TOLERANCES: Tolerances = Tolerances {
    advance_px: 0.05, offset_px: 0.05, baseline_px: 0.05,
    line_height_px: 0.05, bounds_px: 0.5, caret_x_px: 0.5,
};
```

- **0.05 px**（advance / offset / baseline / line-height）：24 px 字号下 f32 噪声约 2e-6 px，
  高出四个数量级，不会抖；同时只有 glyph atlas 常用 0.25 px 子像素量子的 1/5，真实的 advance
  舍入变化仍然会红。0.1 / 0.5 这种整数会开始掩盖半个子像素的漂移。
- **0.5 px**（bounds / caret_x）：两者都是派生量，两个引擎对「用 advance 宽还是墨水宽」合法地
  有分歧，所以它们保持是个 sanity check。
- **不要复用 `tools/css-parity` 的 `DEFAULT_TOLERANCE_PX = 2.0`**：那是 CSS-vs-WKWebView 盒模型
  的数量级，2 px 能吞掉一整对 kern。

**零容差精确比**：`glyph_id`、`cluster`、`cluster_end`、`flags`、`bidi_level`、`direction`、
`script`、所有 count、`LineBox::source`、`break_cause`、`CaretPosition::{byte,line}`、
`Affinity`、`OverflowFlags`。

`font` 按**布局内等价类**比，不比裸 `FontId`：断言 expected 里共享同一 face 的 run 在 actual 里
仍然共享（双向单射），且不同 face 的基数相同。比裸 index/generation 会让 harness 在字体注册
顺序变动时失败，而那不是文本行为。

`id` / `revision` / `font_generation` **不比**：它们说的是一个 layout 从哪来，不是它说了什么，
两个引擎没有理由在这上面一致。

diff 是位置相关的，所以**先比数量，不一致就不下潜**：行数 / run 数 / glyph 数对不上时，在该层
发一条 `count` delta 就停。否则多一个 glyph 会产生上百条错位 delta，报告没法读。

```text
TX-B02 FAIL (3 deltas)
  line 0 run 1 glyph 3 glyph_id: expected 74 got 7764
  line 0 run 1 advance_px: expected 41.50 got 42.10 (Δ=0.60, tol=0.05)
  line 1 metrics.baseline_y_px: expected 55.00 got 54.50 (Δ=-0.50, tol=0.05)
```

## 计数器

`nana_text::TextWorkCounters` 预留了 #89 要求的五个口径。字段约定照抄
`nana_ui_core::work::WorkCounters`：`usize` 是该 pass 必测的；`Option<usize>` 是尚未观测，
**必须保持 `None` 而不是假装 0**——假零读起来像「这份工作没发生」，实际是「没人看过」。

```text
text_nodes_considered   usize           看过的（含判定为未变而跳过的）
text_nodes_shaped       usize           其中真正进了 shaper 的
shape_cache_hits/misses Option<usize>   Phase 0 没有 shape cache → None
layout_cache_hits/misses Option<usize>  同上
glyphs_resolved         Option<usize>   解析出 glyph id 的数量
```

Phase 0 没有产品生产者，这是设计如此。防止它们变成摆设的是**对账**：
`reference_engine_counters_agree_with_the_layout_it_produced.rs` 断言
`counters.glyphs_resolved == Some(layout.glyph_count())`，也就是拿计数器和它声称描述的产物比。
抓的正是「计数器说谎」这一种失效模式，也是整个 #8 计数器文化存在的理由。golden 里也记了
counters，所以计数变化是一次可评审的 diff。

折进 `WorkCounters` 是 UiWorld 接缝上的一个函数，那才是正确时机；现在加五个没有生产者的字段，
等于为零信号拓宽一个 CI 正在裁判的合同。

## 边界如何被机器守住

`scripts/check-engine-boundary.py` 多了三条规则，都带自测
（`scripts/tests/test_engine_boundary.py`，现在真的在 CI 里跑了）：

1. **产品图**：`nana-text` 不得有任何**非 dev** 边通向 `cosmic-text` / `cryoglyph` / `glyphon`。
   dev 边是本阶段有意留的。
2. **源码**（承重的一条）：`crates/nana-text/src/**` 里不得出现 `cosmic_text` / `cryoglyph` /
   `glyphon` 标识符。这就是「核心 API 不出现 cosmic 类型」的机械含义——依赖图本身说不了这句话，
   因为参照引擎是一条合法的 dev 依赖。注释会被剥掉再扫，所以 `lib.rs` 可以正常地把边界写清楚。
3. **allowlist**：`crates/nana-text/src/**` 引用 `nana_ui_core::` 时，只许命中上面那张表里的项。

参照引擎放在 `crates/nana-text/tests/reference/`，**不是** `src/` 下的 `#[cfg(test)] mod`：
后者对 `tests/*.rs` 不可见，corpus harness 就用不上它。原生引擎落地后，删
`tests/reference/` 和 `Cargo.toml` 里的 `[dev-dependencies] cosmic-text` 两处即可，`src/` 完全
不用动。

## #33 迁移基准

`nana-dirty-frame-benchmark --shape layout --position head` 的 2k / 4k / 8k 三格是 Issue #33 的
原始 workload，#89 把它保留为文本迁移基准。

它此前**没有被任何 CI job 编译过**：`required-features = ["benchmark"]` 让它逃出
`cargo test/check --workspace --all-targets`，`runtime-performance.yml` 也没点它的名，删掉它所有
workflow 都是绿的。现在有两道轻量守护：

- `ci.yml` 的 check job 跑 `cargo check -p nana-ui-scene --all-targets --features benchmark`，
  文件不能再静默腐烂或被删除；
- `the_migration_grid_still_spans_two_four_and_eight_thousand_nodes` 断言默认 row 网格仍然产出
  2002 / 4002 / 8002 三个节点规模，网格不能再被静默收窄。

复现（同一台机器跑前后对比才有意义）：

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 1000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 2000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 4000 --samples 150 --warmup 30
```

Issue #33 收敛后的基线（Windows，2026-09-13，见 [脏帧](runtime-dirty-frame.md)）：

| 节点 | TextShape | Layout |
| ---: | ---: | ---: |
| 2,002 | 0.176 ms | 3.68 ms |
| 4,002 | 0.557 ms | 11.47 ms |
| 8,002 | 1.811 ms | 31.75 ms |

规则：**`nana-text` 每落一个阶段，重跑这三格，把数字贴回本表，并说明是哪台机器。
`TextShape` 相对同一轮 `Layout` 的倍率不得变差。** 这不是时间门禁，是人工对比——
接进 `perf/` 合同需要新的 scenario `kind`、extractor 和 fixture，等真有引擎可测再做。

## Phase 0 明确没做的

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 分数 DPI | 覆盖 layout，**不覆盖栅格** | `TextScale` 表达到字号缩放，这已是 layout 能表达的全部。glyph 原点的物理像素对齐在 `scene_paint/text.rs`，完全在 IR 之外。别把 `TX-D01` 读成子像素定位保证。 |
| ellipsis | 记录 overflow，**不插入省略号字形** | cosmic 0.19 的 `Buffer` 没有 ellipsis，产品路径自己替换。`TX-W05` 断言的是 `TRUNCATED_LINES` + `ELLIPSIZED` 与截断后的行数。 |
| IME preedit | span 应用是真的，composition 状态在 source 上 | `TextLayout` 只承载几何；`CompositionSegment` 留在 `TextSource` / `TextSpan`。`TX-E01` 断言 preedit span 确实产生了自己的 run，以及 composition 在 source 上可设可清。 |
| cluster 内部的 caret | 按字节比例插值 | 一个 glyph 可以覆盖多个源字节（连字，或多字节字符）。`caret_geometry` 先把渲染同一 cluster 的所有 cell 并成一个视觉范围——组合记号是零 advance 且与基字同 cluster，RTL 下 HarfBuzz 还会把它排在基字**前面**——再在该范围内按字节比例插值。所以 `of\|fice` 的 caret 落在 `ffi` 连字的三分之一处而不是整个连字之后，阿拉伯语带记号的 cluster 也不会塌到零宽记号上（见 `TX-B01` 的八个 caret 探针，x 随字节偏移严格递减）。落在字素内部的字节偏移本就不是合法 caret 位置，IR 没有源文本可以吸附，插值只保证单调、可区分。 |
| caret affinity（RTL / BiDi 边界） | **记录行为，不是合同** | 边界 affinity 是引擎定义而非规范定义的。Phase 0 把参照引擎的答案记成 golden 并配 `caret_x_px` 容差。 |
| 五个计数器 | 只有参照路径在喂 | 按设计没有产品生产者，靠对账测试防止空转。 |
| script 标注 | `ScriptTag::UNKNOWN` | 参照引擎不导出 per-run script。`ScriptTag` 的位置留好了，由做 segmentation 的那一阶段填。 |
| 多字体 fallback | 覆盖了但很窄 | fallback 是 VF→Noto 的 `A`/`B`，证明 `FontId` 能在 run 中途变、`FALLBACK_FONT` 会置位；不覆盖按 script 驱动的 fallback 选择。 |
| #33 workload | 合同级保留，不是 perf 门禁 | 见上一节。 |

## 怎么跑

```bash
cargo test -p nana-text --all-targets --locked
python3 scripts/check-engine-boundary.py
python3 -m unittest discover -s scripts/tests
python3 scripts/build-text-corpus-fonts.py --check   # 需要 fonttools
```

证明产品路径没动：

```bash
# 注意 --edges normal：默认的 cargo tree 会把 dev 边也列出来，而参照引擎正是一条 dev 边。
cargo tree -p nana-text --locked --edges normal | grep -ci cosmic   # 0
cargo tree -p nana-ui --locked | grep -ci nana-text                 # 0
```
