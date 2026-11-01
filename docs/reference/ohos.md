# HarmonyOS / OpenHarmony（OHOS）编译档

OHOS 是比 [Android 实验宿主](android.md)**低一级**的支持档位：仓库只承诺核心 crate 能为 OHOS 编译、在 OHOS 上的平台分支行为正确。仓库里没有 OHOS 宿主，窗口、XComponent、输入法与剪贴板接入都由应用自己完成。不要把它写进面向用户的跨平台承诺。

OHOS 上的界面走同一条 `UiWorld` → `UiScene` → `SceneWgpuPainter` 路径，不另起树、不另起绘制器，也没有 WebView 外壳。

## Target 与 cfg

| 项 | 值 |
| --- | --- |
| 支持的 target | `aarch64-unknown-linux-ohos` |
| `target_os` | `linux` |
| `target_env` | `ohos` |

OHOS 的 `target_os` 是 `linux`，所以任何写成 `target_os = "linux"` 的桌面分支（X11 / Wayland、portal、AT-SPI、ALSA、XDG 路径、fontconfig XML）都会被 OHOS 误入。规则：

- 判定 OHOS 一律用 `target_env = "ohos"`。
- 新增 `target_os = "linux"` 的桌面分支或 target 依赖时，同时写上 `not(target_env = "ohos")`。

目前已按这条规则隔离的分支：

| 位置 | OHOS 上的行为 |
| --- | --- |
| `nana-ui-platform` 的 `arboard` 依赖、`OsClipboard` | 不编译 |
| `default_shared_clipboard()` | 返回 `UnsupportedClipboard`，见下文剪贴板 |
| `app_data_dir()` | 返回 `None`，不落到 `~/.local/share` |
| `PathPlatform::current()` | 返回 `None`；`PathEnvironment::current()` 报 `UnsupportedPlatform("ohos")` |
| `nana-ui` 的 `accesskit_unix` | 不编译 |
| `nana-ui` 的 `hosted` | `compile_error!`：它是 winit / `nana-window` 桌面宿主，没有 OHOS 后端 |
| `nana-text` 系统字体发现 | 不走 fontdb 的 Linux 扫描，见下文字体 |

## 承诺什么

- `scripts/check-ohos-arm64.sh` 列出的 crate 能为 `aarch64-unknown-linux-ohos` 通过 `cargo check`，CI 的 `ohos-arm64-cross` 每次都跑。
- 上表的平台分支在 OHOS 上取 OHOS 的行为，不进桌面分支。
- 字体发现与回退链的逻辑有宿主机单测（根目录可注入），不需要设备也能验证。

## 不承诺什么

- **没有官方宿主模板。** 仓库里没有 `platform/ohos`，没有 ArkTS 工程、HAP 打包或 XComponent 示例。
- **没有真机或模拟器 CI。** CI 只做交叉 `cargo check`。没有链接、没有 `.so`，也没有在设备上跑过任何代码。
- **窗口与 XComponent 宿主由应用集成。** 从 XComponent 的 `OHNativeWindow` 建 `GpuContext` 与 surface、帧调度、触摸与按键降成 `nana-ui-input` 的规范事件、输入法接入、无障碍投影，都没有在 OHOS 上验证过。wgpu 在 OHOS 上建 surface 能否成功同样未验证。
- **`fetch` / GPU 路径不在 CI 里。** `fetch`（ureq → rustls → ring）的 build script 即使只做 `check` 也要目标平台的 C 编译器，CI 没有装 OHOS SDK。设置 `OHOS_NDK_HOME` 后脚本才检查 `nana-ui-platform` 默认 feature 与 `nana-ui --features gpu`。
- **没有 SDK 时用宿主 `clang` 汇编 `psm`。** `nana-ui-runtime` 的 `stacker` 经 `psm` 汇编一个只含 psm 自带头文件的 `.s`，不需要 OHOS sysroot；脚本把 `CC_aarch64_unknown_linux_ohos` 设成宿主 `clang`（cc-rs 传 `--target=aarch64-unknown-linux-ohos`），有 `llvm-ar` 时一并用它。宿主 gcc 只出 x86 代码，汇编不了；找不到 `clang` 时脚本直接失败。
- **字体配置只按开源样例验证。** `fontconfig.json` 的解析对照的是 OpenHarmony Skia 移植仓库里的 `src/ports/skia_ohos/config/fontconfig.json`，没有读过设备上的文件。`generic` 里的家族别名（如 `HarmonyOS-Sans-Bold`）只认 CSS 通用族名，其它别名不映射。
- 只有 ARM64。`armv7-unknown-linux-ohos` 与 `x86_64-unknown-linux-ohos` 没有检查。

## 宿主接入合同

应用的 OHOS 宿主持有窗口、surface、`GpuContext`、每帧的 `FrameContext` 与帧调度，和其它宿主一样把 `SceneWgpuPainter` 建在自己的 context 上。框架侧的几条约定：

- **剪贴板**：OHOS 不编译桌面剪贴板，也没有默认后端。宿主用系统 Pasteboard 实现 `HostServices::read_clipboard` / `write_clipboard`；Web API 路径则把自己的 `ClipboardHost` 用 `shared_clipboard(...)` 包好，传给 `register_web_api_host_ops_with_resources`。没有注入时 `default_shared_clipboard()` 是 `UnsupportedClipboard`，读写都返回 `HostServiceError::Unsupported`，不会 panic。
- **应用数据目录**：沙箱的文件目录只有 ArkTS `Context` 知道。宿主取到 `filesDir` 后自己调用 `FileStore::open`，框架不私自写盘。
- **桌面宿主**：不要在 OHOS 上开 `nana-ui` 的 `hosted`；用 `--lib` 与 `gpu`，和 Android 宿主一样。

## 字体

`FontSystem::with_system_fonts()` 在 OHOS 上：

1. 读 `/system/etc/fontconfig.json`（OpenHarmony Skia 移植的格式）。只用三个键：`fontdir`（扫描哪些目录）、`generic`（`serif`、`monospace` 等 CSS 通用族名的别名）、`fallback` 里默认集合 `""` 的 BCP 47 标签 → 家族列表。`font_file_map` 不读：它列的文件名在设备上并不总是存在，而每个字面的家族名本来就从字体自己的 name 表读。
2. 扫描配置里存在的目录；配置缺失、解析失败或没有一个目录存在时，扫描 `/system/fonts`。
3. 回退策略先放配置给出的家族，再接内置的 HarmonyOS 链：

| 场景 | 内置家族 |
| --- | --- |
| `sans-serif` / `system-ui` | HarmonyOS Sans → HarmonyOS Sans SC → Noto Sans |
| 简体中文（`Hani`） | HarmonyOS Sans SC |
| 繁体中文（`zh-Hant` / `zh-TW` / `zh-HK` / `zh-MO`） | HarmonyOS Sans TC |
| 日文 / 韩文 | Noto Sans CJK JP / Noto Sans CJK KR |
| 阿拉伯文 | HarmonyOS Sans Naskh Arabic UI |
| Emoji | HMOS Color Emoji → HMOS Color Emoji Flags |
| 符号 | Noto Sans Symbols → Noto Sans Symbols 2 → HM Symbol |

配置里有而内置链没有的文字（如 `und-Gujr`）只用配置的家族。`FallbackPolicy::platform_default()` 在 OHOS 上只返回内置链，不读文件。

## 和 Android 档位的差异

| | Android 实验宿主 | OHOS 编译档 |
| --- | --- | --- |
| 仓库内宿主 | `platform/android`（`GameActivity`） | 无 |
| CI | 交叉 check，含 NDK | 交叉 check，不装 SDK，不含 `fetch` / GPU |
| 构建产物 | `.so` 与可安装 APK | 无 |
| 真机验收 | 有清单，部分项已在真机通过 | 无 |
| 剪贴板 | JNI `ClipboardManager`，`default_shared_clipboard()` 即可用 | 宿主注入；默认 `Unsupported` |
| 应用数据目录 | `app_data_dir` 返回 `getFilesDir()` | `app_data_dir` 返回 `None`，宿主传入 |
| 输入法 / 无障碍 | GameActivity IME 与 TalkBack 动作已接 | 未接，宿主自理 |
| 系统字体 | 扫 `/system/fonts` | 读 `fontconfig.json`，扫目录作后备，HarmonyOS 回退链 |

## 检查

```bash
rustup target add aarch64-unknown-linux-ohos
./scripts/check-ohos-arm64.sh
```

不需要 linker；没有 OHOS SDK 时需要宿主 `clang`，见上文。要连 `fetch` / GPU 路径一起检查，先装 OpenHarmony SDK 的 native 部分，再：

```bash
OHOS_NDK_HOME=/path/to/ohos-sdk/linux ./scripts/check-ohos-arm64.sh
```

`OHOS_NDK_HOME` 指向含 `native/llvm` 与 `native/sysroot` 的 SDK 目录（也接受 `native` 目录本身）；已经设置 `CC_aarch64_unknown_linux_ohos` 时直接用它。
