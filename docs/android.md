# Android

Android 是实验路径，**不是当前产品目标**。不要把它写进应用的平台承诺。

它同样不走系统 WebView：界面仍进 Runtime / UiScene，窗口和图形设备由宿主 Activity 掌握，不调用桌面的 `run_runtime`。

现在能证明的是交叉编译能过、桌面侧的引擎冒烟能跑。不能当成已经具备桌面同款的输入法、无障碍、中文排版和真机画面。

控制槽的输入边界（NativeActivity 没有 InputConnection，`PlatformCapabilities::android_mvp().ime` 仍为 `false`）：

- 点按控制槽输入框会唤起软键盘（`show_soft_input`），焦点移开或窗口销毁会收回（`hide_soft_input`）；见 `SlotRuntime::text_input_focused` 与 `HostState::sync_soft_input`。
- 软键盘提交的文本以硬件风格 KeyEvent 走既有键盘路径（US-QWERTY 子集）；没有 composition/preedit，CJK 候选框不可用。要完整 IME 需要 GameActivity / GameTextInput 或自定义 Activity 的 InputConnection，那会换掉 NativeActivity 后端，当前不做。
- 无障碍树第一期已发布：`accesskit_android::InjectingAdapter`（embedded-dex）把委托注入 Activity decor view，`AccessTreeProjector` 复用桌面投影器发布控制槽根与 Button/Switch/TextInput 的 name/role/value；读屏动作尚未映射回 Runtime。滚动与虚拟列表留第二期。

没有 V8 预编译库时，宿主可以先不链引擎，只验证能编过。要在设备上跑 Vue，需要自行准备对应架构的 V8 档案。网络仍然默认全关。剪贴板没有真实后端，明确说不支持。

```bash
./scripts/check-android-arm64.sh
```

编过只说明依赖和接口能对上。平台工程笔记在 `platform/android/README.md`。
