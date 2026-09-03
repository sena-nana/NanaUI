# Nana 默认应用图标

`nana.png` 是选定的角色剪影：冰蓝短发与猫耳、深蓝蝴蝶结、柔和珍珠灰白底板。
底板外为透明 RGBA。原始画稿由内置 ImageGen 根据用户提供的 Nana 角色参考图生成，
经用户选定灰白底版本，再通过 ImageGen 移除外侧棋盘格背景。

设计提示：将 Nana 的猫耳、短发、刘海及蝴蝶结提炼为扁平头像剪影；
脸部使用底色留白，不画五官；保留冰蓝头发与深蓝蝴蝶结，底色为柔和珍珠灰白。
透明导出提示：保留图标构图和配色，仅将圆角底板外的棋盘格替换为真实透明通道。

Runtime、Windows ICO 和 macOS ICNS 共用 `src/mark.rs`，从同一张源图按比例
缩放并添加系统图标留边。不要把概念稿中的棋盘格当作透明背景导入。

## 接入验证（2026-09-03）

- `cargo test -p nana-app-icon --all-features --locked -- --test-threads=1`：7 项通过。
- `cargo test -p nana-app-icon --no-default-features --locked`：3 项通过。
- `cargo fmt -p nana-app-icon -- --check`、`git diff --check -- crates/nana-app-icon`：通过。
- 通过实际 `mark::rasterize` 与编码路径导出 16/32/48/128/256/512 PNG，
  人工检查 32px 和 256px 输出；透明留边测试覆盖全部导出尺寸。
- `magick identify target/nana-icon-preview/NanaUI.ico`：解出 16/32/48/256 四档图像。
- `sips -g pixelWidth -g pixelHeight target/nana-icon-preview/NanaUI.icns`：macOS 识别为 512px 图标。
- 未启动桌面消费者查看 Dock，也未在 Windows 实机验证 PE 嵌入与任务栏显示。
