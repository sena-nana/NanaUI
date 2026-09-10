# 四应用消费迁移与发布验收

本轮覆盖 LiliaBilibili、LiliaCode、NanaLive、NanaShader。冻结基线为
`6adf38db4bece07d6b67a1c36735f0eb18c509b5`；本文件记录未发布工作树的联调证据，
不能作为最终 git pin 验收。初始各仓库 HEAD、status、binary diff 和 staged diff
保存在 `/tmp/nanaui-consumer-upgrade-20260909/`，已有业务改动未重置。

## 公共合同与消费范围

- `AppContext::reconcile_children` 将已有节点完整顺序作为单一 Runtime 事务发布；
  同序不提交，省略子树 park，支持合法 reparent 和未挂载父节点，非法输入原子失败。
  框架 settings/workspace/dock 与 Code、Shader 的重复协调实现迁入此接口。
  专用 slots 和 Panel/Overlay transfer 仍使用各自生命周期入口。
- `SettingsRow::stack_below` 显式启用按实际行宽堆叠，默认行为不变；
  动态 hint 无→有→无由装配层处理。Bilibili 使用 `<480`；Code 保留窗口
  `<=900` 和 360 宽字段产品策略；Live 保留静态策略。Vue 转发和组件语义同步。
- 系统文件选择请求由宿主执行，结果携带窗口和请求身份，经事件主动唤醒回流。
  每窗一个活动请求，重复/忙碌通过独立拒绝事件返回；关闭和晚到结果使用宿主
  attempt token 隔离。四应用保留请求发起时的账号、对象、token、草稿基线等业务语义。
  原生平台执行器集中到 nana-window，取消保持正常空结果。
- `Thumbnail` 补充 fit 与显式响应式几何；Bilibili 静态封面迁入该组件，
  Avatar、OverlayVisibility 和保留式虚拟列表复用现有公共能力。
- 迁移期间修复隐藏子节点重新显现后的布局缓存失效，恢复 Bilibili 离线页
  聚焦行的布局与可见性；没有放宽原有功能断言。

应用核心、播放器媒体流程、Live2D、任务审批、Shader 领域控件、应用卡片组合与
Peek/Diff 的 Panel 组合继续归业务。本轮未增加单场景定位 API。

公开使用合同见 [application-api.md](application-api.md)、
[components.md](components.md)、[window.md](window.md)。

## 验证记录

本地配置仅在命令行或临时 CARGO_HOME 中启用：
`/tmp/nanaui-consumer-upgrade-20260909/local-framework.toml`。
四应用仓库默认配置不再启用本地 patch；当前联调 Cargo.lock 仍解析工作树源码。

| 范围 | 已取得的证据 | 未完成项 |
| --- | --- | --- |
| NanaUI Runtime | 最终 931 个 Runtime 测试与严格 Clippy 通过，包含 live descendant 提出时焦点、IME、撤销/重做、tooltip/loading 保留及实际 park 生命周期回归；Vue SettingsRow 6 个用例、wrapper inventory 164 个用例通过 | 无 |
| NanaUI 文件对话框 | macOS nana-window 10 个配置/协议测试通过；宿主生命周期 3 个用例通过；Windows GNU all-targets 交叉检查通过；Linux GNU all-targets `--locked` 严格 Clippy 通过，含旧 portal 实际句柄兼容与可取消后端 | 三平台真实交互尚未验收 |
| Bilibili | workspace 1146 项测试通过（含文档测试）；fmt、workspace/all-targets 严格 Clippy（含 visual-capture）、visual-capture 构建通过；24 张图逐张查看 | 最终发布 pin；完整页面状态笛卡尔积和全部原生播放器走查 |
| Live | `cargo xtask verify` 通过：1162 通过、9 忽略，随后严格 Clippy 通过；ui-offscreen 构建和定向行为验证通过；浅/深色、宽/窄窗和高 DPI 图片逐张查看 | 最终发布 pin；真实外部输出和系统选择器交互 |
| Shader | workspace/all-targets/all-features check、test、严格 Clippy 与 fmt 通过；750 通过、13 忽略；11 张既有离屏验收图逐张查看 | 最终发布 pin；系统文件选择器原生交互 |
| Code | 公开 API 迁移完成，完整本地 metadata 可解析；正式 verify/performance/agent-debug 入口已执行 | 缺失的私有框架能力导致编译失败；没有本轮性能或原生截图验收证据 |

完整 `cargo metadata --locked --all-features` 的本地依赖图显示：Bilibili 11、
Code 8、Live 10、Shader 10 个框架包均指向同一个 NanaUI 工作树。
每个应用只有一个 wgpu 版本：Bilibili 30.0.1，其余 30.0.0。
这仅证明联调依赖收敛，不能证明 git 来源。

非作者交叉复审已覆盖框架与消费者迁移。最后发现的 live descendant 提出/park
问题已修复：先提出节点，再停放旧祖先；生命周期按最终树去重计算，保留焦点、
IME、撤销历史和仍挂载的 tooltip/loading 状态。真实 park 正常暂停，非法事务
不产生副作用。最终 931 项 Runtime 测试和严格 Clippy 通过，非作者复核未发现
该修复的残留问题。Shader 冻结后追加 153 项 UI 消费回归通过（11 项显式忽略）。

`cargo fmt --all -- --check` 与 `git diff --check` 已通过。格式门禁另发现基线中
`l3-dev-entry.rs`、`world/tests.rs` 和 Vue `tree/tests.rs` 三处长表达式未按当前
rustfmt 换行，本轮只做格式修正，没有改变行为。

各应用详细证据及保留项：

- [Bilibili](../../LiliaBilibili/docs/verification/2026-09-09/nanaui-upgrade.md)
- [LiliaCode](../../LiliaCode/docs/design/nanaui-upgrade-blockers.md)
- [NanaLive](../../NanaLive/docs/reports/nanaui-consumer-upgrade-2026-09-09.md)
- [NanaShader](../../NanaShader/docs/nanaui-upgrade-validation.md)

## 明确边界

LiliaCode 原私有 patch 目录为空，缓存约 130 个框架修订中未找到 BrowserView、
DonutChart、TimeSeriesLayer 等真实实现；还涉及 Markdown 图片和编辑会话 API。
没有通过空实现、删除功能或屏蔽 feature 掩盖该阻塞。现有应用未完成接线另行列在
Code 报告中。正式 verify 当前先被本地 path lock 的 git pin 门禁拒绝；直接编译
以及 performance/agent-debug 的实际构建确认源码能力缺失依然存在。

macOS 原生验收 probe 已构建为
`/tmp/nanaui-consumer-upgrade-20260909/NanaDialogProbe.app`，但计算机控制工具
报告桌面锁定，尚未执行选择、取消、并发拒绝、关窗与持续呈帧的真实交互。
原生验收入口是 `crates/nana-ui/examples/hosted-file-dialog-probe.rs`；
配置测试、离屏和交叉编译均不替代原生验收。Linux portal 返回 URI 数组；
zenity fallback 多选采用换行分隔，文件名自身含换行时存在协议歧义，
该场景不计为任意路径支持通过。单选只剥离一个协议结尾换行。

Shader 既有长 ContextMenu 高度超出窗口的问题、Live 低于 640×480 的裁切边界
已在各应用报告明确记录，不声明这些场景已经通过。

## 发布顺序

1. 本地实现、行为回归和独立复审收口，向用户提供可审查 diff 与验证边界。
2. 按仓库规则取得提交、推送确认后发布 NanaUI；确认提交可从消费者使用的远端获取。
3. 四应用所有 15 处框架依赖声明统一钉到该提交（4/4/4/3），重新解析各自锁文件。
   保留 `.pin` 备份和其它 Cargo 配置，不恢复默认 path patch。
4. 不带联调 patch 执行完整 `cargo metadata --locked --all-features`，包括
   devtools、build-dependencies、目标平台依赖；核对框架 git source 的提交一致和
   每个应用 wgpu 单版本，再复跑约定门禁。
5. 框架提交不可获取、Git 来源未验证或 Code 门禁未修复时，不标记四应用升级完成。
