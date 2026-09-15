# Android

Android 是实验宿主路径，**不是当前产品目标**，也**不是第二套产品绘制核**。不要把它写进应用的平台承诺。

界面仍进 Runtime / UiScene，由 `SceneWgpuPainter` 绘制；窗口和图形设备由宿主 Activity 掌握，不调用桌面的 `run_runtime`。Vue + JS 仍然是同一份 Runtime 合同。

Phase 2 合同（相对 NativeActivity）：

- **Activity**：`android-activity` 的 `game-activity` feature，`NanaActivity` 继承 `GameActivity`，因此存在 `InputConnection` / GameTextInput。
- **IME**：composing → `ImeEvent::Preedit`，commit → `Commit`，删除 → `DeleteSurrounding`，全部喂给现有 `RuntimeInputAdapter::dispatch_ime`。不另造 Android 文本状态机。`EditorInfo` 从焦点编辑器镜像 password / multiline。`PlatformCapabilities::android_mvp().ime = true` 表示这条路径已接通，不是桌面同款验收。
- **剪贴板**：JNI `ClipboardManager` 实现 `ClipboardHost`；`default_shared_clipboard()` 的 Android 分支使用它；web-api 与 slot `RuntimeInputAdapter::with_clipboard` 走同一后端。`clipboard = true`。
- **TalkBack**：Click / Focus / SetValue / SetSelection 投影后，Button / Switch / TextInput 可激活（TextInput 的 Click 会聚焦以便唤起 IME）。滚动与虚拟列表仍留后期。

交叉编译和桌面侧 `nana-android-host --lib` 单测只覆盖编译与动作合同，**不能**当成已经具备真机 CJK 候选框、系统剪贴板或 TalkBack 画面。

没有 V8 预编译库时，宿主可以先不链引擎，只验证能编过。要在设备上跑 Vue，需要自行准备对应架构的 V8 档案（`RUSTY_V8_ARCHIVE`）。网络仍然默认全关。

```bash
./scripts/check-android-arm64.sh
```

这条 check 现在也挂在 push / PR 的 `android-arm64-cross` job 上（V8 stub 分支，
不做多小时的 GN 构建），所以交叉编译断掉会在 PR 上直接红，而不是等到有人手动跑。
没有 `RUSTY_V8_ARCHIVE` 时该 job 只证明 stub 能链，不证明设备上的 V8。

要出可安装的 APK，必须走 `dist` 档，并且用 GameActivity 的 Gradle 包装（需要
`androidx.games:games-activity:4.4.0`，不要开 prefab）：

```bash
./scripts/check-android-arm64.sh --build --dist
./scripts/package-android-host-apk.sh
```

dev 档会把约 390 MB 的 DWARF 和约 46 MB 的符号表内嵌进 `.so`。`package-android-host-apk.sh`
仍会对 stripped `.so` 设 60 MiB 上限。APK 里 `.so` 以未压缩方式存放，配合
`extractNativeLibs="false"`，装机后不再解压出第二份。

编过只说明依赖和接口能对上。平台工程笔记在 `platform/android/README.md`。

## 真机验收清单（本机交叉编译不能代替）

在已安装的 debug APK 上逐项勾：

| 项 | 期望 | 证据 |
| --- | --- | --- |
| 点控制槽 Input | 软键盘弹出；焦点离开后收回 | 真机 / 模拟器 |
| CJK 候选 | 拼音/笔画候选框相对 caret；选定后写入 Runtime `TextInput` | **必须真机 IME**，不能用 `adb shell input keyevent` 冒充 |
| 剪贴板 | 槽内复制/粘贴与系统 ClipboardManager 互通；JS `clipboardWriteText` 同一后端 | 真机 |
| TalkBack | Button 激活计数、Switch 切换、TextInput 获得焦点并进入编辑 | 真机 TalkBack |
| 滚动 / 虚拟列表 | 未做 | 后期 |
| V8 Vue | 设备上设置 `RUSTY_V8_ARCHIVE` 后的发行包 | 真机；CI stub 不算 |

桌面 host 配置下的 Android host `--lib` 回归验证 Rust 侧动作队列、输入、IME 映射和无障碍投影合同，不能替代上表。

配置 Android NDK 的 LLVM clang 后，`cargo check --locked --no-default-features --target aarch64-linux-android` 覆盖 `nana-android-host`。构建需设置 `CC_aarch64_linux_android` 指向 NDK clang；仓库的 `scripts/android-env.sh` 会自动导出。默认 `engine-v8` 仍需要 CI / 发行提供 `RUSTY_V8_ARCHIVE`。这只是 ARM64 交叉编译合同证据。
