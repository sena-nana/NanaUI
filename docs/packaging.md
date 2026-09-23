# 打包与分发

Issue #226 的框架合同。它规定了一次打包产出什么、资源怎么装进包、谁持有哪把钥匙，以及怎么证明交付物能用。

发行以 Steam 和独立安装包为主，应用内自更新只是次要能力。正常启动始终只有一个主进程、一个 EXE：打包能力不引入常驻启动器、不增加 IPC，也不需要查找 A/B 版本目录。

```text
cargo build --profile dist            （#158 落地后由 dist pipeline 调用）
        │  已构建的可执行文件
        ▼
nana-packager package                 读 nana-package.toml、资源目录
        │  pack plan → .nrpack → 平台布局 → manifest → 签名状态 → Steam 元数据
        ▼
<out>/app/                            交付物
        │
nana-packager validate --run --tamper-suite
```

## 组成

| crate | 作用 | 进入应用？ |
| --- | --- | --- |
| `nana-package` | manifest 读取、`.nrpack` 格式与只读 reader、`KeyProvider`、发布者签名校验、身份标记解析 | 是，经 `nana-ui/packaged-resources`；只用纯 Rust 依赖，不含 C 库和随机数源 |
| `nana-packager` | 配置、pack plan、打包、平台布局、Steam 输出、签名 adapter 合同、最终产物校验、CLI | 否，只是构建工具 |
| `nana-ui-core::packaged` | `nana://res/` 路径规范化，以及进程级 `PackagedResourceSource` 钩子 | 是，不带额外依赖 |

`scripts/check-package-boundary.py` 守住以下边界：

- `nana-package` 不能依赖 zstd C 库、ring、rustls、getrandom 或 rand。
- `blake3` 必须用 `pure` 后端。
- 可嵌入的 Runtime 类 crate 不能依赖打包相关 crate。
- 除 dev 依赖外，任何成员都不能依赖 `nana-packager`。

外部宿主可以完全不用这套打包（Issue #226 §14），`nana-ui` 默认不开 `packaged-resources`。

Packager 不调用 cargo，只接收已经构建好的 `--exe`。所以 #158 的 dist pipeline 落地后仍是唯一的构建入口，打包只是其中一步，不会出现两条互相竞争的发布流水线。

## 逻辑位置与平台布局

业务代码从 `ApplicationPaths` 取逻辑位置，不要自己拼 `./runtime/...`。见[诊断与应用路径](diagnostics.md)。Packager 往这些位置放文件，有测试保证它与运行时的解析一致（`layout_agrees_with_application_paths`）。

| 逻辑位置 | Windows / Linux | macOS |
| --- | --- | --- |
| 应用根 | `App/`（其中只有 `App.exe` 与 `runtime/`） | `Name.app` |
| 可执行文件 | `App.exe` / `App` | `Contents/MacOS/App` |
| RuntimeBin | `runtime/bin` | `Contents/Frameworks` |
| RuntimeResources | `runtime/resources` | `Contents/Resources` |
| RuntimePlugins | `runtime/plugins` | `Contents/PlugIns` |
| RuntimeTools | `runtime/tools` | `Contents/Helpers` |
| RuntimeManifest | `runtime/manifest` | `Contents/Resources/manifest` |

Windows 根目录原则上只放一个 EXE。确实受平台或第三方约束的文件（例如 Steamworks 要求 `steam_api64.dll` 放在 exe 旁边）要写进 `platform.windows.root_exceptions`，并注明原因；原因会写进 manifest，validator 也按这份清单检查。

`backend = "portable"` 会写入 `runtime/manifest/portable` 标记，可写数据随之落到 `<root>/data`。安装版的数据、缓存、日志和崩溃文件都不会写进安装目录。

## nana-package.toml

```toml
schema_version = 1

[application]            # 必须与二进制里的身份标记一致
id = "dev.nana.fixture"
name = "Nana Fixture"
version = "0.1.0"
vendor = "Nana"

[executable]
file_name = "NanaFixture"            # Windows 上得到 NanaFixture.exe

[distribution]
backend = "steam"                    # steam | installer | portable
[distribution.steam]                 # 只填 id，不放凭据
app_id = 480
depots = { windows = 481, macos = 482, linux = 483 }

[platform.macos]
minimum_system_version = "11.0"
url_schemes = ["nanafixture"]
document_types = [{ name = "Fixture Doc", extensions = ["nfx"], role = "Editor" }]
info_plist_extra = "packaging/Info.extra.plist"   # 最后合并；同名键以它为准，身份相关键不可覆盖

[platform.windows]
root_exceptions = [{ source = "vendor/steam_api64.dll", reason = "Steamworks 从 exe 目录加载" }]

[runtime]
bin     = [{ source = "vendor/foo.dll", platforms = ["windows"] }]
tools   = [{ source = "vendor/ffmpeg" }]
plugins = [{ name = "fx", source = "vendor/fx.plugin", abi = "nana-plugin-1", version = "1.0.0" }]

[resources]
root = "assets"                      # 逻辑路径 = 相对 root 的路径
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

每张表都拒绝未知字段。名字里带 secret、private、token、password 或 credential 的字段直接报错，并提示正确的传入方式。密钥只从以下渠道读取：

- 内容密钥：环境变量 `NANA_CONTENT_KEY_<NAME>`（64 位十六进制），或命令行 `--content-key-file NAME=PATH`。
- 发布者签名种子：环境变量 `NANA_PUBLISHER_SIGNING_KEY`，或命令行 `--signing-key-file PATH`。

密钥文件如果放在配置所在的 git 工作树里，会被拒绝。

## 资源包 `.nrpack`

### 分组与启动类

pack plan 把 `resources.root` 下的每个文件分给恰好一个 pack。没有被任何 pack 认领的文件不打包。以下情况会报错：

- 同一文件被两个 pack 认领；
- 路径只有大小写不同；
- 出现 `..`、反斜杠或控制字符。

pack 在 manifest 里记录其路径前缀；运行时按最长前缀把查找路由到唯一一个 pack。

资源按启动依赖分为三类（§7）：

| class | 约束 |
| --- | --- |
| `early-splash` | 不加密，不超过 1 MiB，不依赖其它 pack |
| `bootstrap-ui` | 可以加密，但只能用 `embedded` 或 `process-start` 的钥匙 |
| `protected` | 普通业务资源 |

- `bootstrap-ui` 如果用了 `after-bootstrap-ui` 的钥匙，报 `StartupKeyCycle`。这把钥匙只能靠这个 UI 驱动的流程取得，应用会永远卡在空白窗口。
- `depends_on` 只能指向同类或更早的类。DFS 会报出依赖环。

目前 EarlySplash 只是一个类别，Early Splash 本身还没实现（#225）。

### 格式（v1，小端）

```text
[header 4096 B][数据区：每个 entry 一段 16 字节对齐的 extent，其余补零][TOC，4096 对齐]
```

header 的字段表在 `crates/nana-package/src/pack/format.rs` 开头，包括：

- magic、版本；
- flags（加密、签名）；
- `pack_id = BLAKE3(app_id, pack_name)` 的前 16 字节（跨构建稳定）；
- 块大小、class；
- 钥匙 id 与代次；
- 条目数、TOC 偏移与长度、数据区终点；
- stored TOC 的 BLAKE3；
- 发布者钥匙 id；
- Ed25519 签名，覆盖 header 前 192 字节。

TOC 由三部分组成：

- 96 B 定长条目，按 key 字节序排序。查找是二分，启动时不逐条解析。
- 32 B 块记录，每块带 `record_hash`。
- 字符串表。

每个 entry 按 `block_size` 切块，逐块单独压缩（zstd；压不小就存原文），加密时再逐块单独 seal。读一个资源只碰它自己的块。

### 读取顺序（fail-closed）

1. header 与保留位；
2. 签名（按 `TrustPolicy`）；
3. manifest 钉住的 pack id、TOC hash、class 与代次；
4. TOC hash；
5. 向 `KeyProvider` 取钥匙，认证并解密 TOC（AAD 含 header 前 88 字节，未签名的加密 pack 改了 header 也会失败）；
6. 在读任何数据之前检查 `max_bytes`；
7. 每块依次：`record_hash` → AEAD → 受限窗口的 zstd 解压；
8. 整个 entry 的 BLAKE3。

以上全部通过才把字节交给 decoder，失败一律返回错误。不会解密到临时目录，也不会每帧解密。

## 加密与签名

| 能力 | 原语 | 说明 |
| --- | --- | --- |
| 压缩 | zstd（打包端 C 库，运行时用纯 Rust `ruzstd` 解码） | 逐块压缩，不整包压缩 |
| 加密 + 认证 | XChaCha20-Poly1305（RustCrypto） | 随机 192-bit nonce；块 AAD 绑定 pack、entry key、块序号、钥匙代次、明文长度与 flags，块不能挪位或替换 |
| 元数据保护 | 同上，TOC 整体 seal | 没有钥匙时连 entry 名字都看不到 |
| 发布者签名 | Ed25519（strict 校验） | 签 pack header 与 `package.json`；与内容密钥分离，只有公钥进入应用 |
| KeyProvider | `nana_package::KeyProvider` | 每个 pack 挂载时调用一次；怎么拿到钥匙是应用或分发渠道的策略（内嵌、授权服务、Steam API） |

**客户端持有的钥匙只能防随手提取。** `embedded` 和 `process-start` 的钥匙随程序发给每个用户，AEAD 只能挡住直接解包。真正的机密性需要从服务端取钥匙（`after-bootstrap-ui`），而按上面的规则，这种钥匙不能用来保护 BootstrapUI。防篡改靠发布者签名，它的信任根（钉住的公钥）必须编译进经过平台签名的二进制。

## Steam 与 delta

Steam 包不带 Nana 自更新器，也不保留旧版本；版本部署、回滚和分支都由 SteamPipe 负责。`<out>/steam/` 下会生成：

- 不含凭据的 `depot_build_<depot>.vdf`，ContentRoot 指向 `app/`；
- 带文件哈希的 `nana-steam-build.json`。

资源小改动只应改动 pack 的有限区域，为此 packager 把 `--baseline <上一版 out>` 同时当作布局来源和密文来源：

- 内容未变的 entry：原位保留原字节（及原 nonce）。
- 变了但还放得下：在原 extent 里重写，多出的尾部补零。
- 放不下或新增：放进空洞（first-fit），不够就追加到末尾。
- 删除：清零。
- 空闲率超过 `max_free_ratio`，或传了 `--compact`：整理重排，报告里标明（此时 delta 会很大）。
- TOC 明文和 header 都没变：连 sealed TOC 一起复用，重打包逐字节一致。

`--cache DIR` 是 artifact cache，以（pack、key、明文 hash、codec、块大小、钥匙 id 与代次）为键。没有 baseline 的机器也能按字节复用未变的 record。缓存命中前会完整校验，缓存被改坏只会导致重新编码。

同一把钥匙、同一 nonce、同一明文原样复用不属于 nonce 重用；entry 改名或钥匙换代时一律用新 nonce 重新 seal。换代（`generation` + 1）必然导致全量 delta，报告里 `key_rotation: true`。

**发布流程必须保留上一版产物，或者持久化 cache。** CI runner 用完即弃的话，每块都会拿到新 nonce，整包都会变。

`crates/nana-packager/tests/steam_delta.rs` 是 delta 门禁，数据写到 `target/performance/issue226/`。它用 1 MiB 定长块近似 SteamPipe 的 chunk；真实补丁大小要用带凭据的 steamcmd preview build 才能测，不在这个仓库里。

| 场景 | 门禁 |
| --- | --- |
| 无变化 + baseline | 字节一致，重新编码 0 字节 |
| 只有 cache | 所有 record 字节一致 |
| 改一个小资源 | 变化的 chunk ≤ header + 该 extent + 新旧 TOC 所跨 chunk；重新编码 ≤ 该 extent |
| 资源变大后搬家 / 新增 / 删除 | 同上，加上旧、新 extent |
| 钥匙换代 | 允许全量，必须标 `key_rotation` |

## manifest

`<RuntimeManifest>/package.json` 可附带 `package.json.sig`（Ed25519 签名，覆盖 `nana.package-manifest.v1\0` 加上文件字节）。manifest 记录：

- 应用身份与 build id、target triple；
- `nana-package` 版本、构建 profile、是否 debug 构建；
- StaticAppPlan：#158 落地前明确写 `unavailable`，不编造；
- 分发后端与 `self_update`；
- 逻辑布局与根目录例外；
- 每个 pack 的 id、TOC hash、class、前缀、加密方式、钥匙名 / id / 代次、是否签名、依赖；
- 插件 ABI / 版本；
- 每个文件的大小与 BLAKE3；
- 签名报告。

每个结构都拒绝未知字段，也没有自由格式的 map，manifest 里放不进秘密。打包结束前还会扫描全部产物，查找已加载的每把钥匙和种子，覆盖原始字节、大小写十六进制、base64 与 base64url。只要命中一处，这次打包就失败。

运行时读取：`NanaApplicationBuilder::resource_packs(options)` 在 `start()` 时读 manifest 并按 options 校验签名，身份不符记 `package.identity_mismatch` 故障；之后 `NanaApplication::package_manifest()` 可以取到它。

**平台签名在 manifest 之后。** Authenticode、codesign、SteamStub 都会在 manifest 生成之后改写 exe，所以 manifest 里 exe 和库的哈希标为 `pre_platform_signing`。codesign 会封印 `Contents/Resources`，因此 pack 与 manifest 必须在签名之前定稿：packager 先写 manifest，再跑平台签名 adapter；manifest 只列出配置了哪些 adapter，实际结果写在 `package-report.json`。

## 应用侧

```rust
let identity = nana_ui_platform::application_identity!(
    id: "dev.nana.fixture",
    name: "Nana Fixture",
    version: env!("CARGO_PKG_VERSION"),
    vendor: "Nana",
);
let _session = NanaApplication::builder(identity)
    .resource_packs(
        ResourcePackOptions::new()
            .keys(my_key_provider)
            .trust(TrustPolicy::RequirePublisher(PINNED_KEY))
            .loose_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")),
    )
    .start();
```

- `application_identity!` 在二进制里留下 `NANA-IDENTITY-V1` 标记。字段不合法时直接编译失败；标记能扛住 fat LTO 与 strip。packager 拿它和配置比对，不一致拒绝打包；validator 拿它和 manifest 比对。
- 样式、`@font-face`、`url()` 与 Vue 的 `@import` 直接写 `nana://res/ui/app.css`。packaged 样式表里的相对引用在包内解析，不会逃到文件系统。
- 开发期（从 `target/` 运行）没有 manifest 时，同一套逻辑路径从 `loose_root` 目录读取。
- 安装版和便携版里，没有显式 base 的相对 `url()` 以 `runtime_resources` 为基准，不再以 CWD 为基准；开发与嵌入宿主保持 CWD。
- 每个 pack 在第一次被查到时才打开。启动时只读 manifest，不扫描全部 pack。

## 最终产物校验

```bash
nana-packager validate "out/app/Nana Fixture.app" --trust-key ed25519:… --run --tamper-suite \
    --env NANA_FIXTURE_CONTENT_KEY=…
```

| 检查 | 内容 |
| --- | --- |
| `manifest.signature` / `signing.publisher` | 按 `--trust-key` 校验；没给 key 时报 not executed，不报 pass。manifest 自述的签名状态不算证据 |
| `signing.<adapter>` | validate 当场调用 adapter 的 verify；目前 adapter 未实现，一律报 not executed |
| `layout.single-root-entry` | 根目录只有可执行文件、`runtime/` 和登记过的例外；macOS 的 `Contents/MacOS` 里只有主程序 |
| `files.manifest-match` | 每个文件的大小与哈希一致，且没有 manifest 之外的文件。平台签名会改写可执行文件和库，签名后的包要加 `--allow-resigned`：这些文件的哈希变化降为 warn，签名本身须用 `codesign --verify` / `signtool verify` 另行验证 |
| `identity.binary-matches-manifest` | 二进制里的身份标记与 manifest 一致 |
| `dependencies.discoverable` | 按 PE / Mach-O / ELF 导入表检查：Windows 隐式导入不能指向 `runtime/bin`（加载器不去那里找，需要 delay-load 加 `AddDllDirectory`，或登记为根例外）；macOS 的依赖必须在 bundle 内；Linux 的 `NEEDED` 需要 `$ORIGIN/runtime/bin` RUNPATH |
| `pack.*` | 按 manifest 钉住并完整读一遍（加密 pack 需要 `NANA_CONTENT_KEY_<NAME>`，否则报 not executed） |
| `distribution.no-updater` | 没有 updater 文件，也没有 `NANA-UPDATER-V1` 标记；Steam 包命中即失败 |
| `run.launch-from-foreign-cwd` | 在无关目录下以 `NANA_PACKAGE_VALIDATE=1` 启动；应用在开窗前自检（布局、manifest、身份、每个 pack 经 `nana://res/` 读一条），打印一行 JSON 后退出 |
| `tamper.*` | 先跑未篡改的副本作对照（`tamper.control`，必须通过，否则后面的用例没有意义）；再分别破坏 manifest、pack header、TOC、首个真实 record（按 TOC 定位，不是补零区），删掉 pack，以及 `--tamper-drop-env NAME` 去掉应用的钥匙来源。每种都必须由应用自检报出 `"ok":false`；崩溃或卡死算失败 |
| not executed | EarlySplash、UiReady 交接（#225）、安装包往返（F）、StaticAppPlan（#158）：明确报未执行 |

`--run` 只能在与包同平台的主机上执行，交叉目标报 not executed。PR CI 用 `--release` 构建 fixture；真正的 `dist`（fat LTO）产物放到定时或发布流程里校验。

## 输出目录与报告

`--out` 下的 `.nana-packager-out` 标记这个目录由 packager 创建，只有带这个标记的目录才会被清空重建；失败后可以直接重跑。泄密扫描命中时，`app/` 与 `steam/` 会被删除，也不写报告。`package-report.json` 按 pack 记录：条目数、pack 大小、源字节、相对 baseline 新增或改动的源字节（`changed_source_bytes`）、本次重新压缩和加密的字节（`encoded_bytes`，即本地重建成本）、复用字节、cache 命中、原位保留 / 改写 / 搬迁 / 新增 / 删除的条目数、空闲字节、是否整理、是否换钥匙、耗时，还有相对 baseline 的 Steam 式 delta。压缩参数相对 baseline 变了会警告（未变的条目也要重新编码）。

## CLI

```text
nana-packager package   --config nana-package.toml --exe PATH --out DIR [--target TRIPLE]
                        [--baseline PREV_OUT] [--cache DIR] [--profile dist] [--build-id ID]
                        [--compact] [--content-key-file NAME=PATH]...
                        [--signing-key-file PATH]
nana-packager validate  APP_ROOT [--trust-key ed25519:HEX] [--run] [--tamper-suite] [--env K=V]...
                        [--tamper-drop-env NAME]... [--allow-resigned] [--json]
nana-packager keygen    content|publisher --out PATH     （文件权限 0600，不覆盖已有文件）
nana-packager inspect   PACK [--content-key-file NAME=PATH]
nana-packager delta     OLD NEW
nana-packager macos-app --exe PATH --name NAME --identifier ID --out DIR [--icon ICNS] [--no-strip]
```

`macos-app` 取代原来的 `nana-package-app`，只生成一个裸 `.app`：没有资源包也没有 manifest，版本号取自二进制的身份标记。

## 尚未实现

- 平台签名 adapter（codesign、公证、Authenticode、MSIX、Linux 包签名）：已有合同与状态上报，实际一律报 `NotExecuted`，打包后请用平台工具签名。
- 安装包后端（MSIX / MSI / dmg / pkg / deb / AppImage），以及它的往返校验。
- 可选自更新（G）。它的标记 `NANA-UPDATER-V1` 已经保留，不允许出现在 Steam 包里。
- EarlySplash 的实现，以及 UiReady 交接（#225）。
- StaticAppPlan 指纹（#158）。
- JS / Vue 的应用 bundle 还不从 pack 读取；`nana://app/` 与 `nana://res/` 是否互为别名尚未决定。
- Android APK 的 assets 不是文件，pack 挂载需要一个 AssetManager 数据源。
- Linux FHS 安装（exe 在 `/usr/bin`）没有路径映射。`$ORIGIN/runtime/bin` 只能在链接期加入，打包之后补不上。
- TOC 目前只有单一区域：条目数到万级（TOC 超过 1 MiB）时，每次变化都会跨越多个 chunk。分页 TOC 留给 v2。
