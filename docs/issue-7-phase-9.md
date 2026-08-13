# Issue #7 Phase 9：Iced 退出门禁与 Epic 收口审计

本阶段在 `2026-08-14` 对 Phase 0–8 的实现和证据做最终审计。结论是：**Nana-owned Runtime / UiScene 架构已经建立，但 Iced 退出门禁尚未全部满足，因此保持 Iced compatibility backend，Issue #7 不应关闭。** 删除 Iced 核心路径会使现有组件、IME、accessibility 与平台宿主发生功能回退，不是合格的“完成迁移”。

## 退出门禁

| 门禁 | 当前结果 | 证据或剩余缺口 |
| --- | --- | --- |
| Runtime coverage | 部分通过 | identity、hierarchy、style、text、layout、hit-test、pointer capture/event route、focus/IME、animation/deadline、accessibility、render content 已有单一权威；compatibility component animation、平台 adapter 与完整 lifecycle 仍未迁移完成。 |
| Component parity | 未通过 | 现有 Nana components 仍由 Iced compatibility widgets 绘制；尚无 Nana-owned 标准控件 painter 的完整行为与视觉等价实现。 |
| IME parity | 未通过 | Runtime 有 backend-neutral composition/selection 生命周期及 CJK deterministic tests；Windows、Linux、Android 的真实候选窗、selection、提交与窗口交互未验收，Android host 仍有明确限制。 |
| Accessibility parity | 部分通过 | 本阶段补齐 Runtime role/state/tree/bounds/focus projection，并由 Vue semantic props 驱动；AccessKit/平台 adapter 仍在 Iced 侧，尚未证明无回退。 |
| Vue fixture parity | 通过 | `nana-ui-vue --features iced-view --lib` 380 项功能测试通过，UiWorld 是 retained authority。 |
| Desktop / Android coverage | 未通过 | backend-neutral Runtime/Scene 对 Android、Windows、Linux 可编译；这不等于真实窗口、输入、IME、accessibility 或 Android lifecycle 验收。 |
| Performance no regression | 部分通过 | Phase 0/5 Iced 离屏红线未回退；Runtime 5000 节点首批 systems P95 2.121 ms、idle 0，leaf paint mutation 固定 1 个 work node且 systems P95 0.002 ms；Vue 5000 节点 construction P95 9.999 ms、idle semantic 0。尚无真实窗口 UI + Live2D 交错 A/B。 |
| WGPU / native decision | 通过 | Phase 0/7 的证据支持保留 WGPU；五项 RHI 重开条件未满足，不实现 Nana Native RHI。 |

## 本阶段补齐的缺口

审计发现 Runtime 原先没有 backend-neutral accessibility 数据，导致 retained authority 在语义树处中断。本阶段加入：

- `AccessibilityRole`、`AccessibilityState` 与稳定 ID 的 `AccessibilityNode` 投影；
- role、label/value、disabled、checked、selected、focused、bounds 与 hierarchy；
- comment 排除、隐藏状态过滤、文本默认 label，以及 accessibility 独立 dirty work；
- Vue widget semantic props 到 Runtime accessibility component 的映射；
- 全量 snapshot 用于首建，dirty-node projection 用于平台 adapter 的增量消费。

继续对照 Issue 原文审计时又发现 Vue `InputState` 独立持有 pointer capture。该双权威已删除：Runtime 现在拥有 document-scoped capture、原子替换/释放、subtree 销毁失效、capture change stream 与稳定 event route；Vue host ops 只是 compatibility adapter。

AnimationSystem 现由 Runtime 持有 stable animation identity、目标 Entity、easing、deadline 与 active lifecycle；start/replace/stop 进入同一原子 mutation batch，subtree 销毁自动取消。host 从 `AppContext` 查询下一 deadline 并以显式单调时间采样，静态 UI 不轮询，采样不强制 Live2D 或整棵 UI redraw。现有 Iced-local component animation 尚未迁移，因此该 Runtime 完成项不被提升为 component/backend parity。

复核后没有在 compatibility adapter 中计算并丢弃增量投影，也没有伪造一个未连接 AccessKit/系统 API 的“native adapter”。后续真正替换 Iced accessibility 时应消费 `SystemWork.accessibility`，而不是建立第二棵语义树。

## 验证结果

- `nana-ui-runtime --all-features --locked`：24 项通过；
- `nana-ui-scene --all-features --locked`：5 项通过；
- `nana-ui-vue --features iced-view --lib --locked`：380 项通过；
- Runtime/Scene `clippy -D warnings`：通过；
- Runtime/Scene Android、Windows、Linux cross-check：通过；
- hosted Vue/Iced library check：通过，仅有 vendored `arboard` 既有 warnings；
- Iced dependency boundary：通过；`nana-ui-runtime` / `nana-ui-scene` 不依赖 Iced、WGPU 或平台 GPU API。

最新机器可读性能报告为：

- [`performance/2026-08-14-issue7-phase3-runtime.json`](performance/2026-08-14-issue7-phase3-runtime.json)
- [`performance/2026-08-14-issue7-phase5-vue-runtime.json`](performance/2026-08-14-issue7-phase5-vue-runtime.json)
- [`performance/2026-08-14-issue7-phase6-scene.json`](performance/2026-08-14-issue7-phase6-scene.json)

这些结果证明 backend-neutral core 的确定性、增量边界和当前 macOS 离屏性能，不替代真实桌面、Android、辅助技术或 Live2D 产品负载验收。

## Epic DoD 结论

已建立的根架构包括稳定 Runtime identity/ECS、原子 mutation、typed application API、active-only AnimationSystem、Vue retained authority 迁移、UiScene/RenderGraph 与同 WGPU context 的 custom texture 合成。Native RHI 和 Live2D native backend 经证据门禁得出 NO-GO，正确完成方式是不增加冗余后端。这里不再把 Phase 0–8 的阶段性实现等同于 Epic 全部 DoD；后续仍须逐项核对 compatibility animation migration、professional component、platform 与真实 workload 要求。

Epic 的最终“退出 Iced”条件仍有四类实质缺口：Nana-owned standard component painter、各目标真实 IME、平台 accessibility adapter、desktop/Android 与 UI + Live2D workload 验收。因此本阶段状态为 **HOLD**：保留 Iced compatibility core，不删除依赖、不关闭 Issue，也不把 compile/offscreen evidence 提升为真实平台验收。

待这些门禁具备实现与真实环境证据后，才能重开最后的 dependency removal；届时应先逐项替换 adapter ownership，再删除 Iced，而不是一次性重写。
