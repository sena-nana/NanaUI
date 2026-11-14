# 测试与门禁

NanaUI 对测试目标数量、调试符号体积、并发隔离与反卡死规则设有硬性约束，目的在于控制本地与 CI 的链接开销，并防止多测试线程并发争抢宿主 GPU、窗口系统或全局进程状态导致桌面卡死。

门禁检查脚本位于 [check-test-hygiene.py](https://github.com/sena-nana/NanaUI/blob/main/scripts/check-test-hygiene.py)，`cargo-nextest` 超时与串行分组配置位于 [nextest.toml](https://github.com/sena-nana/NanaUI/blob/main/.config/nextest.toml)。

## 构建配置与目标卫生

- **调试符号与依赖优化（`Cargo.toml`）**：
  - `[profile.dev]` 与 `[profile.test]` 统一设置 `debug = "line-tables-only"`，`[profile.dev.package."*"]` 设置 `debug = false`，在保留 panic 行号回溯的同时避免全量 DWARF 链接膨胀。
  - 加解密与压缩叶子包（`chacha20`、`poly1305`、`blake3`、`ruzstd`、`zstd-sys`）在 `profile.dev` 下保留 `opt-level = 3`，确保 `nana-packager` 与打包解包测试不会在未优化路径下严重退化。
- **收敛可测试目标**：
  - 除 `scripts/check-test-hygiene.py` 中 `ALLOWED_TEST_BINS` 与 `ALLOWED_TEST_EXAMPLES` 明确豁免的基准与探针目标外，所有 `[[bin]]` 与 `[[example]]` 都必须设置 `test = false`。
  - 纯示例、夹具与校验工具 crate（如 `examples/component-gallery`、`examples/reactive-sfc`、`examples/runtime-host-fixture`、`tools/css-parity`、`third_party/wgpu-fork-tests`）在 `[lib]` 上设置 `doctest = false`。
- **集成测试数量上限与域分组**：
  - 每个包最多允许 **4** 个集成 `[[test]]` harness（`MAX_TESTS_PER_PACKAGE = 4`），整个 workspace 最多允许 **30** 个集成 `[[test]]` harness（`MAX_TESTS_WORKSPACE = 30`）。
  - 新增测试用例应并入既有域 harness（如 `nana-ui-runtime` 的 `contracts` / `perf_contracts`、`nana-packager` 的 `packager`），不要按单个 issue 新建集成测试二进制。
- **禁止按 issue 编号命名测试文件**：
  - 任何 `tests/` 目录以及 `crates/nana-ui-runtime/src/world/` 下禁止出现 `issue<数字>*.rs` 文件；运行时布局回归按主题组织在 `crates/nana-ui-runtime/src/world/tests/` 下。

## 反卡死、隔离与规模分级

`scripts/check-test-hygiene.py` 会静态扫描 `crates/`、`examples/`、`tools/`、`third_party/` 下的测试代码并强制执行以下规则：

1. **禁止无界 GPU 等待**：测试代码禁止调用 `wgpu::PollType::wait_indefinitely()`，也不得使用未指定 `timeout: Some(...)` 的 `PollType::Wait`（通常限定在 10 秒以内）。
2. **禁止默认路径长阻塞休眠**：非 `#[ignore]` 测试路径中不得出现 `Duration::from_secs(N)` 且 `N >= 2` 的阻塞 `sleep`。
3. **临时目录必须防并发碰撞**：测试代码若同时使用 `env::temp_dir()` 与 `process::id()`，必须搭配单调递增原子计数器（`AtomicUsize` / `AtomicU64` / `AtomicU32` 或 `unique_test_suffix(`），防止同一进程内的并发测试线程写入同一路径。
4. **禁止测试内嵌套 `cargo` 构建/运行**：测试代码不得派生执行 `cargo build`、`cargo test` 或 `cargo run`，避免锁竞争与嵌套编译阻塞。
5. **100k 级发布态压测默认 `#[ignore]`**：十万节点或大规模渲染压测须标为 `#[ignore]`，按需在 `--release -- --ignored` 下显式执行；默认测试输入保持有界规模。

## `cargo-nextest` 分组与超时策略

`.config/nextest.toml` 为 `cargo-nextest` 预设了安全超时与资源互斥组：

- **默认超时**：`profile.default` 配置 `slow-timeout = { period = "15s", terminate-after = 2 }` 与 `fail-fast = false`（`profile.ci` 为 `period = "20s"`，`nana-packager` 覆盖为 `period = "30s"`）。
- **`gpu-serial`（`max-threads = 1`）**：覆盖 `nana-gpu`、`nana-frame-exchange`、`nana-ui`、`nana-ui-devtools`、`nana-wgpu-fork-tests`、`runtime-host-fixture`、`nana-mobile-probe` 与 `binary(ui-snapshots)`。
- **`os-global`（`max-threads = 1`）**：覆盖 `nana-window`、`nana-diagnostics` 与 `nana-ui-platform`，避免进程级全局单例、环境变量或原生窗口句柄状态相互干扰。

## 本地安全执行方式

- **不要并行启动多个 `cargo test` 或 GPU 负载**：同一时间只跑一个 Cargo 测试命令，并显式限制 `-j`（例如 `-j 6`）。
- **直接调用 `cargo test` 跑 GPU 或原生窗口套件时必须串行线程**：添加 `-- --test-threads=1`。
- **常用命令**：

```bash
# 1. 静态门禁：目标数量、命名与反卡死检查
python3 scripts/check-test-hygiene.py

# 2. 运行普通 CPU crate 测试
cargo test -j 6 -p nana-ui-runtime --lib --locked -- --test-threads=4

# 3. 串行运行 GPU / 原生窗口 crate 测试
cargo test -j 6 -p nana-gpu --locked -- --test-threads=1
cargo test -j 6 -p nana-ui --locked -- --test-threads=1
cargo test -j 6 -p nana-window --locked -- --test-threads=1

# 4. 打包校验与 Steam delta 用例
cargo test -j 6 -p nana-packager --test packager --locked steam_delta
```

变更诊断与路径约定可同时参考 [诊断与应用路径](diagnostics.md)。
