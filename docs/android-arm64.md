# Android ARM64 矩阵（Issue #5 MVP #10）

## 目标

`nana-js-*` / `nana-ui-vue` / `nana-android-host` 在 `aarch64-linux-android` 上可
`cargo check` / `cargo build`；无 System WebView — Rust 持有 QuickJS + Vue 自定义渲染宿主
+ wgpu/Vulkan Surface。

## 架构边界

| 层 | Android MVP | 说明 |
|----|-------------|------|
| **NanaUI（壳 / 布局 / 通用控件）** | chrome fill + Primary **control-slot**（Icon/Text/Input/Switch/Button）+ Motion/KeyEvent | **完整 `DesktopShell` 不假接**；`iced_shell=false` |
| **Vue（应用层）** | `VueHost` + QuickJS 已 boot；树文档 / layout resolve 在跑 | 保留 CSS、自造组件与业务逻辑自由度；可选 `nana-*` 语义桥，非强制 |
| **禁止路径** | System WebView、Blitz / paint-stub | 与桌面 Blitz 移除决策一致 |

Surface 上已有 **壳 chrome + slot 控件/指针/KeyEvent**；**完整 DesktopShell** 与 **软 IME（InputConnection）** 为 **defer**（非 Vue 自定义渲染缺口）。

## 本机探测（2026-08-06，落地后）

| 项 | 状态 |
|----|------|
| `rustup` target `aarch64-linux-android` | **已安装** |
| `ANDROID_HOME` | `~/Android/Sdk`（`scripts/setup-android-ndk.sh`） |
| `ANDROID_NDK_HOME` | `~/Android/Sdk/ndk/27.2.12479018`（NDK r27c） |
| linker | `aarch64-linux-android24-clang`（NDK LLVM） |
| `platform/android/` 宿主 | **`nana-android-host`**（NativeActivity + QuickJS + `AndroidShellStub` + wgpu clear） |
| `nana-ui-platform` | Android MVP capability flags + `SurfacePhase` |
| 交叉编译产物 | `libnana_android_host.so`（ELF aarch64） |

## 一键流程

```bash
# 1) 安装 NDK（需 openjdk@21 + android-commandlinetools；无需 sudo）
./scripts/setup-android-ndk.sh

# 2) 导出 linker / CC / bindgen sysroot（必须用 bash source）
source scripts/android-env.sh

# 3) check（默认）或 build（产出 .so）
./scripts/check-android-arm64.sh
./scripts/check-android-arm64.sh --build
```

可选：磁盘紧张时把产物放到独立目录：

```bash
export CARGO_TARGET_DIR="$PWD/target-android"
./scripts/check-android-arm64.sh --build
# → target-android/aarch64-linux-android/debug/libnana_android_host.so
```

## 验证命令（源码回归）

```bash
source scripts/android-env.sh
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target-android}"

cargo check -p nana-js-engine --target aarch64-linux-android --locked
cargo check -p nana-js-quickjs --target aarch64-linux-android --locked
cargo check -p nana-ui-vue --target aarch64-linux-android --locked \
  --no-default-features
cargo check -p nana-android-host --target aarch64-linux-android --locked

# 链接证据：
cargo build -p nana-android-host --target aarch64-linux-android --locked
file "$CARGO_TARGET_DIR/aarch64-linux-android/debug/libnana_android_host.so"
# 期望：ELF 64-bit LSB shared object, ARM aarch64, …
```

### 本机已通过证据（2026-08-06）

- `nana-js-quickjs`：Android 无预生成 bindings → 启用 `bindgen` feature（见
  `crates/nana-js-quickjs/Cargo.toml` target 依赖）后 `cargo check/build` 通过。
- `nana-android-host`：`libnana_android_host.so` 已产出（debug，含符号；ARM aarch64 ELF）。
- 引擎默认 **QuickJS**；**不**链接 V8。`VueHost` 已 attach 并按 Primary viewport 做 layout resolve；Surface 帧仍为 wgpu clear；壳几何由 **`AndroidShellStub`** 驱动；**Nana Iced `DesktopShell` 绘制仍开放**（与 Vue 自定义渲染路径无关）。

## 宿主边界

```
platform/android (nana-android-host)
  ├─ android_main (android-activity NativeActivity)
  ├─ AndroidShellStub — 壳几何 + chrome bands（非完整 DesktopShell）
  ├─ VueHost + QuickJS — Vue 自定义渲染 / layout resolve
  ├─ GpuSurface — wgpu 30 / Vulkan / ANativeWindow
  ├─ chrome scissor fill + Primary iced control-slot strip
  └─ MotionEvent + KeyEvent → iced（软 IME / InputConnection **defer**）
```

打包提示：`platform/android/README.md`、`app/NATIVE_ACTIVITY.txt`（cargo-apk /
手动 `jniLibs/arm64-v8a`）。模拟器 KeyEvent 证据见下文；真机仍可选。

## MVP #10 签字

交叉编译宿主 + NDK 脚本 + **`AndroidShellStub` 几何合同** + **`VueHost` boot** = **部分完成**
（编译门禁 OK；Nana Iced 壳绘制开放；Vue 自定义渲染不受限）。
Issue #5 总表与最终验收命令：
[`docs/vue-backend-deps.md`](vue-backend-deps.md)、
[`docs/performance/2026-08-06-issue5-final-acceptance.md`](performance/2026-08-06-issue5-final-acceptance.md)。

## 仍开放（仅 defer / 可选；非「宣称未闭合」）

下列**不是**能力位假绿或文档宣称缺口；均为显式 defer、正确未宣称，或可选加深：

1. **真机交叉验证** — **可选**；模拟器 KeyEvent 已绿，不阻塞宣称面。
2. **完整 DesktopShell** — **defer / 勿假接**；`iced_shell=false`；slot 控件/指针/KeyEvent 已落地。
3. **软 IME（无 KeyEvent）** — **defer**；NativeActivity 无 InputConnection；勿只 `show_soft_input` 却宣称 `ime=true`。
4. **V8 on Android** — **非 MVP / defer**。
5. **CI Android matrix** — 基础设施可选；脚本已有，workflow 未强制。
6. **Android clipboard 真后端** — **defer**；`android_mvp().clipboard=false`。桌面已接 `arboard`（`desktop().clipboard=true` + shim `navigator.clipboard`）。

## 收敛结论（2026-08-10）— IME 真后端 vs 缺口盘点

### 判定：本轮 **不做** NativeActivity `ImeHost` 真后端

| 候选切片 | 成本 | 可验收？ | 决策 |
|----------|------|----------|------|
| `AndroidApp::show_soft_input` + 现有 KeyEvent | 低 | **否** — 现代软键盘多数不发 KeyEvent；只弹键盘 ≠ 可输入 | 不做（避免假 `ime=true`） |
| NativeActivity + 自定义 Java `InputConnection` | 高 — Java 子类、打包、与 iced `InputMethod`/commit 桥 | 理论可，但超出「最小切片」 | **Defer** |
| 迁 **GameActivity** + `GameTextInput` | 高 — activity 后端、事件模型、APK/Java 依赖重切 | 业界推荐的真 IME 路径 | **Defer**（单独里程碑） |
| `android-activity` NativeActivity `text_input_state` / `set_text_input_state` | — | API 存在但实现为 **NOP** | 不能当后端 |

依据：`android-activity` 文档与实现明确 — NativeActivity **无内建 InputConnection**；`text_input_*` 为空操作；真软键盘文本需 GameActivity 或自研 Java 桥。

因此：`PlatformCapabilities::android_mvp().ime` **保持 `false`**；`UnsupportedIme` 保留。KeyEvent→iced（US-QWERTY）继续作为「可打字」最小证据（硬件 / 少数发 KeyEvent 的软键盘）。

### 「宣称支持」盘点（相对能力位 / 文档）— **宣称面已闭合**

| 项 | 宣称 / 能力位 | 现状 | 判定 |
|----|---------------|------|------|
| Vulkan Surface + QuickJS | `vulkan_surface` / `rust_js_engine` | 绿（交叉编译） | **闭合**；真机帧=可选 |
| Shell chrome bands/fill | `shell_chrome_*` | 绿 | **闭合** |
| Iced control-slot | `iced_control_slot/widget/input` | **Icon**+Text+Input+Switch+Button；指针+KeyEvent | **闭合**（子集；≠ DesktopShell） |
| DesktopShell | `iced_shell=false` | **正确未宣称** | **非缺口**；完整壳 **defer** |
| IME | `ime=false` | stub + KeyEvent 旁路 | **非缺口**；软 IME **defer** |
| Clipboard | Android `clipboard=false` | Android stub；桌面见扩展 **X5**（已闭合） | Android **非缺口（defer）**；勿抬本能力位 |
| Lucide / Icon | 桌面别名→glyph；slot `Icon::Settings` | Nana 线图标 | **闭合于子集**；全量 SVG=按需加深 |
| 真机/APK | 文档 P2 | debug APK **可构建**；模拟器 KeyEvent **绿** | **APK+模拟器闭合**；真机可选 |
| Overlay Vue 双侧同步 | 桌面硬闸 | `note_toggle` active/toggled | **闭合**；保持勿回退；加深对齐扩展 X3（仍非 CSS fixed） |
| 布局 css-parity | 硬闸绿 | hold | **闭合于子集**；`fixed`/`sticky`/2D grid **defer**；Repo/`auto-fit` 见扩展 X1/X2 |

### APK 工具链（2026-08-10 更新）

| 依赖 | 结果 |
|------|------|
| NDK / `check-android-arm64.sh --build` | OK → `.so` |
| `build-tools;34.0.0` | **已装** |
| `cargo-apk` 0.10 | **已装**，但 **无法** 解析本仓库 workspace `Cargo.toml`（toml 0.5 拒绝对多行 inline table） |
| 仓库脚本 `scripts/package-android-host-apk.sh` | **OK** — 用 aapt/zipalign/apksigner 包装预构建 `.so` |
| 可安装物 | `target-android/apk/nana-android-host-debug.apk`（debug 签名） |
| `$ANDROID_HOME/platform-tools/adb` | 有 |
| Emulator / system image | **已装**（见下「模拟器 KeyEvent 证据」） |

```bash
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build
./scripts/package-android-host-apk.sh
# → target-android/apk/nana-android-host-debug.apk
# 有设备/模拟器时：
#   adb install -r target-android/apk/nana-android-host-debug.apk
#   adb logcat -s nana-android-host
# 点 Input → KeyEvent → 期望 log: iced slot Input len=
```

### 模拟器 KeyEvent 证据（2026-08-10）

本机 Apple Silicon（`arm64`）+ Hypervisor.Framework；**非**真机。

| 步骤 | 命令 / 结果 |
|------|-------------|
| 最小 SDK 包 | `JAVA_HOME=/opt/homebrew/opt/openjdk@21 sdkmanager --sdk_root=~/Android/Sdk "emulator" "system-images;android-34;google_apis;arm64-v8a"`（许可已非交互 `yes \| … --licenses`） |
| AVD | `nana_api34_arm64`（`hw.keyboard=yes`）；Homebrew `avdmanager` 默认看不到 `~/Android/Sdk` 的 system-images → 手写 `~/.android/avd/…` 亦可 |
| 启动（推荐） | `emulator -avd nana_api34_arm64 -gpu host -no-snapshot -no-audio -no-boot-anim`（Graphics: Apple M4） |
| 头less 备选 | `-no-window -gpu swiftshader_indirect`：**可 boot**，但本宿主在 `vkCreateGraphicsPipelines` 上 **Vulkan hang/ANR**，不适合本 APK 的 wgpu 证据 |
| 安装 / 启动 | `adb install -r target-android/apk/nana-android-host-debug.apk` → `adb shell am start -n app.nanaui.host/android.app.NativeActivity` |
| 指针 + KeyEvent | `adb shell input tap <slot>`（1080×2400、`scale=2.625` 时 strip 约 `y∈[2200,2368]`）→ `adb shell input keyevent KEYCODE_H` … |
| logcat 硬证据 | `iced slot Button pressed … in_slot=true`；**`iced slot Input len=1..5 in_slot=true`**（KeyEvent→iced Input） |
| 硬闸 | `./scripts/check-android-arm64.sh` **OK**；`cargo test -p nana-android-host --locked` **OK** |

注意：须安装**当前**重建的 APK（含 `scale=` 日志与 SlotInputGate）；旧 APK 可能能画 chrome 却无 slot 输入日志。软 IME / `ime=true` **仍 defer**；完整 DesktopShell **仍不假接**。

### 可验收下一刀（优先级）— **无强制宣称切片**

宣称面已闭合后，以下均为**可选 / 独立里程碑**，勿当作本线未闭合缺口：

1. **真机**（可选）→ 同 APK + KeyEvent / 截图交叉验证。
2. **IME 里程碑（独立）** — GameActivity；验收：`ime=true` + commit → iced。
3. **明确不做** — 假 `DesktopShell`；假 `ime=true`；为 SSIM 假 CSS 定位；假完整控件集。

## 整体收敛状态（2026-08-10）— **原宣称面闭合 + 扩展合同已立**

**判定（原面）**：相对能力位与文档「宣称支持」的 **home/settings / Android slot** 切片，**未 defer 且可验收项已全部闭合**。  
**扩展（同日）**：桌面 L1 增量宣称 **X1–X7**（Repo 证据、grid 诚实策略、Overlay 非 fixed、scrollIntoView、桌面 clipboard、window 泵送、Vue host 深度）——合同见 [`performance/2026-08-10-lilia-fidelity-gap.md`](performance/2026-08-10-lilia-fidelity-gap.md)。**X5 桌面 clipboard 已闭合**；其余扩展项未过验收前勿标闭合。**本文件不改 Android clipboard/IME/DesktopShell 实现宣称。**

| 线 | 状态 | 说明 |
|----|------|------|
| **布局 L1** | **hold / 绿 / 原面闭合** | css-parity 全绿；`fixed`/`sticky`/完整 2D grid / iced 流内 absolute **defer**；Repo/`auto-fit` 策略见扩展 X1/X2 |
| **浮层** | **加深已交 / 原面闭合**；扩展 X3 对齐 Overlay | ConfirmDialog、ContextMenu、Drawer L2+footer；Vue `active`/`toggled`；**勿**宣称 CSS fixed |
| **Android 壳** | **slot 切片绿 / 宣称闭合** | chrome fill + Icon/Text/Input/Switch/Button + Motion/KeyEvent；`iced_shell=false` |
| **Android IME** | **defer** | NativeActivity 无 InputConnection；`ime=false`；KeyEvent 旁路已绿 |
| **桌面 clipboard** | **宣称闭合** | `nana-ui-platform::OsClipboard`（arboard）+ web-api host ops + `navigator.clipboard`；能力位 `desktop().clipboard=true` |
| **Android clipboard** | **defer** | `clipboard=false`；勿假绿 |
| **APK 产物** | **可构建 / 闭合** | `package-android-host-apk.sh` → debug APK |
| **模拟器 KeyEvent** | **绿 / 闭合** | AVD `nana_api34_arm64` + host GPU；`Input len=` logcat 证据 |
| **真机证据** | **可选 / 未强制** | 模拟器已覆盖 KeyEvent 路径 |
| **引擎像素** | **绿（home/settings）**；**Repo=扩展 X1 开放** | Gallery QJS↔V8 L1 SSIM 1.0；Repo 升闸见 fidelity-gap |
| **硬闸** | **原闸保持**；扩展闸待绿 | css-parity、iced-view、overlay Toggle-false、android-host、`check-android-arm64` |

**Defer 清单（勿假实现）**：完整 DesktopShell；软 IME；**Android** clipboard 真后端；V8-on-Android；`fixed`/`sticky`；完整 2D grid / `auto-fit` 布局消费；流内 absolute→Overlay；假 `ime=true`；为 SSIM 假定位。扩展 **X4/X6/X7**（scrollIntoView / window 泵送 / Vue host 深度）与 **X1 Repo 升闸**由桌面并行轨闭合；**桌面 X5 clipboard 已交**（见上表），**非**本 Android 线强制项。

**按需加深（非 Android 宣称阻塞）**：真机帧；CI Android matrix；Lucide 全量 SVG；Markdown 组合；产品浮层嵌套确认流；桌面扩展 X* 由并行轨闭合。

## Cargo / NDK 配置

| 文件 | 作用 |
|------|------|
| `.cargo/config.toml` | `aarch64-linux-android` linker 包装器 |
| `scripts/aarch64-linux-android-clang.sh` | 解析 NDK 并 exec clang |
| `scripts/android-env.sh` | `ANDROID_*`、`CC_*`、`BINDGEN_EXTRA_CLANG_ARGS_*` |
| `scripts/setup-android-ndk.sh` | sdkmanager 安装 NDK 27.2 + platform 34 |
| `scripts/check-android-arm64.sh` | 相关 crate 批量 check/build |
| `scripts/package-android-host-apk.sh` | 预构建 `.so` → 签名 debug APK（绕过 cargo-apk TOML 限制） |
