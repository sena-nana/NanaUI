# Nana Runtime / UiScene contract

本文定义 NanaUI 独立于 compatibility engine 的稳定架构合同。阶段证据见
`issue-7-phase-2.md` 至 `issue-7-phase-9.md`；本文只记录长期不变量。

## Dependency direction

```text
Nana public/application adapters
              |
              v
       nana-ui-runtime
              |
              v
        nana-ui-scene
              |
              v
 compatibility/native backend
```

`nana-ui-runtime` 与 `nana-ui-scene` 不依赖 Iced、WGPU 或平台 GPU API。Iced-derived
code 不得反向依赖 Nana packages。`scripts/check-engine-boundary.py` 持续检查这两个
方向。

## Retained authority

`UiWorld` 是 identity、document ownership、hierarchy、node kind、canonical/computed
style、text、text metrics、layout、interaction、pointer capture/event route、focus/IME、
accessibility 和 render content 的唯一权威来源。对外只使用 `StableNodeId` / typed
`Entity<V>`；Bevy Entity 编码不是 ABI。

所有结构或 component 变更先进入 frame-local `MutationQueue`，整批验证成功后一次
commit。失败批次不发布局部 hierarchy/component 结果；despawn 后 ID 永久 tombstone，
禁止 ABA 复用。

Vue `NodeHandle` 与 `StableNodeId` 无损映射。DOM facade 只保留 namespace、attributes、
scope/event flags 等 compatibility metadata；`MessageBridge` 的 hierarchy 在 renderer
消费前由 Runtime 覆盖，不能成为第二权威源。

## Incremental systems and wakeup

Runtime 以 dirty component 产生确定性的 `SystemWork`，区分 style、text shaping、
layout、input/hit-test、focus/IME、accessibility、render extraction 和 render
removal。静止 world 返回空 work，不运行无关 system，也不要求持续 redraw。外部
text/layout/accessibility backend 只消费显式 work，并把有限结果写回 Runtime。

局部 mutation 只传播到语义受影响的 node/subtree/ancestor；传播遇到已有相同 dirty
状态即停止。动画以 Runtime-owned stable ID 注册，host 显式传入单调时间并消费最近
deadline；Runtime 不创建计时线程。due sample 本身不伪造 render dirty，consumer 仅对
实际属性结果提交 mutation。动画、实时 GPU 内容和普通 UI wakeup 是独立 cadence，实时
source 不得强制整个 Runtime 全量更新。

## Application API

`Entity<V>`、`View`、`AppContext` 与 `ViewContext` 提供 typed state/read/update/remove、
closure event、typed action 和 staged extension install，不暴露 ECS World。一次 context
update 汇集为一个 mutation commit。`Task`/`Subscription` 只包装标准 Future/Stream；
executor、waker 和取消生命周期由 host adapter 拥有。

## Render extraction and Scene

Runtime 只输出 `ExtractedNode` delta 与 tombstone removal。`UiScene` 保存稳定 primitive
cache，并表达 Quad、Text、Custom、bounds、affine transform、clip chain、累计 opacity、
z-index 和 document order。普通局部更新不重建 hierarchy order 或无关 primitive；
hierarchy 改变时才重算 document order。

`CustomRenderNode` 只有 renderer/resource/revision opaque key，不携带 backend object。
`RenderGraph` 声明 pass dependency、resource access/hazard 和 ordered Draw/InvokeCustom
operation；backend 负责把它映射到同一 Device/Queue/Surface。业务 GPU 内容不得 CPU
readback、Base64/图片编码或额外子窗口后伪装成共享合成。

## Compatibility backend

当前 `nana-ui`、`nana-ui-vue` 与 Android host 是显式 Iced compatibility adapters。
标准 Quad/Text/components 仍由 Iced painter 绘制；hosted custom texture 已从 UiScene
frame graph 解析后在同一 WGPU context 合成。保留成熟 text/layout/IME/accessibility
实现优先于为了“零 Iced”重写。

只有 Runtime、components、IME、accessibility、Vue fixtures、desktop/Android 与性能
全部达到 parity 时，才能退出 Iced runtime/renderer 核心路径。未达到门禁时保留兼容
路径是合同要求，不是临时例外。
