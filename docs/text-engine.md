# 文本引擎骨架（nana-text）

给**改 NanaUI 文本的人**。写应用不需要看这篇：产品文本的**测量与绘制都已经是
NanaUI 自己的**——`nana-text` 排版，`NanaRenderer::text` 画，`cosmic-text` 与 `cryoglyph`
都不在 release 依赖里了（#99）。

Epic #88 要把文本能力从 `cosmic-text` / `cryoglyph` fork 上迁走。#89 是其中的 Phase 0：
先把内部合同、reference backend 和 correctness corpus 固定下来，让后续每一阶段都能对着
同一份结构化基线比较，而不是在 shaping / layout / GPU 三层同时改动时失去可比性。
#90 是 Phase 1：`nana-text` 自有的字体层——注册、代际、匹配、变体坐标与按覆盖率的 fallback，
见「字体层」一节。#91 是 Phase 2：分段、BiDi、HarfRust shaping 与 ShapeRun cache，见「Shaping」一节。
#92 是 Phase 3：单行 Label fast path、断行、行内视觉序、行盒度量、对齐、省略号与 layout cache，
见「Layout」一节。#95 是 Phase 4：UiWorld 保留文本节点、分级 dirty graph 与 retained
`TextLayout`，见「UiWorld 保留文本节点」一节。#96 是 Phase 5：Editable 路径——可编辑存储、
caret / selection、hit-test、IME composition 与按段落失效的编辑器几何，见「Editable 路径」一节。
#97 是 Phase 6：`NanaRenderer::text`——renderer 自有的 glyph IR、栅格化边界、raster cache、
GPU atlas、上传队列与 text pipeline，cryoglyph 由此退出产品路径，见「NanaRenderer::text」一节。
#99 是 Phase 8：全路径 cutover——Runtime 的 `NanaTextShaper` 与画笔都改问同一个进程级
`nana-text` 引擎，Canvas2D 同样换掉，`Cargo.lock` 里再无 cosmic-text（连 dev 边也没有），见
「cutover 之后的产品路径」一节。

## 这是什么

```text
corpus/cases/TX-*.json          输入：文本 + 样式 + 约束 + 探针
        │
        ↓  nana-text（shaping + layout）
   Nana IR TextLayout
        │
        ↓  parity::compare(golden, actual, &tol) -> Vec<LayoutDelta>
corpus/golden/TX-*.layout.json  签入的基线(Phase 0 由 cosmic 参照引擎录下)
```

`compare` 是唯一的结构化 diff。Phase 0 比「参照引擎 vs golden」；原生引擎接上**同一个
函数**比「原生 vs golden」。**参照引擎已随 cosmic-text 一起删除**（见「参照引擎去哪了」），
golden 留下来，现在由 `shaping_matches_the_cosmic_reference_goldens.rs` 与
`layout_matches_the_cosmic_reference_goldens.rs` 两个用例拿原生引擎对着它们跑——文件名里的
「cosmic reference」说的是**这批 golden 的出处**，不是还在跑的引擎。

产品文本的**测量**走 `crates/nana-ui/src/nana_text.rs`（它现在只是
`NanaTextEngineShaper` 的壳），**绘制**走 `crates/nana-ui/src/scene_paint/text/`；两边问的是
`crates/nana-ui/src/text_engine.rs` 里那一个进程级引擎。

## nana-text 自己拥有什么，什么留在成熟 crate 上

`nana-text` 拥有的是生命周期、IR、缓存合同与编排，**不重写标准重型算法**。

| 事项 | 归属 | 理由 |
| --- | --- | --- |
| 文本 IR（`TextKind` / `TextSource` / `TextStyle` / `TextConstraints` / `ShapedRun` / `LineBox` / `TextLayout`） | **nana-text** | 跨层长期 ABI，不能是第三方内部类型 |
| 稳定代际 ID 与失效规则（`FontId` / `ShapeRunId` / `TextLayoutId` / `TextRevision` / `FontGeneration`） | **nana-text** | 缓存正确性的权威 |
| 命中测试 / caret / 选区矩形 | **nana-text** | 纯函数，跑在 IR 之上；写不出来就说明 IR 缺字段 |
| 行布局编排（断行策略、行盒合并、对齐、省略号、layout cache） | **nana-text** | 产品语义与缓存合同，必须与 `TextConstraints` 同一套词汇 |
| 结构化 diff 与容差 | **nana-text** | 迁移验收合同，必须比 cosmic 活得久——它做到了：参照引擎删了，golden 和 `compare` 还在 |
| 排版词汇（变体轴 / kerning / line-break / word-break / text-align / direction / writing-mode / wrap-break / line-height / feature） | **nana-ui-core** | 已是后端中立令牌；重造一套只会在 UiWorld 接缝上长出一个有损转换器 |
| OpenType 表解析、字形轮廓、变体插值 | **成熟 crate**（skrifa / ttf-parser） | #89 非目标明确写了不重实现 |
| 复杂文字整形（GSUB/GPOS、Arabic joining、印度系重排） | **成熟 crate**（harfrust，仅 `shaping/opentype.rs`） | 同上 |
| Unicode BiDi 算法（P–I 规则、L2 重排） | **成熟 crate**（unicode-bidi，仅 `shaping/bidi.rs`） | 同上 |
| UAX #14 断行机会 | **成熟 crate**（unicode-linebreak，仅 `layout/breaks.rs`） | #92 非目标明确写了不自研断行标准表 |
| 分段、span 规范化、fallback 重试、ShapeKey 与 ShapeRun cache、`ShapedText` | **nana-text**（`shaping` 模块） | 何时重塑形、塑形结果能被谁复用的权威 |
| Unicode 算法（BiDi、断行、字素簇、script / emoji 属性） | **成熟 crate**（unicode-bidi / unicode-linebreak / unicode-segmentation / icu_properties） | 同上 |
| 字体注册、代际、`FontId` 签发、face 匹配、fallback 策略与候选、覆盖率缓存、变体坐标解析 | **nana-text**（`font` 模块） | 缓存失效与「为什么用了这个字体」的权威；不能交给第三方 query |
| 系统字体目录扫描、name / OS/2 元数据读取 | **成熟 crate**（fontdb，仅 `font/discovery.rs`） | 不用它的 query 和 fallback |
| 轴、命名实例、彩色表、cmap 读取 | **成熟 crate**（skrifa，仅 `font/face.rs`） | 与 Phase 2 的 harfrust 0.12 同一条 read-fonts 线 |
| glyph IR、raster cache、atlas 策略与生命周期、上传、text pipeline | **NanaRenderer::text**（`nana-ui` 的 `scene_paint/text/`） | #97：renderer 侧的合同，见下节；不属于 `nana-text`，因为它是 device 状态 |
| 字形轮廓栅格化 | **成熟 crate**（swash，只在 `scene_paint/text/raster.rs` 后面） | #97 非目标明确写了不重写 TrueType 栅格器 |
| 矩形打包 | **成熟 crate**（etagere，只在 `scene_paint/text/atlas.rs` 后面） | 同上；Nana 拥有的是 atlas 策略，不是打包算法 |

`nana-text` 只许 import 这些 `nana_ui_core` 项：`DirSpec`、`FontFeatureSetting`、
`FontKerningSpec`、`FontVariationSetting`、`LineBreakSpec`、`LineHeightSpec`、
`TextAlignSpec`、`TextWrapBreak`、`WordBreakSpec`、`WritingModeSpec`。其余（`LayoutStyle`、`ComputedStyle`、
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

**重新录制的路子随参照引擎一起没了。** 原来的 `NANA_TEXT_BLESS=1 cargo test -p nana-text
--test text_parity_corpus` 是拿 cosmic 重录 golden，那正是必须停掉的事。golden 因此是**冻结的
Phase 0 证据**：它记的是被替换的引擎当时的答案，原生引擎每次 CI 都对着它跑。

要重新录制，得先决定新基线代表什么（「原生引擎现在的输出」不再是「迁移前后一致」的证据），
那是一次有意的设计决定，不是一个环境变量。`parity::write_golden` 还在，没有接到任何引擎上。

### 确定性靠 hermetic 字体库

产品路径用的是进程级、从系统字体播种的 `FontSystem`。照它录的 golden 只在一台机器上成立。
对账因此从不碰它：`tests/support/corpus.rs` 给每个用例装一套 hermetic 字体集，只按用例声明的
顺序装载它声明的 fixture。face id 因而确定，fallback 链就是用例的字体列表，缺字用例也白送
（只装 `nana-test-vf` 时，除 `A` 外一切都是 `.notdef`）。

### 字体 fixture

`crates/nana-text/fonts/`，Noto 子集为 OFL，合成字体由脚本生成，合计约 20 KB：

| fixture | 来源 | 覆盖 |
| --- | --- | --- |
| `noto-sans-sc` | `nana_ui_core::fonts::UI_FONT_REGULAR`，**不额外拷贝** | Latin、组合记号、f 连字、kerning、假名、汉字 |
| `nana-test-vf.ttf` | `crates/nana-ui/src/nana_text/fixtures/nana-wdth-bevl.ttf` 的副本，1 260 B | `wdth` + 自定义 `BEVL` 轴；cmap 只有 U+0041，兼作 fallback 与缺字 fixture |
| `noto-sans-arabic.ttf` | Noto Sans Arabic subset | Arabic 连写（`init`/`medi`/`fina`/`rlig`/`mark`） |
| `noto-sans-kr.ttf` | Noto Sans KR subset | 谚文音节 |
| `noto-emoji.ttf` | Noto Emoji（**单色轮廓**）subset | emoji 与 ZWJ 连字 |
| `nana-test-axes.ttf` | 脚本用 fontTools 合成，不下载 | `wght` 100..900 / `wdth` 50..200 / `slnt` -15..0 + 两个命名实例；字体层的范围匹配与 `font-weight` 对 `wght` 优先级 |
| `nana-test-color.ttf` | 脚本用 fontTools 合成，不下载 | COLR v0 + CPAL，覆盖 U+2764 / U+1F525；emoji fallback 优先彩色 face |
| `nana-test-notdef.ttf` | 脚本用 fontTools 合成，不下载 | cmap 有 `A` / `B`，但 GSUB `ccmp` 把 `B` 换成 `.notdef`：覆盖率说能画、shaping 画不出，专测 #91 的 fallback 重试 |

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

Phase 0 没有产品生产者，这是设计如此。当时防止它们变成摆设的是参照引擎的对账用例
（`counters.glyphs_resolved == Some(layout.glyph_count())`，拿计数器和它声称描述的产物比）；
那个用例随参照引擎一起删了。golden 里仍然记着 counters，所以计数变化依然是一次可评审的 diff，
而产品侧的口径由 UiWorld 文本 pass 的计数器守着（见「UiWorld 保留文本节点」）。

折进 `WorkCounters` 是 UiWorld 接缝上的一个函数，那才是正确时机；现在加五个没有生产者的字段，
等于为零信号拓宽一个 CI 正在裁判的合同。Phase 4 起它们由 UiWorld 的文本 pass 生产，并加了解释
零工作帧成本的口径，见「UiWorld 保留文本节点」的计数器一节。

## 边界如何被机器守住

`scripts/check-engine-boundary.py` 多了三条规则，都带自测
（`scripts/tests/test_engine_boundary.py`，现在真的在 CI 里跑了）：

1. **产品图**：`nana-text` 不得有**非 dev** 边通向 `cosmic-text` / `cryoglyph` / `glyphon`。
2. **源码**（承重的一条）：`crates/nana-text/src/**` 里不得出现 `cosmic_text` / `cryoglyph` /
   `glyphon` 标识符。这条在参照引擎还活着时是唯一能说「核心 API 不出现 cosmic 类型」的机械
   手段（依赖图说不了，因为那时它是一条合法的 dev 依赖）；引擎删掉之后它守的是不许有人
   把它的类型再带回来。注释会被剥掉再扫，所以 `lib.rs` 可以正常地把边界写清楚。

这两条都只管 `nana-text`。「别的 crate 不许依赖被替换的引擎」不靠门禁——依赖已经删了，
`Cargo.lock` 里一条记录都没有，再为它立一道门禁是在给一件已经不存在的事上锁。
3. **allowlist**：`crates/nana-text/src/**` 引用 `nana_ui_core::` 时，只许命中上面那张表里的项。
4. **字体层后端**（#90）：`fontdb` 只许出现在 `src/font/discovery.rs`，`skrifa` 只许出现在
   `src/font/face.rs`，`icu_properties` 只许出现在 `src/font/unicode.rs`，`read_fonts` /
   `ttf_parser` 哪都不许点名；这三个模块在 `font/mod.rs` 里不得是 `pub mod`。公开 API 因此
   不可能带出 `fontdb::ID` 之类的第三方 ID。#91 同理：`harfrust` 只许出现在
   `src/shaping/opentype.rs`，`unicode_bidi` 只许出现在 `src/shaping/bidi.rs`。模块不叫
   `harfrust`，就是因为模块名本身也会被这条规则扫到。#92 同理：`unicode_linebreak` 只许出现在
   `src/layout/breaks.rs`。

### 参照引擎去哪了

参照引擎曾经放在 `crates/nana-text/tests/reference/`（**不是** `src/` 下的
`#[cfg(test)] mod`：后者对 `tests/*.rs` 不可见，corpus harness 就用不上它），正是为了让它能被
一次删掉而不动 `src/` 一行。原生引擎落地、产品路径切完之后就删了：

- `tests/reference/`（cosmic 驱动的参照实现）
- `tests/text_parity_corpus.rs`（参照 vs golden，以及 `NANA_TEXT_BLESS` 重录路径）
- `tests/reference_engine_counters_agree_with_the_layout_it_produced.rs`
- `crates/nana-text/Cargo.toml` 的 `[dev-dependencies] cosmic-text` 与工作区那条 fork pin

`src/` 一行未动——这正是当初把它放进 `tests/` 的原因。留下的是 golden 和两个拿**原生**引擎
对着 golden 跑的用例，覆盖面没变：全部 26 条用例、同一个 `compare_golden`、同一批容差。

## 字体层（Phase 1，#90）

`nana_text::font` 输出的是**可供 Phase 2 shaping 消费的确定性字体选择**，不做 shaping。

```text
TextStyle / CSS font-* / NanaVue props
        ↓ FontQuery::from_style
FontQuery { families, weight, stretch, style, language }
        ↓ FontSystem::select（按 query 缓存，代际变化即清空）
FontSelection { primary, fallback_chain, generation, families[] }
        ↓ FontSystem::resolve_text（覆盖率缓存）      ↓ FontSystem::instance
FontAssignment { range, font, reason }          FontInstance { key: FontInstanceKey, ignored_axes }
```

### 来源与生命周期

| 来源 | 入口 | 字节 |
| --- | --- | --- |
| 平台系统字体 | `load_system_fonts` / `with_system_fonts` | 首次需要 details / 覆盖率时读盘，之后常驻 |
| 显式文件 | `register_file` | 注册时读取并校验，坏文件在注册处失败 |
| 内存 bytes / 测试 fixture | `register_bytes(font_blob(..), &FaceDescriptor)` | `Arc` 共享 |

- `FaceDescriptor` 是 `@font-face` 描述符：声明的 `family` **替换** face 自己的名字，
  weight / stretch 是闭区间，style 限定唯一样式。
- 每次变更（注册、卸载、`replace_bytes`、加载系统字体、换 `FallbackPolicy`）**恰好** bump
  一次 `FontGeneration`。selection 缓存整体清空（任何新 face 都可能改变某个 family list 的结果）；
  覆盖率缓存**只**丢被卸载的 face（无关注册不改变一个 face 的 cmap）。
- `FontId` / `FontSourceId` 是代际句柄：卸载后槽位以更高代际重发，旧 id 被拒绝，
  `FontInstanceKey` 因而不会与替换后的 face 混淆。
- `face_data` 给出 `FontData`（`Send + Sync`，持有 `Arc`）。worker 持有期间卸载不释放字节；
  `retired_font_data_alive` 报告被卸载但仍被持有的字节份数，释放时机可观察。
- `FontSystem` 是 `&mut self` 的普通值，不是全局，不持锁。宿主需要共享时自己决定同步方式。

### 匹配

CSS Fonts 4 §5.2：先 `font-stretch`，再 `font-style`，再 `font-weight`，每步只保留最优距离的
face。Issue 草案写的是 style → weight → stretch；这里按 CSS 规范的顺序，因为 CSS / Rust / NanaVue
最终都转成同一个 `FontQuery`，和浏览器不一致的顺序会让同一份样式在两处选出不同 face。

- 可变 face 以**范围**参与：`wght` 轴给 weight 区间，`wdth` 轴给 stretch 区间，`ital` / `slnt`
  轴让 face 同时支持 normal 与 italic / oblique。
- 400..=500 先找不超过 500 的更重 face，再找更轻，再找 500 以上；<400 先轻后重；>500 先重后轻。
- 仍然并列时：显式注册（文件 / 内存）胜过系统扫描，再由**后注册者**胜出。整个过程不遍历
  HashMap，同一 face 集合与 query 永远选出同一个 face。

### 变体坐标与 #41

每个轴的坐标，从低到高：face 默认值 → query 隐含（`wght` ← weight、`wdth` ← stretch、
italic → `ital = 1`、oblique → `slnt = -14`，仅限 face 有该轴）→ `font-variation-settings` 显式值。

- **`font-weight` 与显式 `"wght"`**：显式 `"wght"` 胜出，并且同时决定 face **选择**与坐标；
  `"wdth"` 对 stretch 同理。这是 `nana-ui` 产品路径已有的规则（`wght` 并进 weight），保留它
  迁移才不会换掉页面拿到的 face。其他任何轴（含 `BEVL`）不参与选择。
- face 没有的轴 **fail-closed**：进 `FontInstance::ignored_axes`，不生效，**绝不**改写成 `wght`。
- `FontInstanceKey { font, coords, synthesis }` 是 Phase 2 shape cache 与 glyph raster cache 的键：
  坐标先 clamp 再省略默认值，所以渲染结果相同的两个请求键相同（`wdth 1000` 与 `wdth 200` 在上限
  为 200 的 face 上同键）。
- 做不到的粗体 / 斜体记在 `Synthesis` 里，由 renderer 决定怎么假。轴动画（#85）不在本阶段。

测试直接用 skrifa 读回轮廓：`BEVL 42`、`wdth 150`、`wght 700` 都让 `A` 的轮廓实际变化，
证明坐标到达了 face，而不只是被记录。

### Fallback

两层：`FallbackPolicy`（有序的 family 名）给候选，`resolve_text` 用 cmap 覆盖率探测；覆盖到但
shaping 仍出 `.notdef` 的 cluster 由 Phase 2 重试。以字素簇为单位，ZWJ / 变体选择符等
Default_Ignorable 码位不要求覆盖。primary 已覆盖且无需彩色时直接返回，不构建候选表。

候选顺序：

- **emoji 呈现**（VS16，或非 VS15 的 `Emoji_Presentation`）：chain 里带彩色表的 face →
  emoji 策略 → chain 其余；
- **其他**：chain 按序（primary 在前）；

两者之后都是：该 cluster 的 script 策略（没有自身 script 的 cluster 沿用前一个有 script 的
cluster，所以 CJK 后面的标点继续找 CJK face）＋语言提示（`ja` / `ko` / `zh-Hant` / `zh-HK`
优先对应字形）→ 无自身 script 时的 symbol 与 emoji 策略 → last resort。

**策略全都不覆盖时再扫一遍字体库**（`scan_database`）：策略是*偏好顺序*，不是「机器上有哪些
face」的全集。✓（U+2713）就是例子——macOS 上它既不在 sans 里也不在 `Apple Symbols` 里，
而在 `Arial Unicode MS` 里；策略找不到就画 `.notdef`，而机器明明有这个字形，是最差的一种答案。
扫描按码位记忆（`scanned`），所以一篇满是同一个缺字的文档只扫一次；字体集合变更时清空。
真的全库都没有才记 `Missing`，渲染 primary 的 `.notdef`——这时「缺字」才真的是「这台机器没有」，
而不是「没有哪张列表恰好写了它」。扫描命中记 `LastResort { family }`，family 是命中 face 自己的名字。

`FallbackPolicy::platform_default()` 按 Windows / macOS / 其他（Linux、Android）给出常见
family；名字解析不到 face 就跳过。hermetic 测试一律从 `FallbackPolicy::empty()` 自己搭。

覆盖率缓存按 `FontId` 存折叠后的码位区间（CJK face 约 3 万映射折成几百个区间），默认预算
4 MiB，超预算按 LRU 淘汰，每次查询是二分查找。

### 诊断

- `FontSelection::families`：每个尝试过的 family 名、它来自哪个请求项（含 generic 展开）、
  解析到哪个 face。
- `FontAssignment::reason`：`Primary` / `FamilyChain { index }` / `EmojiPolicy { family }` /
  `ScriptPolicy { script, family }` / `SymbolPolicy` / `LastResort` / `Ignorable` / `Missing`。
- `describe(FontId)`：来源、路径、名字、匹配用的 weight / stretch 区间、轴、命名实例、是否彩色。
- `FontCounters`：`font_faces_registered`、`font_generation`、`font_query_hits/misses`、
  `fallback_candidates_examined`、`coverage_cache_hits/misses/evictions`、
  `font_fallback_attempts`、`font_fallback_misses`。

### 测试

- `tests/font_system_is_hermetic_and_deterministic.rs`：只用签入的 fixture 与 `nana-ui-core`
  打包的四个 Noto Sans SC 字重，策略在测试里搭。覆盖 preferred family、weight / style /
  stretch、可变范围、CJK / 语言提示、emoji 彩色候选、Latin + CJK 混排、缺字、`BEVL` / `wdth`、
  `wght` 优先级、注册 / 卸载 / 替换与代际、同 query 复用、线程可移交。
- `tests/font_system_platform_acceptance.rs`：读本机字体，`#[ignore]`，在目标平台上手动跑。
  2026-09-16 Windows 11：扫描 315 个 face 用时 15 ms；`Hello, 世界。こんにちは 한국어 🔥`
  分别落到 Segoe UI / Microsoft YaHei UI / Malgun Gothic / Segoe UI Emoji，无 `Missing`；
  冷启动（读盘）约 110–140 ms，热路径约 40 µs。

产品路径从 #99 起就接在这一层上：`crates/nana-ui/src/text_engine.rs` 持有进程唯一的
`FontSystem`，`@font-face`、`local()` 别名和 `sans-serif` 覆盖都落在它上面，画笔的栅格器也
按它签发的 `FontId` 取字体数据。

## Shaping（Phase 2，#91）

`nana_text::shaping`：字符序列 → 不可变的 `ShapedRun`。不断行、不排版、不算 caret——那是 Phase 3。

```text
ShapeRequest { source: &TextSource, style, direction, language, scale }
  → span 规范化：span 覆盖 base，composition span 覆盖普通 span；边界吸附到字素簇起点
  → 字素簇 / script / BiDi 分段
  → 每簇的字体（FontSystem::resolve_text，#90 的覆盖率选择）
  → item = 同一 style 段 × 同一 face × 同一 BiDi level × 同一 script 的最大连续簇序列
  → HarfRust：face bytes + 变体坐标、direction、script、language、features、kern；
    item 之外的文本作为 pre/post context 传入，cluster 是全文字节偏移
  → .notdef 簇段按 #90 候选表重试并拆出 fallback run
  → Arc<ShapedText>，按 ShapeKey 缓存
```

### 输出

- `ShapedText.runs` 按**逻辑序**，每个 run 带 `bidi_level`、`direction`、`script`、`font`、
  物理 px 字号、`RunMetrics`（在该实例坐标下量）。run 内 glyph 按视觉序（HarfBuzz 约定）。
- `ShapedText.paragraphs` 是 UBA 段落与其 base level；`visual_order(range)` 对一行 run 做 L2
  重排。L1（行尾空白复位）属于断行，留给 Phase 3。这些就是 #59 后续 bidi / 竖排 layout 需要的
  run / level 信息。
- `cluster` / `cluster_end` 是源文本字节偏移，从不重写源字符串：没有做会破坏映射的规范化，
  也就不需要 offset map。段落分隔符（`\n` / `\r\n` / U+2029 …）不出 glyph，所以 run 之间可以有缺口。
- `ShapedText` 在 `Arc` 后面，layout 拿去读、要写 origin 时复制 run，不会原地改缓存值：
  宽度从 400 变 300 只重跑 Phase 3。

### 分段

- **字素簇不被切开**：style span 的边界向前吸附到所在簇的起点；emoji ZWJ 序列、组合记号始终
  在一个 item 里（HarfRust 自己的 cluster 合并在其上）。
- **script**：每簇取自身 script，Common / Inherited 沿用前一个有 script 的簇，开头的沿用后面
  第一个。标点、空格、数字不单独开 run。
- **BiDi**：`unicode-bidi`，段落 level 由 `direction` 固定（CSS 语义，不做首强字符探测）。
  `unicode-bidi: bidi-override` 这类方向覆盖没有 CSS 输入可接，本阶段不提供。
- `language` 进 HarfRust（`locl` 等），也进 #90 的 fallback 语言提示。

### Fallback 重试

覆盖率预选之后仍可能画不出（GSUB 换成 `.notdef`、cmap 与布局表不一致）。每个 item shaping 后：

```text
找出含 glyph 0 的簇段 → 对每段取 #90 候选表里未试过、且覆盖该段的下一个 face
 → 该段换 face，两侧剩余部分重新 shaping（上下文变了）→ 直到干净或候选用尽
```

用尽时保留 primary 的 `.notdef`，glyph 标 `MISSING`。family 列表一个 face 都解析不到时，由第一个
已注册的 face 顶上画 `.notdef`，文本不会悄悄消失；只有字体系统里一个 face 都没有时才不出 run，
并计入 `text_bytes_unshaped`。上限：每段最多看
`MAX_FALLBACK_CANDIDATES_PER_RANGE = 8` 个候选，每个 item 最多 `MAX_FALLBACK_RETRIES_PER_ITEM = 32`
次重试，所以「没有任何字体能画」的长串不会变成 文本长度 × 字体数。

### ShapeKey 与 cache

| 进 key | 不进 key |
| --- | --- |
| 文本内容（持有 source 的 `Arc<str>` + 按 revision 记忆的内容 hash） | widget 身份、`TextRevision` 本身 |
| 每个 span 的范围与塑形相关样式：family、物理字号、weight、italic、letter-spacing、features、variations、kerning | line-height、composition 状态 |
| `direction`、`language`、设备 scale、`FontGeneration` | max width / wrap / max lines / ellipsis 等其余约束；颜色、透明度、transform（`TextStyle` 本来就不带） |

- 查找**不复制文本**：key 共享 source 的 `Arc<str>`；hash 在 `TextSource` 内按 revision 记忆，
  同一 revision 反复查找只 hash 一次。相等性先比 hash、再比指针、最后逐字节比，hash 冲突不会被
  当成命中。
- 同文本不同 source（10k 个相同标签）落到同一条目。
- 两道上限：条目数（默认 4096）和保留字节（默认 16 MiB，含 key 保留的文本）。LRU，淘汰计数。
  超过整个字节预算的结果照常返回、不入缓存、也不挤掉别人。
- key 里的字体纪元是「哪个 `FontSystem` × 哪一代」：`FontId` 和代际在每个系统里都从零编号，
  一个 `Shaper` 轮流服务多个系统时不能混用。看到新的纪元时，其他纪元的条目一次性清掉（计入
  evictions），HarfRust 的 per-face 加速数据同时丢弃。

### 计数器

`Shaper::counters()` → `ShapeCounters`：

```text
shape_requests / shape_runs_created / shape_glyphs_created
shape_cache_hits / misses / evictions      shape_cache_bytes / entries（读时的量）
bidi_runs / script_runs                     未命中时切出的 level run 与 script run
fallback_retries / fallback_fonts_examined
text_bytes_hashed                           每个 source revision 一次
text_bytes_cloned_for_shape                 key 共享 Arc<str>，恒为 0；将来引入复制时必须在此计数
text_bytes_unshaped                         字体系统没有任何 face 时未出 run 的字节（不含段落分隔符）
```

没有折进 `TextWorkCounters`：那里的 shape cache 口径要等 UiWorld 接缝真有 pass 时再填。

### 与 cosmic golden 对账

`tests/shaping_matches_the_cosmic_reference_goldens.rs` 用同一组 hermetic 字体、同一条
fallback 链，把全部 26 条语料按**逻辑簇**逐字段对 golden：glyph 数、glyph id、`cluster_end`、
`MISSING`、`FALLBACK_FONT`、bidi level、字号精确比，advance / offset 用上面的 0.05 px，face
按双射比。按簇比与双方在哪切 run、在哪断行无关。

结果：26 条全部一致。唯一成文的差异只对断行用例放行：golden 是排好的行，软换行处的空白 glyph
与 `max_lines` 截断后的内容不在 golden 里，而不断行的 shaping 保留它们——那是 Phase 3 的事。
变异验证：强制关掉 kerning 后该测试报 45 条 delta。

对账还暴露了**参照引擎的一个缺陷**：用例没写 `font_family` 时，参照引擎拿 fixture id
（`noto-sans-sc`）当 family 名去查「请求的 face」，查不到，于是所有 glyph 都没有
`FALLBACK_FONT`——包括 `TX-B02` / `TX-B03` 里确实来自 Arabic fallback 字体的 glyph。已修正为
family 名并重新 bless；golden 的变化只有这两条用例里 20 处 `flags: 0 → 2`。

### 测试

`tests/shaping_pipeline.rs`：`ffi` 连字与 `liga 0`、组合记号、Arabic 连写与 RTL、mixed BiDi 的
level 与 L2 重排、RTL 下数字与标点的 level、CJK + Latin 的覆盖率 fallback 与 `FALLBACK_FONT`、
emoji ZWJ、`wdth` 轴、缺字 `.notdef`、`.notdef` 重试（`nana-test-notdef`）、跨 script 的 style
span、落在字素中间的 span 边界、源字节 / cluster 映射；cache 侧：10k 相同标签只 shape 一次、
同 source 反复查找只 hash 一次且零复制、宽度 / 行高 / 同内容新 revision 不重塑形、方向 / scale /
字体代际会重塑形并清旧代、条目与字节上限及 LRU。

本阶段保证的是 paint / transform 根本进不了 ShapeKey；UiWorld 里它们不产生 shape request，
由 Phase 4 的 dirty graph 在 Runtime 上验证（见「UiWorld 保留文本节点」）。

## Layout（Phase 3，#92）

`nana_text::layout`：不可变的 `ShapedRun[]` + `TextConstraints` → 不可变的 `TextLayout`。
**shaping 与 layout 解耦**：宽度、wrap、对齐、max-lines 变化只重跑这一层，不碰 HarfRust。

```text
ShapedText（#91，Arc 共享）+ TextConstraints
  → 单行 Label fast path，或：
  → 按 UBA 段落切段 → UAX #14 断行机会（unicode-linebreak，仅 layout/breaks.rs）
  → 以 shaped advance 贪心断行（行尾空白在软换行处悬挂）
  → 每行 L1（行尾空白复位）+ L2（视觉序）
  → 行盒度量、对齐、省略号
  → 不可变 TextLayout，按 LayoutKey 缓存
```

### 单行 Label fast path 与降级

`TextKind::Label` 且满足全部条件时走 fast path：不换行、`max_lines` 为 `None` 或 `1`、
没有堆叠方向的预算（横排是 `max_height_px`，竖排是 `max_width_px`）、**`segments()` 只切出一段**。竖排的单行 label 同样走 fast path。
fast path 只做一件事：把自己的 run 累出 advance、算一次行盒、（需要时）裁一次省略号、
出一个 `LineBox`。它不建 editor state、不扫段落结构、不找断行机会
（`line_break_candidates` 恒为 0）、不为 color / transform 变化重排（那些根本进不了
`LayoutKey`，见下）。

最后一条不自己重推，而是问 `segments()`（下面「断行」一节的同一个函数）：显式换行、forced break、
以及**结尾的**换行（它不产生新段落，却产生一个空的末行）因此以同一种方式降级，不会有一条漏网。
任一条件不成立就降级到 paragraph path——任何 wrap、多行或零 `max_lines`、高度预算、
需要回退的 writing mode 同理。降级是可观测的：`label_fast_paths` / `paragraph_paths`
两个计数器分别计入。

### 断行

断行机会来自 UAX #14（`unicode-linebreak`），**是否**在某个机会处断由 shaped advance 决定，
从不按码位数估算。

硬换行有两个来源：**段落分隔符**（UBA class B，即 `shaping::PARAGRAPH_SEPARATORS`：`\n`、`\r`、
`\r\n`、U+001C–U+001E、U+0085、U+2029）由 shaping 的段落结构给出，不重新问 UAX #14；
**forced break**（`breaks::FORCED_BREAKS`：VT、FF、U+2028 LINE SEPARATOR）段落结构不管，
由 layout 在段内切开（`has_forced_break` 是一次普通字符扫描，不是 UAX #14 pass，且只在**建**
layout 时问一次）。

两张表各有一条对账测试，因为**三处**必须同时同意一个字符是分隔符：shaping 丢掉它（不出 glyph）、
layout 在那里断行、`preserve_lines: false` 把单字节的那些折成空格。任一处漏掉，分隔符就会以
`.notdef` 方块的形式画在它刚刚结束的那一行上。`PARAGRAPH_SEPARATORS` 对着 `unicode-bidi`
实际的分段逐码位校对，`FORCED_BREAKS` 对着 `unicode-linebreak` 的 mandatory break 校对，
`FOLDED_SEPARATORS` 则断言自己恰好是这两张表里所有单字节成员。两个标准对 U+001C–U+001E 的看法
不同（UBA 分段、UAX #14 不断），按更严格的 UBA 读法处理。
分隔符本身落在两段之间，没有行覆盖它，因此不绘制——与 `\n` 待遇相同。这份列表由
`forced_breaks_agree_with_uax14` 对着 `unicode-linebreak` 自己的数据逐码位校对，Unicode
改版新增一个也漏不掉。带 forced break 的 Label 与带 `\n` 的一样降级到 paragraph path。

| 约束 | 策略 |
| --- | --- |
| `wrap: None` | 不换行，段落即一行；`line_break_candidates == 0` |
| `wrap: Word` | 只在 UAX #14 机会处断；放不下的长词溢出（`CLIPPED_WIDTH`） |
| `wrap: WordOrGlyph` 或 `word-break: break-word` | 先按词，放不下的词再按字素簇切 |
| `wrap: Glyph`、`word-break: break-all`、`line-break: anywhere` | 每个字素簇边界都是机会 |

- **行尾空白悬挂**：软换行处的空白不绘制，`LineBox::source` 也不含它（与参照引擎一致）；
  硬换行与段落末尾的空白是作者写下的内容，glyph 保留在行上。两种情况下它都**不进行宽**：
  `metrics.width_px` / `bounds` 是去掉行尾空白后的宽度，溢出判定（`CLIPPED_WIDTH`、省略号裁切）
  与对齐都用它。断行判定本来就不数行尾空格，其余环节必须一致——否则 `"Save "` 会在一个装得下
  `"Save"` 的盒子里被裁成 `"Sa…"`，居中时也会比 `"Save"` 偏左半个空格。
- 悬挂在软换行处的空白字节不属于任何行的 `source`，但仍是合法的 caret 位置：
  `caret_geometry` 把它们解析到所挂那一行的行尾（或下一行的行首，取决于 caret 报的是哪一行）。
- 悬挂的方向跟着段落方向：L1 把行尾空白归到段落的**末端**，RTL 段落里那是视觉左侧，
  因此行的起笔位置比对齐框左移悬挂宽度。画在框内会把所有真实 glyph 顶出另一侧。
- 容器窄到一个字素都放不下时，仍然放一个字素——否则会产生空行与死循环。
- **软换行不会产生空行**：段首空白后面有一个断行机会，在那里断会让首行什么都不画、空白也无处可去，
  因此这种机会直接跳过（emergency 切分同理）。空行只来自空段。
- **段（segment）是断行的单位**：每个 BiDi 段落按 forced break 再切一刀，段与段之间的分隔符
  不属于任何段，因此不绘制。两种段只为 caret 存在：空文本没有 BiDi 段落、以段落分隔符结尾的
  文本后面没有段落——两者都补一个空段，所以空输入框有行盒，行尾按 Enter 后 caret 也不会消失。
  `intrinsic_widths` 量的是**同一批段**，`max-content` 因此是「最宽的那一行」，不是两行首尾相接的宽度。
- 断点永远在 shaper 的 cluster 之间，因此不可能切开 UTF-8 序列、字素簇或连字。

### BiDi 视觉序

每行独立做 L1（行尾空白复位到段落 level）与 L2（按 level 重排），用的是 #91 已经算好的
run level，不重跑 UBA。`LineBox::runs` 是**视觉序**的连续区间，`ShapedRun::source` 仍是
**逻辑**字节范围，两者同时可读——caret / 选区 / 命中测试（`nana_text::edit`）直接跑在上面。
一个 run 被行切开时，每一片保留原 run 的 `ShapeRunId`：id 说的是「这些 glyph 由哪次 shaping
产生」，切行不产生新的 shaping。整段 run 保留 shaper 报的 `advance_px`，只有被切开的片才重新
按 shaper 的逐 glyph advance 求和。

### 行盒度量与 strut

```text
line_height = max(strut 的行高, 该行各 run 样式要求的行高)
ascent / descent（上报值） = max(strut, 该行各 run 的字体度量)
half_leading = (line_height - (strut.ascent + strut.descent)) / 2
baseline     = top + half_leading + strut.ascent
```

`strut` 就是 CSS 的 strut：**基础样式自己那张 face 的度量**，由引擎接缝
（`NativeTextEngine`）从 `FontSystem` 取。它是「禁止 baseline 因单个 emoji/CJK fallback 抖动」
的机械实现——`Search` 与 `Search عربي` 的 baseline 完全一致，更高的 fallback face 只抬高
上报的 ascent 与行盒，不移动基线。

不给 strut 时（`LayoutRequest` 默认），基线落在该行最高 run 上，也就是参照引擎的规则；
语料对账就跑在这一模式下，见下。空行没有 run，行高取基础样式。

`line-height` 由样式解析（`LineHeightSpec` → px，未写时是 `font-size × 1.2`，与
`nana_ui_core::text_line_box_height_px` 同一个数）。span 有自己的 `line-height` 时，
**按 shaper 用的同一条规则**解析：`TextSource::SnappedSpans` 把 span 边界吸附到所在字素簇的
起点（shaper 切不开一个簇），再按「最后一个 composition span → 最后一个普通 span → base」取。
shaping 与 layout 必须同解，否则一个落在簇中间的 span 会按 span 的字号塑形、却按 base 的行高
量进行盒，glyph 就会溢出自己的行盒。没有 span 时这条路径一次解析都不做。

**分数 scale**：`max_width_px` / `max_height_px` 是逻辑 px，乘 `scale` 后与物理 px 的
advance 比较；layout 全程保留浮点，**不**向整数像素取整——那是 renderer / glyph 路径的事。

### 对齐

`TextConstraints::align`（`nana_ui_core::TextAlignSpec`）：`start` / `end` 跟随段落方向，
`left` / `right` 是物理方向。没有 `max_width_px` 就没有可对齐的容器，所有关键字都把行放在原点。
行宽按去掉行尾空白算，所以 `"Save "` 与 `"Save"` 居中在同一处。

超出容器的行**不被推回原点**：slack 取负值，`end` / `right` / `center` 让它按 CSS 那样往起始边
外溢，于是被裁剪的容器露出的是调用方对齐的那一端。`bounds.x` 因此可以是负数——`TextRect` 本来就
不把负值抹平。

`justify` **明确延期**：`TextAlignSpec` 里没有这个关键字，产品也无从表达，因此不半做。

### 省略号

`constraints.ellipsis` 为真，且（**不换行**时行超宽，或被 `max_lines` / `max_height_px` 截断）时：

```text
候选行 → 预留已塑形的省略号宽度 → 在 cluster 安全边界处裁 → 发出截断行 + 省略号 run
```

- 省略号走**正常** shaping / fallback / cache：接缝把 `…` 当作普通 `TextSource` 交给 shaper，
  同一样式下 10k 个截断标签只塑形一次（测试断言 `shape_cache_misses == 2`：正文一次，省略号一次）。
- 裁切单位是 shaper 的 cluster，所以不会切开 UTF-8、字素簇或连字；ZWJ 序列要么整段留下要么整段裁掉。
- 裁切点总是**去掉行尾空白之后**的位置：省略号顶替它替换掉的文字，就从那段文字最后一个可见簇之后开始。
  放在悬挂空白之后画，会画到行宽之外、容器之外，而且没有任何东西说得出来。
- 省略号 run 的 `source` 是裁切点上的**空区间**，glyph 的 cluster 也是——它不占源文本的任何字节，
  caret、命中测试与选区因此永远不会落到它身上。
- **换行开着时，超宽的行不裁**。换行的段落只会因为一个断不开的长词而超宽，而那个词整个都在这一行上：
  裁掉它等于让这段字节从所有行里消失，`LineBox::source` 留下一个中间的洞，谁也画不出、选不中、
  命中不到，而且没有任何 flag 说得出。这种行就让它伸出去，并置 `CLIPPED_WIDTH`。
  省略号真正该做的截断——`max_lines` / `max_height_px`——照常，并且带 `TRUNCATED_LINES`。
- RTL 段落里省略号放在视觉末端（左侧）。
- 截断但没有（或没能）塑形出省略号时，只报 `TRUNCATED_LINES`，不报 `ELLIPSIZED`：没画就不声称画了。
- 截断行的 `break_cause` 说的是**哪条预算用完了**：`MaxLines` 或 `MaxHeight`。在 `max_lines`
  根本没设的 layout 上写 `MaxLines`，等于对消费者断言一条从未存在的约束。

### LayoutKey 与 cache

| 进 key | 不进 key |
| --- | --- |
| shaped runs 的**身份**（持有 `Arc<ShapedText>`，按指针比较） | widget 身份 |
| 请求方 source 的 `TextRevision`（见下） | |
| 已塑形省略号的身份 | 颜色、透明度、transform、z-index、背景（`TextStyle` 本来就不带） |
| `TextKind` | |
| 全部 `TextConstraints` 字段（宽高、wrap、word-break、line-break、max-lines、ellipsis、preserve-lines、direction、align、writing-mode、tab-width、scale） | |
| 每个 run 解析后的行高、空行行高、strut | |

- key **持有** `Arc<ShapedText>` 而不是裸指针：持有才让指针可比——否则同一地址可能被另一段文本复用。
- **`TextRevision` 进 key，与 ShapeKey 相反**：`TextLayout` 自带产出时的 revision，
  `is_stale` 拿它作比较，所以一份交给第二个 source 的 layout 不能还写着第一个 source 的 revision——
  那个 source 会每帧都把自己当前的 layout 读成陈旧，且永远如此。同文本不同 revision 的两个节点
  因此各拿一份 layout；底下的 shaping 仍然共享，重活在那边。
- `tab_width` 也在 key 里，尽管本阶段**没有**实现制表位（`\t` 按普通字符塑形）：它是一条已声明的
  约束，key 漏掉一条，等于将来实现它的那天缓存会发回错的 layout。
- `ConstraintsKey` 是逐字段解构写出来的，给 `TextConstraints` 加字段会在这里编译失败，而不是
  悄悄产生一个忽略该字段的缓存。
- LRU，条目数（默认 4096）与字节（默认 8 MiB）双上限；超过整个字节预算的结果照常返回、不入缓存。
  shaped 文本由 shape cache 计费，layout cache 不重复计。
- cache 还回答一个别处没有的问题：这次 miss 是**新文本**还是**同一份 shaped 换了约束**
  （`constraint_only_relayouts`）——resize 风暴要看的就是这个数。

### 本阶段没做的

| 项 | 状态 |
| --- | --- |
| 制表位 | `tab_width` 已进 `LayoutKey`，但没有任何一行应用制表位；`\t` 按普通字符塑形 |
| `justify` | `TextAlignSpec` 没有这个关键字，产品无从表达，明确延期 |
| 竖排编辑（#59） | 可编辑文本仍横排兜底并计数，见上 |
| CSS 空白折叠 | 完全不做：连续空格原样保留，因此 `preserve_lines: false` 下 CRLF 折成**两个**空格，与手写两个空格是同一回事；要折叠的调用方自己规范化文本（那时挪动偏移是它自己的事） |

UiWorld 里「只改颜色 / transform 的帧不产生 layout request」由 Phase 4 在 Runtime 上验证；
本阶段保证的是它们根本进不了 `LayoutKey`——`TextStyle` 不带 paint，`LayoutRequest` 也没有第二条通路。

### intrinsic min/max content width

`Layouter::intrinsic_widths`：`min-content` 是最宽的不可再断片段，`max-content` 是不换行时的宽度，
都按 shaped advance 与 UAX #14 机会算，与当前约束是否允许换行无关——容器正是为了决定宽度才问这两个数。
不缓存：它是一趟 advance 累加，没有宽度可以作 key。

### writing mode 与 #59

`vertical-rl` / `vertical-lr` 真正按列排（`TextKind::Editable` 除外，见下）。做法是**在行相对
坐标里排竖排，只在边界上转一次坐标**，断行、对齐、截断、缓存都不知道页面被转了过来：

- **朝向**：`font/unicode.rs` 用 ICU 的 `Vertical_Orientation`（UAX #50）给每个字素簇定朝向。
  `U` / `Tu` / `Tr` 直立，`R` 侧卧（`text-orientation: mixed`）。`Tr`（括号、长音符、破折号）也按
  直立处理：它们要的是竖排**字形**，由直立 run 上的 `vert` 替换（或 HarfRust 的 Unicode 竖排
  表现形式兜底）给出。朝向进 item 切分，与字体、bidi 级、script 并列。
- **整形**：直立 run 用 HarfRust `TopToBottom` 整形——字体的 `vmtx` / `vhea`、`vert` / `vkna`
  替换字形都由它负责；`y_advance` 为负，入 IR 时转成沿行的正长度，偏移保留 HarfRust 的约定
  （相对横排原点，已减去竖排原点 `(h_advance/2, v_origin_y)`）。侧卧 run 就是横排整形，由画笔
  顺时针转 90°。`ShapedRun::orientation` 记下是哪一种；`ShapedGlyph::advance_px` 在三种朝向下
  都是**沿行**的步进。`ShapedText::vertical` 是排版取写作方向的唯一来源，整形缓存键带上它。
- **排版**：`max_width_px` / `max_height_px` 永远是**物理**盒子；`TextConstraints::inline_budget_px`
  / `block_budget_px` 把它们换成行预算与堆叠预算——竖排时高度管一列多长、宽度管能叠几列。
  竖排的基线是列中线（`baseline_y_px = top + height/2`），行盒高度就是列宽。`LineBox` / run 的几何
  仍是行相对的：`x` 沿列向下，`y` 从块起始列算起。
- **映射**：`TextLayout::physical_x_of_block` 与 `physical_size` 是逻辑到页面的唯一换算；
  `vertical-rl` 从盒子右缘往左叠列。Runtime 的 `text_metrics_of_layout` 读 `physical_size`，竖排
  不报 ascent（列挂在中线上，交给按字母基线对齐的盒布局只会错位）。
- **Runtime 约束**：竖排文本量完盒子后总把内容高度作为 `max_height`（与横排总给宽度对称），
  宽度只在要截断时给——`nana_text_constraints` 按 `lays_out_vertically` 决定哪一维是截断预算。
  直立 run 的步进是竖排度量，不写进横排富文本读的逐字宽度缓存；单字快路径也不回答竖排。
- **画笔**：列中线整像素对齐；直立字形放在 `(中线 + offset_x, pen − offset_y)`，侧卧 run 把
  em box 居中在中线上、以 `GlyphSynthesis::ROTATE_CW` 光栅化（进栅格键）。entry 以自己的列堆栈
  右缘为锚，盒子多出来的宽度加在绘制原点上，所以盒子只变宽时实例整份复用。

**可编辑文本**仍横排兜底：跨列的光标移动、选区与命中测试还没做，把字形竖着画而光标按横排几何
画只会更糟。此时 `TextLayout::unsupported_writing_mode` 置位、`vertical_writing_fallbacks`
计数，场景里编辑器的文本图元也按 `horizontal-tb` 画。`sideways-*` 与 `text-orientation` 由盒
布局在更早处拒绝，到不了这里。

**静态可选文本**（`user-select`）的选区不走编辑器几何（那份是横排的）：竖排节点直接读 Runtime
保留、画笔也在画的那份 layout，指针点经 `TextLayout::line_space_point` 转进行空间做 `hit_test`，
`selection_rects` 经 `page_rect` 转回页面，锚点与画笔同为内容盒右缘（`vertical-rl`）。

### 计数器

`Layouter::counters()` → `LayoutCounters`：

```text
layout_requests / layout_created
layout_cache_hits / misses / evictions      layout_cache_bytes / entries（读时的量）
line_break_candidates                       看过的断行机会（fast path 恒 0）
lines_created / runs_placed / ellipsis_runs_used
constraint_only_relayouts                   同一份 shaped、新约束
shape_runs_reused_for_layout                被 layout 读走而不是重塑形的 run
label_fast_paths / paragraph_paths          走了哪条路（只记真正建出的 layout）
vertical_writing_fallbacks                  竖排请求被横排兜底的次数（只剩可编辑文本）
```

`vertical_writing_fallbacks` 会折进 `TextWorkCounters`（#59）：编辑器的横排兜底本身是对的——
光标与字形对不上更糟——但它**在屏幕上是看不见的**，所以必须在计数器里响。`> 0` 就是「这一帧
有编辑器要了竖排而没拿到」。`TextLayout::unsupported_writing_mode` 是同一件事的
逐节点版本，随保留 layout 一路带到场景上。

对账测试断言 `lines_created` / `runs_placed` 等于它们声称描述的 layout 的行数与 run 数，
`TextWorkCounters::glyphs_resolved` 等于 `layout.glyph_count()`。

### 引擎接缝：`NativeTextEngine`

`nana_text::NativeTextEngine` 实现 `TextEngine`，把 #90 字体层、#91 shaper、#92 layouter 装在一次调用后面：

- `preserve_lines: false` 时先把单字节的行分隔符（`\n` / `\r` / VT / FF）折成空格**再**塑形
  （`TextSource::with_folded_newlines`）——塑形与断行必须看到同一串字节；它们都是单字节，
  所有 span 范围、cluster 与 caret 偏移保持不变，revision 也保持不变（同一次编辑的另一种读法）。
  折叠结果按 revision 记在 source 上，每帧重排既不复制也不重新 hash。
  `U+2028` / `U+2029` 各三字节，折叠会挪动其后所有偏移，因此无论 `preserve_lines` 怎么写都仍是换行。
- 需要时塑形 `…`，取基础样式那张 face 的度量作 strut，填 `TextWorkCounters` 的五个口径。
- `TextEngine::layout` 返回 `Arc<TextLayout>`：layout 不可变，同一帧里同文本同约束应当拿到**同一份**，
  而不是它的拷贝。

Runtime 通过 `NanaTextEngineShaper` 持有它（见「UiWorld 保留文本节点」）；从 #99 起产品宿主
`NanaTextShaper` **就是**它的壳，绘制走 `NanaRenderer::text`，两边问同一个引擎。

### 与 cosmic golden 对账

`tests/layout_matches_the_cosmic_reference_goldens.rs` 用**同一个** `compare_golden`、同一批 golden、
同一组容差，把 26 条语料的原生 layout 与 Phase 0 录下的参照 layout 逐字段比，
并且把每条用例自带的 hit-test / caret 探针在**原生 layout** 上重跑一遍一起比。

结果：26 条全部一致。只有三类 delta 被放行，且都是「两个引擎如何描述」而不是「排得不一样」：

| 字段 | 原因 |
| --- | --- |
| `script` | 参照引擎不导出 per-run script，记的是 `Zzzz`（Phase 0 已成文的缺口） |
| `run_count` | 切 run 的位置不同：原生按 script / style 段切，cosmic 按 face / level / size 切。glyph 相同，只是分组不同 |
| `overflow` 的 `ELLIPSIZED` 位 | cosmic 画不出省略号，参照对「要了省略号且被截断」直接置位；对账这一跑也不提供已塑形的省略号，因此原生只报 `TRUNCATED_LINES`。其余 overflow 位逐位精确比 |

`compare` 在数量不一致时**按设计**停止下潜，所以 `run_count` 放行会让那几行的几何无人比对。
同一个测试因此再按**行 × glyph cell**（视觉序的 glyph id、cluster、cluster_end、x）比一遍，
这种比法与分组无关，正好补上那三条用例。另有一条测试把过滤器摘掉、断言剩下的字段**只有**这三个，
过滤器因此藏不住第四类差异。

对账跑在「无 strut、无省略号」模式下：那两处正是原生引擎刻意与参照不同的地方
（strut 稳住基线、省略号真的画出来），它们各自有自己的 fixture。

### 测试

`tests/layout_engine.rs`：二十二条来自 code review 的回归（RTL 行的悬挂空白挂在起始边外、
RTL 软换行处的 caret 留在原行、高度截断报 `MaxHeight`、段首空白不产生空行、
省略号不画到容器外、U+001C–U+001E 结束一行且不绘制、span 边界落在字素簇中间时行盒按 span 的
行高算、结尾换行的 Label 也降级、超宽的行按对齐往起始边外溢、换行时超宽的行不因省略号丢字节、
行尾空白悬挂不算溢出也不影响对齐、空文本两条路径都出一行、行尾换行留下 caret 可落的空行、
被截断的空行保留自己的字节、同一行被裁两次只记一个省略号 run、U+2028 结束一行且不绘制、
layout 自带请求方的 revision、`max-content` 是最宽的一行、悬挂空白里的 caret 有位置、
折叠每个 revision 只做一次、折不动的分隔符仍然断行），加上：Label fast path（10k 标签 `paragraph_paths == 0`、
`line_break_candidates == 0`；10k 同文本标签只建一个 layout）、显式换行 / wrap / max-lines 触发降级、
换宽度只重排不重塑形、resize 只动受影响的那一段、word wrap 不切词、长词按 `word-break` 溢出或切开、
汉字无空格断行、显式换行与空段落、mixed BiDi 单行与换行后每行各自重排、strut 稳住 baseline（以及不给
strut 时基线确实会动）、`line-height` 与 half-leading、分数 scale、六种对齐 × 方向、
max-lines 与 max-height 截断、省略号的 cluster 安全裁切与零字节占用、省略号只塑形一次、
intrinsic min/max、竖排 fail-closed、layout cache 的上限与 LRU、字体代际让旧 layout 变陈旧、
计数器与产物对账、caret / hit-test / 选区直接跑在原生 layout 上。

## UiWorld 保留文本节点（Phase 4，#95）

Runtime 对文本只保存逻辑状态、revision 与句柄；shaping / layout 的不可变结果由 `nana-text` 持有。

```text
UiWorld 节点的 TextNodeState（NodeStore 侧表，每节点一条）
  revisions { content, shape, constraint, paint, edit }
  stamp     解析时的 content / shape / constraint revision + 后端代际
  layout    TextLayoutId（本 UiWorld 的 TextLayoutStore）
        │
        ▼
NativeTextEngine（SharedTextEngine，跨文档 / 窗口共享）
  ShapeRun cache ── Layout cache
        │
        ▼
ExtractedNode.text_layout ──► ScenePrimitiveKind::Text { layout: Option<RetainedTextLayout> }
```

### Dirty graph

`nana_ui_runtime::TextDirty` 说「什么变了」，`TextDirty::work()` 是依赖图的唯一出处，
`TextNodeState::invalidate` 只通过它 bump revision：

| 类 | 工作 | bump 的 revision |
| --- | --- | --- |
| `CONTENT` / `FONT` / `SHAPE_STYLE` | shape + layout + scene 几何 | content（仅 `CONTENT`）、shape、constraint |
| `CONSTRAINT` | layout + scene 几何 | constraint |
| `EDIT_STATE` | editor overlay（Phase 5） | edit |
| `PAINT` | 只重新提取 paint | paint |
| `TRANSFORM` / `OPACITY` | 只走 compositor | 无 |

变更在发生处分类，而不是事后比较：

- `SetText`：文本真的变了才算 `CONTENT`；`SetTextInput` / `ReplaceTextSelection` 每次都算（editor
  状态本身每次都会变）。任何内容
  失效使文本变空时立即释放保留的 layout（非 `Text` 元素没了文本就不再进任何文本 pass）。
  `SetIme` / `SetTextSelection` 是 `EDIT_STATE`。
- 计算样式落定时（`world/style.rs`）`classify_computed_style_change`：字体族 / 字号 / 字重 /
  斜体 / 字距 / feature / 变体轴 / kerning / direction → `SHAPE_STYLE`；行高 / word-break /
  line-break / writing-mode → `CONSTRAINT`；颜色 / 前景角色 / 选区色 → `PAINT`；opacity →
  `OPACITY`；其余字段不产生文本工作。
- `SetStyle` 只比较文本约束真正读的 `LayoutStyle` 字段（wrap / white-space / 省略号 / line-clamp /
  高度与最大高度是否确定 / 边框 / padding / 对齐）。paint、transform、opacity 与它们同在一个
  `LayoutStyle` 上，**刻意不比**。其余约束输入都会调度 layout、进 layout-scoped 趟。对齐不移动
  任何盒子，而且只有 retained layout 读它（宿主度量不读），所以只对持有 layout 的节点记
  `CONSTRAINT` 并标 TEXT。layout-scoped 趟跳过不可见节点，因此不可见→可见的纯文本节点由同一帧的
  调度趟一并重新解析，不多跑一趟 flush（editor / EmptyState / ModalFrame 的文本本就每趟重测，不在
  此列）；这一趟失败时它们留到重试。
- `WriteLayout` 只有盒子尺寸变了才是 `CONSTRAINT`；只移动位置（#33 的 head 插入）不是。
  布局写回的 padding、布局类动画（width / height / padding）直接写样式时同样记 `CONSTRAINT`。
- visual 变化只在「文本走哪条路径 / 前导指示器占多宽」变了时记 `CONSTRAINT` 并标 TEXT
  （`text_visual_key`；加一个 Checkbox 不改盒子尺寸），进度这类采样值不算。
- 字体集合变化不逐节点 bump：解析戳里带后端代际（宿主的 shaper 类型与 `TextShaper::font_generation()`——换一个 shaper 就是另一次测量，或引擎
  的字体系统身份 + `FontGeneration` + 语言提示代际——`set_language` 同样改变 `locl` 与 fallback）。`RuntimeDocument::flush` 每帧开头调
  `UiWorld::observe_text_shaper`，代际一变就把测量过的已解析节点（含只量一个空行盒的空 Text
  节点）与 EmptyState / ModalFrame / TextInput 排进这一帧（`FONT`），静止文档也会在新字体上重新结算；按字符与样式缓存的 glyph advance 与宿主路径的 `TextLayoutCache` 同时清空（它们的 key 不含测量者，换 shaper 时同样清空）。

transform / opacity 不 bump 任何 revision（Runtime 也从不需要发出 `TRANSFORM`：transform 从来不是文本的
输入），所以稳态 compositor 动画在构造上就碰不到 shaping /
layout；`retained_text_node.rs` 用真实动画帧验证：这类帧连一趟 Runtime 工作（文本 pass 在内）都
不产生，revision 原样不动。样式层面 opacity 归 `OPACITY`、颜色归 `PAINT` 由颜色 / opacity /
transform 的样式变更测试守住。

现在消费这张图的是文本 pass：它只读 content / shape / constraint 三个 revision 决定是否重做
shape / layout。scene 提取仍按 RENDER 脏整节点重新提取，`SCENE_PAINT` / `SCENE_GEOMETRY` /
`EDITOR_OVERLAY` / `COMPOSITOR` 与 `paint` / `edit` revision 是给绘制 retained layout（#97 换了
renderer，#99 才把产品路径的段落换成 `TextLayout`）和可编辑路径 #96 的合同，还没有更细的提取
消费者。

### 零工作快路径

两趟文本 pass（`shape_text` 与 layout-scoped 的 `shape_text_for_layout*`）对每个候选先做
`plain_text_is_current`：读侧表里的一条 `TextNodeState`，比较三个 revision 与后端代际。
**在构建 editor presentation、计算约束、clone 文本、构建 key、查任何 cache 之前**就决定跳过，
单节点成本与文本长度无关。

- 状态放在 `NodeStore` 的侧表而不是 `NodeRecord` 里：大 scope 的扫描付的是被访问记录的大小，
  几十字节的条目比整条记录更留得住 cache。
- 可见、没有自身文本的盒子（容器）解析为「无」并同样打戳，下一趟一次读表即跳过。空的 `Text` 节点
  不算这种盒子：它的行盒由调度趟测量，layout 趟不替它打戳。
- EmptyState / ModalFrame 的内建文本与 TextInput 的 presentation 每趟重测，不打戳。
- debug 构建里跳过路径会重算约束并与戳里记的约束比较，漏掉的 `CONSTRAINT` 失效会当场断言，
  而不是在屏幕上变成错的换行。

### 两种后端

`TextShaper` 多了两个默认方法：

- `font_generation()`：宿主测量所用字体集合的代际。`@font-face` 注册之后已解析文本会重新测量，
  Runtime 的 `TextLayoutCache` key 也带上它，旧字体下的度量不会被新字体命中。
- `take_text_work()`：宿主在 `shape()` 里做的文本工作。`NanaTextEngineShaper` 交出引擎的 shape /
  layout cache 与建出 layout 的计数（节点数由 pass 自己数），所以组件文本的引擎工作也在帧计数里；
  它的 `font_generation()` 折叠整个引擎代际（字体系统身份、字体代际、语言代际）。
- `text_engine()`：返回 `Some(SharedTextEngine)` 时，纯文本节点经 `nana-text` 解析，保留
  它读出度量的那份 `TextLayout`。**只有能绘制 retained layout 的宿主才该返回引擎**，否则同一节点
  会出现两个测量权威。`NanaTextEngineShaper` 是这样的宿主：纯文本走 `text_engine()`，其余文本
  （EmptyState / Modal / editor）的 `shape()` 也走同一个引擎。产品的 `NanaTextShaper` 从 #99 起
  正是它，因此返回引擎；`SceneWgpuPainter` 用**同一个**引擎、按同一节点的约束在逻辑 px 里排出
  同一份 `TextLayout`，再解析成 `NanaGlyphRun`，所以两边不会得出两个盒子。

度量合同：宽 = 最宽行的 `width_px`，高 = 各行 `height_px` 之和，ascent = 首行 baseline − top；
未声明行高按宿主一直用的 1.2em 传给引擎；只写了带 `mono` 的具名字体族时补上 `monospace`
兜底，与宿主一致。

### 句柄与生命周期

- `TextLayoutStore`（`nana-text`）：`{index, generation}` 槽位；generation 取自进程级单调序列，
  所以一个句柄只被签发它的 store 接受——交给另一个 UiWorld 与交一个已释放的句柄一样被拒。序列是
  `u32`，进程内签发约 43 亿次后回绕；此后跨 store 的句柄须同时撞上同一 index 与 generation 才会被
  误接受。
  `resolve(id, font_generation)` 另外拒绝旧字体代际下排出的 layout（`StaleLayout::FontGeneration`）。
- 解析结果（戳、保留或释放 layout）在整趟成功后才落地（不再是纯文本的节点——editor / EmptyState /
  ModalFrame——的释放在循环里立即做，它们本就不打戳）；某个节点度量非法让这一趟失败时，前面
  节点既不打戳也不换句柄，重试会重新解析它们，而不是凭一个没写回度量的戳跳过。
- 节点重新解析时，若引擎交回的是同一个 `Arc<TextLayout>`，句柄不变（`text_layouts_reused`）；
  否则释放旧槽、签发新句柄。节点删除（含整棵子树 / 文档内容拆除）时随节点释放；reparent 不释放。
  文本变空、节点变成 editor / EmptyState / ModalFrame、宿主不再提供引擎时也释放，为引擎建的文本副本
  （`TextSource`）随之释放。
- `ShapedRun.instance` 记下 glyph 实际塑形所用的 face 坐标与合成（粗体 / 斜体），绘制方不必
  再从样式重新解析一次实例。
- `ScenePrimitiveKind::Text.layout` 与 `ExtractedNode.text_layout` 是 renderer-neutral IR
  （`RetainedTextLayout { id, layout }`），相等性按「同句柄且同一份 layout」判，重排即场景变化。
  UiScene 不持有任何第三方引擎的 buffer。

### 计数器

`UiWorld::last_text_work_counters()`（帧内跨 pass 累加）返回 `nana_text::TextWorkCounters`，
#95 在原有字段上加了解释性口径：

```text
text_nodes_considered        看过文本节点的次数（含跳过的；一帧里调度趟与 layout 趟各算一次）
text_nodes_revision_skipped  只凭 revision 判定无工作的
text_nodes_shaped            真正进了 shaper 的
text_source_clones           为建 source / presentation 复制文本的次数
text_bytes_hashed            喂给内容 hash 的字节（每个 source revision 一次）
shape_cache_lookups          shape cache 查询（hit + miss）
layout_cache_lookups         layout 级缓存查询（hit + miss；引擎 layout cache 与宿主路径的
                             TextLayoutCache 合计，分开看用 WorkCounters 的 text_layout_cache_*）
layouts_created              真正建出的 layout
constraint_only_relayouts    其中复用已塑形 run 的（约束变化，不是新文本）
text_layouts_reused          解析后句柄不变的节点
```

宿主 shaper 路径同样计数：`TextLayoutCache` 的 hit / miss 进 `layout_cache_hits/misses` 与
`layout_cache_lookups`，它的 key 复制并 hash 整串文本，所以每个 key 同时计 `text_source_clones`
与 `text_bytes_hashed`；只有真的 miss 才算 `text_nodes_shaped`。没有自身文本、却被调度趟测量的
盒子不计入文本节点。引擎的工作只记在这里：`WorkCounters` 的 `text_shaped_runs` /
`text_layout_cache_*` 保持宿主 `TextShaper::shape` 与 `TextLayoutCache` 的原义。没跑文本 pass
的空闲帧不覆盖这份计数，与 `last_work_counters` 一致；失败的一趟不留下任何计数给下一趟。

`nana-dirty-frame-benchmark` 把它们按帧平均（小数，不把只在部分帧发生的工作抹成 0）写进每格的
`text_work`，并支持
`--engine measure|nana-text`（后者用内置 UI 字体的真实引擎解析每个标签）。

### 测试

`crates/nana-ui-scene/tests/retained_text_node.rs` 全部走 `RuntimeDocument::flush`：
标签保留的 layout 与度量同源且进入场景、只改对齐 / 只改 line-clamp 也重排保留的 layout、非 Text
元素清空文本释放 layout、宿主不再提供引擎时全部释放、语言变化让组件文本在引擎宿主下重测、移动全部标签但不改尺寸的帧里每个标签都是候选且零文本
工作、句柄不变、同约束重新解析拿回同一份 layout 时句柄复用、颜色 / 语义前景 / opacity /
transform 不碰 shaping 与 layout、宽度变化只重排（`constraint_only_relayouts == layouts_created`）、
内容变化只 shape 改动的那个节点、feature 变化重塑形但不复制文本、相同标签共享一份 layout 且
建出的 layout 数不随数量增长、#33 head-dirty 工作负载（两种后端）每个候选都凭 revision 跳过且
候选数线性、字体集合变化让静止文档也换掉旧代际 layout（EmptyState 这类不打戳的文本同样重测）、空 Text 节点在
新字体下重测行高、标签变成 EmptyState 时释放 layout、字体代际相同的另一个宿主 shaper 也会重新测量、失败的一趟不留戳且重试会重新解析、同一趟里
组件文本经同一引擎测量（不死锁，且引擎这一帧建出的 layout 全部计入帧计数）、删除 / reparent / 拆除的
句柄生命周期、
transform + opacity 稳态动画不动 revision、padding 动画重排、高度只变「确定」也算约束变化。
「帧内零文本工作」的断言都与帧前的计数比较：计数保留上一个跑过文本 pass 的帧，只有被这一帧改写
过的值才是这一帧的工作。
`nana-text` 侧：`TextLayoutStore` 的释放后复用、跨 store、旧字体代际；`ShapedRun.instance`
带变体坐标；语言提示变化推进引擎代际。Runtime 单测覆盖 dirty graph 本身与样式分类。

### 本阶段没做的

| 项 | 状态 |
| --- | --- |
| ~~产品绘制 retained layout~~ **已接**（#99） | `SceneWgpuPainter` 直接画 `ScenePrimitiveKind::Text.layout` 指的那一份 `TextLayout`——量它的和画它的是同一个段落，不是两个按构造相等的段落。盒子对不上的节点（尾随控件、ListItem 覆盖文本盒）退回自己排一份，见「画笔取保留 layout」 |
| Editable 文本 | 见 Phase 5（#96）：presentation 仍每趟重测、不打戳，引擎宿主的探针改由段落几何回答 |
| font-size / 字体轴动画 | Runtime 尚无 CPU 写回路径；一旦写回计算样式，会按 `SHAPE_STYLE` 分类 |

## Editable 路径（Phase 5，#96）

Editable 是 Paragraph layout 之上的附加层，不是每段文本都背着的状态：标签从不持有 session 或几何，
没有被编辑的文本不跑这里的任何代码。

```text
EditableText（私有存储 + TextRevision）── EditState（selection / composition / goal x）
        │                                        │
        └──────── EditSession：输入、删除、移动、IME ─┘
                            │ display text（committed + preedit）
                            ▼
        EditorGeometry：每段一个 TextLayout，按段落失效
        hit_test / caret_rect / selection_rects / line_bounds / vertical / visual_move
```

### 存储与 revision

- `EditableText` 的存储类型私有，只经 `replace / insert / delete / set_text` 改动；字节没变的
  替换返回 `Ok(None)`，不推进 revision。调用方读 `&str`，从不命名存储类型——以后换 rope /
  piece table（由大文档 benchmark 决定）不改 API。`TextEdit { range, inserted_len, revision }`
  足以把旧偏移映射进新文本（`map_offset`）。
- 偏移是 committed 文本的 UTF-8 字节，出命令时总在 grapheme cluster 边界上；`utf16_offset` /
  `offset_of_utf16` 给按 UTF-16 计数的平台 IME / a11y API，`grapheme_index` / `offset_of_grapheme`
  做 grapheme 序号转换。
- `EditRevisions { text, selection, composition }` 三个 revision 独立：caret 移动只推进 selection，
  composition 变化只推进 composition，caret blink 什么都不推进。

### 导航

`editable::navigation` 是 grapheme / word / 逻辑行的唯一定义，Runtime 的 `text_editing` 直接委托给它。
扫描限定在偏移所在的逻辑行（UAX #29 在换行后必断，WB3a / GB5），所以大文档里按词移动的成本是一行
而不是全文；`CR LF` 是一个 cluster，窗口因此包含行尾换行。单元测试逐偏移对照整串分段的结果。

`Motion`：`GraphemeBackward/Forward`（逻辑序）、`Left/Right`（视觉序，需要几何；没有几何时退化为
逻辑序）、`WordBackward/Forward`、`LineStart/End`（视觉行；无几何为逻辑行）、`LineUp/Down`（保持
goal x；无几何按 grapheme 列）、`ParagraphStart/End`、`DocumentStart/End`。非扩展的水平移动遇到
非空选区时收拢到对应边（`Left/Right` 有几何时收拢到屏幕上的左/右边，RTL 文本里左边是逻辑末尾）。
`delete(motion, geometry)` 同样可以按视觉行删除到行首 / 行尾。输入文字后 caret 取 `Upstream`，贴着刚输入
的字（LTR 行尾输入 RTL 字时不会跳到 RTL 段另一侧；软换行处也留在本行）。几何只有在由本 session 当前
text + composition revision 同步过时才被使用；`EditRevisions` 带 session 身份（进程内唯一，克隆即新
session），另一个 revision 数值恰好相同的 session 不会误用它。陈旧几何退回逻辑移动。

### Caret affinity 与 hit-test

- `TextLayout::caret_geometry`：`Upstream` 画在前一个 cluster 的逻辑末端，`Downstream` 画在后一个
  cluster 的逻辑起点。同方向文本里两者 x 相同；在 BiDi 边界上它们是同一字节的两个不同视觉位置。
- `TextLayout::hit_test` 保持 IR-only 语义（语料 golden 记录它）。`hit_test_text(text, x, y)` 额外
  拿源文本：一个 glyph 画多个 grapheme 时（`ffi` 连字）落到最近的 grapheme 边界，affinity 指向被点中的
  cluster——点在 BiDi 边界哪一侧，caret 就画在哪一侧。
- `caret_stops(line, text)`：一行里所有 caret 位置按 x 排序（边界两侧各一个，软换行行尾只有 upstream）。
  画在同一 x 的不同位置按行的阅读顺序相邻排列。`EditorGeometry::visual_move` 按 stop 序号逐个移动，保证
  从任何 caret 出发、任一方向都能到达每个 grapheme 边界；代价是在 BiDi 边界上有一次按键只改变逻辑位置、
  屏幕上不动（两个逻辑位置画在同一处，都要能到达：LTR 行末尾是 RTL 文本时，行的逻辑末端只画在那里）。
  软换行悬挂的空白里的每个位置作为该行行尾的 upstream stop 加入。行边缘按段落书写方向跨到相邻行。
- `hit_test_text` 点在行外时取该侧视觉最边上的 stop（同 x 取阅读顺序最靠外的，LTR 行右侧即文本末尾，
  点击后输入是追加），而不是行的逻辑边界——软换行行末尾是反向文本时，逻辑边界画在另一侧。
- 单次查询（caret、hit）逐 cell 扫描；需要一行内所有位置的 `caret_stops` 才建按 cluster 排序的索引并二分，
  且对行文本只做一趟 grapheme 分段，行长线性。同一 cluster 的多个 cell（组合记号）合并成一个视觉范围。

### Selection

`selection_rects` 由逻辑范围 + layout 派生，不改 shaped glyph；一行混排 BiDi 产生多个不相接的 rect，
多行每个视觉行至少一个。只看与范围相交的行。选区颜色是 paint，不 reshape。

### IME composition

composition 是 transient overlay，不是先 commit 再撤销：`Composition { replaced, text, selection }`
代表 preedit 替换掉的 committed 范围（开始组字时的选区）。committed 文本组字期间不动；显示与排版用
display text（`display_text` / `display_offset` / `committed_offset`）。

| 操作 | 语义 |
| --- | --- |
| `set_preedit(text, selection)` | 开始或更新；空 text 等于 cancel；IME 自己的光标 / 目标段在 text 内 |
| `commit(text)` | 替换 `replaced`（没在组字时替换选区），结束组字，caret 在插入文本后 |
| `cancel_composition` / `blur` | 丢弃 preedit，committed 文本与选区与开始前完全一致；焦点转移即 cancel |
| `delete_surrounding(before, after)` | 没在组字：删除选区前后字节与选区本身（沿用 Runtime 合同）。组字中：锚点是 preedit 替换的范围，**保留它**，只删前后字节——否则取消组字会丢字；preedit 随文本移动 |
| `surrounding_text(before, after)` | committed 文本片段，窗口按字符边界向内收缩，带片段内选区 |
| `ime_cursor` / `candidate_rect` | 组字时是 preedit 光标，否则是选区 focus；矩形来自几何 |

这两条规则是 `editable::ime` 里的纯函数（`surrounding_deletion` / `surrounding_window`），不持有
`EditSession` 的宿主照同一规则执行。`set_text` 在字节不变但取消了组字时返回 `EditChange::Composition`。

组字期间普通输入、删除与 caret 移动一律拒绝（IME 拥有 caret 附近文本）。几何把 preedit 标成
`CompositionSegment::Preedit` / `PreeditTarget` span，它们各自成 run，绘制方可以装饰。

### EditorGeometry 与增量失效

文本在 `\n` 处分段，每段单独经引擎排版（`TextKind::Editable`）后纵向堆叠。shaping 不跨换行、UBA 在换行
处开新段，所以单段排版与整段排版行一致（`paragraph_geometry_matches_laying_the_whole_text_out` 对照
caret x / 行顶 / 宽高）。`max_lines`、`max_height_px`、省略号作用于整段文本，`preserve_lines == false`
会把换行折成空格，这些约束下整体作为一段，而且不做前后缀复用——文本变了就整段重排。

`sync` 比较段落字节（前缀段 + 按长度差对齐的后缀段）与每段的 composition 标记，只重排中间变化的段落：

- 编辑一个字符：重排 1 段，其余段落保留原 `Arc<TextLayout>`；拆段 2 段，合段 1 段（shape cache 命中）。
- composition 更新：只重排 preedit 所在段。
- selection / caret 变化：`sync_session` 凭 revision 直接返回，零比较零排版。
- style / 约束 / 引擎 epoch 变化：全部重排（shaping 仍可命中缓存），不计入编辑工作。

查询（`caret_rect`、`hit_test`、`selection_rects`、`line_bounds`、`vertical`、`visual_move`）只读保留的
layout，从不 shape 或 layout。

`sync` 先比一遍字节：段落已经排出这份文本（且 style / 约束 / epoch 未变）就原样返回，段落连同它们的
layout 一个不动、`revisions` 保留，`GeometrySync::unchanged` 置位。**想缓存任何从几何派生出来的东西，
只能凭这个标记**——`paragraphs_laid_out == 0` 不代表几何没动：删掉让首段为空的那个换行会把一个段落
合并掉、其后每个偏移平移，却没有任何段落被重排。引擎宿主的编辑器测量缓存就是这么钉住的。

增量 sync 与「只见过当前文本」的一份几何逐项等价（`incremental_syncs_land_where_syncing_from_scratch_does`
走完一串编辑，比对段落 start / top / 行数、尺寸、每个 caret 与一片命中网格）。

### 计数器

`TextWorkCounters` 新增：

```text
editable_mutations              改变 committed 文本的编辑
editable_bytes_inserted/deleted
caret_only_updates              只移动 caret（结果为收拢选区），不含编辑顺带的 caret 移动
selection_only_updates          只改选区（结果非空）
composition_updates             preedit 开始 / 更新 / 结束
paragraphs_reshaped_from_edit   增量 sync 重排且 shaping 未命中缓存的段
paragraphs_relayout_from_edit   增量 sync 重排的段
hit_test_queries                对保留几何的命中查询
caret_geometry_queries          对保留几何的 caret 查询
```

结构门禁（`crates/nana-text/tests/editable_text.rs`）：caret blink 反复查询 caret 时引擎
`shape_cache_misses` / `shape_cache_hits` / `layout_created` 全不变；selection-only 的一串移动同样不变。

### Runtime 接入与分阶段门

| 阶段 | 状态 |
| --- | --- |
| 1. 内部 fixture | `EditSession` + `EditorGeometry` 覆盖 Latin 输入删除、拼音组字提交、日文目标段、韩文字母组字、emoji / 肤色修饰删除、组合记号移动、连字内 caret、阿拉伯混排视觉移动 / affinity / 选区、换行多行选区、点击与拖选、组字中失焦、取消组字、剪贴板 |
| 2–4. TextInput / TextArea / 编辑器 | **语义委托 `nana-text`**：grapheme / word / 行导航、选区合法性、IME 删除周边（组字中保留 preedit 替换的选区）与宿主上报的 surrounding text 窗口（`clip_ime_surrounding`：放得下的选区完整上报，预算两侧互补）都走 `nana-text` 的规则；文本与 composition 仍存在 `TextInputState` / `ImeComposition` 里（产品合同，Vue / JS 同样读写它们），没有换成 `EditSession`；Runtime 的 `SetTextInput` / `SetTextSelection` / `ReplaceTextSelection` / `SetIme` 记入上面的编辑计数（随下一趟文本 pass 上报）。**几何按宿主分阶段**：`NanaTextEngineShaper`（能绘制 retained layout 的引擎宿主）为每个编辑器节点保留一份 `EditorGeometry`，`text_position` / `text_caret_position` / `text_highlights` / 新增的 `TextShaper::text_hit_at_point` 与编辑器度量都由它回答；上下移动与翻页在支持点命中的宿主上用「caret 位置 + 末行位置 + 一次点命中」解析（目标 y 取相邻行内侧 0.5px，行高不同也不跳行），不再对位置探针二分。几何按节点保留：同一份文本快照的探针批次（`with_text_probes`）只同步一次；批次外的单个探针（上下移动、左右视觉移动、点击、每趟度量）各做一次与文本长度成正比的**块比较**以确认几何仍是这份文本（不 shape、不 layout，字节没变时段落一个不动、`revisions` 保留）；探针一律按 presentation 的约束提问（`text_input_presentation_constraints`）——问别的约束等于在问编辑器没有被绘制的那份几何，保留几何的宿主还会为一个节点摆两份布局；编辑器的 shape 不经过 Runtime 的内容寻址 layout cache（`retains_measurement`）；`TextShaper::horizontal_offset` 按单行独立排版，不碰编辑器几何；#99 之后产品 `NanaTextShaper` 就是 `NanaTextEngineShaper` 的壳，测量与绘制同源，编辑器几何因此也走这条路 |
| 5. Vue / NanaVue | 同一 `TextInputState` / `ImeComposition` 合同，经 Runtime 生效 |

引擎宿主的保证（`crates/nana-ui-scene/tests/editable_text_node.rs`，走 `RuntimeDocument::flush`）：
caret / 选区移动整帧 `layouts_created == 0` 且引擎 shape miss 不变；在 30 段 TextArea 中间打一个字只新建
1 个 layout（`paragraphs_relayout_from_edit == 1`）；每次 preedit 更新只新建 1 个 layout，preedit 不计
`editable_mutations`；点击经 `text_hit_at_point` 命中且不排版；软换行行尾（`"中" * 120`）行内点一次、下一行行首点一次拿到同一个偏移，caret 分别画在两行上。

顺带修正：多行编辑器的值保留换行，不再跟随 `white-space` 折叠（此前引擎宿主会把 TextArea 排成一段）；
组字中焦点移走时取消的 preedit 不再继续画在原编辑器里（`remove_ime` 重新派生 presentation）；
初次挂载编辑器不再计作一次编辑；
单行字段的选区 / preedit x 改用本批次已持有的 presentation 布局探测，不再额外排一份不换行的全文。

### Runtime caret affinity

`TextSelection { anchor, focus, affinity }` 的 affinity 属于 `focus`——caret 那一端，类型就是
`nana_text::Affinity`（Runtime 重导出为 `TextAffinity`），探针直接把它交给引擎的 caret 几何，不再多一层
转换。同一个字节偏移在屏幕上可以是两个位置：没有悬挂空白的软换行处（CJK 这类行尾即下一行行首）
`Upstream` 是上一行行尾、`Downstream` 是下一行行首；BiDi 边界两侧同理。只有指针命中与视觉移动知道
用户指的是哪一个，所以它们把 affinity 一路写进选区；纯按字节派生选区的构造（`TextSelection::new` /
`caret`，以及所有编辑）取默认的 `Downstream`，不换行的文本两者画在同一处。

- affinity 计入 `TextSelection` 的相等性：同一偏移换一侧就是屏幕上的另一个位置，`SetTextSelection`
  因此照常标脏重画，并记一次 caret 更新。
- 编辑把 affinity 归零：附加光标的偏移被编辑挪动过就退回 `Downstream`（当初解析在哪一侧不再作数），
  `TextSelection::new` 与合并选区同样如此；吸附到原子边界改了落点时也归零。
- 探针合同：`TextShaper::text_caret_position(offset, affinity, ..)` 回答「带这个 affinity 的 caret 画在
  哪」，`text_position` 仍是「这个边界的原点」（选区条带、run 起点、括号框等按字节派生的几何用它）；
  `text_hit_at_point` 返回 `TextHit { offset, affinity }`。默认实现忽略 affinity，不换行或分不出两侧的
  宿主每个偏移只有一个位置——譬如测试里的轻量 shaper，一律 `Downstream`。
- `NanaTextEngineShaper` 直接用 `EditorGeometry::caret_rect(offset, affinity)` 与 `hit_test` 的 affinity，
  #96 里「命中软换行行尾退回前一个 grapheme」的兜底已经删掉：那个行尾位置现在点得到，caret 留在被点
  中的行，BiDi 边界点哪一侧就画哪一侧。
- a11y 没有 affinity（AccessKit 的选区只有字符下标），`AccessibilityAction::SetSelection` 落 `Downstream`。

### 视觉序的左右箭头

`TextCaretIntent::Left/Right` 在**能问到几何**时走视觉序，问不到时退回逻辑 grapheme 步进
（没有换行与方向信息的宿主，「左」本来也只能是这个意思）。探针是
`TextShaper::text_caret_visual_step(offset, affinity, rightwards, ..)`，引擎宿主用
`EditorGeometry::visual_move` 回答——定义只有 `nana-text` 一份，Runtime 不重造。

规则一条，不按平台分叉：**Left/Right 一律视觉序**（浏览器与 Windows 编辑框的行为；纯 LTR
文本里视觉序与逻辑序完全一致，所以只有 BiDi 与换行处会变）。两处可见的差别：

- `abc ابج` 里按 Right 走进阿拉伯语词，逻辑上是从词尾往词首走，因为那在屏幕上是从左往右；
- 软换行处（CJK 行尾这类没有悬挂空白的换行）行尾与下一行行首是同一个字节的两个 affinity，
  于是是**两次**按键：一次落在行尾（`Upstream`），一次落到下一行行首（`Downstream`）。逻辑步进
  会跳过行尾那个位置，直接越过一个字符。

探针只在**画出来的就是值本身**时问：secure 字段画的是圆点、空字段画的是占位符，
这两种按字素步进（一列相同圆点里「左」本来也只有这个意思），否则等于拿没被绘制的那份
布局回答，还会把值——密码也在内——排版进宿主保留的几何与按文本做键的 shape cache。
组字期间一次 caret 移动都不会发生（`focused_text_editor` 在组字时返回 None），而组字
**结束**留下的空 preedit 不算组字。

`nana-text` 的 `EditSession` 在 Left/Right 且选区非空时会塌缩到选区的**视觉边缘**；Runtime 这条
路仍是「从 focus 起步一格」（`moved_selection`）。两边的这条差别留在 IME 后端接 `EditSession`
那一步一起收，不在本次改。Word/Line 意图按定义是逻辑的，垂直移动走自己的几何路径。

### 为什么文本还没搬进 EditSession

Issue #96 的「IME 单一语义后端」目前只兑现了一半，而且是有意的：**规则**已经全部委托给
`nana-text`（grapheme / word / 行导航、IME 删除周边与 surrounding window、caret 几何与命中、
视觉序移动），**存储**仍在 `TextInputState` + `ImeComposition`。剩下的这一半不是补几行能收的，
它要先定三件事：

1. **多光标**。`EditSession` 是单选区的；Runtime 支持 N 个光标同时编辑，`replace_selection` 是
   一趟多点 splice。把编辑交给 session 就得先让 session 有多选区，或者接受 Runtime 继续在外面
   remap 附加光标（现在就是这样）。
2. **显示文本不只有 composition**。session 的显示文本 = committed + preedit；Runtime 画的那份
   还叠了折叠摘要、inlay、secure 掩码。几何是按**画出来的那份**同步的，所以要么让 session 认识
   这些产品概念（产品概念漏进 `nana-text`，不行），要么继续由 world 派生显示文本——那样 session
   就只是 committed 文本 + 组字的权威，`EditorGeometry::sync_session` 的 revision 快路仍然用不上。
3. **产品合同**。`TextInputState.value` 是 `pub String`，Vue / JS 直接读写；撤销日志按
   `TextInputState` 快照存。存储换位置要么保留一份派生视图，要么动公开 API。

所以这一项的下一步不是「改成 `EditSession`」，而是先挑一条：给 `nana-text` 的 session 加多选区，
还是把 `TextContent.value` 换成 `Arc<str>`（顺带消掉每帧 3 处整值克隆并给探针一个天然身份）。
两条都是独立的一块工作，不该混在 #96 的尾巴里做。

### 本阶段没做的

| 项 | 状态 |
| --- | --- |
| caret blink | Runtime 目前没有 blink；它属于 scene overlay 的可见性 / 不透明度（paint），不得推进任何文本 revision。`nana-text` 侧门禁已钉住「只查询几何」零文本工作 |
| ~~Runtime 视觉序左右移动~~ **已接** | 见「视觉序的左右箭头」：`text_caret_visual_step` 探针 + `EditorGeometry::visual_move`，无几何时退回逻辑步进。选区非空时的塌缩端仍与 `EditSession` 不同 |
| a11y composition | 现有 a11y 合同只有 value / selection / editable（caret 即 selection focus），AccessKit 没有 composition 范围，不伪造；字符级 geometry 同理 |
| 局部 cluster splice | 不做；最小失效单位是段落 |
| IME 语义后端换 `EditSession` | 规则已全部委托，存储没换；三个前提见上一节 |
| 大文档存储 | 仍是 `String`，**基准跑完后确认不换**：310 KB 文档上一次编辑的 memmove 是整帧成本的 0.5%，见「大文档编辑基准」 |

## 性能与许可证收口（#99）

- **性能矩阵**：[text-release-matrix-2026-09-20](performance-data/text-release-matrix-2026-09-20/README.md)
  ——static steady / paint-only / compositor-only 三条门禁、#33 的 head-dirty
  网格（2k/4k/8k 节点上 `text_nodes_shaped` / `text_bytes_hashed` /
  `layouts_created` 全为 0）、以及 8000 行编辑器的每次编辑只重排一段。
  没覆盖的几项（constraint-only resize、text-heavy table 等）在那篇里逐条写明。
- **第三方与许可证**：[third-party.md](third-party.md)——release 依赖图 587 个
  外部 crate 全是宽松许可证，`cryoglyph` 已不在 `Cargo.lock` 里，`cosmic-text`
  只剩 `nana-text` 的一条 dev 边；从被替换引擎照抄的一处（`SubpixelBin::split`）
  已就地署名。

## NanaRenderer::text（Phase 6，#97）

绘制这一半从 cryoglyph 换成了 NanaUI 自己的子系统。目录是
`crates/nana-ui/src/scene_paint/text/`：

```text
已塑形的段落
        │  resolve
        ▼
NanaGlyphRun / PlacedGlyph              glyph.rs
        │  GlyphRasterKey
        ▼
GlyphRasterCache ── GlyphRasterizer     raster_cache.rs / raster.rs
        │  GlyphImage
        ▼
GlyphAtlasManager ── GlyphUploadQueue   atlas.rs / upload.rs
        │  GlyphAtlasEntryId
        ▼
TextPipeline                            pipeline.rs / mod.rs
        ▼
      WGPU
```

resolve 以下的每一层都不知道段落是谁排的。#99 换掉的正是 resolve 之上的那两处——段落改由
`nana-text` 引擎排，rasterizer 的 face 来源改成引擎的字体层——下面的 raster cache、atlas、
上传队列与 pipeline 一行未动。

### 画笔取保留 layout

段落的来源有两条，优先第一条：

1. **Runtime 保留的那一份**（`ScenePrimitiveKind::Text.layout`）。句柄本身就是身份——
   `TextLayoutId` 是两个 `u32`，代际槽位不会以同一代际重发，所以直接打包成 `u64`
   （最高位置 1，见 `PARAGRAPH_IS_RETAINED`），**不读字符串、不做哈希**。wrap、省略号、
   max-lines、`white-space`、方向这些约束全都是 Runtime 已经定好的，照单全收，不再从场景
   对它们的描述里重建一遍。
2. **画笔自己排**（`lay_out` + `ShapeCache`）。给的是没有句柄的文本（编辑器 presentation、
   EmptyState / Modal 的内建文本），以及句柄不可用的情况。

唯一要成立的前提是**对齐盒**：glyph 落位是在 `max_width_px` 里对齐的，而画笔把段落贴在盒子
左边，所以 `layout.constraints.max_width_px != Some(bounds.width)` 时必须退回第二条——
带尾随控件的 Switch、用 content geometry 覆盖了文本盒的 ListItem 就是这种节点，否则居中的
文字会画得不居中。这是一次浮点比较，判据自检：两边什么时候不一致，画笔当帧就自己排，不会画错。

`text_retained_layouts_drawn` 计的是走第一条的段落数，和 `shape_cache_misses` 一起读就知道
某个 workload 实际在哪条路上。

A/B（`nana-text-paint-benchmark`，交替两种跑序各 3 轮取 min，见
[performance-data](performance-data/text-paint-retained-layout-2026-09-20/)）：

| workload | batch p50（画笔自己排） | batch p50（取句柄） | |
| --- | --- | --- | --- |
| `static-unique` 10k | 2.949 ms | 2.070 ms | −29.8% |
| `static` 10k | 2.373 ms | 1.995 ms | −15.9% |
| `mutate-1pct` 10k | 2.222 ms | 2.037 ms | −8.3% |
| `mutate-1pct` 1k | 0.144 ms | 0.128 ms | −10.8% |

`static` 这一行值得解释：两边 `shape_cache_misses` 都是 0，省下来的是**每节点每帧一次
HashMap 查询**——旧路径要 `shape_cache.holds(hash)` 确认段落还在缓存里，句柄由场景持有，
不需要问。`static-unique` 那 30% 则是旧路径为 1 万条不重复段落维护缓存的代价，现在一条不存。

### 三条生命周期，刻意不一样

| | 归属 | 键 | 谁能复用 |
| --- | --- | --- | --- |
| 已塑形段落 | 一个 painter 的 CPU 状态 | 文本 + 样式 + 盒子 + 字体代际 | 同一 painter 的每一帧 |
| glyph 位图 | 一个 painter 的 CPU 状态 | face 实例 + 尺寸 + 亚像素桶 + 合成 + 字体代际 | 该 painter 的每个 target |
| atlas 落位 | **device** 状态 | 同上，经代际句柄访问 | 同一 device 的每个窗口 |

**颜色、节点不透明度、场景变换都不在栅格键里**。同一个字两种颜色共用一张位图，一条动画
标签不会每帧重栅格化。

### 句柄，不是坐标

`GlyphAtlasEntryId { index, generation }`。instance 是在这一帧全部落位都定下来之后，
从句柄回读矩形才建出来的——所以 atlas 可以在同一帧里搬动（compact）或淘汰某个字形，
而这一帧已经记下它的 run 只会丢掉那个字形，绝不会采样到现在占着那块矩形的另一个字。
槽位回收时 generation 自增，旧句柄被拒绝并计入 `atlas_stale_handle_rejects`。

跨帧保留的 draw command（`PreparedBatch`）另有一道闸：`text_placement_epoch`
（淘汰数 + 搬动数）变了就重建，因为那批 instance 里烤着的矩形已经不是那个字形的了。

### 两条管线，一张 atlas

- **Axis**：平移下的像素对齐文字。每字一个 24 字节 instance、四个顶点，由这一批本来就带的
  scissor 裁剪，Nearest 采样。shell 里的字几乎全走这条。
- **Affine**：旋转 / 缩放 / 圆角裁剪下的文字。每字六个顶点，带与 `Quad` 同一份 homography
  和 fragment clip，Linear 采样。

两条共用同一个 atlas bind group，所以一条旋转标签换进来的页，正立的标签直接命中。
一个 segment 固定一对（mask 页, color 页）；页变了就**断开**而不是重排，run 内的字序
因此始终是文档序。

### atlas 策略

页 1024²（mask 1 MiB / color 4 MiB），总预算 48 MiB。每个字形四周留 1 texel 的透明
gutter，并且**真的上传那圈 0**——一块被淘汰后重用的矩形还留着上一个字的像素，旋转的四边形
会采到自己矩形外半个 texel。

两张 1×1 的占位页只为把 bind group 填满：一个从不画 emoji 的 shell 因此不会为 color 页
付 4 MiB。放不下时依次尝试：开新页 → 重排（`compact`，把活着的字形按高度重新打包，
`atlas_relocations`）→ 淘汰最冷的四分之一（`glyph_atlas_evict`）。**这一帧用过的字形不会被
淘汰**，所以正在画的东西不会被从底下抽走。

重排要求每个活字形的位图都还在 raster cache 里，否则整个重排被拒绝、atlas 原样不动——
搬完却补不回像素会把这一帧正在画的字变成空白。

### 只传新增区域

新字形 = 栅格化 → 分配矩形 → 入队一个上传区域 → 在用它绘制之前提交。每帧重传整张 atlas
是被禁止的，`glyph_upload_regions` / `glyph_upload_bytes` 就是这条的证据。

in-flight 安全性来自 `wgpu::Queue::write_texture` 本身：它在调用时就把字节拷进 queue 自己的
staging，并把传输排在下一次提交之前。手写 ring 需要 painter 看不到的 fence——painter 既不
拥有 submit 也不拥有 surface。

排队的位图是 `Arc<GlyphImage>`，这同时是生命周期合同：raster cache 可以在上传真正发生前
淘汰产出它的那一条，队列不会指向已释放的字节。

### 计数器

`SceneWgpuPainter::text_glyph_counters()`：resolve 请求、栅格化次数、raster cache
命中 / 未命中 / 淘汰 / 字节、atlas 命中 / 未命中 / 淘汰 / 页数 / 字节 / 占用千分比、
上传区域与字节、搬动次数、过期句柄拒绝数、pipeline draw 数。

### 与 cryoglyph 的像素差

改绘制那一天，component-gallery 的 561 张快照里 **551 张逐字节不变**，1 张是本来就抖动的
`gallery-sidebar-collapsed-dark`（曾因自转的 Spinner 被当成抖动；2026-09-20 复核连跑两遍零差异），
剩下 9 张 `motion-*` 变了，且是**变好**：

旧的 affine 路径把 mask 存成「RGB=255 + A=覆盖率」的 RGBA 图，着色器又做
`sampled.rgb * color.rgb`，于是字形外缘的抗锯齿被乘了两遍覆盖率。新路径 mask 页是
R8Unorm，覆盖率只作用在 alpha 上，旋转文字的边缘因此不再被压暗（最大 36/255）。

栅格尺寸按 f32 位精确入键，没有分桶：1/64 px 的分桶会把 15.6px 的标题挪到 15.59375，
抗锯齿边最多动 11/255——这一阶段没有理由改渲染。键空间由 raster cache 的字节预算和 LRU
兜底，而上面那层已塑形段落本来就按同一个精确尺寸做键。

### CPU：一次多余的遍历，量过

同机（Apple M4，release）400 行标签、约 1 万字形、每帧重建批次、塑形全部命中缓存：

| | batch p50 | batch p95 |
| --- | --- | --- |
| cryoglyph（拉丁） | 0.272 ms | 0.288 ms |
| NanaRenderer::text（拉丁） | 0.316 ms | 0.344 ms |
| cryoglyph（每行 24 个不重复汉字） | 0.293 ms | 0.338 ms |
| NanaRenderer::text（同上） | 0.314 ms | 0.348 ms |

差的那 0.02–0.04 ms 是**多出来的那一遍**：cryoglyph 边遍历边写顶点，这里先记下句柄，
等这一帧所有落位都定了再回读矩形建 instance。这正是代际句柄的用处——中途搬动或淘汰
不会让先写好的 UV 指向别的字形——所以它不是可以顺手省掉的开销。

两个想省掉它的改法都写出来量过，都落在噪声里：把矩形随句柄一起缓存进 placement、
按 epoch 走快慢两条路（0.324 ms，反而略慢，多一个分支）；把 placement 从 32 字节
压到 20 字节（0.317 ms）。后者留下了——它本身更省内存、颜色改成每 run 打包一次而不是
每字形一次；前者删掉了，多出来的双路径没有换来任何东西。20 字节这条让 affine 路径的
颜色也走 8 位量化，与 axis 路径和 cryoglyph 一致，代价是旋转文字最多 1/255 的色差。

换来的是每帧不再有 per-node 的 GPU 分配（旋转文字过去每个不同变换要一张纹理、一个
bind group、一个顶点缓冲），以及同 device 的窗口之间不再各自持有一张 atlas。

### 这一阶段没做的

| 项 | 状态 |
| --- | --- |
| 逻辑 `TextLayout` 与 raster scale 解耦 | 塑形仍在物理 px 上做（hinting 要求如此），所以 DPI 变化会重塑形一次。renderer 这一侧已经只把 scale 放进栅格键；真正的解耦要等 #99 把 `nana-text` 的逻辑 layout 接上来 |
| 持久 GPU instance / 零 prepare | #98 做了，见下一节 |
| SDF / MSDF / LCD | #97 非目标。`GlyphRenderMode` 与 `AtlasPageKind` 是留好的扩展点 |
| 跨 Device 共享 CPU 位图 | 一个 painter 一个 raster cache。同 Device 多窗口共享（`swap_target` 只换 per-target 缓冲），换 Device 会重栅格化一次 |

## 保留期文本（Phase 7，#98）

Phase 6 把**画**这一半换成了 NanaUI 自己的子系统，但每一帧仍然从零重建 instance：
遍历每个字形、查 atlas、写 24 字节、再和上一帧比对。只要屏幕上有任何一处在动，
批次就要重建，这笔钱就要付一遍——一个 48k 字形的界面因为角落里一个 spinner
在转，每帧要走四万八千次。

Phase 7 让它不用走。

```text
layout 没变     ── 还是那一段塑形结果
phase 没变      ── 位图就是为这个亚像素相位栅格化的
字体代际没变    ── face id 还是那一批
atlas 没变      ── 矩形还是这些字形的
        ↓
      复用 TextGpuEntry 的 instance range
        ↓
      画
```

前三条在 `prepare` 里比对，任何一条不成立就**只**重建这一个 entry。第四条不重建而是
**修补**：entry 为每个字形留着 atlas 句柄，搬动之后重读矩形就行，不塑形、不栅格化、
不上传。

### 三张表，各自的生命周期

`scene_paint/text/` 现在多了 `entry.rs`：

| | 内容 | 什么时候写 |
| --- | --- | --- |
| entry block | 一段文本解析出来的 instance（24 B/字形）+ 每字形一个 atlas 句柄（8 B） | 文本 / 相位 / 字体代际变了 |
| run row | 该段文本的**整像素原点、颜色、不透明度、用哪份 presentation**（48 B） | 它移动、改色、淡入淡出了 |
| presentation row | 变换与裁剪（160 B），按位去重 | 变换或裁剪变了 |

instance 里**不再**有位置、颜色和变换——它只有相对 run 原点的偏移、atlas 矩形，以及
一个 run 行号。所以：

- **移动**（整像素）：改 run row 的两个 float。亚像素移动仍要重解析，因为那真的换了位图。
- **改色**：改 run row 的四个 float。纯色文本的字形不带自己的颜色，它们继承 run 行；
  只有 rich span 里与段落色不同的字形才带，改色也只动它自己那一块。
- **淡入淡出**：run row 的 `opacity`，着色器里乘在 alpha 上。**不再**折进颜色，所以
  rich text 淡入不会因为 span 颜色变了而重新塑形——这是 Phase 6 遗留的一个真缺陷。
- **旋转 / 透视**：presentation row 的一行。四个角的单应变换搬进了顶点着色器。

### 一条管线，不再是两条

Phase 6 的 Axis / Affine 两条管线合成了一条。差别缩成 run row 里的两个 flag：
角点要不要过单应变换、采样是 nearest 还是 linear。旋转文字因此也是每字形一个
24 字节 instance、四个顶点，而不是六个顶点各带一份变换和裁剪。

顺带修好的：旋转 / 缩放文字过去不吃 `clip-path: polygon()` 和 `circle()`——affine 着色器
把多边形数硬编成 0。现在多边形跟着 presentation row 一起进 GPU，文字和它旁边的 Quad
被同一个形状裁。

### arena：画序与存储序不是一回事

instance 在 GPU 上的位置由 `InstanceArena` 发，一块一直归它的 entry 所有：

- 一段文本变了，动的只有它自己那几百字节；后面的段落不搬家。
- 块按 size class 留 1/8 的余量（至少 4 个槽），余量填成尺寸为 0 的 instance。
  打字打到第十二个字符不会越级，也就不会让它后面的每一段都重传一遍。
- 相邻的两块合成一个 draw——余量在中间也没关系，它画不出像素。不相邻就多一个 draw。
- 攒够 8 个「多出来的 draw」，或者空洞占了一半，就按画序整理一次（`repack`），
  下一帧又是一个 draw。整理会推进 arena 代际，跨帧保留的 draw command 因此重建。

run 行号同理是**每个 entry 一个固定槽**，不是画序下标。一个列表滚掉第一行，
留下的十一行不会因为「都往前挪了一位」而把每个 instance 重写一遍。

### in-flight

`queue.write_buffer` 在调用时就把字节拷进 queue 自己的 staging，传输排在下一次提交之前，
所以 painter 不需要自己的 ring——它既不拥有 submit 也不拥有 surface。缓冲区换掉时，
wgpu 自己保证旧的活到引用它的命令完成为止；这一侧要保证的是**没有句柄还指着旧布局**，
那是 arena 代际和 `GlyphAtlasEntryId` 的代际两道闸。

atlas 槽位多了一个引用计数：一个 entry 还在画的字形，不会因为「这一帧没人查过它」
就被当成冷的淘汰掉。淘汰仍然可以拿它——上限是硬的——只是排在没人引用的后面。

### 计数器

`text_glyph_counters()` 在 Phase 6 那一批之外新增：

```text
text_gpu_entries_active / created / destroyed / reused
text_gpu_entry_glyphs
text_instance_rebuilds        // 重新解析了一段文本
text_instance_patches         // 只改了矩形或 run 行号
text_instance_upload_bytes
text_presentation_upload_bytes
text_prepare_nodes_considered / skipped / culled
```

`rebuilds` 与 `upload_bytes` 是这一期的验收面：静止帧两个都必须是 0。

### 基准

`nana-text-paint-benchmark`（同机 Apple M4，release）。每一格都是**会重建批次**的帧——
静止那一格靠一个每帧换文本的小节点把批次缓存打掉，因为那正是产品里的情形：
角落里有个东西在动，旁边的字要不要重算。

**工作计数**，一万个标签（48 431 个屏上字形；`static-unique` 是 68 892 个）：

| workload | resolve/帧 | rebuild/帧 | instance 字节/帧 | 重塑形/帧 |
| --- | ---: | ---: | ---: | ---: |
| static | 58 432 → **2** | — → 1 | — → 144 | 0 → 0 |
| static-unique | 78 892 → **2** | — → 1 | — → 144 | 10 001 → **0** |
| color | 58 431 → **0** | — → 0 | — → **0** | 0 → 0 |
| opacity | 58 431 → **0** | — → 0 | — → **0** | 0 → 0 |
| transform | 58 431 → **0** | — → 0 | — → 550 509 | 0 → 0 |
| mutate 1% | 58 548 → **697** | — → 100 | — → 65 302 | 1 → 1 |

`static` 那 2 次是每帧真的换了文本的那个小节点——批次缓存正是被它打掉的。屏上另外
一万个标签一个字形都没有重新解析。动画三格 60 / 120 / 240 Hz 的每帧数字一致
（表里只列一次），因为这里没有任何一项是按时间摊的：改一次颜色就是改一行。

**耗时**用一千个标签那一档（4 842 个字形），同机 Apple M4、release、五次取中位，
彼此相差不超过 3%：

| workload | batch p50（前 → 后）|
| --- | --- |
| static | 0.765 → **0.322** ms |
| static-unique | 11.979 → **0.330** ms |
| color | 0.760 → **0.320** ms |
| opacity | 1.100 → **0.552** ms |
| transform | 2.054 → **0.514** ms |
| mutate 1% | 0.780 → **0.344** ms |

一万标签那一档的**毫秒不引用**：那个视口是 4400×4400，帧与帧之间受内存带宽和调度
影响，同一份二进制连跑三次可以是 13.2 / 25.0 / 14.5 ms。计数是稳定的，判定用计数。

读法：

- **`color` / `opacity` / `transform` 的 instance 字节是 0**（transform 在一万标签那格
  不是，见下），三者都只写表。颜色写 run 行，不透明度走 opacity group，旋转写
  presentation 行。
- **`transform` 一万标签那格还有半兆字节**：容器在转，每帧有几十个标签转进转出视口，
  arena 里它们的块就不再挨着，攒够预算就整理一次。整理是一次连续写，不是重新解析——
  同一格的 `resolve/帧` 和 `rebuild/帧` 都是 0。
- **`static-unique` 的 12 ms → 0.33 ms 不是保留期换来的**，是塑形缓存的淘汰改成
  「最近两帧问过的不淘汰」：一屏里不重复的段落多过缓存容量时，它过去会为了给下一段
  腾地方而挤掉刚画过的那一段，然后每帧把整屏重新塑形一遍。
- **后一半的加速不在文本路径上**。把 `prepare` 分段短路量出来（同机，一万标签，
  static，三次取中位）：

  | 量到哪一步 | 保留期落地时 | 现在 |
  | --- | --- | --- |
  | `prepare` 直接返回（painter 自己的逐节点循环） | 6.75 ms | **4.65 ms** |
  | 加上塑形键的构造与哈希 | 7.12 ms | 5.40 ms |
  | 全程 | 8.74 ms | 6.80 ms |

  painter 的逐节点循环原来占七成，而且里面有三样东西是每个图元每帧都在做的无用功：
  三份候选裁剪盒建了再丢、祖先链的环路保护每次都建一个 `HashSet`、以及没有
  compositor layer 的场景照样沿父链走一遍。这三条都修了，循环本身 6.75 → 4.65 ms。
  文本自己剩下约 2 ms，其中最大的一项是塑形键哈希——#99 把 `nana-text` 的 layout
  句柄接上来之后，那一项可以换成一次代际比较。

  循环里原来还有每个图元各自一趟的祖先链查询——`opacity_groups` 每帧问两遍（绘制
  一遍、剔除一遍），`compositor_paint_opacity` 在 `draw_primitive` 里问一遍。这两条
  的代价随**树深**涨，而一个真实 shell 的文字坐在十到二十层元素之下。现在按节点
  记忆祖先那一半（不是 group 的节点共用父节点那一份 `Arc`，所以不分配）：

  | 包裹层数（`--depth`） | 前 | 后 |
  | ---: | --- | --- |
  | 0 | 0.319 ms | 0.307 ms |
  | 4 | 0.410 ms | 0.308 ms |
  | 8 | 0.483 ms | 0.311 ms |
  | 16 | 0.631 ms | **0.303 ms** |

  原来每多八层多两成，现在是平的。表里其余各格都是 `--depth 0`，也就是最扁的那棵
  树——真实界面的收益比表里的大。

- **样式动一次的账不在 painter 里**。`batch p50` 只是批次阶段；一个容器改样式的帧
  里，时间在 `RuntimeDocument::flush`——一千个标签、十六层包裹是 4.4 ms 对 0.96 ms。
  基准因此多了一列 `flush p50`。

  那笔钱是这样花的：容器改一次样式，它下面每个节点的图元都要重建，而每个节点在这
  一趟里各走两遍父链——一遍算继承的变换 / 不透明度 / 裁剪，一遍算父链把它放在绘制
  序的哪里（移除一次、插入再一次）。兄弟节点这两样是一模一样的，而这一趟正好按顺序
  走子树，所以各留一个位置记住上一次算的是谁的：

  | 包裹层数 | color | opacity | transform |
  | ---: | --- | --- | --- |
  | 0 | 1.94 → 1.78 ms | 2.29 → 2.00 ms | 2.99 → 2.65 ms |
  | 8 | 3.04 → 2.33 ms | 3.67 → 2.87 ms | 3.16 → 2.34 ms |
  | 16 | 3.71 → **2.64** ms | 4.53 → **3.55** ms | 5.05 → **3.44** ms |

- **场景里的节点改成 `Arc<ExtractedNode>`**。`rebuild_node_primitives` 借不出那个
  节点——后面要 `&mut self` 去插图元——所以它一直是整份克隆，而 `ExtractedNode` 有
  784 字节。容器改一次样式要重建每个后代，那就是每帧近一兆的 memmove。存成 `Arc`
  之后那份克隆是一次引用计数加一；插入那一侧多的是一次分配，但一帧只插改动过的那
  几个节点，重建的却是整棵子树。`flush p50`（各跑五轮取最小，前后交错跑）：

  | 用例 | 前 | 后 |
  | --- | --- | --- |
  | 一千标签 color，扁 | 1.616 ms | 1.568 ms |
  | 一千标签 opacity，扁 | 1.737 ms | 1.706 ms |
  | 一千标签 transform，扁 | 1.868 ms | 1.812 ms |
  | 一千标签 opacity，十六层 | 3.124 ms | 3.031 ms |
  | 一千标签 transform，十六层 | 2.904 ms | 2.790 ms |
  | 一万标签 opacity，扁 | 29.222 ms | **26.843 ms** |

  节点越多省得越多，因为省掉的是按字节算的那一项。渲染出来的 557 张画廊帧逐字节不变
  ——这一条只动存储。

- **淡入淡出不再重排整个场景**。绘制序只问一个节点「是不是半透明」——
  `is_opacity_group` 看的是 `opacity > 0 && opacity < 1`，不是那个值。但判断要不要
  重排时比的是 `local_opacity` 的 bits，于是一个容器从 0.35 淡到 0.37，每帧都要把
  全场景的图元重新算一遍键、重新排一遍序。改成比 `paint_order_facts`（z-index、
  是否半透明、非 identity 的 filter、mix-blend、是否开层叠上下文）之后：一千标签
  opacity 1.911 → 1.693 ms，十六层包裹下 3.338 → 2.725 ms，一万标签 29.162 →
  25.595 ms。

- **图元记住自己在绘制序里的位置，重建就地覆写**。原来重建一个节点的图元要对每个
  图元做四次 B 树操作——从 `primitives` 里删掉再插回去，从 `ordered` 里删掉再插回去
  ——而那两个键各要走一遍父链，删的那一次算完就扔。绝大多数时候删掉和插回去的是同
  一个图元、同一个键。

  现在图元和它的 `SceneOrderKey` 存在一起：删除直接用存着的键；重建不先清空，
  `insert_primitive` 就地覆写，键没变就不动 `ordered`；每次重建给写过的槽盖一个戳，
  没盖到的槽就是这个节点不再有的图元，收尾时退掉。`apply_delta` 里那句「先把抽取到
  的节点的图元全删掉」也随之去掉。绘制序的 stack 换成 `Arc<[(i32, usize)]>`，一个
  节点的所有图元共用一份。

  | 用例 | 前 | 后 |
  | --- | --- | --- |
  | 一千标签 color，扁 | 1.574 ms | 1.155 ms |
  | 一千标签 opacity，扁 | 1.549 ms | 1.101 ms |
  | 一千标签 transform，扁 | 1.822 ms | 1.411 ms |
  | 一千标签 opacity，十六层 | 2.452 ms | 1.643 ms |
  | 一万标签 mutate-1pct | 0.324 ms | 0.240 ms |
  | 一万标签 opacity，扁 | 23.664 ms | **18.720 ms** |
  | 一万标签 transform，扁 | 27.532 ms | 23.178 ms |

- **淡入淡出不再重新抽取整棵子树**。容器改一次不透明度，运行时原来把整棵子树标成
  `STYLE | RENDER`（`inherited_paint_changed` 把 opacity 和继承的颜色算作一类），
  一百个标签的用例里 `render_extraction` 是 102 个节点。可是容器只要有后代可继承
  就是 opacity group，那一份不透明度在合成那层应用一次，不会落到后代图元上
  （`compute_ancestor_state` 里 `if !is_opacity_group(ancestor)` 才乘）——那一百个
  后代重新抽取、重新建出来的图元逐字节相同。

  `ComputedStyle::opacity` 在整个工作区里只有三处读它（它自己的 `Default`、算它的
  `resolve_style`、把它归成 `TextWork::COMPOSITOR` 的文本脏位分类），渲染那一侧一
  处也没有。所以不透明度现在只标 `STYLE`：后代照样解析出新的累积值，但不再抽取。

  这一步之前先补了场景层的一个隐患：祖先的不透明度是后代**烘进图元**的唯一一样
  东西（变换和裁剪绘制时会重推），过去全靠运行时把子树标脏兜着。现在场景自己用
  `inherited_opacity`（是 group 就是 1.0）判断这个数变没变，变了就重建保留期后代。
  顺带把 `attribute_epoch` 的那一格也让开：推它是为了让后代重推变换和裁剪，一次
  不改变 group 身份的淡入淡出什么几何都没动。

  | 用例 | flush 前 | flush 后 | batch 前 | batch 后 |
  | --- | --- | --- | --- | --- |
  | 一千标签 opacity，扁 | 1.212 ms | 0.117 ms | 0.474 ms | 0.314 ms |
  | 一千标签 opacity，十六层 | 1.914 ms | **0.124 ms** | 0.854 ms | **0.312 ms** |
  | 一万标签 opacity，扁 | 20.861 ms | 3.907 ms | 9.449 ms | 6.126 ms |

- **改变换也不再重新抽取整棵子树**。同一个形状：后代的图元建在自己的空间里，由上面
  那条链投影过去，而那条投影绘制时本来就会重推——`projections` 记着建图元那一刻的
  投影，`draw_primitive` 用当前投影和它的逆求出一个 delta，补到图元的变换和它自己
  那几个裁剪上（保留期后代跟着祖先滚动走的就是这条路）。所以变换现在只给子树标
  `TRANSFORM | INPUT`，`RENDER` 只给节点自己；投影/奇异变换那批仍由
  `unadjustable_projections` 单独排进重建。

  等价性有测试钉着：一棵四层的树（hidden overflow 裁剪 + 滚动偏移 + opacity group +
  后代自己的变换），只抽取容器和整棵重建两种走法逐个图元比绘制出来的变换、裁剪和
  不透明度，差在 1e-4 以内——重推是拿一次逆去凑，重建是把链重新乘一遍；误差不累积，
  每帧的 delta 都从同一个记下来的 base 推。

  代价挪到了 batch：后代每帧都走重推那条路，所以那条路本身也收了两处——祖先状态按
  父节点记一个位置（绘制侧没有括号来清它，按 `(instance, attribute_epoch)` 盖戳），
  图元自己没有裁剪时直接共用祖先那一份 `Arc`。

  | 用例 | flush 前 | flush 后 | batch 前 | batch 后 |
  | --- | --- | --- | --- | --- |
  | 一千标签 transform | 1.191 ms | 0.388 ms | 0.446 ms | 0.544 ms |
  | 一万标签 transform | 21.417 ms | **7.718 ms** | 7.541 ms | 9.487 ms |

  一帧总账：一千标签 1.64 → 0.93 ms，一万标签 28.96 → 17.21 ms。

- **动了的子树自己刷新可见性索引**。采样说变换动画里 paint 的 48% 花在
  `VisibilityIndex::new` 上：祖先的几何一变，索引就被整个丢掉，下一次查询从头建一遍
  ——文档里每个图元重算一次投影包围盒，每个节点重新哈希一遍，每条父链重新走一遍去
  重建子树区间。可是祖先的变换改的是子树落在哪里，不是子树里有哪些图元。

  现在按 `inherited_roots` 刷新：`descendants` 记着每个节点子树占的操作区间，
  `refresh_range` 走进那些区间把叶子重算、往上重新求并，代价 O(区间 + log n)。下行
  时要把滚动留下的懒平移推下去，否则它会叠到刚算好的叶子上（`moving_one_subtree_...`
  就是钉这一条的，故障注入能打红）。

  为此基准加了一个 `transform-panel`：转一个装八个标签的面板，其余上万个图元不动
  ——真实 shell 的动画是这个形状，而 `transform` 转的是整个文档，是最坏情况。
  一帧总账（flush + batch）：

  | 用例 | 前 | 后 |
  | --- | --- | --- |
  | transform-panel，一千标签 | 0.512 ms | 0.312 ms |
  | transform-panel，一万标签 | 9.933 ms | **5.698 ms** |
  | transform，一万标签（最坏） | 19.733 ms | 17.224 ms |

- **按节点索引的表换成整数哈希**。标准库默认的 SipHash 是带密钥的 MAC：对键来自外部
  输入的表是对的默认，对一个进程外没人能选的计数器是白付。绘制一个图元要问好几张这样
  的表（`projections`、`draw_attributes`、`nodes`、两个祖先记忆……），采样里光哈希就
  占 paint 的 9.7%。现在它们都是 `NodeMap` / `NodeSet`，混合那步用 rustc 的
  「转、异或、乘奇数」。

  这改变了这些表的遍历顺序，所以「557 张画廊帧逐字节不变」在这一条里是承重的：没有
  哪一处渲染结果依赖过它们的顺序。

  数字这次给不准：测的时候本机 load 接近 10，同一个二进制在两轮之间能差到一倍。九轮
  配对取比值中位数——transform-panel 一万标签 -13.3%，transform 一万标签 -6.1%，
  color 一千标签 -14.0%，九轮里大多数轮次都在 1.0 以下。采样给出的上限是 9.7%，两者
  量级一致。

- **没变过的段落不再每帧重造一次 shape key**。一万个在屏标签的稳态帧里，采样说最大的
  一格是 `TextPipeline::prepare`（自用时间 2315，整帧约七千）：每个标签每帧都要把
  `ShapeKeyRef` 的二十来个字段装起来、连同整个字符串哈希一遍，再拿它去塑形缓存里比一
  次，比出来的结论永远是「和上一帧一样」。

  现在场景在 `SceneDraw` 上多给一个 `revision`——写这个图元的那次重建。它没变，加上
  设备缩放和字体集都没动，shape key 就不可能变，直接用 entry 记着的那个哈希。富文本
  不走这条（它的 key 带着按颜色切的 span，而调用方可以覆盖那个颜色）。

  `revision` 从**进程内**全局计数器取。按每场景计数是不行的：画笔按 (node, slot, pass)
  认 entry，里面没有一样东西说这是哪个场景的。按场景计数那一版所有测试都是绿的，是
  557 张画廊帧里的 258 张把它抓出来的。

  一帧总账（九轮配对取比值中位数）：static 一万标签 -11.4%，mutate-1pct -9.4%，
  transform-panel -6.2%。

  顺带修了两件事。一是 `transform-panel` 的面板当时用 `Fill` 尺寸，把其余 9992 个标签
  挤出了视口（`text_prepare_nodes_culled` 9999.9 / 10001），那量的是一份没人画的文档；
  修好之后 9998 个被自己的 entry 答掉。二是 #8 的文本门禁在真机上从来没被判定过——
  `text_counters` 挂在报告上的时机在不变量算完之后，五条全是 `not-evaluable`。现在
  五条全 ok，其中 `text_instance_rebuilds=1`。

- **稳态帧上又刮掉两层**。段落的宽高（`measure` 要走一遍 layout run）记在 entry 上，
  因为那是形状的宽高而形状已经由 `revision` 钉住；presentation 行的「和上一行一样」
  改成先比那四个来源值，而不是先造出一百六十字节的行再哈希成四十个字。一帧总账：
  static 一万标签 -4.7%，transform-panel -12.3%，mutate-1pct -6.7%。

  画笔自己那两张热表（`EntryStore::index`、塑形缓存）也换成 `IdHasher`——采样里
  painter 的 SipHash 有 206/213 出自 `prepare`，而塑形缓存的键本来就是一个哈希。
  三个用例一致 -5%，九轮里每轮都在 0.94–0.97。

  **两条量完之后撤回的**，记在这里免得下次再试一遍：

  - 把 `FragmentClip` 的 `PartialEq` 改成只比 `polygon_count` 以内的顶点。采样把
    `FragmentClip::eq` 排到 paint 的 11%，改完十一轮配对的比值中位数是 ±0.4%——
    那十六个浮点本来就是零且被向量化了。符号级自用时间在这个粒度上会骗人。
  - 「一个 run 的输入都没变就还是上一帧那个 run」：把 ink、原点、flags 和剔除结论
    一起记在 entry 上。七个失效输入里故障注入证明六个是承重的，而量出来只有 1–2%
    （第一版把整个 `FragmentClip` 放进键里，反而慢 3–4%）。一个这么精细的缓存换
    一两个点，不值。

- **「每个节点都改了」的那一帧**（用例 `color`：一万个标签各自换前景色，是 flush 占
  大头的形状）。采样指向两处，都和颜色无关：

  - **结构变没变，事后对账改成当场记账**。帧计划和剔除索引建立在「有哪些图元、各自
    绑着什么」之上；原来判断它变没变，是进重建循环前给每个碰到的节点拍一张图元清单
    快照（一次区间扫描加一次 `Vec` 分配），重建完再拍一次逐个比——一万个节点就是两万
    次扫描加两万次分配，只为一个布尔值。可是添、删、重新绑定就发生在那一趟里，
    `insert_primitive` 和 `retire_node_primitives` 自己知道。-15.6%。
  - **可见性索引的 `update` 按连续段刷新**。原来每个变了的图元从根走一次十四层的
    `set_bound`。一个节点的图元在绘制序里是连着的，一棵子树的节点也是，所以一次
    delta 变的是少数几段连续区间；复用「动了的子树自己刷新」那条 `refresh_range`
    即可，不需要阈值：段数多时每段 O(区间 + log n)，全变时就是一段 O(n)。-8.2%。

  一万标签 color 一帧总账 15.949 → 12.164 ms（**-24%**）。三处 `structure_changed`
  的置位都做了故障注入：插新键那处本来就有测试盯着，退役和重新绑定两处当时没有，
  分别补在 `a_rebuild_that_drops_...`（要先让帧计划编译出来）和
  `frame_plan_survives_...` 里。

  **又一条量完之后撤回的**：把 `refresh_range` 从递归改成扁平的两趟（推边界路径、
  写叶子、自底向上求并）。采样把它排在第一位，但改完只有 -1.3%——递归不是成本，
  `primitive_bounds`（也就是每个图元一次 `draw_primitive`）才是，两个版本都付。为了
  一个点让两个走同一棵树的函数用两种写法，不值。

  还剩的：整数哈希那一换在 `nana-ui-runtime` 里还没做（`store.rs` 23 处、
  `framework.rs` 18 处、`layout_engine.rs` 16 处……）。而逐图元那条路
  （`draw_primitive` → `primitive_bounds`）现在是 flush 和 batch 两侧共同的大头：
  同一个被改祖先下面所有后代的 delta 数学上是同一个（`A_new ∘ L ∘ L⁻¹ ∘ A_old⁻¹`），
  按祖先记一份就够，但那要先证明「下面那段链没动」。再往下就是把整帧的绘制列表保留
  下来（damage 跟踪），那比上面任何一条都大。

### 怎么跑，怎么判

```bash
cargo run --release --locked -p nana-ui --features gpu \
    --bin nana-text-paint-benchmark -- --output target/performance/issue98/text-paint.json

# #8 门禁那一行：一千个标签，其中一个每帧换文本
python3 perf/runners/nana/run.py --scenario gpu-scene-text-retained \
    --output target/performance/issue98/nana-text-retained.json
python3 perf/contract.py --self-test
```

`perf/scenarios/gpu-scene-text-retained.json` 的 `params.text_ticker` 是这条门禁能成立的
前提：不换文本的话 painter 直接复用上一帧的批次，那一帧什么都没做，counter 全是 0，
门禁也就永远不会红。extractor 会核对报告里回显的 `text_ticker`，跑了不动的场景不算数。

五条判据各自独立成立（`retained_text_tests.py` 逐条打脸验证）：
`text_instance_rebuilds ≤ 1`、`glyph_rasterized ≤ 4`、`glyph_upload_bytes ≤ 4096`、
`text_instance_upload_bytes ≤ 4096`、`text_prepare_nodes_skipped ≥ 900`。

### 与 #97 的像素差

同一台机器上，component-gallery 的 561 帧里 17 帧变了，全部是**每通道 ≤ 2/255、
至多 619 个像素**，并且全在透明度动画或变换动画的文字上：

- 不透明度不再折进颜色再量化成 8 位，而是以 f32 乘在着色器里的 alpha 上。一条从 0
  淡入的文字过去要等 alpha 越过 1/255 才出现。
- 旋转 / 透视的四个角改在顶点着色器里算。同一套公式，f32 的最后一位可能不同。

另外两处是**行为修正**，不是噪声：

- 圆角 / 多边形裁剪下的文字过去不吃 `clip-path: polygon()` 和 `circle()`——旧的 affine
  着色器把多边形数硬编成 0。
- 只有平移但带圆角裁剪的文字过去不做像素对齐（它走的是 affine 路径，栅格相位对应
  变换前的位置）。现在它是一条平移 run，和它旁边的文字一样对齐到整像素。

### 这一阶段没做的

| 项 | 状态 |
| --- | --- |
| 亚像素移动的零重建 | 移动不到整像素时字形的栅格相位真的变了，位图就是不一样的。要零重建只能量化相位，那会改现有渲染，这一期没有理由改 |
| 行级裁剪 | entry 与视口无关，所以一段超长不换行的文字现在把整段 instance 都交给 scissor 去裁。段落级的裁剪在 `prepare` 里按 ink 做 |
| 逻辑 layout 与 raster scale 解耦 | 仍是 #99。DPI 变化换 shape key，也就换 entry |
| 每节点的固定开销 | 一万个文本节点的帧里 6.75 ms 是 painter 自己的逐节点循环（见上表），与文本无关，quad / icon 同样付。文本自己剩 2.0 ms，其中 0.37 ms 是塑形键哈希——#99 把 `nana-text` 的 layout 句柄接上来之后，那一项可以换成一次代际比较 |

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

Phase 0 落地后复测（Windows，2026-09-16，另一台机器，p50；三格都是 `text_shaped == 0`、
`cache_lookups == 0`、`skipped_unchanged` 等于非空文本节点数）：

| 节点 | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.117 ms | 3.11 ms | 3.8% |
| 4,002 | 0.593 ms | 8.55 ms | 6.9% |
| 8,002 | 0.814 ms | 18.55 ms | 4.4% |

Phase 2 落地后复测（同一台 Windows 机器，2026-09-16，p50；三格仍是 `text_shaped == 0`——
产品路径没有接入 shaper，这一行确认的是没有回归）：

| 节点 | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.190 ms | 4.42 ms | 4.3% |
| 4,002 | 0.423 ms | 9.70 ms | 4.4% |
| 8,002 | 0.835 ms | 20.05 ms | 4.2% |

Phase 3 落地后复测（Linux 容器，4 vCPU Xeon @ 2.80 GHz，2026-09-16，p50；三格仍是
`text_shaped == 0` / `cache_lookups == 0`）：

| 节点 | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.242 ms | 4.421 ms | 5.5% |
| 4,002 | 0.743 ms | 13.064 ms | 5.7% |
| 8,002 | 1.609 ms | 28.520 ms | 5.6% |

这一轮换了机器（前三轮都在同一台 Windows 上），所以只在**本轮内部**看倍率，不与 Windows 那几轮比
绝对值。`nana-ui-scene` 的依赖图里没有 `nana-text`（`cargo tree -p nana-ui-scene --edges normal`
数 0），基准二进制因此不可能因本阶段改动而改变；这三行是这台机器上的新基线，下一阶段在同一台机器上
复测才有可比性。

Phase 4 落地前后同机对比（Apple M4，10 核，macOS，2026-09-17，p50，150 samples / 30 warmup）。
这一轮换了机器，只在本轮内部比：

| 节点 | 前 TextShape | 前 Layout | 前 倍率 | 后 TextShape | 后 Layout | 后 倍率 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2,002 | 0.038 ms | 1.118 ms | 3.4% | 0.017 ms | 1.083 ms | 1.5% |
| 4,002 | 0.119 ms | 3.055 ms | 3.9% | 0.060 ms | 2.590 ms | 2.3% |
| 8,002 | 0.300 ms | 6.610 ms | 4.5% | 0.126 ms | 6.136 ms | 2.1% |

「后」三格的 `text_work` 每帧都是 `text_nodes_considered == text_nodes_revision_skipped`
（1000 / 2000 / 4000），`text_nodes_shaped`、`text_source_clones`、`text_bytes_hashed`、
`shape_cache_lookup`、`layout_cache_lookup`、`layouts_created` 全为 0；`--engine nana-text`
（每个标签保留真实 `TextLayout`）三格为 0.016 / 0.062 / 0.131 ms，计数器相同。

剩下的增长是候选扫描本身：每个候选一次侧表查找，节点翻倍时它跟 Layout 一样随工作集出 cache
略超线性，但相对 Layout 的倍率已经基本持平（2.3% / 2.1%）。改动前的额外部分来自每个候选都要读整条
`NodeRecord`、查 visual、解引用计算样式、算约束，才能决定跳过。

Phase 5 落地前后同机对比（同一台 Apple M4，macOS，2026-09-17，p50，150 samples / 30 warmup）。
这一格工作负载里没有编辑器，Phase 5 预期不改变它；「前」是改动前同一棵树的构建：

| 节点 | 前 TextShape | 前 Layout | 前 倍率 | 后 TextShape | 后 Layout | 后 倍率 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2,002 | 0.050 ms | 2.891 ms | 1.7% | 0.040 ms | 1.529 ms | 2.6% |
| 4,002 | 0.078 ms | 4.951 ms | 1.6% | 0.074 ms | 4.609 ms | 1.6% |
| 8,002 | 0.162 ms | 7.942 ms | 2.0% | 0.140 ms | 7.180 ms | 2.0% |

TextShape 绝对值前后持平，2k 一格倍率的变化来自 Layout 本身在这台机器上的抖动（同一构建连跑两次
Layout 在 1.25–1.53 ms 之间），不是文本工作；三格 `text_work` 仍是全部候选凭 revision 跳过、
`layouts_created == 0`。

#96 收尾的一轮编辑器性能改动之后复测（同一台 Apple M4，2026-09-18，p50，150 samples /
30 warmup）。这一格里没有编辑器，改动只经过共享的 shape 路径（`CountingShaper` 多问一次
`retains_measurement`），确认没有回归：

| 节点 | TextShape | Layout | TextShape / Layout |
| ---: | ---: | ---: | ---: |
| 2,002 | 0.014 ms | 0.963 ms | 1.4% |
| 4,002 | 0.037 ms | 2.155 ms | 1.7% |
| 8,002 | 0.122 ms | 5.595 ms | 2.2% |

（这一轮 Layout 的绝对值比上一轮低不少——同一台机器不同时间的负载差别，只看倍率。）

规则：**`nana-text` 每落一个阶段，重跑这三格，把数字贴回本表，并说明是哪台机器。
`TextShape` 相对同一轮 `Layout` 的倍率不得变差。** 这不是时间门禁，是人工对比——
接进 `perf/` 合同需要新的 scenario `kind`、extractor 和 fixture，等真有引擎可测再做。

## 大文档编辑基准（#96）

`nana-text-edit-benchmark`：一个聚焦的 `TextArea`，按行数扫（每行 ~38 B），每个格子测**同一次交互**
的四个时间，就是为了回答「存储要不要换 rope」这个 #96 留给真实数字的问题：

- `input_ms`——事件本身（`replace_focused_text` / `move_focused_text_caret` / 指针按下）。批次外的
  探针与值克隆都在这里；
- `flush_ms`——紧随其后的那一帧（`RuntimeDocument::flush`），并按 `FrameStage` 拆开；
- `storage_ms`——同一次编辑打在一个裸 `EditableText` 上：只有存储，没有布局、没有 presentation、
  没有帧。**这就是 rope 能改善的那个数**；
- `value_clone_ms`——一次整值 `String::clone`，作为「O(文档) 一遍」的单位。

打字是 insert/backspace 成对跑的（只记其中一半），否则一长串样本会把被打的那一行越打越长，
后面的样本测的就不是格子声称的文档了。

复现：

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-text-edit-benchmark
./target/release/nana-text-edit-benchmark --action type  --position head --samples 80 --warmup 20
./target/release/nana-text-edit-benchmark --action caret --position head --samples 80 --warmup 20
```

首轮（Apple M4，macOS，2026-09-18，release，p50，80 samples / 20 warmup，`--position head`）：

| 行 | 字节 | type input | type flush | 其中 TextShape | storage | 整值 clone |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 9.4 KB | 0.008 ms | 0.035 ms | 0.026 ms | 0.0002 ms | 0.0002 ms |
| 1,000 | 37.9 KB | 0.021 ms | 0.105 ms | 0.093 ms | 0.0006 ms | 0.0005 ms |
| 4,000 | 154.9 KB | 0.081 ms | 0.388 ms | 0.363 ms | 0.0025 ms | 0.0025 ms |
| 8,000 | 310.9 KB | 0.166 ms | 0.762 ms | 0.714 ms | 0.0050 ms | 0.0055 ms |

纯光标移动（不改文本，`--action caret`）同机同轮：250 / 1,000 / 4,000 / 8,000 行的 flush 为
0.013 / 0.035 / 0.113 / 0.222 ms，`layouts_created == 0`、`editable_mutations == 0`。

**结论：不换存储。** 310 KB 文档上敲一个字符 ~0.93 ms，其中 `String` 的 memmove 是 0.005 ms
（0.5%），跟一次整值克隆同阶；把它换成 rope 最多省下这 0.5%，而同一次按键里另有 ~180 倍于它的
O(文档) 工作。真正的缺口是那些工作，`--action caret` 那一行说得更清楚：**一次不改文本的光标移动，
在 310 KB 文档上也要 0.22 ms**，而它欠的工作是零。

profile（`/usr/bin/sample`，8,000 行）定位到的按帧 O(文档) 项，按大小排：

| 项 | 8,000 行上的量级 | 怎么修掉的 |
| --- | ---: | --- |
| `metrics_of_geometry` 累加全部段落 | ~0.05 ms / 帧 | 宿主按编辑器缓存这份测量，`GeometrySync::unchanged` 决定能不能留——`paragraphs_laid_out == 0` 不等于几何没动（删掉首段那个换行会合并掉一个段落且不排版任何段落） |
| `bracket_pair_colorization` 整文档栈扫描 | ~0.37 ms / 编辑帧 | 着色只是括号字符序列的函数：改动区间里前后都没有括号字符时只平移偏移，缓存文本按同一处 splice 就地更新；真敲了括号才重扫。`text_shape_stats::bracket_rescans` 钉住这一点（minimap 行长表仍是整篇重扫，默认关闭） |
| `TextLayoutKey` 的整文本 SipHash | ~0.19 ms × 每帧 4 次 shape | `TextShaper::retains_measurement(id)`：宿主自己保留了该节点测量时（编辑器的段落几何），内容寻址的 Runtime layout cache 对它只有开销，直接跳过 |
| 前后缀 diff 逐字节扫描 | ~0.14 ms / 编辑 + 0.06 ms / 帧 | `changed_byte_range` 先 `==`（相等是最常见情况），再按 64 字节块 memcmp；逐字节 zip 编译不出向量化比较，而文档头部的编辑正是后缀扫描的最坏情况 |
| 为两个布尔值构建整份 presentation | ~0.09 ms / 按键 | 框架每次按键都问 `text_shape_constraints`，它却要先构建 `TextInputPresentationSource`；改为从节点直接派生 `text_input_kind` |
| 只动光标也重发整个值 | 每帧 2 次整值克隆 + 内容标脏 | 组件同步在值与附加光标都没变时改发 `SetTextSelection` |
| `EditorGeometry::sync` 的段落表重建 | ~0.09 ms / 编辑帧 | 保留的段落原地不动，只 splice 掉前后缀之间那几段并给后缀的 `start` 加长度差 |
| 无 snippet 会话也克隆编辑前的值 | 1 次整值克隆 / 按键 | 只有 snippet 会话需要它 |

同机同轮复测（Apple M4，macOS，2026-09-18，release，p50，80 samples / 20 warmup；
「后」是这一轮改动全部落地、基准自身的取样缺陷也修掉之后重跑的）：

| 行 | 字节 | type input | type flush | caret input | caret flush |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 9.4 KB | 0.008 → 0.003 ms | 0.035 → 0.013 ms | 0.002 → 0.009 ms | 0.013 → 0.010 ms |
| 1,000 | 37.9 KB | 0.021 → 0.009 ms | 0.105 → 0.031 ms | 0.006 → 0.013 ms | 0.035 → 0.023 ms |
| 4,000 | 154.9 KB | 0.081 → 0.027 ms | 0.388 → 0.086 ms | 0.019 → 0.030 ms | 0.113 → 0.067 ms |
| 8,000 | 310.9 KB | 0.166 → 0.051 ms | 0.762 → 0.168 ms | 0.036 → 0.051 ms | 0.222 → 0.126 ms |

310 KB 文档上一次按键从 0.93 ms 降到 0.22 ms。`caret input` 变大是这一轮同时接入的**视觉序
左右移动**：方向键现在要问一次几何探针（见「视觉序的左右箭头」），换来的是 BiDi 与换行处
caret 不再跳位。

还剩下的按帧 O(文档) 项（都已量到，等各自的前提）：

| 项 | 8,000 行上的量级 | 为什么还没修 |
| --- | ---: | --- |
| a11y / scene primitive / extraction 的整值克隆 | 每帧 3 次 | `TextContent.value` 是 `String`；换成 `Arc<str>` 能一次消掉这三处克隆，并给探针一个天然的身份（指针相等），但那是跨 6 个 crate 的公开 API 变更 |
| presentation 每帧重建显示文本 | 1 次整值克隆 + 一次 memcmp | 要么按 TextDirty 位缓存（漏一个失效就画出过期文本），要么等编辑路径改成传「编辑」而不是传整值 |
| 撤销日志按整值快照 | 每步 2 份整值 | 表示法是明确的既有设计（`text_history.rs` 有说明）；已按字节封顶（8 MiB/编辑器，超了丢最老的步，单步再大也保留一步），所以 310 KB 文档不再最坏占到 124 MB |

## Phase 0 明确没做的

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 分数 DPI | 覆盖 layout，**不覆盖栅格** | `TextScale` 表达到字号缩放，这已是 layout 能表达的全部。glyph 原点的物理像素对齐与亚像素分桶在 `scene_paint/text/`，完全在 IR 之外。别把 `TX-D01` 读成子像素定位保证。 |
| ellipsis | 记录 overflow，**不插入省略号字形** | cosmic 0.19 的 `Buffer` 没有 ellipsis，产品路径自己替换。`TX-W05` 断言的是 `TRUNCATED_LINES` + `ELLIPSIZED` 与截断后的行数。Phase 3 的原生引擎真的会塑形并放置 `…`，见「Layout」。 |
| IME preedit | span 应用是真的，composition 状态在 source 上 | `TextLayout` 只承载几何；`CompositionSegment` 留在 `TextSource` / `TextSpan`。`TX-E01` 断言 preedit span 确实产生了自己的 run，以及 composition 在 source 上可设可清。 |
| cluster 内部的 caret | 按字节比例插值 | 一个 glyph 可以覆盖多个源字节（连字，或多字节字符）。`caret_geometry` 先把渲染同一 cluster 的所有 cell 并成一个视觉范围——组合记号是零 advance 且与基字同 cluster，RTL 下 HarfBuzz 还会把它排在基字**前面**——再在该范围内按字节比例插值。所以 `of\|fice` 的 caret 落在 `ffi` 连字的三分之一处而不是整个连字之后，阿拉伯语带记号的 cluster 也不会塌到零宽记号上（见 `TX-B01` 的八个 caret 探针，x 随字节偏移严格递减）。落在字素内部的字节偏移本就不是合法 caret 位置，IR 没有源文本可以吸附，插值只保证单调、可区分。 |
| caret affinity（RTL / BiDi 边界） | **记录行为，不是合同** | 边界 affinity 是引擎定义而非规范定义的。Phase 0 把参照引擎的答案记成 golden 并配 `caret_x_px` 容差。 |
| 五个计数器 | 只有参照路径在喂 | 按设计没有产品生产者，靠对账测试防止空转。 |
| script 标注 | 参照引擎为 `ScriptTag::UNKNOWN` | 参照引擎不导出 per-run script。Phase 2 的 shaper 已填上（见「Shaping」）。 |
| 竖排（#59） | Phase 0 不做，且 fail-closed | Phase 3 的 layout 遇到 `vertical-*` 按横排排出并置位 `unsupported_writing_mode`。#59 之后只剩可编辑文本这样兜底，其余按列排，见「writing mode 与 #59」。 |
| 多字体 fallback | 语料里覆盖了但很窄 | 语料的 fallback 是 VF→Noto 的 `A`/`B`，证明 `FontId` 能在 run 中途变、`FALLBACK_FONT` 会置位。按 script / 语言 / emoji 驱动的候选选择由 Phase 1 字体层提供（见「字体层」），参照引擎不走它。 |
| #33 workload | 合同级保留，不是 perf 门禁 | 见上一节。 |

## 怎么跑

```bash
cargo test -p nana-text --all-targets --locked
python3 scripts/check-engine-boundary.py
python3 -m unittest discover -s scripts/tests
python3 scripts/build-text-corpus-fonts.py --check   # 需要 fonttools
# 平台系统字体验收（读本机字体，不进 CI）
cargo test -p nana-text --release --test font_system_platform_acceptance -- --ignored --nocapture
```

证明它一条边都没有（dev 边也没有）：

```bash
grep -c '^name = "cosmic-text"$' Cargo.lock   # 0
```

Phase 4 起 `nana-ui-runtime` 依赖 `nana-text`（保留文本节点与句柄）；#99 起 `nana-ui` 的测量与
绘制都经过它，`nana-ui-web-api` 的 Canvas2D 也是。全工作区的同一条断言：

```bash
cargo tree --workspace --locked --edges normal | grep -ci cosmic   # 0
```
