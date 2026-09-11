# Android

Android 是实验路径，**不是当前产品目标**。不要把它写进应用的平台承诺。

它同样不走系统 WebView：界面仍进 Runtime / UiScene，窗口和图形设备由宿主 Activity 掌握，不调用桌面的 `run_runtime`。

现在能证明的是交叉编译能过、桌面侧的引擎冒烟能跑。不能当成已经具备桌面同款的输入法、无障碍、中文排版和真机画面。

控制槽的输入边界（NativeActivity 没有 InputConnection，`PlatformCapabilities::android_mvp().ime` 仍为 `false`）：

- 点按控制槽输入框会唤起软键盘（`show_soft_input`），焦点移开或窗口销毁会收回（`hide_soft_input`）；见 `SlotRuntime::text_input_focused` 与 `HostState::sync_soft_input`。
- 软键盘提交的可打印字符映射为 `ImeEvent::Commit`，经 `RuntimeInputAdapter::dispatch_ime` 写入 Runtime `TextInput`（与桌面 composition 路径同一套，不另造 Android 文本状态机）。NativeActivity 没有 InputConnection，因此没有 composition/preedit，CJK 候选框不可用。要完整 IME 需要 GameActivity / GameTextInput 或自定义 Activity 的 InputConnection，那会换掉 NativeActivity 后端，当前不做。
- 无障碍树第一期已发布：`accesskit_android::InjectingAdapter`（embedded-dex）把委托注入 Activity decor view，`AccessTreeProjector` 复用桌面投影器发布控制槽根与 Button/Switch/TextInput 的 name/role/value。`SlotActions` 将 TalkBack 请求排队，宿主在发布周期调用 `project_action` 并送入 Runtime 的 typed action；滚动与虚拟列表留第二期。

没有 V8 预编译库时，宿主可以先不链引擎，只验证能编过。要在设备上跑 Vue，需要自行准备对应架构的 V8 档案。网络仍然默认全关。剪贴板没有真实后端，明确说不支持。

```bash
./scripts/check-android-arm64.sh
```

这条 check 现在也挂在 push / PR 的 `android-arm64-cross` job 上（V8 stub 分支，
不做多小时的 GN 构建），所以交叉编译断掉会在 PR 上直接红，而不是等到有人手动跑。

要出可安装的 APK，必须走 `dist` 档：

```bash
./scripts/check-android-arm64.sh --build --dist
./scripts/package-android-host-apk.sh
```

dev 档会把约 390 MB 的 DWARF 和约 46 MB 的符号表内嵌进 `.so`（曾经的 473 MiB
产物里只有 55 MiB 是真正装载的段）。`package-android-host-apk.sh` 无条件跑一次
`llvm-strip --strip-all`，并对 stripped `.so` 设了 60 MiB 上限：超了直接失败，因为
那基本只可能是误传了 dev 档产物。APK 里 `.so` 以未压缩方式存放
（`aapt -0 .so` + `zipalign -p`）配合 `extractNativeLibs="false"`，装机后不再解压出
第二份。

编过只说明依赖和接口能对上。平台工程笔记在 `platform/android/README.md`。

桌面 host 配置下的 Android host 回归当前为 49 项通过；这只验证 Rust 侧动作队列、
输入、IME 映射和无障碍投影合同，不能替代 Android 真机 TalkBack 或 CJK IME 验收。

配置 Android NDK 28.2 的 LLVM clang 后，`cargo check --locked --no-default-features
--target aarch64-linux-android` 已通过（包括 `nana-android-host`）。构建需设置
`CC_aarch64_linux_android` 指向 NDK 的 `clang.exe`；仓库的 `scripts/android-env.sh`
会自动导出该变量。默认 `engine-v8` 仍需要 CI 提供 `RUSTY_V8_ARCHIVE`。这只是 ARM64
交叉编译合同证据，不能替代真机画面、TalkBack 或 CJK IME 验收。

进一步执行 `cargo build --locked --no-default-features --target aarch64-linux-android`
成功链接 `libnana_android_host.so`（470,374,192 bytes，NDK 28.2）；构建元数据和
SHA-256 保存在 `docs/performance-data/high-refresh-2026-09-06/android-arm64-build.json`。
NDK `llvm-readelf -h` 进一步确认该文件为 ELF64、`Type: DYN`、`Machine: AArch64`，
并包含可执行与可写 LOAD 段；不是仅按 Cargo target 目录推断架构。
