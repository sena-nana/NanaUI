# Issue #1 当前验收矩阵

本矩阵只记录仓库中已经实现并由构建、测试或视觉快照证明的能力。当前工作重点是
WGPU 30 渲染基座与可复用的 Lilia 风格工作区框架；NanaShader 不在修改范围内。

## 版本基线

- Iced：`0.15.0-dev`，分叉基线 commit
  `3c81aac2e1b48125efdf0c996fbbb9c72c06ae50`；
- Iced WGPU 后端：随分叉迁移到 WGPU `30.0.0`；
- Cryoglyph：本地同步迁移到 WGPU `30.0.0`；
- Rust edition：2024，最低 Rust `1.92`。

`Cargo.lock` 和 `cargo tree` 的当前结果只包含 WGPU 30。Iced 与 Cryoglyph
分叉通过本地路径联调。远端 `sena-nana/iced` 当前仍是 WGPU 29，
`sena-nana/cryoglyph` 尚不存在，因此在三仓修改经确认、提交并固定 Git
revision 前，GitHub Runner 不能复现本机结果。

## 当前结果

| Issue #1 验收方向 | 状态 | 当前证据 |
| --- | --- | --- |
| Cargo workspace 与三平台 CI | Workspace 已完成；CI 配置待依赖交付 | `nana-ui`、`nana-window` 和平台矩阵已配置，但相邻路径依赖尚未变成远端 Git revision |
| WGPU 30 渲染基座 | 已完成本地迁移 | NanaUI 全目标检查通过；Iced 全 workspace/all-targets 检查通过；Cryoglyph 全目标检查通过 |
| 可复用工作区框架 | 已完成本阶段 | `WorkspaceController`、动态 `WorkspaceRegions`、便捷 `WorkspaceSlots`、`workspace_view` 与 `app_shell` |
| Workspace Region | 已完成本阶段 | 动态注册/注销、自定义 ID、role/placement/scope、可见性、240ms 折叠过渡、尺寸约束、响应式策略、拖动调整、双击复位、JSON 持久化 |
| Region 几何 | 已完成本阶段 | 任意注册区域的逻辑/物理像素矩形与窗口 DPI；折叠释放空间，overlay 不占用 primary |
| 通用侧边栏 | 已完成本阶段 | 单层 Frame、Section、28px 左对齐 Row、26px Footer 图标按钮；滚动主体、固定外槽、160ms 分组高度/箭头折叠动画与交互状态 |
| 设置界面 | 已完成本阶段 | 与普通列表共用单个 Sidebar Region；稳定 Tab/别名恢复、普通/full-page 页面、SettingsRow/Card 与宿主持久化契约 |
| 工作区 Demo | 已完成本阶段 | Code/Github/Live2D 均使用单个 220px 侧栏和独立设置工作区；返回时保留应用布局，设置操作连接真实 Region 状态 |
| LiliaUI 风格主题与壳层 | 已完成本阶段 | dark/light 语义令牌、Noto Sans SC 四字重、36px 标题栏、紧凑导航和工作区表面层级 |
| Component Gallery | 已完成本阶段 | 基础控件、选择控件、Tooltip、Dialog、Context Menu 与状态交互 |
| `GpuView` / `RenderSlot` | 已完成基础合同 | viewport/scissor、当前 RenderPass 与同 Encoder 独立 Pass |
| 宿主掌握 WGPU 上下文 | 已完成基础合同 | 宿主创建唯一 Device/Queue，Iced 与宿主场景共享 |
| 宿主纹理直显 | 已完成基础合同 | `GpuTextureView` 直接采样宿主 `TextureView` |
| 按需重绘 | 已完成 | `ControlFlow::Wait`，没有空闲帧定时订阅 |
| 原生窗口材质 | 已实现，待目标机复验 | macOS Vibrancy、Windows Mica/Acrylic 与纯色回退 |
| 离屏视觉基线 | 已重新生成 | WGPU 30 上的 Code/Github/Live2D Workspace、Gallery dark/light 与浮层 PNG |
| Release 运行时基线 | 已复测 | Apple M4/Metal 首帧、示例体积、静置 CPU/RSS 与列表 benchmark |
| NanaShader | 未修改 | 当前工作区与仓库状态检查均未发现 NanaShader 变更 |

## 2026-07-30 本机验证

```text
NanaUI  cargo test --workspace --all-targets --locked       62 passed / 9 suites
NanaUI  cargo check --workspace --all-targets               passed
NanaUI  cargo clippy --workspace --all-targets --all-features -- -D warnings
                                                               passed
Iced    cargo check --workspace --all-targets               passed
Iced    cargo test --workspace --lib                        6 passed / 18 suites
Cryoglyph cargo check --all-targets                         passed
Cryoglyph cargo test --lib                                  passed
```

`ui-snapshots --locked` 已在 WGPU 30 上成功生成 21 张验收图，覆盖 1280×720
单侧栏 Workspace、Sidebar、Settings、可编辑圆角及其运行时应用、loading
组合按钮和交互式 Card。

单侧栏修正后，Release 列表 benchmark 两次复跑的 100/500/1000 项 total
median 分别为 0.515/0.668/0.750ms 与 0.472/0.670/0.917ms。本次将列表状态
圆点从字体字形改为居中几何后再次复跑，对应结果为
0.536/0.670/0.876ms，仍处于前两次端到端波动区间，因此不更新性能基线或作出
回退结论。

Release `hosted-gpu-demo` 已在 WGPU 30 上完成真实 macOS Surface present：
冷缓存首帧 468.473ms，后续五次中位数 174.247ms，均返回 Vibrancy；静置
15 秒为 0.0% CPU、99,168KiB RSS。`transparent-window` 已成功创建真实窗口。

## 当前交付边界

本轮按要求完成的是 NanaUI 框架、WGPU 30 基座和 LiliaUI 工作区结构 Demo；
没有修改 NanaShader，也没有把等价 WGPU 场景描述为真实 live2d-rs 集成。
Issue #1 中以下消费者级验收明确保留到后续：

- NanaShader 正式编辑器外壳；
- live2d-rs 真实预览与同帧总耗时；
- Shader 异步编译、旧 Pipeline 保留和编译诊断；
- NanaShader 业务面板与 Runtime 渲染图集成。

WGPU 30 迁移已形成本地 Iced `31030688`、按钮布局 `e7429b5f`、Cryoglyph
`3fe41b1` 与 NanaUI 基线 `cfaa286` 提交，但尚未推送。只有底层 revision 可从
远端获取并写入 NanaUI 清单后，三平台 CI 才能从“配置存在”升级为“可复现并已运行”。

## 尚需目标平台验收

- macOS 与 Windows 的系统材质最终观感；
- Windows/Linux 字体栅格化与控件像素差异；
- 鼠标 resize 命中、IME、窗口实时 resize 和多 DPI；
- 远端 macOS/Windows/Linux CI。

这些项目不能由 macOS 本机离屏 PNG 替代，因此不在文档中虚假标记为已验证。
