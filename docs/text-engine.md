# 文本引擎骨架（nana-text）

给**改 NanaUI 文本的人**。写应用不需要看这篇：`nana-text` 现在不在产品路径上。

Epic #88 要把文本能力从 `cosmic-text` / `cryoglyph` fork 上迁走。#89 是其中的 Phase 0：
先把内部合同、reference backend 和 correctness corpus 固定下来，让后续每一阶段都能对着
同一份结构化基线比较，而不是在 shaping / layout / GPU 三层同时改动时失去可比性。
#90 是 Phase 1：`nana-text` 自有的字体层——注册、代际、匹配、变体坐标与按覆盖率的 fallback，
见「字体层」一节。#91 是 Phase 2：分段、BiDi、HarfRust shaping 与 ShapeRun cache，见「Shaping」一节。

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
| 复杂文字整形（GSUB/GPOS、Arabic joining、印度系重排） | **成熟 crate**（harfrust，仅 `shaping/opentype.rs`） | 同上 |
| Unicode BiDi 算法（P–I 规则、L2 重排） | **成熟 crate**（unicode-bidi，仅 `shaping/bidi.rs`） | 同上 |
| 分段、span 规范化、fallback 重试、ShapeKey 与 ShapeRun cache、`ShapedText` | **nana-text**（`shaping` 模块） | 何时重塑形、塑形结果能被谁复用的权威 |
| Unicode 算法（BiDi、断行、字素簇、script / emoji 属性） | **成熟 crate**（unicode-bidi / unicode-linebreak / unicode-segmentation / icu_properties） | 同上 |
| 字体注册、代际、`FontId` 签发、face 匹配、fallback 策略与候选、覆盖率缓存、变体坐标解析 | **nana-text**（`font` 模块） | 缓存失效与「为什么用了这个字体」的权威；不能交给第三方 query |
| 系统字体目录扫描、name / OS/2 元数据读取 | **成熟 crate**（fontdb，仅 `font/discovery.rs`） | 不用它的 query 和 fallback |
| 轴、命名实例、彩色表、cmap 读取 | **成熟 crate**（skrifa，仅 `font/face.rs`） | 与 Phase 2 的 harfrust 0.12 同一条 read-fonts 线 |
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
   `harfrust`，就是因为模块名本身也会被这条规则扫到。

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
| script 标注 | 参照引擎为 `ScriptTag::UNKNOWN` | 参照引擎不导出 per-run script。Phase 2 的 shaper 已填上（见「Shaping」）。 |
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
