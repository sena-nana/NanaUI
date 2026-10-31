# Winit Windows IME 修复

上游：https://github.com/rust-windowing/winit

固定 revision：`874a1e31f194d11313c1ac133b28037b99db3e19`，版本 `0.31.0-beta.2`，许可证 Apache-2.0。
本目录仅保留 Windows 后端，其余后端、`winit-core` 和 `dpi` 使用相同上游 revision。

修复 `Window::request_ime_update` 的 Windows IMM 锁重入：先更新输入法能力并保存缩放比，再释放 `WindowState` 锁，最后调用 `ImmAssociateContextEx` 或候选窗口定位。
这些 Windows API 会同步发送窗口消息，窗口过程需要取得同一个锁。

第三方源文件保持上游布局。升级时检查该修复是否已进入上游，然后移除对应 Cargo patch。
原生回归由 VChat `UI/Tests/native_input.py` 验证 Microsoft 拼音预编辑、提交、重复发送、窗口移动和关闭。
