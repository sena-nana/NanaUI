# 原生窗口材质

原生材质由 `nana-window` 管理，NanaUI 控件不读取窗口句柄，也不直接调用平台 API。宿主先创建透明窗口，再把实现 `raw_window_handle::HasWindowHandle` 的窗口交给 `apply_system_material`。

| 平台 | 首选 | 回退 |
| --- | --- | --- |
| macOS 10.10+ | Vibrancy / `UnderWindowBackground` | 完全不透明的主题背景 |
| Windows 11 | Mica | Acrylic，再失败则使用不透明背景 |
| Windows 10 1809+ | Acrylic | 完全不透明的主题背景 |
| Linux / 其他 | 由合成器决定，不调用未支持的原生 API | 完全不透明的主题背景 |

`MaterialOutcome` 明确返回实际应用的效果和回退原因。宿主只有在获得原生效果时才使用半透明 UI 背景；不支持或调用失败时改用完全不透明背景，保证内容可读。主题深浅切换会先清除旧效果，再重新应用材质。

当前本机 macOS 26.5.2 运行验证返回 `Vibrancy`，WGPU Surface 使用受支持的预乘或后乘 Alpha 模式。由于验证时系统处于锁屏状态，只能证明原生 API 调用、Surface 配置和首帧合成成功，不能证明最终像素表现；解锁后的截图与拖拽 resize 仍需补测。

Windows 与 Linux 当前只有条件编译结构和 GitHub Actions 目标，尚未获得真实 Windows 10、Windows 11 和 Linux 合成器运行证据。Acrylic 在部分 Windows 10/11 版本拖拽和 resize 时存在上游已知性能限制，因此不能把编译通过视为平台验收。
