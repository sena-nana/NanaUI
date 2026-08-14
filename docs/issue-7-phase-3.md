# Issue #7 Phase 3：增量 UI systems 与 renderer-neutral extraction

本阶段在 `2026-08-14` 将 Phase 2 的 identity/hierarchy runtime 扩展为可执行的增量 UI 数据流：canonical style → computed style → text shaping → layout input/writeback → hit-test/focus/IME → render extraction。Vue retained tree 的权威迁移仍属于 Phase 5；本阶段不引入第二套 layout 或 renderer。

## 权威数据与系统边界

- `NodeStyle` 复用 `nana-ui-core::LayoutStyle`，字号、字重、字体、行高、字距、颜色、透明度、显隐和 z-index 不再另建重复字段；大样式通过 `Arc` 共享。
- `ComputedStyle` 只保存继承结果。父节点先于子节点解析；影响字体度量的样式变更会精确触发 text shaping。
- `TextContent` 只拥有文本。`TextShaper` 是 backend-neutral 注入契约，Runtime 不估算字宽；`nana-ui-vue::IcedTextShaper` 使用当前可见 Iced renderer 的 advanced shaping/CJK fallback 路径。
- `LayoutInput` 暴露有序 hierarchy、共享 `LayoutStyle` 和真实 text metrics；当前 Iced layout 通过 `WriteLayout` 回写 painted geometry，因此不存在第二布局权威。
- hit-test 使用按 document 构建的紧凑索引；一次 root→leaf 传播累计 transform 与 overflow clip，事件时不遍历 ECS World。同 z-index 按 document order 取最上层节点。
- focus 与 IME 都通过同一 `MutationQueue` 提交；失去 focusability/visibility 或销毁节点时清除 composition。IME selection 校验 UTF-8 边界。
- committed `TextInputState` 将 value 与 anchor/focus selection 绑定稳定 Entity，preedit 继续由独立 `ImeComposition` 持有；selection replacement 按 UTF-8 边界原子执行，accessibility 与 render extraction 消费同一 committed value。
- `ScrollOffset` 与未滚动 `LayoutBox` 同属 UiWorld；scroll mutation 只让后代 input/render 失效，不触发布局重算。hit-test 按祖先 scroll offset 平移后代，同时把 scrollport 自身保持为 viewport clip。`overflow:auto|scroll` 与 hidden 一样建立 paint/input clip。
- pointer capture 以 document + pointer ID 存在 Runtime，并由同一 mutation batch 捕获/替换/释放；subtree 销毁自动发布 lost-capture。capture/target/bubble route 直接从 Runtime hierarchy 投影，adapter 不再维护第二份 capture authority。
- animation 以稳定 `AnimationId` 和目标 Entity 存在 Runtime；start/replace/stop 与其他 mutation 同批原子提交，目标 subtree 销毁会取消对应 deadline。Runtime 不创建线程或读取系统时钟，host 传入同一 epoch 的单调 `Duration`，并从 `AnimationFrame.next_deadline` 安排精确唤醒。
- `ExtractedNode` 是 renderer-neutral snapshot，包含 hierarchy、源样式、computed style、geometry、text metrics、focus/IME；增量 extraction 同时输出 `render_removals`。Accessibility delta 同样包含 updated nodes 与稳定 ID removals；subtree 删除会同时更新父节点 children，renderer 或平台语义树都不会遗留幽灵节点。

## 增量调度

每个 Entity 拥有内部 `DirtyMask`，`UiWorld` 另维护 dirty ID set。`take_system_work` 只访问实际变化节点并按稳定 ID 排序；静态帧不扫描 World。动画不占用伪造的 dirty bit：deadline 查询只访问 active-animation store，due frame 只产出已到期动画样本；采样值的真实 style/layout/render 影响仍由 consumer 通过 mutation 提交。结构 mutation 的传播规则为：

- 改变 parent：子树重算继承样式/text/layout/input/focus/render，旧/新祖先重算 layout/render；
- 同 parent sibling reorder：仅移动节点 input/render，祖先 layout/render，不重算未变化的后代样式；
- style/text/interaction：按影响面传播；layout writeback 只触发 input/render，避免 layout 自激循环；
- scroll offset：只传播 subtree input/render，canonical layout 保持未滚动坐标；
- despawn：从 dirty set、focus、IME 和 hit index 移除，并发布 render removal。

阶段复核中删除了每帧全 World dirty 扫描、初始化节点重复写 `ALL` dirty bits、sibling reorder 对整棵子树无条件失效，以及通过重写 layout 模拟滚动等冗余。公共 system 方法对 stale ID 返回 `UiWorldError`，不依赖调用者维护内部不变量。

## 功能门禁

Runtime 功能测试覆盖：

- style 继承、真实 shaper 注入、layout input/writeback；
- z-index/document-order hit-test；
- focus/IME 生命周期与 composition extraction；
- 隐藏/不可 focus 节点自动清理 focus；
- 非法 style/text metrics/layout/IME 的批次原子失败；
- 静态帧无 work、sibling reorder 精确 invalidation；
- subtree despawn 的 renderer removal 与 stale handle 行为。
- animation 非法时序/stop 的批次原子失败、start/replace/stop、easing、deadline、完成与 subtree 自动取消；并覆盖 `ViewContext` mutation → `AppContext` host wakeup 的实际调用链。
- CJK committed selection replacement、非法 byte offset 原子失败，以及 focused text-input 查询；IME preedit 不能挂到没有 committed text state 的普通节点。

`nana-ui-vue` 的 CJK shaping test 通过实际 Iced paragraph backend，不以字符串或日志匹配替代功能。macOS runtime test/clippy 通过；backend-neutral runtime portability check 通过。Vue/Iced 检查仅出现 `vendor/arboard` 既有 deprecated/unsafe warnings。

## 性能门禁

机器可读报告见 [`performance/2026-08-14-issue7-phase3-runtime.json`](performance/2026-08-14-issue7-phase3-runtime.json)。Apple M4 release，10 次 warmup、60 个样本。`systems` 包含 style、focus、layout input、hit index 和 render extraction；benchmark 节点无文本，因此不伪造 shaping workload。

| 节点 | Initial commit P95 | Initial schedule P95 | Initial systems P95 | Reorder commit P95 | Reorder systems P95 | Idle schedule P95 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 0.226 ms | 0.017 ms | 0.150 ms | 0.002 ms | 0.032 ms | 0.000 ms |
| 500 | 0.557 ms | 0.031 ms | 0.304 ms | 0.001 ms | 0.064 ms | 0.000 ms |
| 1000 | 0.866 ms | 0.047 ms | 0.475 ms | 0.001 ms | 0.101 ms | 0.000 ms |
| 5000 | 4.836 ms | 0.243 ms | 2.420 ms | 0.002 ms | 0.501 ms | 0.000 ms |

`rebuild_hit_test` 在 retained order 改变时按 document 重建，因此 reorder systems 随文档大小增长；加入正确的累计 transform/clip 后曾因逐节点回溯祖先升至约 3 ms，改为单次 root→leaf 传播后 5000 节点 P95 为 0.501 ms。静态 world 的 drain 为 O(1) 空集路径。首批 5000 节点 commit + schedule + systems P95 合计 7.499 ms，低于一帧 16.67 ms；这不是最终应用帧时间承诺。

报告 schema v6 另加入 leaf paint-only mutation，并在每个样本断言只有 1 个 style/render work node、layout/input work 均为空。100–5000 节点的 commit P95 均不高于 0.001 ms，systems P95 均不高于 0.003 ms，证明该路径不随文档大小做全树无效工作。此前所有 `SetStyle` 无条件失效 subtree layout 的冗余已按 inherited text/paint、visibility、transform/stacking、layout semantics 分类消除；分类使用只读字段比较，不克隆含 String/Vec 的完整 style。

同一 schema 测量 Runtime-owned per-pointer hover 在两个带 interaction style 的 leaf 间交替：100–5000 节点 transition P95 均为 0.000 ms，并逐样本断言只调度 old/new 两个 style/render node。没有 interaction paint 的节点仍记录 hover/press authority供事件语义使用，但不产生无价值的 style work。

同一报告还测量 animation 独立 cadence：deadline 由按 `(deadline, AnimationId)` 排序的索引维护，start/replace/stop/cancel 为 O(log n)，最近 deadline 为 O(1)，advance 只遍历 due 项。每个 case 注册与节点数相同的 100–5000 个 animation，但仅 1 个在采样时刻 due；最近 deadline 查询与该次稀疏采样 P95 均为 0.000 ms（100 节点 P99 0.001 ms），并断言完成后下一 deadline 来自其余 future animation。由此不再以“只有 1 个 active animation”的弱场景推断扩展性。后续 Workspace/Sidebar/Dock 复核已移除 `nana-ui` 中最后的 Iced `Animation` 状态，但完整 component/backend parity 仍取决于 painter、平台 adapter 与 gallery 迁移，不能因 animation contract 完成而宣称整项 Epic 完成。

`nana-ui::RuntimeAnimationClock` 已将 Runtime monotonic `Duration` epoch 接到 hosted event-loop `Instant`：`next_wakeup` 可与 GPU/Web/Live2D deadline 取最小值，`wake` 只采样 due animation，既不创建 timer 也不替 host 请求 redraw。后续 Workspace 复核又把 layout、viewport、resize 与 collapse transition 下沉到 backend-neutral `nana-ui-core::WorkspaceModel`；Sidebar expansion 和 Dock dwell 也只保存 `Duration`。Iced controller 仅转换 host subscription/clock，不再持有动画权威。

复测命令：

```bash
cargo test -p nana-ui-runtime --locked
cargo clippy -p nana-ui-runtime --all-targets --all-features --locked -- -D warnings
cargo test -p nana-ui-vue --features iced-view --locked runtime_text::tests::shapes_cjk_through_the_visible_renderer_backend
cargo run --release -p nana-ui-runtime --bin nana-runtime-benchmark --features benchmark --locked -- --output docs/performance/2026-08-14-issue7-phase3-runtime.json
```

## 阶段结论

Phase 3 的 style、text shaping、layout、input/hit-test、focus/IME、animation/deadline、dirty propagation 与 render extraction 已形成单一可执行链路；没有迁移 Vue 权威树、没有复制 CSS/layout 规则、没有创建新的绘制后端。经多轮复核后的剩余成本是 retained-order 改变时重建 document hit index，已有测量且低于本阶段门禁。Runtime AnimationSystem 经过功能、原子性、生命周期、context 接入和 active-only 性能复核，确认可进入后续 component/host migration。
