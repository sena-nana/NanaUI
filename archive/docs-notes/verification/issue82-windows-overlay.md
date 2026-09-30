# Issue82 Windows Forward 穿透验收

日期：2026-09-26

Issue82 只验收 Windows 真机上的 whole-window passthrough 与 Forward fallback。它不扩展 #159 的 InputRegion/per-region hit-test，也不新增 backend。

## 执行命令

```powershell
cargo build -p nana-ui --example desktop-overlay-probe --features hosted,bundled-fonts --locked
python -m pip install -r scripts/requirements-desktop-overlay.txt
python scripts/validate-desktop-overlay.py
```

## 结果

最终 probe 运行通过，报告位于 `target/desktop-overlay-native.json`：

- whole-window route 为 `1 → 0 → 1`；底层窗口在 passthrough 期间收到点击，Forward 在 opaque hit 区恢复 NanaUI 点击。
- `MousePassthroughChanged` 的成功/失败结果与实际 `WS_EX_TRANSPARENT` 样式最终一致；未知窗口 ID 正确返回失败。
- 透明区域像素在锁定、解锁和 overlay 关闭后均为 `0x00ff00`。
- 创建 overlay 不抢焦点；native resize 增长 `[45, 45]`；taskbar 显示/隐藏和隐藏后重新显示均恢复预期状态。
- 15 个 Forward 状态转移样本：p50 `0.0572 ms`，p95 `3.2762 ms`，低于 100 ms 门限。
- 环境：Windows 11 `10.0.26300`，Python `3.14.3`，DPI `1.5`，GPU `NVIDIA GeForce RTX 5060`（另报告虚拟显示器）。

探针会等待 winit 异步 native hit-test 样式真正落地后再计时；这避免把 `MousePassthroughChanged` 事件先于 Windows 样式更新的正常调度顺序误判为失败。光标定位允许一次操作内的有限重试，最终报告中的请求/实际坐标全部一致。

## 自动化与边界

- `nana-ui` 的 Forward 路由和缺失窗口失败回归测试通过。
- 该证据覆盖当前 Windows 会话、DPI 和 GPU；没有替代 macOS/Linux 平台 acceptance。
- 未覆盖异形窗口、per-region hit-test、Wayland global pointer、设备移除故障注入或产品特定 passthrough 状态；这些属于其他合同或后续 Issue。
