# 文本引擎骨架（nana-text）

给**改 NanaUI 文本的人**。写应用不需要看这篇：`nana-text` 现在不在产品路径上。

Epic #88 要把文本能力从 `cosmic-text` / `cryoglyph` fork 上迁走。#89 是其中的 Phase 0：
先把内部合同、reference backend 和 correctness corpus 固定下来，让后续每一阶段都能对着
同一份结构化基线比较，而不是在 shaping / layout / GPU 三层同时改动时失去可比性。
#90 是 Phase 1：`nana-text` 自有的字体层——注册、代际、匹配、变体坐标与按覆盖率的 fallback，
见「字体层」一节。#91 是 Phase 2：分段、BiDi、HarfRust shaping 与 ShapeRun cache，见「Shaping」一节。
#92 是 Phase 3：单行 Label fast path、断行、行内视觉序、行盒度量、对齐、省略号与 layout cache，
见「Layout」一节。

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
| 行布局编排（断行策略、行盒合并、对齐、省略号、layout cache） | **nana-text** | 产品语义与缓存合同，必须与 `TextConstraints` 同一套词汇 |
| 结构化 diff 与容差 | **nana-text** | 迁移验收合同，必须比 cosmic 活得久 |
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
| 字形栅格化、图集、GPU instance | 现有 cryoglyph 路径 | #89 非目标；由 Epic #88 的后续阶段接手 |

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
4. **字体层后端**（#90）：`fontdb` 只许出现在 `src/font/discovery.rs`，`skrifa` 只许出现在
   `src/font/face.rs`，`icu_properties` 只许出现在 `src/font/unicode.rs`，`read_fonts` /
   `ttf_parser` 哪都不许点名；这三个模块在 `font/mod.rs` 里不得是 `pub mod`。公开 API 因此
   不可能带出 `fontdb::ID` 之类的第三方 ID。#91 同理：`harfrust` 只许出现在
   `src/shaping/opentype.rs`，`unicode_bidi` 只许出现在 `src/shaping/bidi.rs`。模块不叫
   `harfrust`，就是因为模块名本身也会被这条规则扫到。#92 同理：`unicode_linebreak` 只许出现在
   `src/layout/breaks.rs`。

参照引擎放在 `crates/nana-text/tests/reference/`，**不是** `src/` 下的 `#[cfg(test)] mod`：
后者对 `tests/*.rs` 不可见，corpus harness 就用不上它。原生引擎落地后，删
`tests/reference/` 和 `Cargo.toml` 里的 `[dev-dependencies] cosmic-text` 两处即可，`src/` 完全
不用动。

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
优先对应字形）→ 无自身 script 时的 symbol 与 emoji 策略 → last resort。都覆盖不到记
`Missing`，渲染 primary 的 `.notdef`。

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

产品路径**仍未**接入字体层：`nana-ui` 继续用 cosmic-text 的 `FontSystem`。接入属于后续阶段。

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

### 与 cosmic 参照对账

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

产品路径**仍未**接入：UiWorld 里的 paint / transform 变更不产生 shape request 这件事，要等接缝
接上才能在产品上验证；本阶段保证的是它们根本进不了 ShapeKey。

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
没有 `max_height_px`、`writing_mode` 是横排、shaping 只报了一个段落。
fast path 只做一件事：把自己的 run 累出 advance、算一次行盒、（需要时）裁一次省略号、
出一个 `LineBox`。它不建 editor state、不扫段落结构、不找断行机会
（`line_break_candidates` 恒为 0）、不为 color / transform 变化重排（那些根本进不了
`LayoutKey`，见下）。

任一条件不成立就降级到 paragraph path——**显式换行**（shaping 报出第二个段落）、任何 wrap、
多行或零 `max_lines`、高度预算、需要回退的 writing mode。降级是可观测的：
`label_fast_paths` / `paragraph_paths` 两个计数器分别计入。

### 断行

断行机会来自 UAX #14（`unicode-linebreak`），**是否**在某个机会处断由 shaped advance 决定，
从不按码位数估算。

硬换行有两个来源，合起来正好是 UAX #14 的全部 mandatory break：段落分隔符（`\n` / `\r` /
`\r\n` / U+0085 / U+2029）由 shaping 的段落结构给出，不重新问 UAX #14；VT、FF 与
U+2028 LINE SEPARATOR 段落结构不管，由 layout 在段内切开（`breaks::FORCED_BREAKS`，
`has_forced_break` 是一次普通字符扫描，不是 UAX #14 pass，且只在**建** layout 时问一次）。
分隔符本身落在两段之间，没有行覆盖它，因此不绘制——与 `\n` 待遇相同。这份列表由
`forced_breaks_agree_with_uax14` 对着 `unicode-linebreak` 自己的数据逐码位校对，Unicode
改版新增一个也漏不掉。带 forced break 的 Label 与带 `\n` 的一样降级到 paragraph path。

| 约束 | 策略 |
| --- | --- |
| `wrap: None` | 不换行，段落即一行；`line_break_candidates == 0` |
| `wrap: Word` | 只在 UAX #14 机会处断；放不下的长词溢出（`CLIPPED_WIDTH`） |
| `wrap: WordOrGlyph` 或 `word-break: break-word` | 先按词，放不下的词再按字素簇切 |
| `wrap: Glyph`、`word-break: break-all`、`line-break: anywhere` | 每个字素簇边界都是机会 |

- **行尾空白在软换行处悬挂**：不绘制、不计入行宽、不推下一行，`LineBox::source` 也不含它
  （与参照引擎一致）。硬换行与段落末尾的空白是作者写下的内容，保留在行上，但**溢出判定
  （`CLIPPED_WIDTH` 与省略号裁切）一律按去掉行尾空白后的宽度**——断行判定本来就不数行尾空格，
  否则 `"Save "` 会在一个装得下 `"Save"` 的盒子里被裁成 `"Sa…"`。
- 容器窄到一个字素都放不下时，仍然放一个字素——否则会产生空行与死循环。
- 空文本没有 BiDi 段落，但仍然出**一行**（与 Label fast path 对同一份 source 给出的行一致）：
  空输入框也要有行盒和可落脚的 caret。
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
`nana_ui_core::text_line_box_height_px` 同一个数），span 有自己的 `line-height` 时按覆盖该
run 首字节的 span 取——与 shaper 解析 span 重叠的顺序相同，行高与塑形不会各认一个 span。

**分数 scale**：`max_width_px` / `max_height_px` 是逻辑 px，乘 `scale` 后与物理 px 的
advance 比较；layout 全程保留浮点，**不**向整数像素取整——那是 renderer / glyph 路径的事。

### 对齐

`TextConstraints::align`（`nana_ui_core::TextAlignSpec`）：`start` / `end` 跟随段落方向，
`left` / `right` 是物理方向。没有 `max_width_px` 就没有可对齐的容器，所有关键字都把行放在原点。

`justify` **明确延期**：`TextAlignSpec` 里没有这个关键字，产品也无从表达，因此不半做。

### 省略号

`constraints.ellipsis` 为真，且（**不换行**时行超宽，或被 `max_lines` / `max_height_px` 截断）时：

```text
候选行 → 预留已塑形的省略号宽度 → 在 cluster 安全边界处裁 → 发出截断行 + 省略号 run
```

- 省略号走**正常** shaping / fallback / cache：接缝把 `…` 当作普通 `TextSource` 交给 shaper，
  同一样式下 10k 个截断标签只塑形一次（测试断言 `shape_cache_misses == 2`：正文一次，省略号一次）。
- 裁切单位是 shaper 的 cluster，所以不会切开 UTF-8、字素簇或连字；ZWJ 序列要么整段留下要么整段裁掉。
- 省略号 run 的 `source` 是裁切点上的**空区间**，glyph 的 cluster 也是——它不占源文本的任何字节，
  caret、命中测试与选区因此永远不会落到它身上。
- **换行开着时，超宽的行不裁**。换行的段落只会因为一个断不开的长词而超宽，而那个词整个都在这一行上：
  裁掉它等于让这段字节从所有行里消失，`LineBox::source` 留下一个中间的洞，谁也画不出、选不中、
  命中不到，而且没有任何 flag 说得出。这种行就让它伸出去，并置 `CLIPPED_WIDTH`。
  省略号真正该做的截断——`max_lines` / `max_height_px`——照常，并且带 `TRUNCATED_LINES`。
- RTL 段落里省略号放在视觉末端（左侧）。
- 截断但没有（或没能）塑形出省略号时，只报 `TRUNCATED_LINES`，不报 `ELLIPSIZED`：没画就不声称画了。

### LayoutKey 与 cache

| 进 key | 不进 key |
| --- | --- |
| shaped runs 的**身份**（持有 `Arc<ShapedText>`，按指针比较） | widget 身份、`TextRevision` |
| 已塑形省略号的身份 | 颜色、透明度、transform、z-index、背景（`TextStyle` 本来就不带） |
| `TextKind` | |
| 全部 `TextConstraints` 字段（宽高、wrap、word-break、line-break、max-lines、ellipsis、preserve-lines、direction、align、writing-mode、tab-width、scale） | |
| 每个 run 解析后的行高、空行行高、strut | |

- key **持有** `Arc<ShapedText>` 而不是裸指针：持有才让指针可比——否则同一地址可能被另一段文本复用。
- `ConstraintsKey` 是逐字段解构写出来的，给 `TextConstraints` 加字段会在这里编译失败，而不是
  悄悄产生一个忽略该字段的缓存。
- LRU，条目数（默认 4096）与字节（默认 8 MiB）双上限；超过整个字节预算的结果照常返回、不入缓存。
  shaped 文本由 shape cache 计费，layout cache 不重复计。
- cache 还回答一个别处没有的问题：这次 miss 是**新文本**还是**同一份 shaped 换了约束**
  （`constraint_only_relayouts`）——resize 风暴要看的就是这个数。

**产品路径仍未接入**：UiWorld 里「只改颜色 / transform 的帧不产生 layout request」这件事，要等
接缝接上才能在产品上验证；本阶段保证的是它们根本进不了 `LayoutKey`——`TextStyle` 不带 paint，
`LayoutRequest` 也没有第二条通路。

### intrinsic min/max content width

`Layouter::intrinsic_widths`：`min-content` 是最宽的不可再断片段，`max-content` 是不换行时的宽度，
都按 shaped advance 与 UAX #14 机会算，与当前约束是否允许换行无关——容器正是为了决定宽度才问这两个数。
不缓存：它是一趟 advance 累加，没有宽度可以作 key。

### writing mode 与 #59

竖排 **fail-closed**：本阶段不实现 `vertical-rl` / `vertical-lr` 的 glyph orientation 与竖排字体
度量，也不自造 Unicode Vertical_Orientation 数据表。请求竖排时 layout 按横排排出，并且**明说**：
`TextLayout::unsupported_writing_mode` 置位，`vertical_writing_fallbacks` 计数。横排度量绝不
冒充竖排度量。IR 这一侧已经带上 `writing_mode` 与 `base_direction`，#59 接手时不必先拆掉一个
写死横排的假设。

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
vertical_writing_fallbacks                  竖排请求被横排兜底的次数
```

对账测试断言 `lines_created` / `runs_placed` 等于它们声称描述的 layout 的行数与 run 数，
`TextWorkCounters::glyphs_resolved` 等于 `layout.glyph_count()`。

### 引擎接缝：`NativeTextEngine`

`nana_text::NativeTextEngine` 实现 `TextEngine`，把 #90 字体层、#91 shaper、#92 layouter 装在一次调用后面：

- `preserve_lines: false` 时先把单字节的行分隔符（`\n` / `\r` / VT / FF）折成空格**再**塑形
  （`TextSource::with_folded_newlines`）——塑形与断行必须看到同一串字节；它们都是单字节，
  所有 span 范围、cluster 与 caret 偏移保持不变，revision 也保持不变（同一次编辑的另一种读法）。
  `U+2028` / `U+2029` 各三字节，折叠会挪动其后所有偏移，因此无论 `preserve_lines` 怎么写都仍是换行。
- 需要时塑形 `…`，取基础样式那张 face 的度量作 strut，填 `TextWorkCounters` 的五个口径。
- `TextEngine::layout` 返回 `Arc<TextLayout>`：layout 不可变，同一帧里同文本同约束应当拿到**同一份**，
  而不是它的拷贝。

产品文本路径**仍未**接入：`nana-ui` 继续走 cosmic-text + cryoglyph。

### 与 cosmic 参照对账

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

`tests/layout_engine.rs`：五条来自 code review 的回归（换行时超宽的行不因省略号丢字节、
行尾空白悬挂不算溢出、空文本两条路径都出一行、同一行被裁两次只记一个省略号 run、
U+2028 结束一行且不绘制），加上：Label fast path（10k 标签 `paragraph_paths == 0`、
`line_break_candidates == 0`；10k 同文本标签只建一个 layout）、显式换行 / wrap / max-lines 触发降级、
换宽度只重排不重塑形、resize 只动受影响的那一段、word wrap 不切词、长词按 `word-break` 溢出或切开、
汉字无空格断行、显式换行与空段落、mixed BiDi 单行与换行后每行各自重排、strut 稳住 baseline（以及不给
strut 时基线确实会动）、`line-height` 与 half-leading、分数 scale、六种对齐 × 方向、
max-lines 与 max-height 截断、省略号的 cluster 安全裁切与零字节占用、省略号只塑形一次、
intrinsic min/max、竖排 fail-closed、layout cache 的上限与 LRU、字体代际让旧 layout 变陈旧、
计数器与产物对账、caret / hit-test / 选区直接跑在原生 layout 上。

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

规则：**`nana-text` 每落一个阶段，重跑这三格，把数字贴回本表，并说明是哪台机器。
`TextShape` 相对同一轮 `Layout` 的倍率不得变差。** 这不是时间门禁，是人工对比——
接进 `perf/` 合同需要新的 scenario `kind`、extractor 和 fixture，等真有引擎可测再做。

## Phase 0 明确没做的

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 分数 DPI | 覆盖 layout，**不覆盖栅格** | `TextScale` 表达到字号缩放，这已是 layout 能表达的全部。glyph 原点的物理像素对齐在 `scene_paint/text.rs`，完全在 IR 之外。别把 `TX-D01` 读成子像素定位保证。 |
| ellipsis | 记录 overflow，**不插入省略号字形** | cosmic 0.19 的 `Buffer` 没有 ellipsis，产品路径自己替换。`TX-W05` 断言的是 `TRUNCATED_LINES` + `ELLIPSIZED` 与截断后的行数。Phase 3 的原生引擎真的会塑形并放置 `…`，见「Layout」。 |
| IME preedit | span 应用是真的，composition 状态在 source 上 | `TextLayout` 只承载几何；`CompositionSegment` 留在 `TextSource` / `TextSpan`。`TX-E01` 断言 preedit span 确实产生了自己的 run，以及 composition 在 source 上可设可清。 |
| cluster 内部的 caret | 按字节比例插值 | 一个 glyph 可以覆盖多个源字节（连字，或多字节字符）。`caret_geometry` 先把渲染同一 cluster 的所有 cell 并成一个视觉范围——组合记号是零 advance 且与基字同 cluster，RTL 下 HarfBuzz 还会把它排在基字**前面**——再在该范围内按字节比例插值。所以 `of\|fice` 的 caret 落在 `ffi` 连字的三分之一处而不是整个连字之后，阿拉伯语带记号的 cluster 也不会塌到零宽记号上（见 `TX-B01` 的八个 caret 探针，x 随字节偏移严格递减）。落在字素内部的字节偏移本就不是合法 caret 位置，IR 没有源文本可以吸附，插值只保证单调、可区分。 |
| caret affinity（RTL / BiDi 边界） | **记录行为，不是合同** | 边界 affinity 是引擎定义而非规范定义的。Phase 0 把参照引擎的答案记成 golden 并配 `caret_x_px` 容差。 |
| 五个计数器 | 只有参照路径在喂 | 按设计没有产品生产者，靠对账测试防止空转。 |
| script 标注 | 参照引擎为 `ScriptTag::UNKNOWN` | 参照引擎不导出 per-run script。Phase 2 的 shaper 已填上（见「Shaping」）。 |
| 竖排（#59） | 不做，且 fail-closed | Phase 3 的 layout 遇到 `vertical-*` 按横排排出并置位 `unsupported_writing_mode`，不把横排度量冒充成竖排。 |
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

证明产品路径没动：

```bash
# 注意 --edges normal：默认的 cargo tree 会把 dev 边也列出来，而参照引擎正是一条 dev 边。
cargo tree -p nana-text --locked --edges normal | grep -ci cosmic   # 0
cargo tree -p nana-ui --locked | grep -ci nana-text                 # 0
```
