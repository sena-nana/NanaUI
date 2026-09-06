# 四项架构修复 + tree.rs 拆分

按风险从低到高分五个阶段,每阶段独立可验证,行为合同不变。

## Phase A:死代码与轻度清理(低风险,先行)

1. 删除 `apply_img_content_image`(`widget_map.rs:890-895`)及其唯一测试调用(约 `widget_map.rs:1043`)——它是 `bridge.rs:3115` img 分支的死副本,全仓库无生产调用方。
2. 去重 `register_action`:提取共享实现,`AppContext::register_action`(`framework.rs:6572-6590`)与 `ExtensionRegistrar::register_action`(`framework.rs:360-378`)改为调用同一私有函数,消除逐字重复。

## Phase B:WidgetKind 单表宏 + 全量对账测试

1. 在 `bridge.rs` 仿照 `component_catalog!`(`nana-ui/src/component_support.rs:101-133`)新增声明宏 `widget_kind_table!`,表项为 `(变体, parse 别名数组, as_str, element_tag)`,一次生成:
   - `parse`(保留现有 `nana-` 前缀剥离与多别名 arm,如 `"input"|"text-input"`、`"tr"|"td"|"th"`、`"hr"`),
   - `as_str`、`element_tag`,
   - `pub const ALL: &[WidgetKind]`(目前不存在)。
2. 三张现有 match(`bridge.rs:191-436`)改为宏生成,公开 API 不变。
3. 新增测试:
   - 全量 roundtrip:`ALL` 中每个 kind 满足 `parse(as_str()) == Some(kind)`、`parse(element_tag()) == Some(kind)`;`as_str`/`element_tag` 全局唯一——封住 `parse` 缺 arm 静默 `None` 的缝。
   - Vue↔Runtime 对账:每个 kind 的 `element_tag()`/`as_str()` 能在内建 `ComponentRegistry` 中 resolve 成功;当前确实无落点的纯布局 kind 列入显式注释的豁免清单(实现时以测试实际输出确定)。

## Phase C:resolve 特判规则表化 + 归一化去重

1. 将 `resolve_widget_component_type`(`tree.rs:3157-3222`)的 6 个"属性组合 → 控件降级"if 特判收成**有序规则表**(纯函数 `(谓词, tag)` 数组),保持现有优先级(Button+icon 先于 Chip 等隐式顺序显式化),单元测试逐条锁定顺序。
2. 减少重复分配:`try_tag` 链对相同候选字符串去重;`ComponentRegistry` 增加 `resolve_normalized`(接受已归一化 tag),Vue 侧每个不同候选只 normalize 一次;`resolve_tag` 公开行为不变。
3. 不引入 trait/策略模式——规则表就是有序函数数组,防止继续无结构加分支。

## Phase D:增量同步(核心,收益最大)

现状:`prepare_runtime_window`(`hosted_adapter.rs:511-530`)**每帧**构建全量快照(克隆全部 widget,O(n)),`sync_semantic_styles`(`tree.rs:1390`)在 revision 变化时对**全部 widget** 重做解析+分配(每 widget 最多 4-5 次 String 分配 + `Arc::new(layout.clone())`),而 mutation 入队有幂等守卫、解析本身没有。

1. **bridge 脏标记**:`MessageBridge` 增加 `pending_changes`(脏 widget 集 + `structure_changed` + `all`)。审计全部 24 个 `self.bump()` 调用点(bridge.rs:2323-5121),分类:定点 mutation 记录 widget id;插入/删除/重排/roots 变化置 `structure_changed`;theme 等全局置 `all`。提供 `changed(ids)` 辅助方法统一"bump + 记录",`bump()` 加注释声明必须经辅助方法调用。
2. **快照携带变更集**:`snapshot()` 将 `std::mem::take` 的变更集附到 `SemanticSnapshot`(新字段)。防御式回退:revision 变化但变更集为空(被其他消费者取走,如 agent session)→ 全量同步,保证正确性优先于性能。
3. **sync 走脏集**:revision 未变早退(现状);`structure_changed`/`all`/防御回退走现有全量循环;否则用 snapshot.widgets 建一次仅引用的 id 索引,只投影脏 id。把现循环体(1403-1570 的 set_text_input 清值、`project_migrating_component`、node_style/interaction/accessibility 投影)提取为两条路径共用的单 widget 投影函数,确保逻辑一致;移除/无障碍 delta 处理留在结构路径并注释不变式。
4. **无变化帧跳过快照构建**:`VueHost` 暴露廉价 revision getter;`prepare_runtime_window` 在 bridge revision == `document.synced_semantic_revision()`(新增 getter)时完全跳过 `semantic_snapshot()`,仅保留 `flush_host_frame`/`resolve_layout` 等既有步骤。
5. **差分测试**:同一 bridge 状态,doc A 走脏集增量同步、doc B 全量同步,断言 Runtime 投影(style/interaction/component type/a11y)完全一致;另加定点测试:属性变更只投影 1 个 widget、结构变更回退全量。

## Phase E:tree.rs 拆分(机械移动,最后做)

以 3086 行天然切分线拆分 `tree.rs`(10840 行),公开 API 与行为不变,经 `tree.rs` 再导出:
- 保留:`NanaTreeDocument` 文档核心(节点 CRUD、host ops、scroll metrics、accessibility snapshot、sync 入口);
- `tree/layout.rs`:布局盒/坐标变换(现 115-430);
- `tree/gpu_slots.rs`:host texture / GPU 媒体槽(现 908-952、2101-2154、3086-3113);
- `tree/component_binding.rs`:`resolve_widget_component_type`、`try_bind_registered_component`、`project_migrating_component`、shell kind、landmark(现 3157-4700);
- `tree/kits/`:settings/markdown/workspace/dock/split-pane/app-shell 各 from_widget 装配(现 4787-6000)。

bridge.rs 本次不拆(职责耦合更深,已列入后续)。

## 验证(按 $nanaui-validation)

- 每阶段:`cargo test -p nana-ui-vue -p nana-ui-runtime -p nana-ui` + workspace clippy。
- Phase B/C:新对账测试与顺序锁定测试即行为证据。
- Phase D:差分测试 + 用 `nanaui-agent-debug` 无头会话(VueAgentSession)对代表性示例截图与 a11y 树前后对比,确认无行为漂移;性能证据用可重复断言(属性变更只投影脏 widget)而非墙钟时间。
- Phase E:全量测试 + clippy 确认纯移动。

在当前分支工作,不默认创建 PR;全部完成后经用户确认再提交。