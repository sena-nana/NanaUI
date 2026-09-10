# LiliaCode 组件恢复

## 来源与边界

原任务 `01a066c5-4b6a-7b81-958c-1825eb3330e7`（「对比并规划还原 Tauri 版本」）及其子任务记录包含这批组件的实现、修正和验证。当时框架候选基于 `d0f3ccf`，保存在 `/tmp/lilia-nanaui-parity/isolated-source-d0f3ccf`，未发布到当前可获取 Git 历史；本次检查该目录已经没有源码。历史验证只证明当时的候选，不代表当前恢复结果。

本轮从原任务的文件修改和命令输出恢复最终实现，按模块适配已发布的 `3efde14e6ec78fd23951e643ef178bba58b3ff47`。原始记录、提取源码及来源索引保存在工作区相邻的 `nanaui-restoration-recovery-20260910/`，不再以 `/tmp` 作为唯一源码副本。未将原任务里的命令作为指令直接重放，也未整文件覆盖当前框架。

## 恢复范围

- 图表：DonutChart、TimeSeriesLayer、坐标轴标签、工具提示；保留当前实时序列和断点合同。
- 样式：SemanticColorMix、Button leading icon，沿用 Runtime/Scene 和 Vue 同树投影。
- 文本与图片：NativeMarkdown 的真实绘图、数学、Mermaid、图片资源与激活；ImageViewer 固有尺寸、场景绘制顺序与指针路由；TextInput UTF-16 长度限制、TextArea 纵向尺寸调整。
- 输入：聚焦键盘策略、编辑身份切换时清除撤销历史、使用真实命中投影的坐标转换、富文本指针捕获及取消。
- 浏览器：BrowserView 和宿主原生请求、macOS WKWebView、导航与截图。Windows/Linux 内嵌浏览器继续明确不可用，未新增假功能。
- LiliaCode：主窗口和弹出会话的附件原子区间、截图发起任务隔离，以及以上组件消费。旧 `popover_content_root` 私有协议改为当前 `TriggeredMenuOverlay` 的直系子节点协调，避免重建另一套浮层树。

## 验证状态

所有应用 manifest 仍指向第一阶段已发布的 `3efde14e6ec78fd23951e643ef178bba58b3ff47`；本恢复批次尚未提交、发布。以下为显式本地框架配置的联合验证，不能代替最终 Git pin 验收。

| 验证 | 当前结果 |
| --- | --- |
| Code desktop all-targets / workspace all-targets check | 通过，缺失 API 编译错误清零 |
| Code workspace 与文档测试 | 最终保留式时间线修正后 986 通过、1 项既有忽略；desktop 565 项及 workspace/all-targets check 通过 |
| NanaUI Runtime all-features lib | 第四轮行高修正后最终全量 1053 通过，含历史边界、测量与 Fixed 投影 |
| NanaUI Scene all-features | 保留后代裁剪缓存修正后，最终全量 121 通过 |
| 普通 RuntimeInputAdapter 路由 | 76 通过，含真实带 padding 链接点击、拖动取消、捕获被接管与 park |
| SVG 宿主图片解析 | 14 通过，含已有 UI 字体的拉丁与中文真实像素 |
| TextPipeline 实际 GPU 文本 | 8 通过，含 Auto ASCII 精确宽度、省略及缓存回访 |
| 原生浏览器 | nana-window 5、Runtime 宿主 5 通过；hosted/bundled-fonts all-targets 严格 Clippy 通过 |
| 语义混色与 Vue wrappers | 混色 3、Vue wrapper 166 通过 |
| 内容离屏图 | 最终宽度修正后 20 张浅/深、窄/宽、1×/2×已重新生成并逐张检查，含真实长业务气泡 |
| 图表离屏图 | 最终 14 张全部逐张检查，GPU 回归 3 通过，含滚动 Fixed Tooltip；按钮 3、混色 3 通过 |
| 统一严格 Clippy | Runtime、Scene、nana-ui all-targets/all-features 通过；devtools agent/all-targets 通过 |
| 文本集成 | Workspace 负对照确认旧行为覆盖交互；borrowed 1、TextArea resize 3、TextInput UTF-16 限制 5 项独立回归全部通过 |
| 键盘录制徽标 | 修前 GPU 像素回归确认背景覆盖文字，修后 12 个主题/缩放/状态组合通过并逐张看图；gpu-only 与 hosted all-targets 严格 Clippy 通过 |

日志前缀为 `/tmp/nanaui-consumer-upgrade-20260909/restoration-`，原始失败日志与修正后的结果均保留，完成的日志另存相邻 recovery 的 `validation-logs/`。`cargo xtask verify` 的 boundary-check 通过，但正常 pin gate 明确拒绝 path lock，未绕过。桌面解锁后已重新运行原生验收，历次锁屏和中途失败均保留为历史证据；第六轮窗口矩阵运行完成，MCP 补拍和性能统一口径验收均已通过，见下文记录。

macOS 框架探针已验证 WKWebView 导航、后退/前进、隐藏恢复、实际 PNG 回流与待完成截图时关闭父窗。文件对话框五种模式均通过；重复/Busy 拒绝不终结原请求，取消正常返回空路径，关闭父窗只回流一次 `WindowClosed`。7 个接受的请求各完成一次，sheet 打开期间仍持续呈现。证据见相邻 recovery 的 `browser/evidence-current/acceptance-report.json`、原生截图及 SHA256 清单。进程退出后的晚回调执行无法直接观测，回调抑制另由单元测试覆盖。

Code 首组重试 `lilia-agent-debug-1789008162279` 已实际打开 960×600 浅色窗口，生成 34 张截图；构建前后 NanaUI 源码指纹一致且正确识别 CARGO_HOME 提供的显式本地依赖。矩阵在模型选择断言终止。诊断组 `lilia-agent-debug-1789009603617` 证明键盘正确执行了 reasoning 的 low→medium，以及模型的 gpt-5.4-mini→gpt-5.4；旧硬编码断言恰好拒绝该正常结果。脚本现按实际候选推导下一项，并恢复没有中间观察的连续按键。诊断组另确认三个圆环的滚动提示框、键盘录制文字及用量日期已修复；该组在实际发送引用验证处停止。两组失败图与元数据均持久保留，不计完整矩阵通过。

引用失败来自回放删除正文 token 后，UI 仍显示撤销索引中的条目。领域发送端按原合同排除了已删除引用；现删除外置幽灵标签，调试计数、优化请求与发送条件只读取有效引用，保留撤销索引。删除→撤销→重做的真实 TextArea 与发送上下文回归通过，harness 改为保留正文引用再发送。最终三项 Code 调试行为测试、fmt 与 xtask 严格 Clippy 通过；补充 desktop 全库严格 Clippy 仍有 49 个 lib / 57 个 lib-test 结构和风格告警，清单另存 `restoration-code-composer-context-clippy-findings.md`。未添加 allow 或改动原正式 verify 门禁。完整实窗矩阵与性能继续复验。

第三轮 `lilia-agent-debug-1789010992612` 在 960×600 浅色、1× 下生成 56 张图，构建前后源码指纹一致。连续键盘选择、实际请求中的有效引用、Markdown、图片查看、审批与草稿恢复均已通过。随后 Memory 标题撤销回空导致保存禁用：首次输入与离焦后再次编辑被错误合并；负向回归已经复现；Runtime 焦点和选择的历史边界修正后 20 项定向测试及独立复审通过，实窗后续仍待重跑。该轮未走完 Memory、任务弹窗及其余三组，不计完整矩阵通过。全部失败证据保存在相邻 recovery 的 `code-native-failures/`。

## 历史：第四轮实窗及行高修闭

四组记录 `lilia-agent-debug-1789013151357`、`1789013232482`、`1789013287817`、`1789013340410` 使用同一框架源码指纹 `9028d808f205a323ab49cd77320320c648a8dd371f9872c8eefc01fd1ac3e469`，构建前后均一致。960×600 浅/深和 1440×900 浅色分别完成 61 张图与 `summary.ok=true`；1440×900 深色生成 52 张图后因桌面重新锁定中止。全部 235 张图已逐张检查；这些实窗均为实际 1×，不作为原生 2× 证据。Memory 标签、撤销后保存、尺寸调整和真实独立任务窗口在前三组完成；窗口身份由实际新增 CGWindow ID 确定。

宽窗长消息暴露另一项行高缺口：父气泡先用未受 `max_width` 约束的宽度测量 Markdown 高度，再收窄最终宽度，换行后复制按钮超出气泡。真实 TimelineContent 回归在修前失败；框架现先按最终宽度约束测量子树，百分比仍相对原 containing block，ContentBox/BorderBox 与 min-width 顺序保持既有合同，不改变滚动内容的自然高度。83 项布局测试与真实业务负/正回归通过，非作者复审通过。另将外观默认提示改为用户可理解的系统模糊说明，删除平台实现名。第四轮截图是这两项修改之前的证据，完整最新源码矩阵及性能仍待重跑。

最新 Code `cargo test --workspace` 980 通过、1 项既有忽略，桌面 559 项通过；`cargo check --workspace --all-targets` 通过，均使用显式本地框架配置。日志为 `restoration-code-workspace-{tests,check}-after-width-fix.log`。全部第四轮产物保存 `code-native-matrix4/`，逐图记录位于 `matrix4-visual-review/`、`matrix4-priority-review/` 与 `native-settings/`。

## 非作者复审与修闭

- 原任务源码恢复后逐块按当前 Runtime、Scene、宿主合同复核，没有恢复已被现有生命周期接口替代的私有 popover 根节点协议。
- 修复了超大数学/Mermaid 源码先进入缓存再拒绝的问题；Markdown 装饰复用 Scene 既有绘制入口并独立分配图元身份。
- 文本拖动在 pointer capture 被其他节点接管后按取消处理，不释放新持有者；park 后退出当前交互并保留编辑值与身份。
- 图表 Tooltip 的 ASCII 多行溢出来自 Runtime 始终采用 Advanced 塑形、Painter 对 ASCII Auto 改用 Basic 的字宽差异。统一为现有 Runtime 策略，非作者复审通过；首次 ASCII 塑形成本与字距外观可能变化，稳定帧继续使用现有缓存；本批最终实窗性能数据见下文。
- 圆环改用现有圆形边框与互斥扇区裁切，不引入 SVG 栅格路径或新 GPU ABI；按钮按实际 computed 字体参与测绘。最终 10 张 GPU 图与无缝/半透明像素回归通过，非作者复审通过。
- 实窗发现的 KeyCaptureLayer / KeymapLayer 空白是既有绘制顺序错误：徽标背景在文字之后覆盖。调整框架图元层级，保留业务挂载；修前负向像素回归失败、修后通过，12 张图与双 feature Clippy 证据保存在相邻 recovery 的 `key-badges/`。
- Code 最近用量记录此前直接显示毫秒时间戳，现与同页自动化运行共用既有 UTC 日期时间格式。格式换算未改变，第二、三轮实窗图已确认可读。
- Fixed 图元原先被重复应用外层滚动和裁剪。现在 Scene、增量可见性与 Runtime 命中共享视口边界语义，保留原结构上的叠放、透明度和生命周期；相对/固定切换会使未提取后代的缓存失效。Runtime 3、Scene 1、滚动图表 GPU 1 项定向回归及四张离屏图通过，独立复审闭合菜单层级和非交互浮层快路径问题，证据保存在相邻 recovery 的 `fixed-layout/`。

### 第三轮实窗修闭

- Memory 撤销：实际失焦或选择变化后结束输入合并，失败事务与 no-op 不切断；覆盖 pointer、Tab、a11y、modal、生命周期与样式隐式失焦。Code 原有行为测试强化后修前失败、修后通过，Runtime 20 项通过，独立复审通过，日志持久保存 `edit-history/`。
- Todo 重叠：临时应用布局检查证实视口在 Todo 上方且仍有滚动裁剪；Scene 只提取缩小后的 scrollport 时，保留后代仍使用旧 400 高裁剪，而完整重建为 250。负/正对照及变换、组件裁剪边界均闭合，独立复审通过，证据保存 `retained-clip/`。
- Memory 复选框按实际 size 预留标签空间；Markdown 使用局部坐标累计换行，解决右对齐时的舍入误差。三种 Checkbox 尺寸、空标签与真实字体 960/1440 回归通过，独立复审通过；任务菜单文字完整，贴窗口底边不作为裁字缺陷处理。

## 图表最终证据

图表矩阵包含浅/深主题各 560×420 逻辑像素、1× DPI，以及各 320×420 逻辑像素、2× DPI（输出 640×840），每种组合记录圆环和趋势图悬停，共 8 张。另有深色 160×160、2× DPI 的 1 扇区与 48 扇区半透明圆环，共 2 张；像素回归遍历安全内部区域，确认没有露底或重复 alpha 叠色。10 张图均已逐张查看，按钮标签完整，多行 Tooltip 全部位于背景内，坐标轴与图例可读。

最终 `chart_controls` 两项 GPU 测试通过；`button_icon` 三项（含自定义 18px/700 字体）与 `semantic_mix` 三项通过。独立 Runtime `--features charts --lib charts` 的 17 项与 Vue wrappers 166 项通过。截图、日志和 SHA-256 清单已持久保存在 [图表证据](../../nanaui-restoration-recovery-20260910/charts/evidence/README.md) 与同目录 `SHA256SUMS`，不依赖临时目录作为唯一证据。

## 四应用兼容

第四轮行高修正后的源码已分别通过四应用 workspace/all-targets 检查，其中 NanaShader 包含 all-features；LiliaCode 在后续时间线修正后完成 986 项 workspace/文档测试、1 项既有忽略。其余三应用本条只记录兼容编译，不把第一阶段的旧提交测试与当前恢复源码拼接为全量验收。最新汇总见相邻 recovery 的 `consumer-after-width-fix-checks.json`，初批记录保留 `consumer-local-checks.json`。

## 最终本地验收：第六轮与性能

第五轮两个窄窗组共 122 张图完成，但第三组启动返回 `agent_debug_ready_timeout`，不计完整通过。逐图复审发现任务菜单背后的时间线为空；现使用框架保留式虚拟列表和同一 `VirtualListLayout` 管理范围、定位和内容高度，在实际呈现后批量回填已安装行的测量值。缓存按任务、业务 key、内容和实际宽度隔离；保留可见 key/inset 或明确尾锚，用户滚动优先，稳定反馈不请求重绘。主窗口尾滚动意图在成功同步相应任务后才交付，scroll-only 事件保留实测视口。原加载更早记录页脚保持在滚动区外。

实际负向回归还发现虚拟行销毁后遗留的已 park 可选子节点；TimelineContent 现在显式清理自己拥有的剩余实体，主时间线和任务弹窗均调用。普通离屏行仍沿用框架销毁合同，焦点/IME 等保留行维持身份，并非永久保留全部离屏组件。20 项定向回归、三轮实体数量稳定验证和非作者复审通过；最终 workspace 测试 986 通过、1 既有忽略，desktop 565/565，workspace/all-targets check、fmt、桌面构建通过。日志与负/正对照保存相邻 recovery 的 `timeline-measurement/`。

第六轮 `1789017421571`、`1789017487276`、`1789017540530`、`1789017591909` 对应 960×600 / 1440×900 × 浅/深色，四组均完成 61 张截图、`summary.ok=true`，全部 244 张已逐张复审；其中两张 MCP 宽窗图的部分可见问题已通过下述脚本修正和定向补拍修闭。源码构建前后均为 528 个文件、指纹 `38b10f6f4e3f8d6a6cb7e69e488e23ebf8c8a002594a4f7d96d2f84ee3d452a2`，截图均为实际 1×；独立任务窗为 430×760。原任务菜单空白、长气泡复制按钮越界、Memory 撤销/保存、Todo 裁剪、外观提示、原生浏览器与 MCP/审批回流在本轮范围内通过。完整产物为 `code-native-matrix6/`，逐图复核分为 `matrix6-visual-review/`、`matrix6-priority-review/` 与 `native-settings/matrix6-settings-review.json`。

最终内容离屏探针 20 张和图表 14 张已在宽度修正后重新生成、逐张复审，包括 2× 内容、真实长消息、指针调整 TextArea 与滚动图表工具提示。证据为 `content-after-width-fix/` 与 `charts/evidence-after-width-fix/`。它们与原生 1× 截图分别记录，不宣称原生 2× 验收。

MCP 补拍已完成：原脚本将命中目标部分露出判为已展开；现使用完整布局边界和实际 scroll viewport 验证包含，并按真实 Wheel 方向滚动。首个修正候选的滚轮符号错误已由原生负对照捕获并修正；不改变产品滚动或聚焦策略。`1789019155044`（宽浅）与 `1789019255885`（宽深）两组定向扩展回放均通过，开关、标签与完整焦点圈可见。两组共 14 张扩展图逐张复核；MCP 实际 UI 几何、截图、窗口/源码/产物身份与原始失败证据保存 `extensions-reveal-final/`。原 244 张的两张问题图保留为历史负证据，由同主题补拍修闭，不改写旧图。xtask 库 28 项通过、1 项原生探针默认忽略；定向滚轮行为测试及 all-targets 严格 Clippy 通过，忽略式原生探针随后显式执行并通过。

最终 `cargo xtask performance` schema 2 在无并发构建/GPU 测试时通过，主 run `lilia-performance-1789018767913`。五个隔离目录均使用相同 performance-v1 千条语料；语料 SHA、NanaUI 源码指纹、可执行文件和宿主库身份全部一致。启动指标为 spawn 到 debug ready（不含构建/语料准备/产物核验，不代表首帧或 OS 缓存冷启动），5 次 P95 2148.76 ms。每类 30 次操作从 handler 开始到主窗口 `window_frame_presented`，输入 P95 6.20 ms、面板缩放 P95 14.23 ms，均不含传输/排队时间，也不代表持续 FPS。千条时间线加载观察就绪 91.07 ms，不代表同时绘制千行。

CPU 按两次进程采样完成时刻的实际 1.01361175 秒和 10 核归一化；本次累计 CPU 计数均为 1.04 秒，报告 0%，仅表示短采样窗未超出计数精度。主进程 RSS 单次采样 203554816 字节（194.125 MiB），不含子进程/GPU，也不是峰值。5 次启动和 30+30 次原始帧耗时、采样边界和产物身份均持久保存；非作者从原数组重新计算 P95/CPU 与报告一致。既有门槛和环境变量保持，全部通过。实际 `performance.png` 已逐张查看。历史同平台语料不可获取，未作历史性能比较。完整新证据为相邻 recovery 的 `code-performance-schema2/`；旧口径报告保存 `code-performance-final/`，不替代新结果。


以上均使用显式本地 NanaUI 来源，验证后恢复四应用已发布 3ef 锁文件。本轮框架尚未提交/推送，正式 `xtask verify` 与四应用最终真实 Git pin 门禁仍待新提交发布后完成。此前各轮的失败记录仅用于追溯，不作为当前阻断或最终通过证据。

## 默认依赖来源

联合检查使用命令行显式本地配置。检查后恢复各应用原已发布 Git 锁文件，本地恢复锁另存 recovery；没有恢复失效的默认 path patch。Code 已再次以正常 CARGO_HOME、无 --config 的完整 `cargo metadata --locked --all-features` 验证 8 个 NanaUI 包均来自 `3efde14`。该结果只证明默认旧 pin 来源正确，不能证明尚未发布的恢复代码可以从 Git 获取。新恢复批次发布后，必须统一更新四应用 pin、重解析锁文件并跑正式门禁。

## 未验收边界

macOS 框架 WKWebView 与文件对话框探针已经通过上述实机范围；Code 第六轮窗口矩阵运行完成，MCP 补拍和性能统一口径验收均已通过。Windows/Linux 原生实机行为未验收，内嵌浏览器在这两个平台明确不可用。框架探针不代替四应用所有业务导入/导出流程；离屏截图不能证明原生内容覆盖层或宿主窗口行为。

Markdown 的 SVG 数学/Mermaid 沿用已有 intrinsic 尺寸栅格策略，2×图可读但较原生文字柔化，不声称向量级高 DPI。恢复源码位于真实仓库目录，持久提取与截图证据在相邻 recovery 目录；尚未发布的本地恢复不能被描述为四应用最终升级完成。
