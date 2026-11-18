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
