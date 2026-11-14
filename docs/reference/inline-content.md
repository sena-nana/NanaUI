# InlineContent

`InlineContent` 是 Runtime 的 canonical 内容值。`PlainText` 共用原有字符串与单段文本路径，`StyledText` 使用 `RichText` 的 canonical ranges，`Mixed` 在同一份 source 中以 U+FFFC 引用普通 `StableNodeId`。Builder、range replacement、Rust view 与模板均投影到这个值；没有 span wrapper、逐字 UI 节点或第二棵树。

```rust,ignore
let details = cx.create_detached_component(document, Button::new("详情"))?;
let content = InlineContent::builder()
    .text("CPU ")
    .child(details.stable_id())
    .text(" 42%")
    .build();
let paragraph = cx.create_component(document,
    InlineFlow::new(content).width(LengthSpec::Px(240.0)))?;
```

`Text::content`、`Button::content`、`Chip::content`、`ActionMenuItem::content` 接受同一个内容值；原有字符串构造保持可用。纯文字内容不保存额外的 canonical wrapper，删除最后一个 span 自动回到 plain 表示。显式 `accessible_name`、`InlineTextSemantics::accessible_text`、`copy_text` 与绘制字符串分离；未覆盖时使用 atomic child 的当前文字语义，安全输入不会经复制或聚合 a11y 文本投影泄露内容。

## 声明式投影

```rust,ignore
view::inline()
    .text("CPU ")
    .child(view::button("详情"))
    .span(" 42%", RichSpanStyle::new().size(18.0))
```

```text
<Inline>
    "CPU " <Button>"详情"</Button>
    <Span style={RichSpanStyle::new().size(18.0)}>" 42%"</Span>
</Inline>
```

`<Inline>` 的字符串、表达式与 Vue 插值是段落 source，不创建 `Text` 子组件；元素是普通 retained atomic child。`<Span>` 是轻量 range 样式，支持一个文字值及 `style={RichSpanStyle}`，不会成为 Runtime component。动态文字沿用现有 NodeBindings；子项沿用普通 view 的 slot、key 和 lifecycle。

## 唯一布局与结果权威

RuntimeLayoutEngine 通过现有 shared intrinsic cache 测量 atomic node，向 `nana-text` 导出外部逻辑尺寸与 baseline。`nana-text` 负责 shaping、BiDi、CJK、line breaking、clamp 和 object placement；Runtime 将其位置写回同一个 node。子项内部仍由自己的 formatting context 布局。

导出尺寸包含已解析的 physical margins；子项的 LayoutBox 是 margin 内的 border box。文字与子项共享物理文本原点。RTL、vertical-rl/lr 与控件中已经放置的 tight label group 不另行计算一套文字位置。

单行 plain 文字及仅含 paint 样式的单行文字沿用原来的两个 box fragments，不因 glyph advance 改变而重建布局片段；glyph/caret 位置继续直接读取 native retained text layout。多行、影响 shaping 的样式、RTL/vertical、clamp 与 mixed 内容按 native line/run 发布片段。

`LayoutResult` 发布 TextLine、TextRun 和 InlineAtomic fragments，`source` 是该片段对应的逻辑 UTF-8 byte range，`node` 保持 atomic identity。`LayoutResult.participation` 是 Scene 判断 atomic 内容的依据，避免旧的重复 label 兼容逻辑吞掉独立 Text 子项。高级 Button/Chip/menu 文字直接复用 Runtime 的 retained text layout。Button 的文字预算从可用内容宽度扣除固定图标与间距，Menu 使用可伸展的 TextContent 槽；预算不随 Plain/Styled/Mixed 模式或文字自身 advance 改变。chrome 或槽宽改变显式失效文字约束；纯颜色 span 往返保持原布局结果。

Clamp/ellipsis 省略的 atomic subtree 保留 identity 和 state，但无可见 border box；Scene、hit test、a11y 接收同一不可见投影，恢复空间后重新出现。普通静止帧无 Inline work；paint-only 不触及 measure/placement；atomic 的固定外部尺寸与 baseline 保持时，局部重排停在 child，父段落不 reflow。尺寸改变通过已有 typed invalidation 和 bounded shape/layout flush 更新。

## 更新和校验

`MutationQueue::set_inline_content` 是底层批量接口，引用的 node 必须在该批结束后成为段落的直接 child，属于同一 document。它不自动创建或删除子项。`InlineFlow`/控件投影及 `AppContext::set_inline_content` 会维护 atomic 顺序并删除被替换的 atomic subtree。使用 typed component 时，持续的应用状态仍应更新该 component 的 `content`；直接 world mutation 与现有 `set_text` 一样属于低层操作。

提交校验拒绝重复引用、错误 source offset、跨 document、悬空引用、在流外/float 的 atomic child，以及控件内容中嵌套的交互 role 或 focusable node。后来插入后代、改变 role/interaction、reparent 或 style 时也会重新检查受影响的 inline owners。未引用的正常流 child 被拒绝；独立 absolute/fixed chrome slot 走已有 Overlay placement。auto block extent 与依赖该 extent 的 atomic child 形成约束环时，layout 明确失败，不持续迭代。

Mixed 内容需要宿主使用产品 `NanaTextEngineShaper`；只有尺寸估计的 TextShaper 无法提供真实 line/object placement，会返回 `InlineTextEngineRequired`。

## 验证入口与剩余迁移

- `cargo test --locked -p nana-ui-scene --test scene_contracts inline_content`：真实字体、换行、替换、source map、RTL/vertical、margin、clamp、identity、nested interaction 与零父 reflow。
- `cargo test --locked -p nana-ui-runtime --features view-macro --test contracts inline_`：Rust/template、动态 span 与无 wrapper 投影。
- `NANA_COMPONENT_REQUIRE_GPU=1 cargo test --locked -p nana-ui-devtools --features runtime-agent --test devtools_pixels inline_content_pixels`：真实 pixels，独立 atomic 文字、opacity 与 hover。
- `cargo test --release --locked -p nana-ui-scene --features benchmark --test scene_perf inline_cost -- --nocapture`：1k/10k plain Text/Button retained allocator bytes、RSS 观察与 idle flush p50/p95/p99。设置 `NANA_INLINE_COST_OUTPUT` 写 JSON。保留迁移前 executable，交错执行前后 executable，使用 `perf/contract.py --evaluate-inline-cost <manifest.json>` 判断 p95 ≤5%、retained memory ≤8%、entity 与 allocation 门禁；manifest 包含固定 `machine` 名称及至少三次互不重复的 `before`、`after` 报告相对路径。

成本 fixture 的 CPU 范围是 idle flush，不能据此声称首次布局或 dirty-frame CPU 成本已经验收。正在运行其他编译/测试时的 timing 只作观察。所有布局证据继续汇入现有 WorkCounters/catalog；`inline_reflows` 统计 native paragraph placement，`inline_atomic_measure_requests` 统计其向 shared cache 发出的请求，不声称每次请求都是完整 subtree measure。

ListItem/SidebarRow/FileTab、LabeledValue 和 Card 的本批部件迁移已接入同一 LayoutResult。ActionMenuItem 多行 option 的 rich content 继续使用一个 retained text primitive，Scene 将 canonical rich span 合并到组件文本的 paint spans；颜色变化保持 chrome layout generation。Select/Dropdown/SearchDropdown/ContextMenu 的 option 可用 `content(...)` 接收 canonical 内容，但 option row 是数据行而不是 retained paragraph：`label` 保存其纯文本并承担布局预算与 typeahead/a11y，`SelectOptionData::content` 只携带 fill-colour span 供 Scene 合并到行文本；atomic child、改变 shaping/装饰/标注的 span 在 debug 构建中拒绝，release 只保留文本。相关 row helper 与 index 查询见[组件布局审计](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/component-layout.md)。首次布局、dirty-frame 与未测平台边界使用 `target/performance/issue196-final/baseline-manifest.json` 固定记录，不能由单项 idle 成本替代。
