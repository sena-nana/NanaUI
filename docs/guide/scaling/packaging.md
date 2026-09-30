# 打包

发行物是一个主进程、一个可执行文件。打包不引入常驻启动器，也不查找 A/B 目录。Packager 不调用 cargo。你先构建，再打包，再校验。

```text
cargo build --profile dist
        │
nana-packager package
        │
<out>/app/
        │
nana-packager validate --run --tamper-suite
```

三块分工：`nana-package` 在应用里读 manifest 和 `.nrpack`（经 `nana-ui` 的 `packaged-resources`，默认不开）；`nana-packager` 只是构建工具，应用不能依赖它；`nana-ui-core::packaged` 负责 `nana://res/` 路径。边界由 `scripts/check-package-boundary.py` 守着。外部宿主可以完全不用这套打包。

## 安装布局

业务代码从 `ApplicationPaths` 取逻辑位置，不要自己拼 `./runtime/...`。

| 逻辑位置 | Windows / Linux | macOS |
| --- | --- | --- |
| 应用根 | `App/` | `Name.app` |
| 可执行文件 | `App.exe` / `App` | `Contents/MacOS/App` |
| RuntimeBin | `runtime/bin` | `Contents/Frameworks` |
| RuntimeResources | `runtime/resources` | `Contents/Resources` |
| RuntimePlugins | `runtime/plugins` | `Contents/PlugIns` |
| RuntimeTools | `runtime/tools` | `Contents/Helpers` |
| RuntimeManifest | `runtime/manifest` | `Contents/Resources/manifest` |

Windows 根目录放一个 EXE。必须摆在旁边的文件（例如 `steam_api64.dll`）写进 `platform.windows.root_exceptions`，并写上原因。`backend = "portable"` 会写入 `runtime/manifest/portable`，可写数据落到 `<root>/data`。安装版的数据、缓存、日志和崩溃文件不写进安装目录。

## 资源包

`nana-package.toml` 的 `schema_version = 1`。未知字段会被拒绝。字段名里带 secret、private、token、password 或 credential 直接报错。

内容密钥从环境变量 `NANA_CONTENT_KEY_<NAME>`（64 位十六进制）或 `--content-key-file NAME=PATH` 读取。发布者签名种子从 `NANA_PUBLISHER_SIGNING_KEY` 或 `--signing-key-file PATH` 读取。密钥文件放在配置所在的 git 工作树里会被拒绝。

`resources.root` 下每个文件恰好进一个 pack。没人认领的文件不打包。同一文件被认领两次、路径只有大小写不同、出现 `..`、反斜杠或控制字符，都会报错。运行时按最长前缀把查找送到唯一一个 pack。

| class | 约束 |
| --- | --- |
| `early-splash` | 不加密，不超过 1 MiB，不依赖其它 pack |
| `bootstrap-ui` | 可以加密，只用 `embedded` 或 `process-start` 的钥匙 |
| `protected` | 普通业务资源 |

`bootstrap-ui` 如果用了 `after-bootstrap-ui` 的钥匙，报 `StartupKeyCycle`。`depends_on` 只能指向同类或更早的类。`SplashLogo::packaged("nana://res/…")` 从 `early-splash` pack 读 Logo。reader 不调用 `KeyProvider`；pack 超过 4 MiB（`EARLY_SPLASH_MAX_PACK_BYTES`）报 `EarlySplashTooLarge`，不读 TOC。读数据之前再按 TOC 检查 1 MiB 上限。

`.nrpack` 是 v1 小端：4096 字节的 header，数据区每个 entry 一段 16 字节对齐的 extent，TOC 按 4096 对齐。块用 zstd（打包端 C 库，运行时用 `ruzstd`）。加密是 XChaCha20-Poly1305，随机 192-bit nonce。发布者用 Ed25519 签 header 和 `package.json`，应用里只有公钥。读取按八步失败即停，不解密到临时目录，也不每帧解密。客户端自己持有的钥匙只能挡住随手解包。

`--baseline` 复用上一版没变的密文。`--cache` 按明文哈希复用 record。`generation` 加 1 会整包重做，报告里 `key_rotation: true`。丢掉上一版 `out/` 或 cache 的 CI，每块都会拿到新 nonce。

## 应用里挂上

身份宏在二进制里留下 `NANA-IDENTITY-V1`。字段不合法时编译失败。开启 `packaged-resources` 之后，`ResourcePackOptions` 从 `nana_ui` 导出，`nana_package::TrustPolicy` 也经 `nana_ui::nana_package` 可用。`NanaApplication` 不依赖这个 feature。

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
            .trust(nana_package::TrustPolicy::RequirePublisher(PINNED_KEY))
            .loose_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")),
    )
    .start();
```

样式、`@font-face`、`url()` 和 Vue 的 `@import` 写 `nana://res/ui/app.css`。开发期没有 manifest 时，同一套逻辑路径从 `loose_root` 读。安装版和便携版里，没有显式 base 的相对 `url()` 以 `runtime_resources` 为基准。开发与嵌入宿主仍以当前工作目录为基准。每个 pack 在第一次被查到时才打开。

## 发布前校验

```bash
nana-packager validate "out/app/Nana Fixture.app" --trust-key ed25519:… --run --tamper-suite \
    --env NANA_FIXTURE_CONTENT_KEY=…
```

它会核对：manifest 签名（没给 `--trust-key` 就报 not executed）；签名 adapter 的 verify（目前一律 not executed）；根目录只有可执行文件、`runtime/` 和登记过的例外；文件大小和哈希与 manifest 一致；二进制身份标记；依赖能不能被加载器找到；每个 pack 按 manifest 读一遍；没有 updater 文件，也没有 `NANA-UPDATER-V1`。`--run` 在无关目录下以 `NANA_PACKAGE_VALIDATE=1` 启动，应用在开窗前打印一行 JSON 后退出。`--tamper-suite` 先跑未改过的对照，再分别破坏 manifest、header、TOC、首个真实 record，删掉 pack，并用 `--tamper-drop-env` 去掉钥匙。应用声明了 packaged Logo 时，还会从 `early-splash` pack 读它并检查 PNG 头。

平台签名会改写可执行文件。签名后的包加 `--allow-resigned`，这些哈希变化降为 warn。`--run` 只能在与包同平台的主机上执行。

CLI 还有 `package`、`keygen content|publisher`、`inspect`、`delta OLD NEW`、`macos-app`。`macos-app` 只生成一个没有资源包、没有 manifest 的 `.app`。

还没有的：平台签名 adapter、安装包后端、可选自更新、打包校验里的真窗口 Early Splash / UiReady、StaticAppPlan 指纹、从 pack 读取 JS / Vue bundle、Android AssetManager、Linux FHS 路径、分页 TOC。

字段、读取顺序和每一项检查在 [打包与分发](../../reference/packaging.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/performance">
    <p class="next-step-link">性能</p>
    <p class="next-step-caption">没有变更时不刷帧，一条指针事件花多少。</p>
  </a>
  <a class="next-step" href="/guide/scaling/fetch">
    <p class="next-step-link">Fetch</p>
    <p class="next-step-caption">url() 和页面请求共用的宿主白名单。</p>
  </a>
  <a class="next-step" href="/guide/scaling/hot-reload">
    <p class="next-step-link">热重载</p>
    <p class="next-step-caption">开发监听停在 debug，不进 dist。</p>
  </a>
</div>
