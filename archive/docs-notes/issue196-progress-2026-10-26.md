# Issue #196：当前分支推进与关闭缺口

日期：2026-10-26。基于 `main`，起始 HEAD `1f91962c744d7c0f3d8c99cf2ae10b94ad0a60dc`。本轮未创建分支、worktree、提交或修改远端 Issue 状态；工作区同时存在其他任务的 GPU/output/window 改动，本记录仅归属下述 Layout Foundation 工作。

## 按依赖顺序推进的结果

1. **#200 Sticky 产品缺陷已修复。** 原先正常流 y=20 的节点滚动 70 后落到 -50，声明 `top: 0` 没有生效。现在 Runtime Hit/A11y 与 Scene 使用同一个 Sticky projection，包含最近 scrollport、包含块边界、嵌套 Sticky、百分比 inset、RTL/vertical 和 reparent。滚动不改 LayoutBox 或 layout generation。依赖索引只访问当前 scrollport 的 Sticky 分支；1k/10k 无关 Sticky 对照保持同样的命中更新量。GPU 测试覆盖实际 RuntimeDocument 的滚动像素。
2. **#199/#204 执行与投影收口。** Flow / Inline 的 child 分类消费 `ParticipationKind`，atomic inline-block 不再因为内部块而被拆开；Overlay slot placement 移入自己的执行模块。正常流与 Absolute/Fixed 切换正确失效父 contribution / placement plan。Row/Column preset 明确声明 Flex，`from_layout` 保留调用者 intent。Rust、模板与显式 CSS 对照 LayoutNode、bounds、baseline、overflow、fragments；默认值等价由现有 Style authority 比较。
3. **跨 context intrinsic facts 复用修复。** 仅由父 context/constraint 变化触及的节点不再被无条件排除缓存复用；直接 metric mutation 及其 measure 祖先仍失效。缓存按实际 Flow/Flex/Grid/Inline/Overlay 标记来源。1k 实际组件矩阵得到 3000 次跨 context 命中、0 次 miss；状态、焦点、事件所属组件与 CustomRender resource Arc 身份保持。新增祖先访问计入现有 dependency counter，#259 的百种子 union closure 和 100k scaling 门禁保持通过。
4. **#205 首批几何迁移完成。** Button、Chip、ActionMenuItem 声明 lightweight Flex parts；主轴分配、收缩、对齐复用 Foundation Flex。布局一次发布 TextContent / leading / trailing / hint / spinner，ComponentGeometry 只消费已发布盒子并附加绘制数据。纯文字 Button 保持一个实体、两个原有 parts。hint/loading 变化正确更新，hover 保持布局结果身份。另修复“原先空 label 后来增加文字”复用旧 child-only snapshot、漏发文本片段的历史路径问题。
5. **#206 门禁接入现有目录。** `perf/scenarios/catalog.json.nana_layout_gates` 统一登记 #199/#200/#201/#205 与既有 #214/#259/#261/#262/#263；schema 校验测试来源和 WorkCounters 名称。Runtime/framework/dirty-frame/card-update 四个 benchmark 补齐实际 context transitions、intrinsic cache、baseline、cross-context 计数；CI 加入新结构测试及 authoring parity。没有另建 profiler 或伪造未测 counter。

## 第一阶段验证记录

以下针对 InlineContent 迁移前的第一阶段源码；日志保存在本机 `/tmp/nanaui-196-*.log`。截图在 `target/performance/issue196/`，不作为签入的历史基线。

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p nana-ui-runtime --lib -- --quiet` | 2217 passed，0 failed；1 个固定机器 timing 测试默认 ignored |
| `cargo test --release --locked -p nana-ui-runtime --lib issue259_local_frame_time -- --ignored --nocapture` | 本机单次观察通过：1k / 10k / 100k 的 p95 为 49.875 / 35.084 / 47.334 µs，100k/1k = 0.95，门槛 ≤1.75；不是改动前后基线 |
| `cargo test --locked -p nana-ui-scene --lib --test issue200_sticky -- --quiet` | Scene 236 passed，Sticky 集成 10 passed |
| `cargo test --locked -p nana-ui-runtime --features view-macro --test view_macro -- --quiet` | 43 passed |
| `cargo test --locked -p nana-ui-sfc -p nana-ui-view-codegen -- --quiet` | SFC 33、codegen 12 passed |
| `cargo clippy --locked -p nana-ui-runtime -p nana-ui-scene --all-targets --no-deps -- -D warnings` | passed |
| `cargo check --locked -p nana-ui-runtime --all-features --all-targets` | passed |
| `cargo check --locked -p nana-ui-runtime --features benchmark --bins` | passed |
| `cargo check --locked -p nana-ui-scene --features benchmark --bins` | passed |
| `python3 perf/contract.py --check-schema` / `--self-test` | passed |
| `python3 scripts/validate-runtime-performance.py --self-test` | passed |
| `python3 scripts/check-engine-boundary.py` / `check-package-boundary.py` | passed |
| `cargo test --locked -p nana-package -p nana-packager -- --quiet` | package 31、packager 35、Steam delta 6、CLI 2 passed |
| `cargo fmt --all -- --check` / `git diff --check` | passed |

GPU 命令使用 `NANA_STICKY_REQUIRE_GPU=1`、`NANA_COMPONENT_REQUIRE_GPU=1`，执行 `cargo test --locked -p nana-ui-devtools --features runtime-agent --test issue200_sticky_pixels --test issue205_component_pixels -- --nocapture`，2 passed。Sticky 和迁移组件分别要求真实 GPU pixels；组件在真实指针输入后推进颜色动画时钟再比较 hover 像素。

按 CI 的临时 key 流程重新构建 `package-fixture --release --locked`，打包后执行 `nana-packager validate <app> --trust-key <public> --run --tamper-suite`。签名、布局/文件/身份、三个 pack、异目录启动、control 和六种篡改检查全部通过。原生 splash、首帧 handoff、installer round-trip、StaticAppPlan 明确报告未执行，不能从该 headless 自检宣称窗口验收。

## 继续推进：Inline specialization

本轮继续在当前分支接入 canonical `InlineContent`，没有另建布局树或文本引擎。Plain/Styled/Mixed 共用既有字符串、RichText range 和稀疏 atomic 引用；range edit 与直接构造归一，移除最后一个 span 回到 plain。InlineFlow、Text/Button/Chip/ActionMenuItem 的内容 API、Rust `view::inline`、模板 `<Inline>/<Span>` 与 Vue 插值共用该投影；普通子项仍是原 ComponentView/StableNodeId。

Atomic metrics 经共享 intrinsic cache 进入 nana-text object layout，包含 physical margins、baseline、RTL/vertical、wrap、clamp/ellipsis 与动态尺寸变化；Fragment 发布真实 source range，Scene/hit/a11y 消费同一结果。批量校验拒绝错误引用、脱流/float atomic 和控件内嵌套的 interactive role/focusable 后代。复制和 a11y 使用独立文本语义覆盖，不复制显示占位符。

Review 和真实 pixels 检查修复了以下额外路径：plain rich normalization 导致编辑器重复投影打断 smooth scroll；控件 tight label 与 native paragraph 的居中偏移被叠加；Scene 的旧重复 label 规则误吞独立 atomic Text。高级控件文字现在直接复用 retained text layout，atomic 自己绘制。Context 切换但 child border box 不变时，额外发布 child participation，保证结果与冷重算相同。Button 原先从自身 tight text width 反推 paragraph budget，会在 glyph 变化后漏发约束失效；现改为与内容模式无关的 chrome 预算。Menu hint/槽宽变化同步 TextDirty::CONSTRAINT，纯颜色 span 往返验证 native layout identity 和零 measure/placement。仅尺寸估计的宿主 shaper 在控件预算内裁短 no-wrap run 时，单独保留自然宽度，避免新增图标后 auto width 无法扩展；原有增减图标回归断言保留。小高度控件的图标预算与 parts 共用同一缩放函数。

新增 `inline_reflows`、`inline_atomic_measure_requests` 接入同一个 WorkCounters、四个 benchmark 输出、catalog/schema 与 CI。固定 metric atomic 作为现有局部 layout island，文字变化保持外部 metric 时父段落零 reflow；paint-only 零 measure/placement。64/1024 atomic fixture 约束线性工作量，冷对照检查全部 LayoutResult geometry 和 focus。

详细合同和命令见 [inline-content.md](../../docs/reference/inline-content.md)。当前机器原始报告、前后 executable 和 GPU PNG 在 `target/performance/issue196-inline/`；没有把生成输出作为源码基线签入。

## Inline 阶段最终功能验证

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p nana-ui-runtime --lib -- --quiet` | 2219 passed、0 failed、1 ignored；`/tmp/nanaui-inline-runtime-final-v8.log` |
| `cargo test --locked -p nana-ui-scene --test inline_content -- --quiet` | 16 passed；包括 real-font/native objects、RTL resize、source map、secure a11y/copy、paint-only 往返、loading/hint 与短高度图标预算 |
| `cargo test --locked -p nana-ui-scene --lib --test issue200_sticky -- --quiet` | Scene 236、Sticky 集成 10 passed |
| `cargo test --locked -p nana-ui-runtime --features view-macro --test view_macro -- --quiet` | 45 passed |
| `cargo test --locked -p nana-ui-sfc -p nana-ui-view-codegen -- --quiet` | SFC 34、codegen 12 passed |
| `cargo clippy --locked -p nana-ui-runtime -p nana-ui-scene --all-targets --no-deps -- -D warnings` | passed；`/tmp/nanaui-inline-clippy-final-v9.log` |
| `cargo check --locked -p nana-ui-runtime --all-features --all-targets` | passed；`/tmp/nanaui-inline-allfeatures-v7.log` |

GPU 使用 `NANA_STICKY_REQUIRE_GPU=1 NANA_COMPONENT_REQUIRE_GPU=1 cargo test --locked -p nana-ui-devtools --features runtime-agent --test inline_content_pixels --test issue200_sticky_pixels --test issue205_component_pixels -- --nocapture`：3 passed。日志 `/tmp/nanaui-inline-gpu-final-v4.log`；复核 `target/performance/issue196-inline/inline-content.png` 中混排、CJK 和控件内独立星形子项。opacity 变化实际改变 child pixels，hover 保持零布局工作。

`python3 perf/contract.py --check-schema` / `python3 perf/contract.py --self-test`、`validate-runtime-performance.py --self-test`、engine/package boundary 检查均通过。重新编译 `package-fixture --release --locked` 并按 CI 流程打包、执行 `nana-packager validate <app> --trust-key <public> --run --tamper-suite`：启动、签名、pack、control 与六种篡改验证通过，详见 `/tmp/nanaui-inline-package-final-v2.log`。原生 splash/handoff、installer、StaticAppPlan 仍明确 not-executed。

此前缺约束失效、兼容 shaper 图标扩宽失败和 timing 比例失败的原始日志保留；最终结果只引用修复后对应日志，不删除失败观察。

## 最终固定机器成本观察

最终源码预编译 release test executable 后停止本轮编译/功能测试，独立运行 `runtime-timing-final issue259_local_frame_time --ignored --nocapture`：1k / 10k / 100k 的 p95 分别为 31.584 / 38.750 / 50.750 µs，100k/1k=1.61，通过 ≤1.75 门槛。日志 `/tmp/nanaui-inline-local-timing-final.log`。此前 6.83 的失败与 0.78 的复测原始日志保留；这些是本机观察，不作为性能根因证明。

再将原有 `inline-cost-before` 与最终 `inline-cost-after-final` 交错运行九组，交替前后顺序。两者均执行 `--exact inline_plain_text_fixed_machine_cost --nocapture`，用 `NANA_INLINE_COST_OUTPUT` 输出独立 JSON。执行 `python3 perf/contract.py --evaluate-inline-cost target/performance/issue196-inline/validated-cost-manifest.json`，四项全部通过，未更改 p95 ≤1.05 / memory ≤1.08 门槛：

| workload | idle p95 after/before | retained bytes after/before | entity / allocation |
| --- | --- | --- | --- |
| Button 1k | 0.878 | 1.00660 | passed / passed |
| Button 10k | 0.930 | 1.00734 | passed / passed |
| Text 1k | 0.900 | 1.00977 | passed / passed |
| Text 10k | 0.914 | 1.01180 | passed / passed |

报告、manifest、executable SHA-256 与 `validated-cost-verdict.json` 保留在 `target/performance/issue196-inline/`。RSS 只作观察，不能从 allocator 复用的 RSS 推导 live-byte 增量。测量时系统索引服务和其他项目测试/编译仍在运行，前后 process context 已保存；不能由这些小于 1 的 idle 比例声称普遍 CPU 提速或已完成完整 Foundation 性能验收。

## 后续组件迁移：ListItem / SidebarRow / FileTab

ListItem 的 fallback label/note/detail 不再由 `ComponentGeometry` 计算位置，Foundation 用 shared Flex parts 发布 TextContent/Note/Detail；绘制层只消费结果。普通 leading/content/trailing slots 继续读取同一棵 retained tree 的 child placements。SidebarRow、FileTab 共用这条路径。RTL 对齐与隐藏 slot 的预算更新有回归；纯 label 直接发布文字槽，不额外测量 secondary strings。

本批 review 检查了 typed invalidation、primary paragraph budget、隐藏 child、无文字 slot、paint-only、source fragments 与 snapshot reuse。验证结果：

- `cargo test --locked -p nana-ui-runtime --lib issue205 -- --nocapture`：7 passed，覆盖 cold/full oracle、Flex/Grid/Inline 切换、selected/focus 保持、RTL/隐藏 slot，以及 1k/10k 单实体/4 parts 和 idle 零工作。
- Runtime 全量：2222 passed、0 failed、1 ignored（`/tmp/nanaui-row-runtime-full.log`）；其后纯 label 等价快路径由最终 7 项回归和 clippy/all-features 再验证。
- `issue205_row_pixels` 宽/中/窄三行与保存的迁移前 RGBA 逐字节一致；`inline_content_pixels`、`issue205_component_pixels` 一并通过。PNG/RGBA 在 `target/performance/issue196-inline/rows-before.*` / `rows-after.*`。
- Runtime/Scene clippy `--all-targets --no-deps -- -D warnings`、Runtime all-features/all-targets、perf schema/self-test 均通过。
- 独立运行前后 release executable，交错九组 `list_item_fixed_machine_cost`：1k / 10k 的 idle p95 比例 1.03872 / 1.03607，retained allocator bytes 比例 1.00353 / 1.00401；entity 和 allocation 门禁通过。保持原来的 1.05 / 1.08 门槛。manifest 设置 `workload_family: "list_item"`，原始报告和 executable SHA 在 `rows-cost-manifest.json`，结果在 `rows-cost-verdict.json`。这仍只覆盖 idle CPU 与 retained bytes。

## 后续组件迁移：LabeledValue

label/value 的自然宽度与可读性优先级进入 shared Flex parts，LayoutResult 发布 Label/Value，Geometry 只附加文本与 paint；path 中间省略只消费已发布宽度，存储、tooltip 与 a11y 继续保留真实路径。显式 padding 与 RTL 生效，普通 action child 放在 inline end，修复此前单 child 的 SpaceBetween 把 action 放到行首、挤掉 label/value 的问题。

字段更新只重排部件，不重新测量外部盒子。cold oracle 暴露并修复了旧的视觉更新粗清理：内容变更已有 typed seed，不再额外清掉未变化的 child LayoutResult。action 复用位置时继续保留同一结果、焦点和身份。

- Issue #205 最终 10 项通过；包含 value 动态更新 measure=0、RTL、action、cold/full、padding/path raw value、paint-only 与 1k/10k 单实体/3 parts。
- Runtime 全量 2225 passed、0 failed、1 ignored；Scene 236、mixed inline 16、Sticky 10 passed。Runtime/Scene clippy 和 Runtime all-features/all-targets 通过。
- 实际 GPU 四项通过（`/tmp/nanaui-value-gpu-final.log`）。value/Card fixture 对照保留的迁移前图像，action 区与预留 spinner 修正区域外保持逐字节相同；Card 此时尚未改动。`42 → 43` 实际改变 glyph pixels，action 的 LayoutResult 和位置保留。图像为 `surfaces-before.*` / `surfaces-values-after.*`。
- 交错九组 release 成本：LabeledValue 1k / 10k idle p95 比例 1.00555 / 0.99683，retained bytes 比例 1.00488 / 1.00586；entity/allocation 全通过。manifest/verdict 为 `values-cost-manifest.json` / `values-cost-verdict.json`，仍用同一 1.05 / 1.08 门槛。

## 2026-10-27 追加：Card Header

Card 的 Header/Spinner 由 shared Flex parts 计算，Geometry 只投影盒子和 elevation/文字 paint。Runtime 以标题专属字号、字重、单行约束生成 retained paragraph，Scene 复用它；body 字体继承保持原样。所有 context 的 intrinsic baseline 与 LayoutResult 从同一 Header/native layout 读取。LayoutInput 只保留标题的 inline contribution，block 空间已计入 title band；auto/hug 不再重复计入标题高度，spinner reserve 只约束 header。

Review 发现并修复了极窄标题的 ellipsis 外溢：只有溢出的标题（或无 retained layout 的兼容路径）增加 Header clip，正常宽度保留原 clip 链。真实 GPU 验证了宽度 0/4/16/32/60/120 不画出卡片外，ellipsis 不进入 spinner 间隔；正常标题/body 与保存的迁移前图像逐字节相同，spinner 的字体对齐/居中修正区域单独排除历史对照。

- Issue #205 14 项全部通过，含 header baseline 与相邻粗体 Text 对齐、body 字号继承、phase-only 零布局、RTL/窄宽度/空标题动态切换 cold oracle、hugging height 单计与 spinner reserve、1k/10k 单实体/4 parts。
- 全量 Runtime 2229 passed、0 failed、1 ignored；Scene 236、mixed inline 16、Sticky 10 passed。Runtime/Scene clippy、Runtime all-features/all-targets 及 no-default-features 通过。
- 九组 release 交错对照：Card 1k / 10k 的 idle p95 比例 0.99724 / 1.02944，retained bytes 比例 1.00229 / 1.00295，entity/allocation 全通过。保留 `cards-cost-manifest.json` / `cards-cost-verdict.json` 与前后 executable SHA。该 workload 冻结动画时钟，不能代表动画活动帧 CPU；auto size 的纠正也可能改变 Scene 空间索引的分布，不能把 retained 比例解释成单个新字段的成本。

## 2026-10-27 追加：FormField、Select 与菜单行 helper

FormField 的 label、support 和错误指示器现在由 shared Flex parts 发布为 `Label`、`Hint`、`LeadingContent`；control 继续是普通 retained child。Select trigger 发布 `TextContent`、`Hint`、`TrailingContent`，fit-options 的最宽选项和 RTL handle 位置保持原语义。SelectMenuGeometry 与 ComponentGeometry 增加逻辑 index 查询；select/context option 的 label/hint 分割共用 `menu_option_text_boxes`，不产生 wrapper 节点。

本批 review 还发现 ActionMenuItem 的 rich content 虽然已经进入 Runtime，但 Scene 组件文本路径此前只读取普通 `text_spans`，会吞掉 option span 的颜色。现在 Scene 将 canonical rich spans 合并到组件 text primitive；单个 row 继续共享 chrome/layout，paint-only span 更新保留 layout generation。真实 GPU 测试覆盖了 span 到达和 paint-only 不重排。

随后又删除了 Select trigger 的旧 fallback 几何：label/handle 不再从 outer box 和 authored padding 重算，生产路径只读取 `LayoutResult.parts`；动态 popup row 仍由 viewport/scroll 投影计算，并通过共享 text-box helper 处理 label/hint。

- Issue #205 新增 FormField/Select 冷布局、控件结果保持、RTL、highlight paint-only、1k/10k bounded-parts 检查；完整 Runtime lib 当前 2232 passed、0 failed、1 ignored。
- `NANA_COMPONENT_REQUIRE_GPU=1 cargo test --locked -p nana-ui-devtools --features runtime-agent --test issue205_component_pixels -- --nocapture`：2 项通过，包含 FormField/Select 的真实 GPU scene 路径。
- release executable 的 FormField/Select 10k 组合测试：1 项通过，测试体 0.83s（`/tmp/nanaui-form-select-release-focused-time.log`）；这是本机绝对观察，没有本批改动前的同一 executable 对照，不能解释为回归比例。
- 完整 CPU 基线已在 rich paint 修正后重新生成：`target/performance/issue196-final/baseline-manifest.json`，包含 Runtime initial/steady/idle、dirty-frame 350 cells（100 samples/20 warmup）和 Scene initial/local/idle/construction；manifest 固定 machine、HEAD 与 executable/report SHA256。它是当前最终工作区的基线快照，不冒充 Foundation 迁移前的前后比例。

## 2026-10-27 最终交付检查

当前工作区源码的最终结果：

| 检查 | 结果与日志 |
| --- | --- |
| Runtime 全量 lib | 2232 passed、0 failed、1 ignored；`/tmp/nanaui-196-final-runtime-full.log` |
| Scene lib / inline_content / Sticky | 236 / 16 / 10 passed；`/tmp/nanaui-card-scene-full.log` |
| Rust authoring / SFC / codegen | 45 / 34 / 12 passed；`/tmp/nanaui-196-delivery-authoring.log`、`/tmp/nanaui-196-delivery-sfc.log` |
| macOS 真实 GPU | 原有 5 个 test executable 共 6 项，加上 FormField/Select component pixels 2 项通过；强制 GPU 不可用时失败 |
| Runtime/Scene clippy / Runtime features | `--all-targets --no-deps -- -D warnings`、all-features/all-targets、no-default-features 通过 |
| package / packager / Steam delta / CLI | 31 / 35 / 6 / 2 passed；`/tmp/nanaui-196-delivery-packager-tests.log` |
| 最终 signed package | 重新编译 fixture、签名、独立目录启动、control 和 tamper suite 通过；`/tmp/nanaui-196-delivery-package.log` |
| schema / self-test / engine/package boundary | 全通过 |
| fmt / diff whitespace | `cargo fmt --all -- --check`、`git diff --check` 通过 |

对最终 release executable 独立运行 #259：1k / 10k / 100k p95 为 48.625 / 33.417 / 43.750 µs，100k/1k=0.90，通过 ≤1.75；`/tmp/nanaui-196-delivery-timing.log`。保留全部先前 timing 结果，不将某一轮比例解释成根因或普遍提速。

最终程序再与保存的 InlineContent 迁移前程序交错九组，普通 Button 1k / 10k idle p95 比例 0.92782 / 0.94395，普通 Text 为 0.93038 / 0.90482；retained bytes 比例仍为 1.00660 / 1.00734 / 1.00977 / 1.01180，entity 和 idle allocation 均通过。结果在 `delivery-cost-manifest.json` / `delivery-cost-verdict.json`，before 包含第一阶段 Foundation 改动，不能倒推全部 Foundation 的最初成本。

只读检查 NanaLive / NanaType 未发现这些新增 LayoutResult/Fragment/Part/ExtractedNode 的直接 struct literal；本轮没有执行其完整构建或运行，不能把这项源码检查称为 consumer 验收。

本轮没有提交、推送或改动远端 Issue。`main` 的当前 HEAD 为其他并发工作的 `49f0cac68`，本记录所述改动仍在当前工作区。最终读取 #196 为 OPEN。

## 2026-10-27 追加：option row 的 canonical 内容（review 收口）

上一轮在最终交付检查之后开始为 Select/Dropdown/SearchDropdown/ContextMenu option 接入 `content(...)` 与 `SelectOptionData::content`，在 Scene 测试初始化器未补字段处中断（`nana-ui-scene` lib test 编译失败）。本轮 review 后补齐并修正：

- **合同收紧。** option row 是数据行而不是 retained paragraph，Scene 只能把 fill colour 合并进行文本。四处重复的 builder 改为共用 `select::option_content`：`label` 保留纯文本承担布局预算、typeahead 和 a11y，`content` 只保留 fill-colour span；atomic child、ruby/object 与改变 shaping/装饰/标注的 span 在 debug 构建拒绝，release 只保留文本（不再把 U+FFFC 占位符带进 label）。ActionMenuItem 仍走完整的 retained control content 路径，不受影响。
- **ContextMenu hint 回归。** geometry 已把 hint 从 `label · hint` 拆成独立区域，但 Scene overlays 只画 label，带 hint 的 ContextMenu 行在画面上丢了 hint。现在以独立 collection 命名空间 `CONTEXT_MENU_HINTS` 绘制行尾 hint（固定 row slot 已被背景/label/icon 共用）。
- **测宽来源。** ContextMenu 的 label/hint 分割原本用 `estimated_text_width`，现改为 owner 的 `chrome_text_measure`，与 Select menu 行同一来源；无 hint 行几何保持原样。

新增 `quiet_text_beside_labels` 两项：styled option 在 Select 行与 ContextMenu 行都画出 fill span，ContextMenu hint 独立成文本且不再出现 `·` 拼接（修复 Scene hint 前该项失败）；debug 构建拒绝改字号的 option span。

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p nana-ui-runtime --lib -- --quiet` | 2232 passed、0 failed、1 ignored |
| `cargo test --locked -p nana-ui-scene -- --quiet` | lib 236 及全部集成测试通过（含 quiet_text 4、inline_content 16、Sticky 10） |
| `cargo clippy --locked -p nana-ui-runtime -p nana-ui-scene --all-targets --no-deps -- -D warnings` | passed |
| `cargo check --locked -p nana-ui-runtime --all-features --all-targets`、`cargo check --locked -p nana-ui -p component-gallery` | passed |
| `NANA_COMPONENT_REQUIRE_GPU=1 cargo test --locked -p nana-ui-devtools --features runtime-agent --test issue205_component_pixels --test inline_content_pixels` | 3 passed（macOS 真实 GPU） |
| `cargo fmt --all -- --check` | passed |

公开 API 变化：四个 option 类型与 `SelectOptionData` 新增 `content` 字段并去掉 `Eq`。只读检查 NanaLive / NanaType 只经 builder 使用这些类型、没有结构体字面量或 `Eq` 依赖；未构建这两个 consumer。未跑 `--features snapshots` 套件；gallery 快照中没有带 hint 的 ContextMenu。

## 2026-10-27 追加：#201 / #202 远端验收清单的本地核对

只读核对当前工作区，未改源码、未写远端 Issue。判定分三档：**可勾**（有直接测试）、**证据不足**（实现在，缺对应测试）、**不可勾**（实现本身有缺口）。

### #201 Viewport

| 验收项 | 判定 | 证据 / 缺口 |
| --- | --- | --- |
| Scroll 与 child FormattingContext 解耦 | 可勾 | `ScrollView::projected_style` 只写 overflow，没有 ScrollFlex/ScrollGrid；`issue201_scroll_is_orthogonal_...` 在 Block/Flex/Grid 下滚动保持 established context、LayoutBox、generation |
| ScrollView 不拥有第二 layout engine | 可勾 | ScrollView 只持交互/scrollbar 状态，无 measure/placement；注意公开的 `AppContext::set_scroll_metrics` 可注入非布局来源的 metrics |
| content extent 来自 authoritative LayoutResult | 可勾（附注） | `ContentBoundsIndex` 增量读取已提交的 layout 盒子，由 WriteLayout 驱动失效，`LayoutResult.scroll_extent` 同源；`scroll_bounds` 9 项含与全遍历对照。文本编辑器另有 `text_scroll_metrics` |
| scroll-only 不重做 intrinsic/content layout | 可勾 | issue201 measure/placement/shape/spawn 全 0；Sticky 集成与 GPU pixels；Scene `scroll_offset_delta_updates_...` |
| clip chain 供 Scene/Hit/A11y 一致消费 | **不可勾** | 三处各自推导 clip；A11y 只裁 EmptyState/ModalFrame，不读 overflow/scroll clip，也不导出 clips-children；`LayoutResult.clip` 只参与相等比较、无消费者；没有比较三方 clip 矩形的测试 |
| virtualization 与 scroll layout 语义分离 | 可勾 | `virtualize_retained` 按业务 key 物化、只读 scroll offset/metrics；`sync_virtual_list_skips_mutation_...`、issue262 a–e、`virtual_retention` GPU |
| Grid/Inline/CustomRender 自然放入 scroll viewport | **证据不足** | Grid 由 issue201 覆盖；Inline 只有 inline-block 子项，无原生混排段落；CustomRender 在 ScrollView 下没有布局/clip/hit 测试（现有的是 host-texture Thumbnail 与纯 paint 数学） |

性能段要求的 `scroll_viewport_updates`、`scroll_content_extent_recomputes`、`scroll_layout_reflows`、`scroll_clip_updates`、`virtualization_despawned` 不在 WorkCounters；extent 刷新计数只在 test/benchmark cfg 下存在，`virtual_rows_materialized_from_layout` 仅列表。

### #202 Replaced Content

| 验收项 | 判定 | 证据 / 缺口 |
| --- | --- | --- |
| 共用 intrinsic/aspect/fit contract | **不可勾** | intrinsic/aspect 共用 `world/replaced.rs` 与 measure；fit 有两套数学：HostTexture/Video/CustomRender 用 `ContentFit::fitted`，Image object-fit 走 `BackgroundImageFit` + `url_dest_rect`，另有只认 contain/fill 的 `parse_content_fit`；Canvas 不是 `ReplacedResource` |
| Flex/Grid/Inline/Absolute 均可参与 | 证据不足 | URL 图片 Flex→Grid→Inline（issue197）、CustomRender Grid/Inline/Flex 与 Absolute/Fixed（issue199）；缺 natural size 图片在 Absolute/Grid/Inline 的尺寸断言，HostTexture/Video 未在真实布局中断言 AtomicInline |
| context 切换不重建 GPU/resource identity | 证据不足 | Runtime 侧 `Arc::ptr_eq` 与 0 spawn（issue199，fixture renderer）；GPU 侧 rebind 只看 handle/generation，key 不含布局位置，但没有 host-texture 跨 context 无 rebind 的测试 |
| frame/texture 更新不触发布局 | 可勾 | `issue263_video_frames_lay_nothing_out`、`issue263_fit_and_sampling_lay_nothing_out`、`issue259_replaced_content_costs_the_same_at_any_size`、`texture_content_reuses_prepared_ui_and_replacement_rebinds` |
| intrinsic metadata change 精确失效 | 可勾（附注） | issue263 unknown→known 单 seed、共享图片经索引 100 通知/50 seed、显式尺寸下分辨率风暴 0 seed、逐轮冷对照；nana-ui natural size 端到端。缺"相同 metadata 重报 0 工作"的 Runtime 测试；HostTexture/Video 分辨率须应用自行上报 |
| Inline baseline 有正式 policy | 证据不足 | `baseline_metrics` 对 replaced 明确用 border-box 底边，属布局侧规则；判定谓词在 `baseline_metrics` 与 `baseline` 两处重复，没有断言 replaced atomic 底边基线的测试 |
| CustomRender 保持 presentation anchor | 可勾 | `CustomRenderNode` 只含 renderer/resource/revision/fit/params 等呈现字段，params 不被 Runtime 解释；`custom_render_extension_requires_nonempty_backend_neutral_keys`、hit 顺序跟随文档 |

### 本轮运行

| 命令 | 结果 |
| --- | --- |
| Runtime lib：issue201 / 262 / 263 / 199 / 261 | 1 / 6 / 7 / 2 / 7 passed |
| Runtime lib 引用项（scroll_bounds、virtualize_retained、viewport_tests、issue197、participation、issue259_replaced、custom_render、natural_sizes 等 18 个过滤） | 51 passed |
| `nana-ui-core` geometry（ContentFit） | 18 passed |
| Scene lib 引用项（scroll/clip/hit/a11y 一致性、虚拟列表审计、custom render） | 7 passed；Sticky 10、inline_content 16 |
| `nana-ui --features hosted,gpu` natural size / texture identity / scroll paint | 14 passed |
| devtools `hidden_hit`、`host_texture_paint`、`issue200_sticky_pixels`（强制 GPU）、`virtual_retention` | 2 / 6 / 1 / 3 passed（macOS；除 Sticky 外无强制 GPU 开关，同机强制 GPU 的测试可用 adapter） |

结论：#201 可勾 5/7，#202 可勾 3/7。#201 的 clip chain 与 #202 的 fit 统一需要实现改动；其余缺口是补测试。

## 2026-10-27 追加：#201 / #202 补测试，以及测试揭出的三处布局缺陷

按上一节"证据不足"的四项补测试。新测试在 layout verify（每趟对照全量布局）开启下跑，揭出三处产品缺陷，一并修复：

1. **跨 context intrinsic facts 复用路径漏掉自然宽高比换算。** `measure_node` 命中 cross-context intrinsic cache 时直接用缓存的 preferred facts 求 used size，只写了宽的替换盒得到自然高度（120×80 图片宽 60 时高 80 而非 40）；结果依赖缓存路径，Grid 中由 verify 发现。换算抽成 `transfer_natural_ratio`，两条路径共用。
2. **Grid `stretch` 不认自然宽高比。** `ratio_filled_height` 只看样式 `aspect-ratio`，只写了宽的图片被拉到区域高。
3. **Flex 行 `stretch` 同样不认。** `cross_axis_is_definite` 增加 natural ratio 参数；两处均经 `has_natural_ratio` 判定。

另把 `baseline_metrics` 与 `baseline` 里重复的 replaced 判定合成 `aligns_by_bottom_edge`，语义不变（仍含 gradient content image、skipped canvas、ImageViewer，与 replaced 资源索引的口径不同，未强行合并）。

| 新测试 | 证明 | 变异验证 |
| --- | --- | --- |
| `world/issue202.rs` `issue202_a_natural_size_reaches_every_context` | 无尺寸 / 只写宽两张图片在 Flex、拉伸 Flex 行、Grid、拉伸 Grid、Inline、Absolute 下得到 120×80 / 60×40，participation 正确，2 个 replaced seed、0 全文档回退，对照冷布局 | 修复前 Grid（verify）与两种 stretch 失败 |
| `world/issue202.rs` `issue202_replaced_content_sits_on_the_text_baseline_by_its_bottom_edge` | 图片与 CustomRender 在 baseline flex 行和原生 `InlineFlow` 段落中底边落在文字基线 | 关闭底边规则后失败 |
| `world/issue201.rs` `issue201_a_paragraph_and_a_custom_renderer_scroll_without_layout` | ScrollView 内原生混排段落（含 atomic）+ host-texture `GpuTextureView`：滚动零 measure/placement/shape/spawn，generation、全部盒子与 CustomRender 描述不变；hit index 只走 scroll patch，命中与 a11y 都在滚动后位置，滚出视口的 atom 不再命中 | — |
| `nana-ui` `scene_paint/replaced_context_tests.rs` | 真实 RuntimeDocument 中 host texture 在 Flex 列 → Grid → Flex 列 → Flex 行之间移动，同一 painter 逐帧在新位置画出纹理，`gpu_buffer_reallocations == 0`，描述 `Arc` 不变 | 强制 rebind 后失败（reallocations 1） |

发现但未改：普通块级流（Block + inline-block）的行盒把原子项顶部对齐，不做基线对齐，`LayoutStyle` 也没有 `vertical-align`；`LayoutResult` 只为文本发布 baseline，replaced 的底边基线只在布局引擎内部。已写入 `docs/reference/layout.md`。

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p nana-ui-runtime --lib -- --quiet` | 2235 passed、0 failed、1 ignored |
| `cargo test --locked -p nana-ui-scene -- --quiet` | 全部通过（lib 236） |
| `cargo test --locked -p nana-ui --features hosted,gpu --lib -- --quiet` | 966 passed、1 ignored |
| clippy `-D warnings`：runtime/scene `--all-targets`，nana-ui `--features hosted,gpu --all-targets` | passed |
| runtime `--all-features --all-targets`、`--no-default-features` check；`cargo fmt --all -- --check`；`git diff --check`；engine boundary | passed |
| devtools（强制 GPU）`issue205_component_pixels`、`inline_content_pixels`、`issue200_sticky_pixels`、`host_texture_paint`、`virtual_retention` | 2 / 1 / 1 / 6 / 3 passed |
| ui-snapshots A/B（同一 release 二进制，环境变量关闭本轮自然宽高比改动作 before，私有基线在 scratch） | 像素 0 张差异；语义输出逐文件相同 |

语义套件对仓库基线另有 8 个 CHANGED（card / form-field / labeled-value / settings 的 light/dark），开关本轮改动时完全相同，来自此前 #205 迁移，仓库语义基线尚未重录；本轮未 bless。

判定更新：#201 "Grid/Inline/CustomRender 自然放入 scroll viewport" 改为可勾（6/7）。#202 "Flex/Grid/Inline/Absolute 均可参与"、"context 切换不重建 GPU/resource identity" 改为可勾；"Inline baseline 有正式 policy" 在原生段落与 flex 中可勾，但普通块级流行盒无基线对齐，建议附注后勾选或留待行盒补齐（5–6/7）。

## 仍不能记成完成的部分

- **#188/#192/#194 的后续迁移：** 首批 canonical 内容与混排已经落地，但 StandardVisual 的兼容 label/icon 字段仍在。FormField 的 Label/Hint/LeadingContent 与 Select trigger 的 TextContent/Hint/TrailingContent 已进入 LayoutResult；SelectMenuGeometry/ComponentGeometry 提供逻辑 index 查询，select/context option 的 hint 分割共用菜单 helper；ActionMenuItem 多行 rich option 已完成单 text primitive 的 paint/chrome 解耦。Select/ContextMenu option row 现在可携带 fill-colour span，但仍是数据行：不支持 atomic child 与改变 shaping 的 span，不能把首批控件的完整高级内容能力说成这些组件全量完成。
- **#206 专项比例：** 已有真正 InlineContent 迁移前 executable 的交错前后 allocator/idle-flush fixture；该 before 已包含第一阶段 Foundation 改动，不能用于声称全部 Foundation 的前后成本。1k/10k plain Text/Button 无额外实体和 idle allocation，retained live bytes 增量在约 0.7%–1.2%。最终九组 idle CPU p95 比例门禁通过；固定 5% 门槛保持不变，原始失败测量和 fail verdict 仍保留。当前完整 initial/dirty/idle CPU 基线快照见 `target/performance/issue196-final/baseline-manifest.json`，它补齐观测面但不构成历史 before/after 比例。
- **#201/#202 收尾：** ScrollView 在 Flow/Inline、Flex、Grid 的真实滚动保持 LayoutBox 与 generation 不变；#263 metadata/content/fit、100k 索引与 #199 resource identity 门禁已过。远端验收清单尚未逐项据证据更新，未写远端 Issue 或关闭 Epic。

## 未覆盖的边界

GPU pixels 来自 macOS。Windows/Linux 真机与外部 consumer 完整构建/运行未执行；这代表没有相应证据，不据此推断失败。packaging 的原生 splash/handoff、installer、StaticAppPlan 明确 not-executed，属于相应窗口/打包工作流边界，不将这些无关功能一概列为 #196 的新增功能阻塞。

当前结论：已继续完成 InlineContent 核心及 Button/Chip/ActionMenuItem、ListItem/SidebarRow/FileTab、LabeledValue、Card 的本批内容/部件收口，并补上真实混排、pixels、冷重算和成本观察；#196 仍保持开放。剩余工作由尚未完成的迁移与未覆盖的性能、平台证据决定，不以远端 OPEN 状态本身推断功能缺失。

## 2026-10-27 追加：叠层 Stack 与 flex 拆开（#204 / #199）

逐条核对 #199/#200/#204/#206 后发现，`Stack::overlay_layer()` 实际是绝对定位的 flex 行，叠层与行列共用 Flex context。按 SwiftUI `ZStack`、Compose `Box`、Flutter `Stack` 的语义补上叠层：

- `DisplaySpec::Layers`（CSS `display: layers`，NanaUI 扩展），对应 `FormattingContextKind::Layers` / `ParticipationKind::LayerItem`。在流子项共用一块区域，按 `justify-items` / `align-items`（子项 `*-self` 优先）对齐，默认起点；容器随最大子项；脱流子项照常定位；按文档顺序命中，后写的在上面。
- 实现复用网格算法：一行一列 `auto` 轨，子项都放进这一格；placement 用 `Grid2DLayout::fill_layer_cell` 把这一格撑满内容盒。最初用的 `1fr` 轨会在高度不定时吃满可用高度，所以改成了这种做法。
- `Stack::layers()` 新增；`overlay_layer()` 改为铺满父级的叠层；`.justify()` 在叠层上改写 `justify-items`，因此 NanaType 的 `anchor_layer` 写法不变。
- 门禁：
  - `world/issue204.rs` 三项：共用区域与尺寸加冷对照；九锚点、RTL、`stretch`、子项覆盖；后写的那层接住指针。
  - `view_macro` 的 `issue204_layers_css_and_rust_share_one_overlay_context`。
  - `issue199` 千组件切换序列加入 `Layers`。
  - CSS 解析单测。
  - 变异检查：关掉撑满格子后对齐测试失败，关掉同格放置后三项全部失败。

## 2026-10-27 追加：#206 内存门禁与 #199 剩余测试

| 改动 | 证据 |
|---|---|
| #206：布局后组件常驻内存门禁 | `tests/node_memory.rs` 的 `laid_out_nodes_keep_bounded_live_memory`：Text/Button/ListItem 各 1k、10k 个，挂载后跑满样式、文本、布局，量分配减释放。实测 1k 时 3.4/4.6/5.1 KB，10k 时 3.1/4.2/4.2 KB，上限约高 25%，且 10k 每节点不得超过 1k 的 1.15 倍。原测试只量挂载，没有计入 `LayoutResult` 和 parts。已进 CI 和 catalog（`component-memory`）。 |
| #199：模态框的 Overlay context | `issue199_a_modal_frame_places_its_slots_as_overlay_items`：Dialog 建立 `Overlay`，body/action 是 `OverlayItem`，body 变高后与冷布局一致 |
| #199：反复切换不增长 | `issue199_repeated_context_switches_keep_the_retained_cache_flat`：100 个子项在 Grid→Layers→Inline→Flex 之间切换 20 轮，每步一次切换。保留缓存（不计 intrinsic）在第一轮后不变，回到 Flex 时等于只排过 Flex 的冷缓存；在 Grid 期间 `plan_entries` 更多，可见比较能区分不同 context |
| 文档 | `layout.md` 去掉"算法还没拆成独立模块"这句过时说明，改写剩余缺口；`layout_document` 注释改为说明产品帧不走全量 |

模态框的 Overlay 随后也改走 `establish_context`：`UiWorld::places_modal_slots` 认出 `ModalFrame`，measure 与 placement 共用这一入口，placement 断言分派结果为 Overlay。探针确认 measure 本来就不会走到模态框的通用子项路径，所以行为不变；只删掉分派里的模态框分支时，模态框测试失败。

仍未做：`#200` 的 containing block 与 overlay 锚点求解，`#201` 的 clip 链，`#202` 的 fit 统一，`#204` 的跨 facade 工作量对照。

## 2026-10-27 追加：#204 写法对照

- `issue204_row_column_and_css_share_structured_layout_results` 重写：
  - 原测试的 CSS 一路是 `Stack::row(8.0).css(...)`，写的值与预设相同，删掉 CSS 也照样通过。现在改为从空布局写 CSS，并补上带意图的 `from_layout`。
  - 增加一个窄于内容的可用宽度（40px），让内在尺寸参与比较。
  - 发现预设的 `LengthSpec::Shrink` 没有 CSS 写法：`fit-content` 是 `FitContent`，40px 时结果不同（预设容器 88、子项不收缩；`fit-content` 夹到 40、子项被压缩）。因此按"同一份意图"分两组比较，两组之间只在宽松宽度下比较。
- 新增 `issue204_facades_share_identity_and_layout_work`：预设、视图函数 `row()`、带预设意图的 `from_layout` 节点数、节点种类、意图相同，首帧、局部修改和空闲帧的 measure/placement/生成/切换计数相同，空闲帧为零。
- 写法间有意的差别只有命中默认值，已写进 `rust-layout.md`。
- 随后给 `Shrink` 加了 CSS 写法：
  - `width: shrink` / `height: shrink` 是 NanaUI 扩展，与 `fill` 对称。
  - `Stack::min_width(0)` 像 CSS 的 `min-width: 0` 一样记下 `allow_shrink`（布局引擎不读这个字段，只为两种写法得到同一份意图）。
  - 两组对照合并成一组：预设、模板、`from_layout` 和纯 CSS 四种写法在 800px 与 40px 下意图逐字段相同、结果相同。

## 2026-10-28 追加：#204 第 5 条（定位与滚动只是投影）

- 新增 `issue204_position_and_scroll_css_and_rust_share_structured_results`：ScrollView、Rust 的 overflow 盒、纯 CSS 三种滚动容器，各放 sticky 头、absolute 角标和正文。三种写法的意图逐节点相同；参与方式、盒子、内容盒、溢出、滚动范围与偏移、clip 与包含块（按文档顺序比较）、fragments 都相同。
- 对照揭出：CSS 写 `top` / `left` / `padding-*` 时，会在 `logical_*` 组里额外留一条物理边记录（用于和逻辑边排先后），Rust 直接写字段时没有。
  - 新增 `LogicalEdges::has_only_physical`、`LayoutStyle::has_inert_physical_edges` / `drop_inert_physical_edges`。
  - 在 `view/style.rs::apply` 套完全部 CSS 补丁后丢掉这条记录。
  - 最初放在 `resolve_layout_intent`，但那会让每个带这种记录的 CSS 节点多复制一份布局，所以改在补丁应用处做。
  - CSS 单测确认：只有物理边时记录被丢掉、物理字段保值；有逻辑边时原样保留。
- #204 的 7 条验收项现在都有本地测试证据。

## 2026-10-29 追加：#201

逐条核对 #201 的 7 条验收项，补上 10-27 记为"不可勾"的 clip 链，并让它按 CSS 跟着包含块走（`absolute` 的包含块已在 68a191999 改成 CSS 的）。

| 验收项 | 判定 | 证据 |
| --- | --- | --- |
| Scroll 与 child FormattingContext 解耦 | 可勾 | `issue201_scroll_is_orthogonal_to_flow_flex_grid_and_inline_contexts`（Block/Flex/Grid 下滚动不改 context、盒子、generation）；`ScrollView::projected_style` 只写 overflow |
| ScrollView 是 facade，不拥有第二 layout engine | 可勾（附注） | `ScrollView` 只投影 overflow 与 scrollbar 状态，没有 measure/placement；公开的 `MutationQueue::set_scroll_metrics` 仍可注入非布局来源的度量 |
| content extent 来自 authoritative LayoutResult | 可勾（附注） | `ContentBoundsIndex` 读已提交的布局盒，`LayoutResult.scroll_extent` 同源，`world/scroll_bounds.rs` 9 项含全遍历对照。附注：范围仍把逃出滚动容器的 `absolute` 盒子和其中的 `fixed` 盒子算进去，CSS 不算（见下文"仍未做"） |
| scroll-only 不重做 intrinsic/content layout | 可勾 | issue201 全部滚动步骤 measure/placement/shape/spawn 为 0、generation 不变；Scene 侧 `issue201_paint_hit_and_accessibility_share_one_clip_chain` 的 `RuntimeDocument` 帧 generation 不变 |
| clip chain 供 Scene/Hit/A11y 一致消费 | **本轮完成** | 见下表 |
| virtualization 与 scroll layout 语义分离 | 可勾 | `sync_virtual_list_skips_mutation_until_the_visible_range_changes`、`issue262_gate_a`–`e` |
| Grid/Inline/CustomRender 自然放入 scroll viewport | 可勾 | Grid：orthogonal 测试；Inline 段落与 host-texture CustomRender：`issue201_a_paragraph_and_a_custom_renderer_scroll_without_layout` |

### clip 链的实现

- 权威：`world/clip_chain.rs`。`LayoutResult::clip` 改为最近一个够得着本节点的溢出容器（按包含块链，`fixed` 及其下为 `None`）；新增 `LayoutResult::clip_parent`：本节点取裁剪和滚动的节点，一般是父级，包含块在静态溢出容器之外的 `absolute` 节点是它逃出的最外层容器的父级。`UiWorld::layout_clip_chain` 按发布结果列链。`layout_anchors` 按节点（不再按父级）计算；定位种类变化（不只是是否定位）和开始/停止裁剪都记为 anchor 变化，下一次发布连同根节点自身一起重新发布。
- Scene：`compute_ancestor_state` 只对链上的祖先施加裁剪和滚动，被跨过的祖先只传变换和不透明度；祖先状态缓存的键加上 clip parent；滚动快路径对逃出该滚动容器的节点重算剔除范围；clip parent 变化计入层级变化。
- 命中：逃出的条目挂在 clip parent 的条目下（从该条目的子变换加上被跨过祖先的变换建出），滚动就地补丁不带上它；它覆盖的点按绘制顺序排先后；局部补丁先撤掉分支里逃出的条目，再把分支够不着 clip parent 的条目挂回原处；合成器呈现路径和 `layout_projection_transform` 同样跨过。
- 无障碍：逃出节点的变换从 clip parent 的子变换接续；新增 `AccessibilityNode::clip`（链上各容器按此刻滚动留下的视口区域）。组件铬裁剪同样只读链上的祖先。Select 菜单的可用空间同上。
- 顺带修复：一个节点开始裁剪或变成 `fixed` 时，结果沿用旧的"每个子项一个片段"列表，缺 ScrollViewport/Overlay 片段，直到别的变化才补上。冷对照门禁揭出。

### 门禁与变异

| 门禁 | 证明 |
| --- | --- |
| `world/issue201.rs` `clip_chain::issue201_the_clip_chain_follows_containing_blocks` | 流内行、以卡片为包含块的 `absolute`、以页面为包含块的 `absolute`、`fixed` 各自的 `clip` / `clip_parent` / 整条链 |
| `…::issue201_scroll_reaches_a_box_only_through_its_containing_block` | 两个容器 5 组偏移，不跑布局；无障碍位置与 `clip`、命中（就地补丁与重建的索引）都按链 |
| `…::issue201_a_menu_room_is_cut_by_the_clip_chain_alone` | Select 菜单空间只被链上容器截 |
| `…::issue201_a_moving_clip_chain_matches_a_cold_layout` | 卡片定位、里层溢出来回切换，与冷布局逐项相同（oracle 现在也比较无障碍 `clip`） |
| `…::issue201_a_patched_hit_index_follows_the_moving_chain` | 滚动平移活着时变链，局部补丁的索引在整页网格上命中与候选都等于重建 |
| `…::issue201_an_escaped_box_takes_the_transforms_it_passes_over` | 被跨过祖先的样式变换和合成器呈现变换作用于逃出盒子（无障碍、输入投影、命中），其滚动不作用 |
| `…::issue201_an_escaped_box_is_ranked_by_paint_order` | 树序在后的兄弟盖住逃出盒子时接住指针 |
| `nana-ui-scene` `runtime_document_clip_chain_tests.rs` 两项 | 真实 `RuntimeDocument` 帧：Scene 画出的位置与裁剪、无障碍 `bounds`/`clip`、命中、剔除逐个相同，滚动与链变化后亦然 |

变异（改坏后确认变红，再恢复，逐文件 `cmp` 确认与改动版本一致）：clip parent 忽略逃出（4 项 runtime + 1 项 scene 红）；命中构建不挂到 clip parent（runtime 滚动门禁 + 2 项 scene）；无障碍不接续 clip parent 变换（同）；Scene 不跳过被跨过祖先（2 项 scene）；片段复用不认新裁剪（冷对照）；anchor 根节点不重新发布（scene 链变化门禁）；滚动快路径不重算逃出节点剔除（scene 剔除断言）；不按绘制顺序排逃出条目（paint order 门禁）；补丁不撤旧条目（补丁网格门禁，候选出现重复）；`layout_projection_transform` 带上被跨过的滚动、呈现路径不接被跨过的变换、构建不接被跨过的变换（transforms 门禁）；补丁不按 clip parent 挂逃出根（补丁网格门禁 + scene 链变化门禁）。

### 仍未做

- 滚动范围（`ContentBoundsIndex`、`LayoutResult::overflow` / `scroll_extent`）仍把逃出滚动容器的 `absolute` 盒子和其中的 `fixed` 盒子算进这个容器；CSS 的 scrollable overflow 只算包含块链经过它的后代。要把这些盒子的贡献改挂到 clip parent 一侧，并在链变化时失效增量索引。
- 被跨过祖先的 `clip-path` 和组件铬裁剪不作用于逃出盒子（CSS 的 `clip-path` 会作用）；已写进 `layout.md`。
- 性能段点名的 `scroll_viewport_updates` / `scroll_content_extent_recomputes` / `scroll_layout_reflows` / `scroll_clip_updates` / `virtualization_despawned` 仍不在 `WorkCounters`。
- 未在 Windows/Linux 或 GPU 像素以外的外部 consumer 上运行；NanaLive/NanaType 只读检查没有 `LayoutResult` / `AccessibilityNode` 字面量。

## 2026-10-29 追加：#202

逐条重新核对 #202 的 7 条验收项（不沿用 10-27 的判定），补上 fit 统一和 min/max，并把五种替换内容放进同一组门禁。

| 验收项 | 判定 | 证据 |
| --- | --- | --- |
| replaced content 共用 intrinsic/aspect/fit contract | **本轮完成** | intrinsic/aspect：五种替换内容（图片、`Video`、`GpuTextureView`、`GpuView` 画布、普通元素上的 `CustomRenderNode` 锚点）都经 `ReplacedResource` 进同一索引、`LayoutContentKind::Replaced`，自然尺寸只经 `measure.rs` 的 `transfer_natural_ratio`（`issue202_c_every_replaced_kind_is_one_layout_item`）；min/max 按 CSS 2.1 §10.4 保比例（`issue202_d_min_max_keep_the_natural_ratio`，本轮修复）。fit：`ContentFit::fitted_size` 是唯一的 fit 数学，`LogicalRect::fitted`（HostTexture/视频/Avatar/Thumbnail、`texture_uv_at`）、画家 `url_dest_rect`（经 `BackgroundImageFit::object_fit`）、`ImageViewer` 都用它；标记里的 fit 读层叠 `object-fit` 与全部关键字（`issue202_f_markup_fits_every_replaced_kind_alike`、nana-ui `an_image_and_a_host_texture_share_one_fit`） |
| Flex/Grid/Inline/Absolute 均可参与 | 可勾 | `issue202_a_natural_size_reaches_every_context`（图片，六种 context）；`issue202_c_…` 把五种替换内容放进同样六种 context，participation、尺寸、冷对照逐一相同 |
| context 切换不重建 GPU/resource identity | 可勾 | Runtime：`issue199_a_thousand_components_keep_state_focus_and_resource_identity`（`Arc::ptr_eq`、0 spawn）；GPU：nana-ui `a_host_texture_moved_across_contexts_keeps_its_binding`（真实 `RuntimeDocument`，Flex→Grid→Flex 列→Flex 行，`gpu_buffer_reallocations == 0`） |
| frame/texture 内容更新不触发布局 | 可勾 | `issue263_video_frames_lay_nothing_out`（1200 帧 0 布局、intrinsic generation 不变）、`issue263_fit_and_sampling_lay_nothing_out`、`issue259_replaced_content_costs_the_same_at_any_size`、nana-ui `texture_content_reuses_prepared_ui_and_replacement_rebinds` |
| intrinsic metadata change 精确失效 | 可勾 | issue263 B/C/E（unknown→known 单 seed、共享图片 100 通知/50 seed、显式尺寸 0 seed）；本轮 `issue202_e_only_a_new_natural_size_lays_out`：同一尺寸再报一次 drain 为空；换成同尺寸的另一资源 0 replaced seed、0 frontier seed、0 measure、0 intrinsic bump；换成另一尺寸的资源只排读它的 2 个盒子 |
| Inline baseline 有正式 policy | **可勾（基线追加完成）** | 规则在布局侧：`LayoutEngine::aligns_by_bottom_edge` / `baseline_metrics` 以边框盒底边为替换内容基线；原生段落行盒与 `align-items: baseline` 的 flex 行按它对齐，行基线发布在 TextLine 片段（`issue202_replaced_content_sits_on_the_text_baseline_by_its_bottom_edge`）。普通块级流里同行原子的基线对齐已按产品决定补上（`vertical-align: baseline` 默认值），见下文「追加：块级流的原子基线」 |
| CustomRender 保持 presentation anchor 边界 | 可勾 | `CustomRenderNode` 只有 renderer/resource/revision/fit/params/dedicated_pass/checkerboard/zoom/sampling 等呈现字段，params 不被 Runtime 解释（`custom_render_extension_requires_nonempty_backend_neutral_keys`）；`issue202_c_…` 的 Anchor 只挂一个 `CustomRenderNode` 就以 Replaced 参与布局，Runtime 不需要应用状态 |

### 改动

- `nana-ui-core`：`ContentFit::fitted_size`（新，公开）是唯一 fit 数学，`LogicalRect::fitted` 改为调用它后居中，结果与原实现相同；`BackgroundImageFit::object_fit`（`Length` 之外的关键字映射到 `ContentFit`，`Auto` 即 `none`）与 `From<ContentFit> for BackgroundImageFit`。
- `nana-ui` 画家：`url_dest_rect` 不再自写 cover/contain/scale-down，改走 `object_fit()` + `fitted_size`；只有写长度的 `background-size` 保留自己的规则。
- `nana-ui-runtime`：
  - `measure.rs::transfer_natural_ratio` 读 min/max：两边都 auto 时按 CSS 2.1 §10.4 等比夹取；只写一边时，换算出的另一边再受它自己的 min/max 约束。缓存路径与新测量路径同一函数。
  - `ImageViewer` 的 `fitted_bounds` 改为 `ContentFit::ScaleDown.fitted_size`。
  - `gpu_slots.rs::parse_content_fit`（只认 `contain`）换成 `semantic_content_fit`：层叠的 `object-fit` 优先，`fit` 属性作 presentational hint，读全部五个关键字；`Video` 与 `GpuTextureView` 共用。
- `nana-ui-css`：`parse_object_fit` 改为 `ContentFit::from_object_fit` 映射。
- 文档：`layout.md` 的「替换内容」补 min/max、五种替换内容、fit 合同、门禁 A–F 与基线说明，纠正"`<img>` 的 object-fit 落到 `CustomRenderNode.fit`"的旧说法；`learn/core-contracts.md` 行号随之移动。

### 门禁与变异

| 门禁 | 证明 | 变异（改坏→变红→恢复→`cmp` 一致） |
| --- | --- | --- |
| `issue202_c_every_replaced_kind_is_one_layout_item` | 五种替换内容 × 六种 context：`shows_replaced`、`Replaced`、participation；同一 120×80 到达后都是 120×80，5 个 replaced seed、0 全文档回退；冷对照 | `replaced_resource_of` 忽略 custom render：C 与基线门禁 B 变红 |
| `issue202_d_min_max_keep_the_natural_ratio` | 8 种 min/max 组合 × Flex/Grid/行内/absolute，增量与冷布局相同 | 关掉两边 auto 的等比夹取：红；去掉一边换算后的 clamp：红 |
| `issue202_e_only_a_new_natural_size_lays_out` | 同尺寸重报零工作；同尺寸换资源 0 seed/measure；异尺寸换资源 2 seed | `reindex_replaced` 换资源总是 seed：红；同时去掉提交时的 no-op 过滤和 `apply_replaced_metadata` 的相等早退：红（单去任一层仍绿，两层互为兜底） |
| `issue202_f_markup_fits_every_replaced_kind_alike` | 五个关键字下 `Video`/`GpuTextureView` 的 fit 与图片 `object-fit` 相同，样式压过属性，未知关键字为 fill | `semantic_content_fit` 不读 `object-fit`：红；`BackgroundImageFit::Auto` 映射成 `Fill`：红 |
| nana-ui `an_image_and_a_host_texture_share_one_fit` | 五种 fit × 六组盒子/源尺寸，`url_dest_rect` 与 host-texture 的 `LogicalRect::fitted` 目标矩形相同 | `url_dest_rect` 自写 scale-down（当 contain）：红；`Auto` 映射成 `Fill`：红 |

另发现但未改：显式 `aspect-ratio`（非替换内容）`width: 120px; max-width: 60px` 的 absolute 盒子得 60×80，而 Flex/Grid/行内得 60×40。absolute 的尺寸路径对声明的 `aspect-ratio` 不在夹取后回推高度，与本 Issue 的替换内容无关，留给 absolute 尺寸路径。

### 本轮运行

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check`；`git diff --check` | passed |
| `cargo clippy -q -p nana-ui-core -p nana-ui-runtime -p nana-ui-scene -p nana-ui --all-targets -- -D warnings`；另 `-p nana-ui --features hosted,gpu -p nana-ui-css --all-targets` | passed |
| `cargo test -q -p nana-ui-runtime --lib` | 2315 passed、1 ignored |
| `cargo test -q -p nana-ui-runtime --lib --features layout-verify -- issue202 issue263 issue197 issue199` | 19 passed |
| `cargo test -q -p nana-ui-scene --tests` | 全部通过（lib 238） |
| `cargo test -q -p nana-ui-scene --lib --features nana-ui-runtime/layout-verify` | 238 passed |
| `cargo test -q -p nana-ui --features hosted,gpu --lib` | 967 passed、1 ignored（第一次因磁盘满中途退出，清理旧测试二进制后重跑通过） |
| `cargo test -q --workspace --all-targets` | 6066 passed、0 failed |
| `cargo test -q -p component-gallery --bin ui-snapshots --features snapshots --locked` | 12 passed |
| `caffeinate -di cargo run -q --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --output <scratch>` | MATCH: 680（exact） |

### 仍未做

- ~~普通块级流的原子行盒没有基线对齐（也没有 `vertical-align`）。~~ 产品决定采用 CSS 默认的 `vertical-align: baseline`，已在下文「追加：块级流的原子基线」完成。
- HostTexture / Video 的自然尺寸仍需宿主自己用 `set_replaced_metadata` 上报；`GpuTextureView` 默认不上报（见上文 `painted_extent` 的说明）。
- 未在 Windows/Linux 上运行；NanaLive / NanaType 只读检查：只用 `GpuTextureView.fit = ContentFit::…`（Rust 字段）与 `BackgroundImage::url_with_fit`，不受标记 fit 优先级与新增 API 影响；未构建这两个 consumer。

### 追加：块级流的原子基线（`vertical-align: baseline`）

产品决定：普通块级流里同一行的 inline-level 原子（inline-block、替换内容）按 CSS 默认的 `vertical-align: baseline` 对齐（CSS 2.1 §10.8 / §10.8.1、CSS Inline 3），不再顶部对齐；不新增 `vertical-align` 属性。

**规则（一处）。** `LayoutInputMap::atom_line_metrics` 给出原子在行上的 ascent / descent（含外边距），原生段落（`place_native_inline`）和普通块级流的行（`pack_ifc_line_boxes` 与 placement、IFC 的 intrinsic 测量 `wrap_intrinsic_size`）都读它：
- inline-block 以末个流内行盒的基线落在基线上，值取 `baseline_metrics` 的 `last`（原生段落此前读 `first`）；
- 替换内容、`overflow` 非 visible 的 inline-block、没有流内行盒的 inline-block（`UiWorld::atom_sits_on_line_box` / `holds_line_box`）以底外边距边落在基线上；
- 行盒高度 = 对齐后最高顶到最低底的跨度（`ifc_line_metrics`），块级子项独占一行从行顶排；无 strut。

**根因修正：替换内容的边。** 原生段落此前把替换内容的**边框盒**底边放在基线上，`margin-bottom` 垂在基线下（`atomic_metrics` 的 ascent = 边框盒基线 + margin-top）。CSS 在行内格式化上下文里用**底外边距边**。修在 `inline_content::atomic_metrics`：基线改为 `Option`，`None` 即整个外边距盒在基线之上；原生段落与块级流因此一致，`sync_inline_metrics` 的布局后回填走同一规则（`intrinsic_baseline` 改读末行基线）。`align-items: baseline` 的 flex / grid 不改：CSS Align 在那里从边框盒合成基线，仍是边框盒底边。

**失效。** 原子的基线随内容走，原先「固定尺寸 inline-block 是原子边界，里面的变化不重排外层行」只在基线不读内容时成立：
- `UiWorld::atom_baseline_reads_content`：普通块级流（非原生段落）里 `overflow` 为 visible 的非替换原子。这样的固定 inline-block 不再是度量边界（`is_fixed_metric_boundary`、frontier 上行的 `atomic_border`、`fixed_inline_block` 岛与 `affected_edits_are_direct`），自身文字变化不再只排 PLACEMENT。
- IFC 的 measure plan（`MeasuredChild::baseline`）记下每个原子的 ascent，`measure_plan_children_unchanged` 比对；未被本趟查询的 plan 遇到有 ascent 的子项在 measure 前沿上即标 stale（`plan_missed_change`）；`ifc_line_measure_delta` 回写新 ascent。IFC 容器本来不记 placement plan，placement 每次按新 ascent 重排。
- `inline_scope.rs::inline_fixed_size_atomic_does_not_reflow_its_line` 拆成两种：`overflow: hidden` 的原子保持原断言（不重排父级行、不碰另一个上下文）；`overflow` visible 的原子会重排这一行，但仍不碰另一个上下文、测量工作不随它增长、位置不变。

**门禁与变异**（改坏→变红→恢复→`cmp` 一致）：

| 门禁 | 证明 | 变异 |
| --- | --- | --- |
| `issue202_g_atoms_on_a_block_line_share_one_baseline` | 一行里：空 40×20、空 40×50 带 `margin-bottom: 6`、20×30 图片带 `margin-bottom: 4`、`overflow: hidden` 的 50×30（内有标签）以底外边距边落在基线上；60×60 的文字原子与 30×20 的文字原子以末行基线落在同一基线；块高 = 行盒跨度、行顶 = 块顶、各原子顶部不同。随后：高原子 50→70；60×60 盒子里文字折成多行（盒子尺寸不变，原子上移）；不决定行高的 30×20 原子文字折行（块盒不变、原子上移）；空原子里插入文字。每步 `assert_matches_cold` | placement 改回顶部对齐：红；`atomic_metrics` 的 `None` 改回边框盒底边：红；忽略 `overflow`：红；所有原子都当有行盒：红；`atom_baseline_reads_content` 恒为 false（失效收窄）：红（verify 揭出）；IFC 测量不按基线算行高：红；`measure_plan_children_unchanged` 不比 ascent：红（verify 揭出）。`plan_missed_change` 的新分支去掉后门禁仍绿（需要同一容器在两个约束下各有 plan，门禁未构造），作为保守分支保留 |
| css-parity `T-I05`（新） | 块里 20/50/30（`margin-bottom: 6`）三个 inline-block：50 的顶在 0，20 的在 30，30 的在 14（按 CSS 2.1 §10.8 推得，未在浏览器里实测；默认字体的 strut ascent 小于 50，不影响这三个位置） | placement 改回顶部对齐：T-I05 FAIL |

**快照。** 画廊像素 680 个、语义 156 个全部 MATCH，未重录任何基线：画廊里没有普通块级流中的 inline-level 原子，原生段落的原子也没有可见变化。

**本轮运行：**

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check`；`git diff --check` | passed |
| `cargo clippy -q -p nana-ui-core -p nana-ui-runtime -p nana-ui-scene -p nana-ui --all-targets -- -D warnings` | passed |
| `cargo test -q -p nana-ui-runtime --lib` | 2322 passed、1 ignored |
| `cargo test -q -p nana-ui-runtime --lib --features layout-verify -- issue202 issue197 issue199` | 13 passed |
| `cargo test -q -p nana-ui-scene --tests` | 全部通过 |
| `cargo test -q -p nana-ui --features hosted,gpu --lib` | 967 passed、1 ignored |
| `cargo test -q -p nana-css-parity` | 全部通过（含新 T-I05） |
| `cargo test -q --workspace --all-targets` | 6073 passed、0 failed、5 ignored |
| `cargo test -q -p component-gallery --bin ui-snapshots --features snapshots --locked` | 12 passed |
| `caffeinate -di cargo run -q --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --output <scratch>` | MATCH: 680（exact） |
| 同上加 `--semantic` | MATCH: 156 |

**仍未做：**
- 没有自己文字、文字在后代里的 inline-block（比如 `div` 里放一个标签），基线仍是 `baseline_metrics` 对容器的近似（自身字体的首行 `chrome_top + ascent`），不是后代末个行盒的实际位置；后代多行时与 CSS 不同。要按后代推导需要在测量时知道子项的块向偏移，牵涉 measure plan 与固定尺寸早退路径，单独做。
- 没有 strut：行盒不按父级字体留出最小 ascent/descent，空 inline-block 下面不留 descent 空隙。
- NanaLive / NanaType 只读检查：源码里没有 `inline-block` / `DisplaySpec::Inline*` / `InlineFlow`，不受影响；未构建。

## 2026-10-29 追加：#205

逐条重新核对 #205 的 7 条验收项（不沿用 10-26/10-27 的判定），补全 mini-layout 审计表，迁移 Checkbox，把 Select 菜单空间与无障碍铬裁剪改为沿 clip 链，并修复冷对照门禁揭出的「清空的文字留着旧度量」。

| 验收项 | 判定 | 证据 / 缺口 |
| --- | --- | --- |
| 建立 component mini-layout audit 表 | **本轮补全** | `docs/reference/component-layout.md` 的审计表逐项覆盖 `UiWorld::derive_component_geometry` 的全部分支，按 Issue 的 1–4 类标出已迁移、待迁移与不迁移；原表缺 Checkbox、Switch、StatusBadge、ValidationMessage、SelectionOption、EmptyState、Toast、MenuSurface 触发器、Range/Progress、ModalFrame、Panel 等行 |
| 首批通用组件迁移到 Foundation composition | 可勾（附注） | Issue 点名的首批：Button/Chip/ActionMenuItem（`issue205_button_chip_menu_parts_follow_content_changes_without_new_entities`）、ListItem/SidebarRow/FileTab（`issue205_row_text_parts_match_cold_and_preserve_state`、`issue205_rtl_row_parts_respect_live_slots_and_hidden_tools`）、FormField/LabeledValue（`issue205_form_field_parts_*`、`issue205_value_parts_*`）、Card（`issue205_card_header_*`）、Select 触发器（`issue205_select_trigger_parts_*`）此前已迁；Panel 本来就是普通 Flex 列，没有组件几何。本轮另迁 Checkbox（见下）。附注：菜单行（Select/Dropdown/ContextMenu）是字段的数据行，归 3 类，label/hint 分割共用 `menu_option_text_boxes`、摆放走 #200，不进 Foundation；MenuSurface 触发器的文字/图标仍手算 |
| renderer-side label/icon/slot placement 逐步删除 | 进行中 | 本轮删掉 Scene 为 Checkbox 自算的指示框位置和标签让位（`scene/primitives.rs`），Scene 只读 `ComponentGeometry::Checkbox`；Scene 测试 `standard_control_visuals_expand_without_backend_tag_matching` 改为提供 Runtime 发布的几何。剩余：Scene 里 Switch 的 `SWITCH_TEXT_INSET` 兜底 |
| ComponentGeometry 重复 layout authority 删除/降级 | 进行中 | 本轮删除三处：Checkbox 的指示框占位原先在 measure（`visual_content`）、段落预算（`text_shape_constraints` 的 `leading_visual`）、geometry 各算一遍，现都读 `UiWorld::checkbox_parts`；`select_menu_room` 自己沿祖先走、自己判断逃出与 `fixed`，现沿 `UiWorld::clip_ancestors`（发布的 clip 链）；无障碍的 EmptyState/对话框铬裁剪同样改走 `clip_ancestors`。剩余见审计表：Switch、StatusBadge/ValidationMessage、SelectionOption、EmptyState、Toast、MenuSurface 触发器、Range/Progress 文字、`context_menu_height` |
| retained wrapper/entity 爆炸有门禁 | 可勾 | 1k/10k：`issue205_ten_thousand_plain_buttons_keep_one_entity_and_two_parts_each`、`…_rows_…`、`…_form_fields_and_selects_…`、`…_values_…`、`…_cards_…`，本轮 `issue205_ten_thousand_checkboxes_keep_one_entity_and_three_parts`（每个一个实体，parts 恰为 TextContent/ComponentContent/LeadingContent，空闲帧零布局零 spawn）；常驻内存 `tests/node_memory.rs` |
| 业务/visual geometry 不被错误塞进 Layout Foundation | 可勾（附注） | 审计表把编辑器、菜单/树/命令面板/重排列表的行、滚动条、XYPad、resize grip 归 3 类，图表、日历热图、二维码、图、Markdown、ImageViewer 舞台归 4 类，均未加 `LayoutPartKind`；附注：这是按源码的人工分类，没有机器门禁 |
| 旧 API 可作为 facade 继续使用 | 可勾 | Checkbox、Button、Select 等构造与字段未变；Runtime 全量、view_macro、Scene、nana-ui 全部通过（见下）；`ComponentGeometry::Checkbox` 字段不变 |

### 改动

- **Checkbox 迁到部件。** `layout_engine::parts` 新增 `checkbox_parts`（指示框 `LeadingContent`，标签 `TextContent` 取余下宽度、可收缩、交叉轴拉伸，间隔只在有标签时）、`chrome_extent`（部件里除文字以外的宽度与间距、最高的图形）、`text_budget`（可用宽度减去同一份）、`start_row_parts`（图形按内容盒取正方形后起点对齐地求解）。`button_text_budget` 与 Button 的 intrinsic 改用 `text_budget` / `chrome_extent`，数值不变。`UiWorld::checkbox_parts` 从主题读指示框尺寸与间隔，measure、段落预算、`solve_content_parts_at`、geometry 都读它。TextRun 片段因此落在标签部件里（此前 Checkbox 没有部件，文本片段落在整个内容盒，与指示框重叠）。内容盒比指示框还矮时，标签跟着正方形化后的指示框左移，不再空出固定宽度（只在显式压低的 Checkbox 上可见）。
- **菜单空间与铬裁剪沿 clip 链。** `clip_chain.rs` 新增 `UiWorld::clip_ancestors`（沿发布的 `clip_parent` 往上）。`select_menu_room` 与无障碍的铬裁剪都用它，删掉各自的祖先遍历与逃出判断。行为变化：`fixed` 的字段打开的菜单取整个窗口，不再被它所在的滚动容器截（字段本身就画在、点在那些容器之外）；`fixed` 盒子的无障碍范围不再被祖先 EmptyState/对话框的铬裁剪截，与绘制、命中一致；逃出对话框 body 的 `absolute` 盒子不再被 body 裁剪截（命中索引早已如此）。
- **清空的文字交出度量。** 冷对照门禁揭出：Button/Chip/Checkbox 的标签被清空后，盒子仍按旧文字的宽度排。原因是没有文字的盒子在 `take_system_work` 里被去掉 TEXT 位，旧的 `text_metrics` 一直留着。现在仍持有度量的盒子保留 TEXT 位；`shape_text` 把文字为空的非 Text 盒子解析成「没有文字」，度量归零、按度量变化重新布局，此后不再进文字工作。
- 文档：`component-layout.md` 审计表与产品路径、`layout.md` 新增「组件内容部件（#205）」并补 clip 链段落、`layout-result.md`、`components/checkbox.md`「布局」、`components/select.md`「菜单」。

### 门禁与变异

| 门禁（`world/issue205.rs`） | 证明 | 变异（改坏→变红→恢复→`cmp` 一致） |
| --- | --- | --- |
| `issue205_checkbox_indicator_and_label_are_published_parts` | 指示框正方形、居中、起于内容盒；标签起于指示框加间隔、止于内容盒末；无宽度的 Checkbox 的标签部件恰为文字宽（measure 读同一组部件）；`ComponentGeometry::Checkbox` 等于两部件；TextRun 落在标签部件内；1 实体 3 parts；120px 宽的长标签按标签部件宽度折行 | measure 去掉间隔：红；预算不扣指示框：红；geometry 的标签取内容盒：红；不发布 Checkbox 部件：红（连同 10k 门禁） |
| `issue205_checkbox_parts_follow_changes_like_a_cold_layout_and_toggle_paints_only` | 空标签→有标签→长标签/Small→Large/90px→清空→复原，每步与冷布局逐项相同（`assert_matches_cold`）；勾选、悬停零 measure/placement，generation 与 `LayoutResult` 指针不变，无障碍 `checked` 更新 | 去掉「持有度量的盒子保留 TEXT 位」：红 |
| `issue205_checkbox_parts_survive_context_transitions` | 勾选且聚焦的 Checkbox 在 Grid→Inline→Flex 父级间切换：0 spawn/despawn，状态与焦点保留，几何等于部件，与冷布局相同 | geometry 的标签取内容盒：红 |
| `issue205_ten_thousand_checkboxes_keep_one_entity_and_three_parts` | 1k/10k 每个一个实体、三个 parts、空闲帧零工作 | 不发布 Checkbox 部件：红 |
| `issue205_menu_room_and_chrome_clips_follow_the_clip_chain` | 流内行的菜单空间被滚动容器按滚动截；`fixed` 字段的菜单空间是整个窗口、命中在它画的位置；EmptyState 流内子项在铬裁剪内，`fixed` 子项的无障碍范围是它的整个盒子、命中同处 | `select_menu_room` 改回沿树走：红（连同 `issue201_a_menu_room_is_cut_by_the_clip_chain_alone`）；无障碍铬裁剪改回沿树走：红 |
| `issue205_a_cleared_label_lays_out_as_a_cold_unlabelled_control` | 带图标的 Button 与 Chip 清空标签后与冷布局逐项相同、Button 变窄、度量归零，下一帧无工作、零布局、generation 不变 | 去掉 TEXT 位保留：红；`shape_text` 不把空文字解析成「没有文字」（按空行量出行高）：红 |

所有门禁已在 CI 的 `issue205` 过滤里（`.github/workflows/ci.yml` 的门禁循环原有 `issue205`，前缀匹配，不用改）。

### 本轮运行

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check`；`git diff --check` | passed |
| `cargo clippy -q -p nana-ui-core -p nana-ui-runtime -p nana-ui-scene -p nana-ui --all-targets -- -D warnings` | passed |
| `cargo test -q -p nana-ui-runtime --lib` | 2320 passed、1 ignored（加入迁移切换门禁之前；之后由下面的工作区全量覆盖） |
| `cargo test -q -p nana-ui-runtime --lib -- issue205` | 23 passed |
| `cargo test -q -p nana-ui-runtime --lib --features layout-verify -- issue205 issue201 issue199` | 36 passed |
| `cargo test -q -p nana-ui-runtime --features view-macro --test view_macro` | 48 passed |
| `cargo test -q -p nana-ui-scene --tests` | 全部通过（lib 238） |
| `cargo test -q -p nana-ui-scene --lib --features nana-ui-runtime/layout-verify` | 238 passed |
| `cargo test -q -p nana-ui --features hosted,gpu --lib` | 967 passed、1 ignored |
| `cargo test -q --workspace --all-targets` | 6072 passed、0 failed、5 ignored |
| `cargo test -q -p component-gallery --bin ui-snapshots --features snapshots --locked` | 12 passed |
| `caffeinate -di cargo run -q --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --output <scratch>` | MATCH: 680（exact，Apple M4 Metal） |

画廊像素逐张相同：画廊里的 Checkbox 都比指示框高，部件与原来手算的盒子相同；清空标签、`fixed` 字段的菜单、`fixed` 盒子在铬裁剪下的无障碍范围都不在画廊里。

### 仍未做

- 审计表里标「待迁移」的 2 类：Switch（track 与 label/hint、`project` 里的 min-height、段落预算与 Scene 的 `SWITCH_TEXT_INSET` 各一份 track 占位）、StatusBadge/ValidationMessage（padding 预留 + geometry 重算）、SelectionOption、EmptyState、Toast、MenuSurface 触发器、Range/Progress 的文字；ContextMenu 的 `context_menu_height`（被投影覆盖的默认度量高度）。Switch 的标签与 hint 是行里套一列，需要部件支持嵌套方向或两组部件，属下一批。
- Checkbox 在 RTL 下不镜像（与迁移前相同）。按 CSS 应把指示框放到行尾；这会改变 RTL 下的画面，属产品选择，本轮未改。
- 无障碍铬裁剪对「逃出对话框 body 的 `absolute` 盒子」现在与命中一致（不被 body 截），没有单独的门禁覆盖这一支；EmptyState 与 `fixed` 的门禁覆盖了沿链遍历本身。
- 未在 Windows/Linux 上运行；没有跑 `nana-ui-devtools` 的真实 GPU `issue205_component_pixels`。NanaLive / NanaType 只读检查：没有 `ComponentGeometry::Checkbox`、`SelectMenuRoom` 或 Checkbox 部件的直接使用，Checkbox 只经构造器；NanaLive 的 `tool_window_acceptance/narrow.rs` 读行与选项的 `text_metrics`（非空标签，不受清空修复影响）；未构建这两个 consumer。
