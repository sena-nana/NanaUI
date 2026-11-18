# 第三方代码与许可证审计

NanaUI 自身代码采用 **MIT 或 Apache-2.0** 双许可发布（详见仓库根目录的 [LICENSE-MIT](https://github.com/sena-nana/NanaUI/blob/main/LICENSE-MIT) 与 [LICENSE-APACHE](https://github.com/sena-nana/NanaUI/blob/main/LICENSE-APACHE)）。本文面向需要打包分发 NanaUI 应用或进行法务合规审查的维护者，记录当前依赖图的许可证构成、文本引擎自研切换后（#99）旧分支的移除状态，以及仓库内含有的两处显式署名移植代码。

## 核心合规结论

1. **全图宽松许可证（Permissive Only）**：全平台（`--target all`）的 `normal + build` 依赖图共包含 **572 个外部 crate**，全部采用宽松开源许可证（MIT、Apache-2.0、Apache-2.0 WITH LLVM-exception、BSD、ISC、Zlib、Unicode-3.0、0BSD、CC0、BSL-1.0、CDLA-Permissive-2.0 及其组合），不存在任何仅提供 Copyleft 的依赖边（两处 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` 均按多选许可取 MIT / Apache-2.0 分支）。
2. **`cryoglyph` 与 `cosmic-text` 已从产品与 `Cargo.lock` 中彻底移除**：两者既不在普通依赖中，也不在 dev 依赖中。CI 中的 `scripts/check-engine-boundary.py` 强制检查全工作区：任何成员在任何 feature 下均不得引入通向 `cosmic-text`、`cryoglyph` 或 `glyphon`（含改名 fork）的非 dev 依赖边，且 `nana-text/src` 下不得出现这三个标识符。
3. **两处移植代码片段均已就地保留原始版权与许可证声明**（详见下文「显式署名与移植片段」）。

## 文本栈依赖的底层算法库

`nana-text` 自身拥有生命周期、文本 IR、分级缓存与段落编排逻辑，但**不重复发明底层 Unicode 标准表与字体解析算法**。以下外部库被严格封装在各自的私有模块内部（由 `scripts/check-engine-boundary.py` 按文件路径白名单守卫，其类型绝不泄漏到 `nana-text` 的公开 API 中）：

| 外部 crate | 版本（以 `Cargo.lock` 为准） | 许可证 | 职责与唯一允许引用的私有模块 |
| --- | --- | --- | --- |
| `harfrust` | `0.12.0` | MIT | OpenType 整形（GSUB / GPOS，Unicode 17.0.0），仅限 `shaping/opentype.rs` |
| `skrifa` | `0.44.0` | MIT OR Apache-2.0 | 可变字体轴、命名实例、彩色字形表、`cmap` 与字形轮廓/位图 strike 读取，仅限 `font/face.rs` |
| `read-fonts` | `0.41.0` | MIT OR Apache-2.0 | `skrifa` 与 `harfrust` 共用的底层字表读取 |
| `fontdb` | `0.24.0` | MIT | 系统字体目录扫描与 `name` / `OS/2` 元数据读取，仅限 `font/discovery.rs` |
| `ttf-parser` | `0.25.1` | MIT OR Apache-2.0 | `nana-text` 不使用：仅为 `ratex` 数学公式链路中 `fontdb 0.23` / `ab_glyph` 的字表解析器（见下方注释与“依赖安全公告与豁免”） |
| `memmap2` | `0.9.11` | MIT OR Apache-2.0 | 系统字体文件的只读内存映射，仅限 `font/mapped.rs`（`fontdb` 也用它） |
| `unicode-bidi` | `0.3.18` | MIT OR Apache-2.0 | Unicode 双向文本算法（UBA，Unicode 16.0.0），仅限 `shaping/bidi.rs` |
| `unicode-linebreak` | `0.1.5` | Apache-2.0 | UAX #14 断行机会表（Unicode 15.0.0），仅限 `layout/breaks.rs` |
| `unicode-segmentation` | `1.13.3` | MIT OR Apache-2.0 | 字素簇（grapheme cluster）与词边界切分（Unicode 17.0.0） |
| `zeno` | `0.3.3` | Apache-2.0 OR MIT | 字形路径填充与描边栅格化，仅限 `nana-text` 的 `raster/outline.rs`（`raster` 可选特性） |
| `png` | `0.18.1` | MIT OR Apache-2.0 | 彩色位图 strike 的 PNG 解码，仅限 `nana-text` 的 `raster/outline.rs`（`raster` 可选特性） |
| `windows` | `0.62.2` | MIT OR Apache-2.0 | 仅 Windows：DirectWrite 轮廓栅格化与读取系统 ClearType 配置，仅限 `nana-ui` 的 `scene_paint/text/raster_dwrite.rs`（彩色/位图/侧卧字形及回退仍走 `nana-text` 的 `OutlineRasterizer` / `raster/outline.rs`） |
| `etagere` | `0.3.0` | MIT OR Apache-2.0 | 字形图集（glyph atlas）矩形装箱，仅限 `nana-ui` 的 `scene_paint/text/atlas.rs` |

（注：锁文件中同时存在的 `fontdb 0.23.0` 由数学公式渲染链路 `ratex-svg → ratex-font-loader → ratex-unicode-font` 引入，与 `nana-text` 互不可见。`nana-text` 已移除对 `icu_properties`、`swash` 与 `yazi` 的依赖：字形轮廓缩放由 `skrifa` 在 `font/face.rs` 完成，字符属性由 `crates/nana-text/src/font/ucd_tables.rs` 承载。）

### Unicode 数据与版本对齐矩阵

`crates/nana-text/src/font/ucd_tables.rs` 由 `scripts/generate-ucd-tables.py` 从官方 Unicode 17.0.0 UCD（`Unicode-3.0` 数据许可）生成，包含 `DEFAULT_IGNORABLE`、`VERTICAL_ROTATED`、`EMPHASIS_ALLOWED`、`EMOJI_PRESENTATION`、`SCRIPT_TAGS` / `SCRIPT_RANGES` 与 `UNICODE_VERSION`：

| 文本栈数据来源 | Unicode 版本基线 | 覆盖范围 |
| --- | --- | --- |
| `font/ucd_tables.rs`（`scripts/generate-ucd-tables.py`） | `17.0.0` | `Default_Ignorable_Code_Point`、`Vertical_Orientation`（UAX #50）、`General_Category`（着重号过滤）、`Emoji_Presentation`、`Script` |
| Rust 标准库 `char::UNICODE_VERSION` | `>= 17.0.0`（随工具链） | 核心标量属性；`nana-text` 测试要求工具链版本不低于 `ucd_tables.rs`，更新的工具链兼容 |
| `harfrust` `0.12.0` | `17.0.0` | OpenType 整形内建 UCD 表 |
| `unicode-segmentation` `1.13.3` | `17.0.0` | 字素簇与词边界 |
| `unicode-bidi` `0.3.18` | `16.0.0` | Unicode 双向文本算法（UBA） |
| `unicode-linebreak` `0.1.5` | `15.0.0` | UAX #14 断行机会表 |

`scripts/generate-ucd-tables.py --check` 校验生成表的来源 SHA-256 与结构不变量，同时核对根目录 `Cargo.lock` 中 `PINNED_UNICODE_CRATES`（`harfrust`、`unicode-segmentation`、`unicode-bidi`、`unicode-linebreak`）的锁版本未偏移（缓存存在时做全量重算比对）。

## 依赖安全公告与豁免

CI 在 Linux 上分别进入全部 **5 个** 独立 `Cargo.lock` 工作目录执行 `cargo deny check advisories`（每个目录自带 `deny.toml`，避免工作目录不同步引起规则误判）：根工作区 `.`（含 `tools/css-parity`，策略见根目录 `deny.toml`，`[graph] all-features = true`，因此可选 feature 引入的依赖同样受检）、`tools/css-parity-webview`、`crates/nana-icons-tabler`、`perf/micro/stack-depth` 与 `perf/micro/taffy-reference`。所有策略均设 `unmaintained = "all"`。有 semver 兼容修复版本的漏洞一律通过 `cargo update -p <crate>` 解决，不加豁免；下表是仅有的豁免，每条都注明引入路径与移除条件：

| 公告 | Crate | 引入路径 | 移除条件 |
| --- | --- | --- | --- |
| `RUSTSEC-2025-0141`（unmaintained） | `bincode 1.3.3` | 仅可选 feature `syntax-highlighting`：`nana-ui-runtime → two-face → syntect` 加载语法/主题 dump。NanaUI 自身不直接依赖 `bincode`；预编译着色器使用自研确定性编解码器 `scene_paint::naga_ir::codec`（`NAGA1` 格式），默认依赖图中不含 `bincode` | 上游 `syntect` 移除 `bincode 1.x` |
| `RUSTSEC-2026-0192`（unmaintained） | `ttf-parser 0.25.1` | 仅可选 feature `rich-text`：固定版本的 `ratex =0.1.14` 数学公式渲染（`fontdb 0.23` / `ab_glyph` / `owned_ttf_parser`）。`nana-text` 的字体解析使用 `skrifa` | `ratex` 迁移到 `skrifa` |
| `RUSTSEC-2026-0370` / `0371` / `0372`（unmaintained） | `zvariant`、`zbus_names`、`zvariant_derive` 5.x/4.x | Linux 上 `zbus 5`：`accesskit_unix` / `atspi`（无障碍）与 `ashpd`（portal）。这些 crate 已并入 `zbus 6`，`zbus 5` 线上没有修复版本 | `accesskit_unix` 与 `ashpd` 升级到 `zbus 6` |
| `RUSTSEC-2024-0370`（unmaintained，仅 `css-parity-webview`） | `proc-macro-error 1.0.4` | `tao` / `wry` 的 Linux GTK 后端（gtk-rs 0.18 的 `glib-macros` / `gtk3-macros`）；该工具是 macOS WKWebView 参照工具，NanaUI 产品 crate 从不链接 | `tao` / `wry` 脱离 gtk-rs 0.18 |

不再命中的豁免会产生 `advisory-not-detected` 警告，届时应从 `deny.toml` 删除对应条目。

## 已移除的两个历史 Fork 状态

### 1. `cryoglyph`（旧 GPU 文本渲染器）

已从产品与 `Cargo.lock` 中完全移除。`SceneWgpuPainter::text`（位于 `crates/nana-ui/src/scene_paint/text/`）独立实现了全部字形 IR、栅格化边界、栅格缓存、代际引用计数多页 GPU 图集（`atlas.rs`）、上传队列与 WGSL 文本管线（`text_atlas.wgsl`）。本仓库**未从 `cryoglyph` 复制任何源码**（两者共享的 `0.04045 / 12.92 / 1.055 / 2.4` 系数属于 IEC 61966-2-1 标准 sRGB EOTF 公式）。

### 2. `cosmic-text`（旧整形与布局引擎）

已从产品与 dev 依赖中完全移除。迁移期间用于对比的参照引擎（`crates/nana-text/tests/reference/`）及比对脚本已一并删除；保留下来的 `crates/nana-text/corpus/golden/TX-*.layout.json` 是冻结的布局基线数据，由原生 `nana-text` 引擎直接对齐验证。

## 显式署名与移植片段

仓库内仅有以下两处算法片段移植自外部开源项目，均已在对应源码文件头部完整保留原始版权声明与许可证：

1. **`SubpixelBin::split`（`crates/nana-ui/src/scene_paint/text/glyph.rs`）**：
   移植自 `cosmic-text`（`https://github.com/pop-os/cosmic-text`）的 `SubpixelBin::new`，刻意保持相同的量化阈值、分支顺序与向整像素进位逻辑，以确保引擎切换前后同一字形落入完全相同的亚像素分桶。原始版权：`Copyright (c) 2022 System76`，许可证为 `MIT OR Apache-2.0`。
2. **DirectWrite 灰度 / ClearType 覆盖率 Gamma 校正（`crates/nana-ui/src/scene_paint/text/gamma.rs` 与 `shader/text_atlas.wgsl`）**：
   `GAMMA_INCORRECT_TARGET_RATIOS` 系数表、`gamma_ratios` 归一化以及着色器中的 `enhance_contrast`、`apply_alpha_correction` 与亮色字对比度衰减（`corrected_coverage`），移植自 Windows Terminal AtlasEngine 的 `dwrite.cpp` 与 `dwrite.hlsl`（`https://github.com/microsoft/terminal`）。原始版权：`Copyright (c) Microsoft Corporation`，许可证为 `MIT`。后续将 gamma 空间结果转换为线性混合所需覆盖率的步骤为本仓库自研实现。

此外，合成斜体的 `14°` 斜切角（`crates/nana-text/src/raster/outline.rs` 中的 `OBLIQUE_DEGREES`）以及将图集划分为 mask 与 color 两类页面的设计，属于常见的行业行为兼容与架构思路，并非源码复制。

## 复核命令

```bash
# 1. 验证依赖图中无被替换的旧引擎、nana-text/src 中无禁用标识符及 wgpu 边界（CI 必跑）
python3 scripts/check-engine-boundary.py

# 2. 校验 UCD 静态表生成物、版本与来源 SHA-256
python3 scripts/generate-ucd-tables.py --check

# 3. 导出全平台 normal + build 依赖图的外部 crate 与许可证清单
cargo tree --edges normal,build --workspace --locked --target all --prefix none -f '{p}|{l}' \
  | sed 's/ (\*)//' | sort -u | grep -v "(/"

# 4. 检查全部 5 个独立 lockfile 目录的依赖安全公告（CI 必跑）
for dir in . tools/css-parity-webview crates/nana-icons-tabler perf/micro/stack-depth perf/micro/taffy-reference; do
  (cd "$dir" && cargo deny check advisories) || exit 1
done
```
