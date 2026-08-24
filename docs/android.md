# Android

Android 是实验路径，**不是当前产品目标**。不要把它写进应用的平台承诺。

它同样不走系统 WebView：界面仍进 Runtime / UiScene，窗口和图形设备由宿主 Activity 掌握，不调用桌面的 `run_runtime`。

现在能证明的是交叉编译能过、桌面侧的引擎冒烟能跑。不能当成已经具备桌面同款的输入法、无障碍、中文排版和真机画面。

没有 V8 预编译库时，宿主可以先不链引擎，只验证能编过。要在设备上跑 Vue，需要自行准备对应架构的 V8 档案。网络仍然默认全关。剪贴板和输入法在没有真实后端时会明确说不支持。

```bash
./scripts/check-android-arm64.sh
```

编过只说明依赖和接口能对上。平台工程笔记在 `platform/android/README.md`。
