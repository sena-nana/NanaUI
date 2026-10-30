# 第三方代码与许可证审计

NanaUI 自身代码采用 **MIT 或 Apache-2.0** 双许可发布（详见仓库根目录的 [LICENSE-MIT](../../LICENSE-MIT) 与 [LICENSE-APACHE](../../LICENSE-APACHE)）。本文面向需要打包分发 NanaUI 应用或进行法务合规审查的维护者，记录当前依赖图的许可证构成、文本引擎自研切换后（#99）旧分支的移除状态，以及仓库内含有的两处显式署名移植代码。

## 核心合规结论

1. **全图宽松许可证（Permissive Only）**：全平台（`--target all`）的 `normal + build` 依赖图共包含 **572 个外部 crate**，全部采用宽松开源许可证（MIT、Apache-2.0、Apache-2.0 WITH LLVM-exception、BSD、ISC、Zlib、Unicode-3.0、0BSD、CC0、BSL-1.0、CDLA-Permissive-2.0 及其组合），不存在任何仅提供 Copyleft 的依赖边（两处 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` 均按多选许可取 MIT / Apache-2.0 分支）。
2. **`cryoglyph` 与 `cosmic-text` 已从产品与 `Cargo.lock` 中彻底移除**：两者既不在普通依赖中，也不在 dev 依赖中。CI 中的 `scripts/check-engine-boundary.py` 强制检查全工作区：任何成员在任何 feature 下均不得引入通向 `cosmic-text`、`cryoglyph` 或 `glyphon`（含改名 fork）的非 dev 依赖边，且 `nana-text/src` 下不得出现这三个标识符。
3. **两处移植代码片段均已就地保留原始版权与许可证声明**（详见下文「显式署名与移植片段」）。

## 文本栈依赖的底层算法库

`nana-text` 自身拥有生命周期、文本 IR、分级缓存与段落编排逻辑，但**不重复发明底层 Unicode 标准表与字体解析算法**。以下外部库被严格封装在各自的私有模块内部（由 `scripts/check-engine-boundary.py` 按文件路径白名单守卫，其类型绝不泄漏到 `nana-text` 的公开 API 中）：

| 外部 crate | 版本（以 `Cargo.lock` 为准） | 许可证 | 职责与唯一允许引用的私有模块 |
| --- | --- | --- | --- |
| `harfrust` | `0.12.0` | MIT | OpenType 整形（GSUB / GPOS），仅限 `shaping/opentype.rs` |
| `skrifa` | `0.44.0` | MIT OR Apache-2.0 | 可变字体轴、命名实例、彩色字形表与 `cmap`，仅限 `font/face.rs` |
| `read-fonts` | `0.41.0` | MIT OR Apache-2.0 | `skrifa` 与 `harfrust` 共用的底层字表读取 |
| `fontdb` | `0.24.0` | MIT | 系统字体目录扫描与 `name` / `OS/2` 元数据读取，仅限 `font/discovery.rs` |
| `ttf-parser` | `0.25.1` | MIT OR Apache-2.0 | `fontdb` 使用的字表解析器 |
| `icu_properties` | `2.3.0` | Unicode-3.0 | Unicode Script、`Emoji_Presentation`、`Default_Ignorable` 属性，仅限 `font/unicode.rs` |
| `unicode-bidi` | `0.3.18` | MIT OR Apache-2.0 | Unicode 双向文本算法（UBA），仅限 `shaping/bidi.rs` |
| `unicode-linebreak` | `0.1.5` | Apache-2.0 | UAX #14 断行机会表，仅限 `layout/breaks.rs` |
| `unicode-segmentation` | `1.13.3` | MIT OR Apache-2.0 | 字素簇（grapheme cluster）与词边界切分 |
| `swash` / `zeno` / `yazi` | `0.2.10` / `0.3.3` / `0.2.1` | Apache-2.0 OR MIT | 字形轮廓缩放、路径栅格化与 WOFF2 解压，仅限 `nana-ui` 的 `scene_paint/text/raster.rs` |
| `windows` | `0.62.2` | MIT OR Apache-2.0 | 仅 Windows：DirectWrite 轮廓栅格化与读取系统 ClearType 配置，仅限 `scene_paint/text/raster_dwrite.rs`（彩色/位图/侧卧字形及回退仍走 `swash`） |
| `etagere` | `0.2.15` | MIT OR Apache-2.0 | 字形图集（glyph atlas）矩形装箱，仅限 `scene_paint/text/atlas.rs` |

（注：锁文件中同时存在的 `fontdb 0.23.0` 由数学公式渲染链路 `ratex-svg → ratex-font-loader → ratex-unicode-font` 引入，与 `nana-text` 互不可见。）

## 已移除的两个历史 Fork 状态

### 1. `cryoglyph`（旧 GPU 文本渲染器）

已从产品与 `Cargo.lock` 中完全移除。`NanaRenderer::text`（位于 `crates/nana-ui/src/scene_paint/text/`）独立实现了全部字形 IR、栅格化边界、栅格缓存、代际引用计数多页 GPU 图集（`atlas.rs`）、上传队列与 WGSL 文本管线（`text_atlas.wgsl`）。本仓库**未从 `cryoglyph` 复制任何源码**（两者共享的 `0.04045 / 12.92 / 1.055 / 2.4` 系数属于 IEC 61966-2-1 标准 sRGB EOTF 公式）。

### 2. `cosmic-text`（旧整形与布局引擎）

已从产品与 dev 依赖中完全移除。迁移期间用于对比的参照引擎（`crates/nana-text/tests/reference/`）及比对脚本已一并删除；保留下来的 `crates/nana-text/corpus/golden/TX-*.layout.json` 是冻结的布局基线数据，由原生 `nana-text` 引擎直接对齐验证。

## 显式署名与移植片段

仓库内仅有以下两处算法片段移植自外部开源项目，均已在对应源码文件头部完整保留原始版权声明与许可证：

1. **`SubpixelBin::split`（`crates/nana-ui/src/scene_paint/text/glyph.rs`）**：
   移植自 `cosmic-text`（`https://github.com/pop-os/cosmic-text`）的 `SubpixelBin::new`，刻意保持相同的量化阈值、分支顺序与向整像素进位逻辑，以确保引擎切换前后同一字形落入完全相同的亚像素分桶。原始版权：`Copyright (c) 2022 System76`，许可证为 `MIT OR Apache-2.0`。
2. **DirectWrite 灰度 / ClearType 覆盖率 Gamma 校正（`crates/nana-ui/src/scene_paint/text/gamma.rs` 与 `shader/text_atlas.wgsl`）**：
   `GAMMA_INCORRECT_TARGET_RATIOS` 系数表、`gamma_ratios` 归一化以及着色器中的 `enhance_contrast`、`apply_alpha_correction` 与亮色字对比度衰减（`corrected_coverage`），移植自 Windows Terminal AtlasEngine 的 `dwrite.cpp` 与 `dwrite.hlsl`（`https://github.com/microsoft/terminal`）。原始版权：`Copyright (c) Microsoft Corporation`，许可证为 `MIT`。后续将 gamma 空间结果转换为线性混合所需覆盖率的步骤为本仓库自研实现。

此外，合成斜体的 `14°` 斜切角（`raster.rs` 中的 `OBLIQUE_DEGREES`）以及将图集划分为 mask 与 color 两类页面的设计，属于常见的行业行为兼容与架构思路，并非源码复制。

## 复核命令

```bash
# 1. 验证依赖图中无被替换的旧引擎、nana-text/src 中无禁用标识符及 wgpu 边界（CI 必跑）
python3 scripts/check-engine-boundary.py

# 2. 导出全平台 normal + build 依赖图的外部 crate 与许可证清单
cargo tree --edges normal,build --workspace --locked --target all --prefix none -f '{p}|{l}' \
  | sed 's/ (\*)//' | sort -u | grep -v "(/"
```
