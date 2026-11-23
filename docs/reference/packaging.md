# 打包与分发合同

本文规定 NanaUI 应用的打包配置（`nana-package.toml`）、资源包二进制格式（`.nrpack`）、分层密钥与发布者签名，以及最终交付物的自动化篡改校验流程（Issue #226）。日常快速上手步骤请先看指南中的 [打包](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/packaging.md)。

分发形态以 Steam 与独立安装包为主：正常启动始终只有**一个主进程、一个可执行文件**，不引入常驻启动器、不增加额外 IPC，也不依赖 A/B 双目录切换。`nana-packager` 不逆向调用 `cargo`，而是接收已经构建好的可执行文件并完成打包与验证：

```text
cargo build --profile dist
        │  已构建的可执行文件
        ▼
nana-packager package                 读取 nana-package.toml 与资源目录
        │  pack plan → .nrpack → 平台目录布局 → manifest → 签名 → Steam 元数据
        ▼
<out>/app/                            最终交付物目录 / .app Bundle
        │
nana-packager validate --run --tamper-suite
```

## 1. 模块分工与依赖红线

| Crate / 模块 | 职责说明 | 是否进入应用二进制？ |
| --- | --- | --- |
| `nana-package` | 读取 `package.json` 清单、解析只读 `.nrpack`、`KeyProvider` 接口、发布者 Ed25519 验签、二进制身份标记解析 | **是**（经 `nana-ui/packaged-resources` 按需启用；仅含纯 Rust 依赖，零 C 库、零随机数源） |
| `nana-packager` | 解析 `nana-package.toml`、生成打包计划、压缩与加密 `.nrpack`、组装平台目录、产出 Steam 元数据、执行最终产物与防篡改校验 CLI | **否**（纯构建期工具） |
| `nana-ui-core::packaged` | `nana://res/` 逻辑路径规范化与进程级 `PackagedResourceSource` 挂载钩子 | **是**（零外部依赖） |

CI 中的 `scripts/check-package-boundary.py` 强制守护以下边界：`nana-package` 严禁依赖 C 版 `zstd`、`ring`、`rustls`、`getrandom` 或 `rand`，且 `blake3` 必须使用 `pure` 后端；可嵌入的 Runtime 类 crate 不得依赖打包 crate；工作区内任何成员的非 dev 依赖均不得指向 `nana-packager`。

## 2. 逻辑位置与跨平台安装布局

业务代码统一通过 `ApplicationPaths` 获取逻辑目录（详见 [诊断与应用路径](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagnostics.md)），切勿硬编码 `./runtime/...` 相对路径：

| 逻辑位置 | Windows / Linux | macOS（`.app` Bundle） |
| --- | --- | --- |
| 应用根目录 | `App/`（仅含主程序 `App.exe` 与 `runtime/`） | `Name.app` |
| 主可执行文件 | `App.exe` / `App` | `Contents/MacOS/App` |
| `RuntimeBin` | `runtime/bin` | `Contents/Frameworks` |
| `RuntimeResources` | `runtime/resources` | `Contents/Resources` |
| `RuntimePlugins` | `runtime/plugins` | `Contents/PlugIns` |
| `RuntimeTools` | `runtime/tools` | `Contents/Helpers` |
| `RuntimeManifest` | `runtime/manifest` | `Contents/Resources/manifest` |

`[runtime]` 的 `bin`、`tools`、`resources` 分别复制到 `RuntimeBin`、`RuntimeTools`、`RuntimeResources`。条目可以是目录：整体复制，符号链接原样保留（macOS 的 `.framework` 依赖 `Versions/Current` 链接），目录内指向自身之外的链接（绝对路径或越过目录的 `..`）会被拒绝；清单逐个记录其中的文件，链接到文件的按目标内容计。

Linux 安装包（`deb`、AppImage、Flatpak）按 FHS 摆放同一棵树：可执行文件与架构相关的 `runtime/{bin,plugins,tools}` 在 `<prefix>/lib/<pkg>/`，架构无关的 `resources` 与 `manifest` 在 `<prefix>/share/<pkg>/`，另有 `<prefix>/bin/<exe>` 链接、`<id>.desktop` 和 256×256 图标。运行时 `ApplicationPaths` 认出可执行文件位于 `<prefix>/lib/<pkg>/`（或 `libexec`），且前缀是 `/usr`、`/usr/local`、AppImage 的 `$APPDIR/usr` 或 Flatpak 的 `/app` 时，从 `<prefix>/share` 与 `$XDG_DATA_DIRS` 中第一个含 `<pkg>/manifest` 的目录读取 `RuntimeResources` / `RuntimeManifest`，都没有时用 `<prefix>/share`。`/opt/<app>/` 这类自带目录仍按上表读同级 `runtime/`。自检的 `files.present` 按解析出的位置找 manifest 列出的每个文件。

Windows 应用根目录默认只允许放主程序 `.exe`；确因第三方 SDK 限制必须与 `.exe` 同目录的文件（如 Steamworks 的 `steam_api64.dll`），必须在 `platform.windows.root_exceptions` 中显式登记文件路径与原因。当 `distribution.backend = "portable"` 时，打包器会写入 `runtime/manifest/portable` 标记，运行时可写数据统一落入 `<root>/data`；安装版应用绝不往安装目录内写任何数据、缓存或日志。

## 3. 配置文件 `nana-package.toml` 与密钥传入

```toml
schema_version = 1

[application]            # 必须与编译进二进制的 NANA-IDENTITY-V1 标记完全一致
id = "dev.nana.fixture"
name = "Nana Fixture"
version = "0.1.0"
vendor = "Nana"

[executable]
file_name = "NanaFixture"            # Windows 下产出 NanaFixture.exe

[distribution]
backend = "steam"                    # steam | installer | portable
[distribution.steam]                 # 仅填公开 ID，严禁放入账号凭据
app_id = 480
depots = { windows = 481, macos = 482, linux = 483 }

[platform.macos]
minimum_system_version = "14.0"
url_schemes = ["nanafixture"]
document_types = [{ name = "Fixture Doc", extensions = ["nfx"], role = "Editor" }]
info_plist_extra = "packaging/Info.extra.plist"

[platform.windows]
root_exceptions = [{ source = "vendor/steam_api64.dll", reason = "Steamworks 从 exe 同目录加载" }]

[runtime]                            # source 可以是目录：整体复制并保留符号链接（.framework）
bin     = [{ source = "vendor/foo.dll", platforms = ["windows"] },
           { source = "vendor/Foo.framework", platforms = ["macos"] }]
tools   = [{ source = "vendor/ffmpeg" }]
resources = [{ source = "LICENSES.txt" }]   # 不进资源包的零散文件 → RuntimeResources
plugins = [{ name = "fx", source = "vendor/fx.plugin", abi = "nana-plugin-1", version = "1.0.0" }]

[resources]
root = "assets"                      # 包内逻辑路径 = 相对 root 的路径
block_size = 65536
compression = { codec = "zstd", level = 19 }
max_free_ratio = 0.25

[[resources.packs]]
name = "bootstrap"
class = "bootstrap-ui"
include = ["boot/**"]

[[resources.packs]]
name = "ui"
class = "protected"
include = ["ui/**"]
exclude = ["ui/**/*.psd"]
key = "content-main"
depends_on = ["bootstrap"]
max_bytes = 536870912

[keys.content-main]
generation = 1
availability = "process-start"       # embedded | process-start | after-bootstrap-ui

[signing]
publisher = { public_key = "ed25519:…", required = true }
platform = ["macos-codesign", "windows-authenticode"]
```

`backend = "installer"` 时，`[distribution.installer]` 决定构建哪些安装包，见第 7 节。

配置解析采用严格模式：未知字段一律报错；任何字段名若包含 `secret`、`private`、`token`、`password` 或 `credential` 会直接拒绝并提示安全传参渠道。密钥仅允许从环境变量（`NANA_CONTENT_KEY_<NAME>` / `NANA_PUBLISHER_SIGNING_KEY`）或 git 工作树之外的密钥文件（`--content-key-file NAME=PATH` / `--signing-key-file PATH`）读取；打包结束前还会全量扫描输出产物（覆盖原始字节、大小写 Hex、Base64 与 Base64Url），一旦发现任何密钥材料残留立即删除产物并报错退出。

## 4. `.nrpack` 资源包格式、加密与 Steam Delta 友好布局

### 启动资源分级（`ResourceClass`）

`resources.root` 下被匹配的文件必须归属唯一一个 pack（重复认领、仅大小写不同、含 `..` 或反斜杠均报错），运行时按最长路径前缀路由。资源包按启动时序分为三级（`depends_on` 只能指向同级或更早级别）：

| `class` | 启动约束与用途 |
| --- | --- |
| `early-splash` | **不加密**，总体积不超过 `1 MiB`（文件上限 `4 MiB`），不依赖任何其他 pack，供原生 Splash 直接读取 Logo |
| `bootstrap-ui` | 允许加密，但只能使用 `embedded` 或 `process-start` 阶段即可获得的密钥（若引用 `after-bootstrap-ui` 密钥会报 `StartupKeyCycle`，防止因等待 UI 登录取钥匙而死锁在空白窗） |
| `protected` | 普通业务资源，可使用任意阶段（含登录鉴权后下发的 `after-bootstrap-ui`）的内容密钥 |

### 二进制布局与八步 Fail-Closed 读取

```text
[Header: 4096 B][Data Region: 每个条目一段 16 B 对齐的 extent，空隙填零][TOC: 4096 B 对齐]
```

- **分块独立压缩与加密**：每个资源条目按 `block_size`（默认 64 KiB）切块，每块独立使用 `zstd` 压缩（打包端用 C 库高压缩率编码，运行时用纯 Rust `ruzstd` 解码；压缩后未变小则存原文），加密包再逐块使用 `XChaCha20-Poly1305`（随机 192-bit nonce，AAD 绑定 pack ID、条目键、块序号、密钥代次、明文长度与 flags）独立密封，TOC 同样整体加密密封。
- **八步严格读取顺序**：① 校验 Header 与保留位 → ② 按 `TrustPolicy` 校验 Ed25519 发布者签名 → ③ 核对 `package.json` 钉住的 `pack_id`、TOC 哈希、`class` 与密钥代次 → ④ 校验存储态 TOC 的 BLAKE3 → ⑤ 向 `KeyProvider` 取钥匙解密并认证 TOC → ⑥ 读取数据前先核对 `max_bytes` → ⑦ 按需定位目标块，逐块校验 `record_hash`、AEAD 解密并受限解压 → ⑧ 校验完整条目的 BLAKE3。任一步失败立即中止，**绝不解密落地到临时文件夹，也不每帧重复解密**。

### SteamPipe 增量更新（Delta）优化

通过 `--baseline <上一版 out>` 或 `--cache <DIR>` 打包时，未变化条目的密文块（含原 nonce）在原文件偏移（extent）处原样保留；修改后仍放得下的条目原地覆写并将尾部补零；新增或变大的条目填入空闲空洞（first-fit）或追加到末尾（仅当空洞率超过 `max_free_ratio` 或传入 `--compact` 时才重排紧凑化）。因此修改一个小资源只会改动 Header、该条目的 extent 与 TOC 所在的少量 1 MiB 块，最大化复用 SteamPipe 增量补丁（由 `crates/nana-packager/tests/steam_delta.rs` 门禁守卫）。

## 5. 应用侧挂载与最终产物防篡改校验（`validate`）

应用通过 `application_identity!` 宏在二进制中嵌入 `NANA-IDENTITY-V1` 身份标记，并通过 `NanaApplication::builder(identity).resource_packs(...)` 挂载资源包（开发期无 manifest 时自动回退读取 `loose_root` 目录；样式表、`@font-face` 与 `url()` 统一使用 `nana://res/...` 路径）。

发布前执行端到端校验与防篡改测试套件：

```bash
nana-packager validate "out/app/Nana Fixture.app" --trust-key ed25519:… --run --tamper-suite \
    --env NANA_FIXTURE_CONTENT_KEY=…
```

`validate` 会依次核验：
1. `package.json` 与所有 `.nrpack` 的 Ed25519 发布者签名（必须传入 `--trust-key`）；
2. 根目录单一入口约束、所有文件大小与 BLAKE3 哈希（平台签名后需加 `--allow-resigned`）、二进制身份标记一致性；
3. PE / Mach-O / ELF 动态库导入表可达性，以及 Steam 包不含自更新器标记（`NANA-UPDATER-V1`）；
4. `--run`：在无关工作目录下以 `NANA_PACKAGE_VALIDATE=1` 启动应用执行开窗前自检；
5. `--tamper-suite`：先跑未篡改对照组（`tamper.control`），再依次注入「破坏 manifest、破坏 pack Header、破坏 TOC、破坏首个真实数据块、删除 pack、剥离密钥环境变量」六类篡改，验证应用自检全部准确报错拦截且不崩溃、不挂起。

`package.json` 的 `schema_version` 是 2：版本 1 里一直记为 unavailable 的 `static_app_plan` 已删除（Issue #152）。schema 或版本不符的 manifest 在逐字段解析之前就以 unsupported schema 拒绝，所以旧版 packager 打的包要用当前版本重新打包。

## 6. 平台签名适配器

NanaUI 不重写 codesign、公证、Authenticode、GPG、apksigner 或鸿蒙 hap-sign-tool，每个适配器包一层平台工具，并如实报告这一次发生了什么。工具在这里跑不了——主机不对、找不到工具、缺凭据、没有可签的产物——一律报 `not-executed` 并写明原因，绝不报已签名。凭据只从 CI 环境变量读，不进 `nana-package.toml`、manifest 或报告；能经环境或 stdin 交给工具的口令就不放进命令行，只能走命令行的（signtool 的 PFX 口令、hap-sign-tool）在所有报告与错误里打码。

签名分两阶段：**应用阶段**在写完 manifest 之后原地签应用树（codesign 会把 manifest 一起封进 bundle，两者都会改写可执行文件，所以 manifest 记的是签名前哈希，校验时加 `--allow-resigned`）；**产物阶段**签安装包后端产出的文件，或经 `nana-packager sign` 签外部构建的文件。

| 适配器 | 阶段 | 主机 | 做什么 | 凭据与设置 |
| --- | --- | --- | --- | --- |
| `macos-codesign` | 应用 | macOS | 由内向外签 `Frameworks`、`PlugIns`、`Helpers`、`MacOS` 下的 Mach-O 与嵌套 bundle，再签 `.app`；正式身份带 hardened runtime 与时间戳；`codesign --verify --strict --deep` 校验 | `NANA_MACOS_SIGN_IDENTITY`（`Developer ID Application: …`，或 `-` 表示开发用 ad-hoc 签名）、可选 `NANA_MACOS_KEYCHAIN`；`[platform.macos] entitlements` |
| `macos-notarize` | 产物 | macOS | 有 `.dmg` 时签 DMG、提交、staple DMG；否则把 `.app` 打成 zip 提交并 staple `.app`；`stapler validate` 与 `spctl --assess` 校验。需要本次 codesign 用 Developer ID 签过 | `NANA_NOTARY_KEYCHAIN_PROFILE`，或 `NANA_NOTARY_API_KEY`（`.p8` 路径）+ `NANA_NOTARY_API_KEY_ID` + `NANA_NOTARY_API_ISSUER` |
| `windows-authenticode` | 应用、产物 | Windows | 应用阶段总签主 `.exe`，其余 PE 文件只签尚无有效签名的（第三方已签 DLL 保留原签名）；产物阶段签 `.msi` 与安装器 `.exe`；`signtool verify /pa /all` 校验 | `NANA_AUTHENTICODE_CERT_SHA1`（证书库指纹，推荐），或 `NANA_AUTHENTICODE_PFX` + `NANA_AUTHENTICODE_PFX_PASSWORD`；时间戳 `[platform.windows] timestamp_url` 或 `NANA_AUTHENTICODE_TIMESTAMP_URL`（缺了不签）；可选 `NANA_SIGNTOOL` |
| `msix` | 产物 | Windows | 签 `.msix` / `.appx`（打包器不产出 MSIX，用于外部构建的包） | 同 Authenticode |
| `linux-package` | 产物 | 任意有 gpg 的主机 | 给 `.deb`、`.AppImage`、`.flatpak` 生成分离的 ASCII 签名 `<文件>.asc`，`gpg --verify` 校验 | `NANA_GPG_KEY_ID`；可选 `NANA_GPG_PASSPHRASE`（经 stdin）、`NANA_GPG_HOMEDIR`、`NANA_GPG` |
| `android-apksigner` | 产物（仅 `sign` 命令） | 任意 | `apksigner sign`（v2+ 签名方案），`apksigner verify` 校验；口令以 `env:` 引用传入 | `NANA_ANDROID_KEYSTORE`、`NANA_ANDROID_KEY_ALIAS`、`NANA_ANDROID_KEYSTORE_PASSWORD`、可选 `NANA_ANDROID_KEY_PASSWORD`；`NANA_APKSIGNER` 或 `ANDROID_HOME` 下最新 build-tools |
| `ohos-hapsigner` | 产物（仅 `sign` 命令） | 任意有 Java 的主机 | `hap-sign-tool.jar sign-app`（localSign）后替换原 `.hap`，`verify-app` 校验 | `NANA_OHOS_HAP_SIGN_TOOL`、`NANA_OHOS_KEYSTORE`、`NANA_OHOS_KEYSTORE_PASSWORD`、`NANA_OHOS_KEY_ALIAS`、`NANA_OHOS_KEY_PASSWORD`、`NANA_OHOS_APP_CERT`、`NANA_OHOS_PROFILE`、可选 `NANA_OHOS_SIGN_ALG`、`NANA_JAVA` |

`signing.platform` 里只能列打包目标适用的前五个；`android-apksigner`、`ohos-hapsigner` 签的是打包器不产出的 APK / HAP，写进配置会报错并指向 `sign` 命令：

```bash
nana-packager sign --adapter android-apksigner app-release.apk
nana-packager sign --adapter macos-codesign --app "out/app/Nana Fixture.app"
```

`sign` 的退出码：0 表示每个适配器都已校验通过，2 表示有适配器没执行，1 表示失败。`package-report.json` 的 `platform_signing`（应用阶段）与 `artifact_signing`（产物阶段）记每个适配器的结果；manifest 只记配置了哪些适配器。`validate` 会重新调用适配器的校验。

## 7. 安装包后端

```toml
[distribution]
backend = "installer"

[distribution.installer]
formats = ["dmg", "msi", "nsis", "deb", "appimage", "flatpak"]   # 只构建属于打包目标的
summary = "一行简介"                 # 默认应用名
description = "多段说明"
homepage = "https://example.com"
maintainer = "Nana <dev@example.com>" # deb 的 Maintainer，默认 vendor 或应用名
linux_package = "nana-fixture"       # /usr/lib/<pkg>，默认由应用名推出
categories = ["Utility"]
deb_depends = ["libc6 (>= 2.31)"]
icon_png = "packaging/icon-256.png"  # 默认 Nana 标志
msi_upgrade_code = "{…}"             # 默认由应用 id 推出，发布后不要再改
flatpak_runtime = "org.freedesktop.Platform"
flatpak_runtime_version = "24.08"
flatpak_sdk = "org.freedesktop.Sdk"
```

| 格式 | 目标 | 由谁构建 | 在哪构建 |
| --- | --- | --- | --- |
| `dmg` | macOS | `hdiutil`：`ditto` 复制的 `.app`（保留签名、扩展属性与 framework 链接）加 `/Applications` 链接，UDZO 压缩，`hdiutil verify` 校验 | macOS |
| `msi` | Windows | 由应用树生成 WiX v4+ `.wxs`，`wix build`（`NANA_WIX`）。按机器安装到 `Program Files\<Name>`，开始菜单快捷方式，稳定 upgrade code 实现原地升级并拒绝降级 | Windows |
| `nsis` | Windows | 生成 `.nsi`，`makensis`（`NANA_MAKENSIS`）。卸载器只删自己装的文件和因此变空的目录，从不 `RMDir /r` | 任意有 makensis 的主机 |
| `deb` | Linux | 打包器自己用纯 Rust 写出 FHS 布局的 `.deb`，写完回读校验结构，有 `dpkg-deb` 时再用它检查 | 任意主机 |
| `appimage` | Linux | 生成 AppDir（`AppRun` → `usr/bin/<exe>`、`<id>.desktop`、图标、FHS 的 `usr/`），`appimagetool`（`NANA_APPIMAGETOOL`） | Linux |
| `flatpak` | Linux | 生成 `flatpak-builder` 清单（一个模块把暂存树复制进 `/app`），`flatpak-builder` 后 `flatpak build-bundle`；构建机需装好 runtime 与 SDK | Linux |

产物写到 `<out>/installer/`。工具跑不了的格式报 `not-executed`，生成的 `.wxs`、`.nsi`、AppDir、Flatpak 清单留在 `installer/staging/<格式>/`，可以挪到对的主机上完成构建；工具跑了却失败则整个打包失败。`package-report.json` 的 `installers` 记每个格式的文件、大小、BLAKE3 与检查项。`backend = "installer"` 不写 `[distribution.installer]` 时只产出应用树并给出警告。`validate` 只校验应用树；在目标系统上安装、启动、卸载安装包尚未自动化（报告为 `installer.round-trip` not executed）。

## 8. Android APK 资源

APK 里的资源不是文件系统路径。`nana-packager android-assets APP_ROOT --out app/src/main/assets` 把某个打包输出的 `package.json`（及签名）和所有 `.nrpack` 复制到 `assets/nana/manifest/` 与 `assets/nana/resources/`。APK 必须不压缩地存放 `.nrpack`：

```kotlin
android { androidResources { noCompress += "nrpack" } }
```

Android 上 `nana-ui/packaged-resources` 先经应用 `Context` 的 `AAssetManager` 读 `assets/nana/manifest/`，签名按 `TrustPolicy` 校验（`PackageManifest::from_signed_bytes`）；有就从 APK 挂载 pack，没有再回落到 `RuntimeManifest`。每个 pack 经 `AAsset_openFileDescriptor64` 取得 APK 的描述符与该资源的字节区间，以 `FileRange` 交给 `PackReader::open_source`，与桌面同样按需定位读取、同样八步校验，不解出临时文件。被压缩的 pack 拿不到描述符，会以错误报告。`PackagedResourceSource` 契约不变。

## 9. 更新清单与回滚日志

NanaUI 不带更新器，也不带更新界面；`nana_package::update` 是应用自己的更新器（或商店）所依赖的契约：

- **更新清单** `update.json` + `update.json.sig`：目标应用 id、版本、渠道、目标三元组、可选的 `minimum_from_version`、所安装 `package.json` 的 BLAKE3，以及每个安装包文件的格式、大小与 BLAKE3。由签 package manifest 的同一把发布者密钥签名，签名上下文与 package manifest 分开，二者不能互相冒用。`nana-packager update-manifest APP_ROOT --channel stable --artifact out/installer/… --out DIR` 生成；没有发布者密钥就不生成——更新不存在不签名的情形。
- **校验** `UpdateManifest::verify(bytes, sig, installed, policy)`：先验签名再解析，然后要求应用 id、目标三元组、渠道一致，版本**严格新于**已安装版本（按 SemVer 排序，预发布早于正式版），且已安装版本不低于 `minimum_from_version`。合法签名的旧版本因此不会被当作更新重放回来。下载的文件用 `check_artifact` 流式核对大小与哈希。
- **回滚日志** `UpdateJournal`：更新器替换文件前记下 previous / pending 两个版本（`staged`），替换后 `activated`。应用每次启动在开窗前调用 `begin_launch` 并立即落盘：运行的是 pending 版本时计一次未确认启动，返回 `AwaitConfirmation`；应用到达健康点（首个 `UiReady`）调用 `confirm`。pending 版本已有 `max_unconfirmed_launches`（默认 2）次启动没能确认时，下一次启动返回 `RollBack { restore }`，由应用自己的恢复钩子还原旧版本后调用 `rolled_back`。日志放在哪、怎么还原文件归应用。

`distribution.self_update` 仍然拒绝：打包器不产出更新器，Steam 与安装包都不带 `NANA-UPDATER-V1`。打包后真实窗口里 Early Splash → UiReady 的交接验证见 Issue #273。
