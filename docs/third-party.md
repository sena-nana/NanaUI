# 第三方代码与许可证（#99 §8）

NanaUI 自己的代码是 **MIT 或 Apache-2.0**（见 [LICENSE-MIT](../LICENSE-MIT)、
[LICENSE-APACHE](../LICENSE-APACHE)）。这篇记的是文本栈换引擎之后，产品里到底
还有谁的代码、哪些是照抄来的、以及那两个 fork 现在是什么状态。

给**要发布 NanaUI 或者要过法务的人**看。写应用不需要看这篇。

## 结论

- release 依赖图（`cargo tree --edges normal`）里 **587 个外部 crate，全部是宽松
  许可证**：MIT / Apache-2.0 / BSD / ISC / Zlib / Unicode-3.0 / 0BSD / CC0 /
  BSL-1.0 / CDLA-Permissive-2.0 及其组合。没有 copyleft-only 的边（两处
  `MIT OR Apache-2.0 OR LGPL-2.1-or-later` 都可以取宽松的那一支）。
- `cryoglyph` 与 `cosmic-text` **都已不在 `Cargo.lock` 里**——不是「只剩 dev 依赖」，
  是一条边都没有。参照引擎已随之删除（见下）。
- 从被替换的引擎**照抄过一段代码**：`SubpixelBin::split`。已就地署名，见「照抄了什么」。

没有为「别让它回来」立门禁：依赖已经删了，`Cargo.lock` 里一条记录都没有。
`scripts/check-engine-boundary.py`（CI 每次跑）守的是 `nana-text` **源码**里不得出现
`cosmic_text` / `cryoglyph` / `glyphon` 标识符——那条是 #89 的 API 纯净度规则，不是
依赖删除的看门狗。

## 文本栈依赖谁的代码

`nana-text` 拥有生命周期、IR、缓存与编排，**不重写标准重型算法**；下面这些就是
那些算法的出处（版本以 `Cargo.lock` 为准）：

| crate | 版本 | 许可证 | 干什么 |
| --- | --- | --- | --- |
| `harfrust` | 0.12.0 | MIT | OpenType 整形（GSUB/GPOS）。只许出现在 `shaping/opentype.rs` |
| `skrifa` | 0.44.0 | MIT OR Apache-2.0 | 轴、命名实例、彩色表、cmap。只许出现在 `font/face.rs` |
| `read-fonts` | 0.41.0 | MIT OR Apache-2.0 | `skrifa` / `harfrust` 共用的字表读取 |
| `fontdb` | 0.24.0 | MIT | 系统字体目录扫描与 name/OS2 元数据。只许出现在 `font/discovery.rs` |
| `ttf-parser` | 0.25.1 | MIT OR Apache-2.0 | `fontdb` 的字表解析 |
| `icu_properties` | 2.3.0 | Unicode-3.0 | script / Emoji_Presentation / Default_Ignorable。只许出现在 `font/unicode.rs` |
| `unicode-bidi` | 0.3.18 | MIT OR Apache-2.0 | UBA。只许出现在 `shaping/bidi.rs` |
| `unicode-linebreak` | 0.1.5 | Apache-2.0 | UAX #14 断行机会。只许出现在 `layout/breaks.rs` |
| `unicode-segmentation` | 1.13.3 | MIT OR Apache-2.0 | 字素簇 / 词边界 |
| `swash` | 0.2.10 | Apache-2.0 OR MIT | 字形轮廓缩放与栅格化。只在 `scene_paint/text/raster.rs` 后面 |
| `zeno` | 0.3.3 | Apache-2.0 OR MIT | `swash` 的路径栅格化 |
| `yazi` | 0.2.1 | Apache-2.0 OR MIT | `swash` 的 WOFF2 解压 |
| `etagere` | 0.2.15 | MIT/Apache-2.0 | glyph atlas 的矩形打包。只在 `scene_paint/text/atlas.rs` 后面 |

「只许出现在某个文件」这一条由 `check-engine-boundary.py` 机器守着，所以这些
crate 的类型不可能泄进 `nana-text` 的公开 API。

`fontdb` 在依赖图里有 **0.23.0 与 0.24.0 两份**：0.23 是 `ratex-svg` →
`ratex-font-loader` → `ratex-unicode-font` 拉进来的（SVG / 数学公式渲染那条线），
与文本栈无关，两者互不可见。

## 那两个 fork 现在是什么状态

### `cryoglyph`（GPU text renderer）

**已从产品与 `Cargo.lock` 中完全移除。** `NanaRenderer::text`
（`crates/nana-ui/src/scene_paint/text/`）自己实现了它原来的全部职责：glyph IR、
栅格化边界、raster cache、GPU atlas、上传队列、text pipeline。fork 仓库按
#99 的说法归档 / 留 reference branch 即可，本仓库不再引用它。

它原来的许可证是 `MIT OR Apache-2.0 OR Zlib`。**本仓库没有从它照抄源码**：
`atlas.rs` 是代际句柄 + 引用计数 + 多页 + 占位页的结构，与它的 `text_atlas.rs`
不同；`text_atlas.wgsl` 用的是 presentation 表 + run 行 + mat4 投影，与它按实例
打包字段 + `screen_resolution` 的着色器不同。两边都有的
`srgb_to_linear`（0.04045 / 12.92 / 1.055 / 2.4）是 sRGB EOTF 的规范公式
（IEC 61966-2-1），不是谁的著作。共同的**设计**（mask/color 两种 atlas 页、
per-instance content type）照 #97 的定位属于「迁移期的设计参考」。

### `cosmic-text`（shaping / layout）

**已完全移除**，dev 依赖也没了。参照引擎（`crates/nana-text/tests/reference/`）、
拿它对 golden 的 `text_parity_corpus.rs`（含 `NANA_TEXT_BLESS` 重录路径）与
`reference_engine_counters_*.rs` 一并删除；`src/` 一行未动——当初把它放进 `tests/`
就是为了这一刻。

留下来的是 **golden**：`crates/nana-text/corpus/golden/TX-*.layout.json` 是 Phase 0
由参照引擎录下的答案，现在由两个拿**原生**引擎对着它们跑的用例守着，覆盖面没变。
golden 因此是冻结的迁移证据，重录需要先决定新基线代表什么。

fork 曾是 `https://github.com/sena-nana/cosmic-text.git`，pin 在
`061c738ebc28789963f61e82b313717ac67ffd66`（上游 0.19.0 加字体变体轴）。
上游 `https://github.com/pop-os/cosmic-text`，Copyright (c) 2022 System76，
MIT OR Apache-2.0。它的一段代码仍在本仓库里，见「照抄了什么」。

## 照抄了什么

一处，已就地署名：

- **`SubpixelBin::split`**（`crates/nana-ui/src/scene_paint/text/glyph.rs`）是
  cosmic-text `SubpixelBin::new` 的移植——阈值、分支顺序、向整像素的进位都保持
  原样。这是**故意的**：迁移之所以看不出来，正是因为一个字形还落在它原来的那个
  亚像素桶里。Copyright (c) 2022 System76，MIT OR Apache-2.0，与 NanaUI 同一对
  许可证，保留署名即满足 MIT 的要求。

另有两处是**行为兼容，不是照抄源码**，记在这里免得以后被误认：

- 合成斜体的 14° 斜切（`raster.rs` 的 `OBLIQUE_DEGREES`）：与旧后端取同一个角度，
  这样迁移前没有斜体的 face 迁移后也朝同一边倒。
- mask / color 两种 atlas 页的划分：与 cryoglyph / glyphon 同一个思路，实现是自己的。

**范围声明**：上面是按「最可能被移植的地方」查的——`SubpixelBin`、atlas、着色器、
栅格化入口。#97 的 renderer 不是逐行与 cryoglyph 对照过的。再发现移植片段时，
按同样的方式就地署名并补进这一节。

## 怎么重跑这份审计

```bash
# nana-text 的源码里没有被替换引擎的标识符（CI 也跑这一条）
python3 scripts/check-engine-boundary.py

# release 依赖图里的许可证分布
cargo tree --edges normal --workspace --prefix none | sort -u
```
