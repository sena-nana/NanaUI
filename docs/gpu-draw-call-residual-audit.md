# painter 里剩下的非 O(1) draw call，与静止帧的每帧成本（2026-09-10）

这是**调查报告**。第一到七节是调查本身（写的时候工作区没有任何实现改动）；第八节
记录同一次会话里按它的结论落地的两项。它接着 [`gpu-ui-draw-call-plan.md`](gpu-ui-draw-call-plan.md)
的三步（屏外文字剔除、相邻文字 run 合并、重叠感知批次合并）往下问两个问题：

1. 哪些 draw call 路径仍然随节点数线性增长？
2. 在**场景完全没有变化**的一帧上，painter 仍然每帧付出什么？

结论先说：

- **图标 atlas 是唯一一条又常见、又真能压到 O(1) 的线。** 一条 20 个不同字形的工具栏
  就是 20 次 draw；一棵「工具栏 + 4 个裁剪面板 × 40 行」的树今天 34 次 draw，其中
  **24 次是图标**（20 个工具栏字形 + 每个面板 1 次）。把 atlas 打包成一张纹理后实测
  **34 → 15**。
- **「文字跨 scissor 合并」在微基准上是 34 → 3，在真实形状上是 0。** 放开 scissor 之后
  重叠判定立刻接手，把 3 次跨面板合并全部拒掉（`text=[156 merged, 1 no_candidate,
  3 overlap]`）——因为每个面板的行背景 quad 在自己的标签之后开批次，标签必须压在
  它自己的背景上面。**这条不值得做。**
- **HostTexture / URL 背景图 / backdrop / 不透明分组的 O(N) 是硬下限**，不是实现缺陷：
  一张纹理一个 bind group（产品设备 `Features::empty()`，没有 `TEXTURE_BINDING_ARRAY`）、
  一个 backdrop 面板一次「读当前 dest → 可分离模糊 → 合成」。唯一的例外是
  **N 个节点共享同一张 host texture 时今天仍然是 N 次 draw**，这一条是可以修的。
- **静止帧的每帧成本，普通 UI 是 1.8 µs（2002 个节点），GPU 节点是 O(N)**：1024 个
  gpu-view 节点 147.9 µs/帧，其中 `plan.custom_nodes` 被走了**四遍**，其中两遍是
  重复的（`frame_plan()` 每帧被调用两次；`validate_scene` 的循环与 `paint` 的键循环
  做同一组查找）。
  > **落地时的更正**：这里原本还把 `frame_plan()` 记忆化路径上的
  > `validate_plan_resources` 也算成「重复」，理由是「它查的是记忆化 plan 自身的性质」。
  > **那是错的**，见第六节里同样标注的更正。实际落地只去掉了真正重复的两遍，
  > 1024 节点 147.9 → 94.2 µs（−36%），不是下面那个 −50%。
- 线索里其余的每帧成本**全部实测可忽略**：`poll_images` ≤0.7 µs、`gpu_interleaved` 的
  `commands.iter().any` ≤0.21 µs、`painted` 扫描 ≤0.17 µs、`begin_frame` / `truncate`
  在命中缓存的帧上根本不执行（`build = 0.00`）。「任何一个 GPU 节点让整棵树每帧重新
  encode」是真的，但**4096 个 quad 重新 encode 只要 3.75 µs**，且它是必要的。

## 一、怎么量的

两套探针，量完都已从工作区撤掉（工作区干净，`nana-ui` 464 个测试通过，五个基准场景
draw call 复现 5 / 4 / 68 / 16 / 16）。

**(a) 规模扫描**：在 `scene_paint/tests.rs` 的既有夹具上临时加一组 `#[test]`，对每一条
可疑路径按 n = 1…64 造树，读 `last_gpu_work().draw_calls` 与 `last_dest_pass_counts`，
并且**同一个 painter 连画两帧**，第二帧就是「什么都没变」的帧。这套不需要改 painter，
所以数字不受探针影响。

**(b) painter 内探针**（`NANA_DRAW_MIX=1`）：临时的 `scene_paint/probe.rs`，做三件事：

- `GpuWorkSink::record_draw_call` 挂一个钩子，encode 循环在派发每条命令前设置
  「当前类别」，于是每次 draw 都被归到 `DrawCommand` 的变体上——**不建模，直接数**。
- `paint()` 内部分段计时：`poll_images` / `validate_scene` / `frame_plan` / 自定义节点
  键循环 / 缓存比对 / build / upload / `gpu_interleaved` 扫描 / `DestTarget::ensure` /
  命令循环 / blit / `painted` 扫描，并对全部帧取 p50。
- `Batching::target` 记录每次合并的**结果与失败原因**：`merged` / `no_candidate`
  （谓词——scissor、atlas key、顶点连续性——没匹配上）/ `overlap`（重叠判定拒绝）。
  这一列是本轮最关键的仪表：它把「为什么没合并」从推断变成数据。

自校验：探针的类别计数之和必须等于 `GpuWorkObservation::draw_calls`，五个场景上逐个
相等（5 / 4 / 68 / 16 / 16）。分段计时之和与 `total` 相符。

补丁存在会话 scratchpad：`probe-mod.patch`（`Batching::target` 的原因计数 + 各阶段计时
+ encode 循环的类别标注）、`probe-tests.patch`（规模扫描的临时 `#[test]`）、
`exp-text-scissor.patch`（下面的文字实验）。`probe.rs` 本身没有留下副本——它是一个
env 开关 + 一组 thread-local 计数器 + 一个 `report()`，钩子点在上面两个补丁里都写明了。

环境：macOS / Apple M4 / wgpu Metal。规模扫描用 debug 构建（只看 draw call 计数，与
优化等级无关）；所有时间数字用 `--release --locked` 的 `nana-gpu-scene-benchmark`，
p50 取自 7 千–1.4 万帧。

## 二、还有哪些 draw call 随节点数线性增长

`draws` 含最后那 1 次 blit。斜率 = 每加一个节点多几次 draw。

| 形状 | n=1 | 2 | 4 | 8 | 16 | 32 | 64 | 斜率 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 相邻图标，同字形同尺寸 | 3 | 3 | 3 | 3 | 3 | 3 | — | **0** |
| 相邻图标，**不同字形** | 3 | 4 | 6 | 10 | 18 | 34 | — | **1** |
| 相邻图标，同字形**不同像素尺寸** | 3 | 4 | 6 | 10 | 18 | 34 | — | **1** |
| 文字，同一个 scissor | 3 | 3 | 3 | 3 | 3 | 3 | — | **0** |
| 文字，**N 个裁剪容器各一条** | 3 | 4 | 6 | 10 | 18 | 34 | — | **1** |
| 文字，**N 条旋转标签**（Affine） | 3 | 4 | 6 | 10 | 18 | 34 | — | **1** |
| HostTexture，**共享同一个 slot** | 3 | 4 | 6 | 10 | 18 | 34 | 66 | **1** |
| HostTexture，每节点一个 slot | 3 | 4 | 6 | 10 | 18 | 34 | 66 | **1** |
| 背景图 quad，同一个 url | 3 | 3 | 3 | 3 | 3 | 3 | — | **0** |
| 背景图 quad，**N 个不同 url** | 3 | 4 | 6 | 10 | 18 | 34 | — | **1** |
| 纯色 quad（测到 n=4096） | 2 | 2 | 2 | 2 | 2 | 2 | 2 | **0** |
| `backdrop-filter` 面板 | 4 | 6 | 10 | 18 | 34 | — | — | **2** |
| 不透明度分组 | 5 | 8 | 14 | 26 | 50 | — | — | **3** |

pass 数同步长：backdrop 面板是每个 **4 个 backdrop pass + 1 个 color pass**
（copy → 横向模糊 → 纵向模糊 → composite）；分组是每个 **1 个 group pass +
1 个 color pass**。

**quad 已经彻底 O(1)**：4096 个纯色 quad 仍然 2 次 draw。`quad.rs` 的 `pending_urls`
按 url 切分只在 url **不同**时才切，同一个 url 的 4096 个实例是一次 draw。

**每一列的第二帧 realloc 都是 0**——painter 里没有任何路径在稳定帧上创建 GPU 资源。

## 三、真实形状上这些数字各占多少

微基准会误导（第四节有一个具体的例子），所以造了一棵「像真 app」的树：一条工具栏
（N 个**不同字形**的图标）+ P 个 `overflow: hidden` 面板，每个面板 R 行，每行是
（背景 quad、标签、同字形图标）。900×900。

| 树 | 节点 | draws | quad | text | icon | blit |
|---|---:|---:|---:|---:|---:|---:|
| 0 图标 / 1 面板 / 20 行 | 20 | 5 | — | — | — | 1 |
| 12 图标 / 1 面板 / 20 行 | 32 | 17 | 2 | 1 | **13** | 1 |
| 12 图标 / 3 面板 / 20 行 | 72 | 23 | 4 | 3 | **15** | 1 |
| 12 图标 / 3 面板 / **40 行** | 132 | **23** | 4 | 3 | 15 | 1 |
| 20 图标 / 4 面板 / 40 行 | 180 | 34 | 5 | 4 | **24** | 1 |

两件事：

- **行数完全不影响 draw call**（20 行和 40 行都是 23）。前三步的成果在这里成立：
  一个面板里 40 行的 quad / 标签 / 图标各塌成一次。
- **图标占了 24/34 = 71%**：20 个工具栏字形各一次，加上 4 个面板的行图标各一次。

合并结果的原因分布（20 图标 / 4 面板 / 40 行）：

```
quad = [156 merged, 5 no_candidate, 0 overlap]
text = [156 merged, 4 no_candidate, 0 overlap]
icon = [156 merged, 24 no_candidate, 0 overlap]
```

**`overlap` 全是 0。**今天每一次没合并都是**谓词**没匹配上，不是重叠判定。谓词里的
三个条件分别贡献：图标的 20 次是 `AtlasKey` 不同（工具栏字形），其余 4 次和 quad 的
5 次、text 的 4 次是 **scissor 不同**（每个面板一个裁剪域）。

## 四、两条候选的实测收益

### 4.1 打包 icon atlas：34 → 15，值得做

`icon.rs:311` 的 `AtlasKey { icon: icon.as_ptr(), px }` 每一个键对应**一张独立的
wgpu 纹理 + 一个 bind group**（`insert_atlas`，`ATLAS_CAP = 128` LRU）。
`can_extend_run` 要求 run 内 atlas 相同，所以不同字形无法进同一次 draw。

用「把工具栏字形换成同一个字形」来模拟打包后的下限（对 draw call 而言两者等价）：

| 树 | 今天 | 打包 atlas 后 |
|---|---:|---:|
| 12 图标 / 1 面板 / 20 行 | 17 | **6** |
| 12 图标 / 3 面板 / 40 行 | 23 | **12** |
| 20 图标 / 4 面板 / 40 行 | 34 | **15** |

同时 `px = dest_px * 2` 意味着**同一个字形在不同渲染尺寸下也是不同的纹理**：
32 个尺寸各异的同字形图标今天是 34 次 draw，纹理上传 342 KB（同尺寸时 17.9 KB）。
一个 hover 缩放动画会在 128 条的 LRU 里来回抖。打包 atlas 顺带修掉这一半。

要改的是 atlas 的存储与 UV：一张纹理 + 一个矩形打包器（cryoglyph 自己就是这么做的），
`prepare` 从 `uv 0..1` 改成从分配矩形算 UV，`can_extend_run` 去掉 atlas key 一项。
`IconVertex` 已经带 `uv`，管线形状不用动。

### 4.2 文字跨 scissor 合并：微基准 34 → 3，真实形状 0，不值得做

先说它**是安全的**，这一点原方案的判断是对的，而且现在有证据：
cryoglyph 的 `prepare`（`text_render.rs:212-247`）把每一个 glyph 四边形**逐个**裁到
`TextArea::bounds`，连 UV 一起裁；而 `TextBounds` 用 `round()`、`physical_scissor` 用
`floor/ceil`，所以 **per-glyph 裁剪盒永远含于 scissor**。也就是说对 cryoglyph 路径
scissor 本身是冗余的，合并后取并集是逐像素等价的。

实验（`push_text_run` 去掉 `*open == scissor`，命令上存 `union_physical`）：

| | draws | 说明 |
|---|---|---|
| N 个裁剪容器各一条标签，n=32 | 34 → **3** | 微基准 |
| 一张 6 个裁剪容器 + 溢出文字 + 混合 scissor 标签的帧 | 9 → **3**，整帧字节哈希 `cd3d8288cd49abf8` **不变** | 像素证据 |
| `nana-ui` 全量测试 | 470 通过（464 + 6 条探针） | 无回归 |
| 真实 shell（20 图标 / 4 面板 / 40 行） | 34 → **34** | **收益 0** |

真实 shell 上的原因分布把话说死了：

```
text = [156 merged, 1 no_candidate, 3 overlap]     ← scissor 让位之后，重叠判定接手
```

面板 k+1 的行背景 quad 在面板 k 的文字批次**之后**开批次（scissor 不同，quad 合不进
更早的批次），而面板 k+1 的标签与它自己的背景 quad 相交，所以把它折到面板 k 的文字
批次里会让它画在自己的背景**下面**——判定拒绝得完全正确。

顺手也量了「假如 scissor 根本不是批次键」：真实 shell 34 → 33，只省 1 次。所以
「把轴对齐裁剪搬进顶点/实例数据以解放批次合并」这个方向在这棵树上也是没有收益的
——瓶颈是画家算法顺序，不是 scissor。

**结论：文字跨 scissor 合并只在「裁剪域里只有文字、后面不再画东西」时有用**
（纯文本列表、代码编辑器的行）。带行背景的列表——也就是绝大多数密集 UI——收益为 0。
它是一条 20 行的改动，风险低、有像素证据，但**不解决任何实际问题**，按
`AGENTS.md` 的「禁止无价值抽象」应当不做。

## 五、剩下的 O(N) 都是硬下限（一个例外）

### HostTexture：一张纹理一次 draw 是下限，**共享一张纹理时的 N 次不是**

`gpu_texture.rs:857` 每一个 `TextureKey::new(presentation, texture.id)` 建一个
bind group，而 `presentation = Scene { node, slot }`——**键里带节点**。所以
64 个节点即使全部绑同一张 host texture，也是 64 个 bind group、64 个 layer uniform
buffer、64 次 draw。实测两种配置完全一样：

| | n=1 | 8 | 32 | 64 |
|---|---:|---:|---:|---:|
| 每节点一个 slot（64 张不同纹理） | 3 | 10 | 34 | 66 |
| **全部共享一个 slot（一张纹理）** | 3 | 10 | 34 | **66** |

不同纹理的 66 次是诚实的下限：产品设备 `required_features: Features::empty()`，
没有 `TEXTURE_BINDING_ARRAY`，一次 draw 只能绑一张纹理——`gpu-scene-host-textures-64.json`
的 `notes` 已经这么写了。**但共享纹理那一行不是下限**：把 layer uniform
（bounds / clip / opacity / mask 标志）从每节点一个 uniform buffer 改成一条
instance-step 顶点缓冲，相邻同纹理同 mask 的 HostTexture 就能塌成一次实例化 draw
——和上一轮 `DefaultGpuViewRenderer` 的改法逐字同构。

值不值得做取决于产品里有没有「同一张 host texture 画很多遍」的界面。今天的场景库里
没有这种形状（`independent_textures: true`），所以这条**先记账，不做**；真出现了
（同一个视频帧当缩略图墙、同一张贴图铺满网格）再按上面的路子改，成本很低。

### URL 背景图 quad：同理

`quad.rs:728` 的 `draw` 按 `pending_urls` 把一条 `Quads` 命令切成多次
`pass.draw`，N 个不同 url = N 次。原因和 HostTexture 一样——一次 draw 一张纹理。
要压到 O(1) 需要把 url 图片打进一张图集，那是另一个数量级的工程（尺寸、mipmap、
淘汰），而 CSS 背景图在密集区域本来就少。**同一个 url 已经是 O(1)。**

顺带把线索里那条排除掉：`draw` 里逐实例 `Option<String>` 的 clone 与比较，**实测
4097 个实例 3.75 µs**（约 0.9 ns/实例），可以忽略，不用管。

### Affine（旋转/斜切）文字：一条一次 draw

`PreparedKind::Affine` 自带顶点缓冲和 bind group，`can_merge_runs` 直接拒绝，
32 条旋转标签 = 34 次 draw。首帧每条建 2 个 GPU 资源，之后 `affine_cache` 命中
（第二帧 realloc = 0）。真实 UI 里旋转文字是个位数，**不做**。

### backdrop-filter 与不透明度分组：inherent

每个 frost 面板 4 个 backdrop pass（copy / 横模糊 / 纵模糊 / composite）+ 1 个
color pass + 2 次 draw；每个分组 1 个 group pass + 1 个 color pass + 3 次 draw。
这来自语义本身——每个面板读的是**它自己文档位置上的 dest**。唯一可见的窄口子是
「document order 上紧邻、之间没有任何绘制、模糊半径相同」的几个面板可以共享一对
copy+blur，只各自 composite；这种形状（一排同款毛玻璃卡片）存在但不常见，**先记账**。

## 六、静止帧上每帧还在付什么

同一棵树连续画，display list 全部命中 `PreparedBatch`（`build = upload = 0.00`）。
release，p50，单位 µs。

| 场景 | total | validate | frame_plan | 自定义键循环 | 命令循环 | blit |
|---|---:|---:|---:|---:|---:|---:|
| `gpu-scene-ui`（1 个 host texture） | **1.83** | 0.17 | 0.04 | 0.08 | 0.79 | 0.38 |
| `gpu-scene-ui-dense-2k`（**2002 节点**） | **1.83** | 0.21 | 0.04 | 0.08 | 0.75 | 0.38 |
| `gpu-scene-host-textures-64` | 28.50 | 9.79 | 2.83 | 3.67 | **10.00** | 0.50 |
| gpu-view × 16 | 3.92 | 0.71 | 0.29 | 0.92 | 1.21 | 0.33 |
| gpu-view × 64 | 9.46 | 2.50 | 1.08 | 3.08 | 1.92 | 0.33 |
| gpu-view × 256 | 35.88 | 10.42 | 4.92 | 13.50 | 4.79 | 0.38 |
| gpu-view × 1024 | **147.88** | 50.12 | 23.62 | 55.04 | 16.00 | 0.38 |

**普通 UI 没有问题**：2002 个节点的静止帧 1.83 µs。纯 UI 树（无 GPU 内容）走
「只 blit」快路径，约 1.2–1.7 µs。

**GPU 节点是 O(N)，其中两遍是重复的。**一帧里 `plan.custom_nodes` 被走了**四遍**：

1. `validate_scene` 里的 `scene.frame_plan()` → **记忆化路径上仍然跑
   `validate_plan_resources`**（`composition.rs:72`）：一个 `HashMap` 分配 + 每节点
   一次 `scene.primitive()`。
2. `validate_scene` 自己的循环：每节点一次 `primitive()` + 一次注册表查找。
3. `paint` 里**再调一次** `scene.frame_plan()` → 第二遍 `validate_plan_resources`
   + 第二个 `HashMap`。
4. `paint` 的 `resources` / `renderer_versions` 循环：第三遍 `primitive()`、第二遍
   注册表查找、每节点一次 `preparation_version`，外加两个每帧新分配的 `Vec`。

> **落地时的更正（重要）**：下面这段把第 1、3 遍都算成「纯浪费」，理由是冲突「是
> 记忆化 plan 自身的性质，plan 不变它就不变」。**这个理由不成立。**
> `validate_plan_resources` 比的是 `(renderer, revision)`，而 `revision` 的低 32 位
> 是**内容版本**（`gpu_slots.rs::pack_gpu_revision`）——一个视频面板每帧都在改它，
> 且 `node_structure()` 故意**不含** `revision`（含了就每帧重编译渲染图）。所以
> 「两个节点声明同一个 resource 但 revision 不同」这个冲突可以在两帧之间凭空出现，
> 记忆化路径上必须重查。**它是承重的，不是重复。** 因此下面那张表里
> 「去掉两遍冗余扫描」的 −50% 是把一个必需的检查也删掉之后的数字，只能当上界读。
> 实际落地去掉的是真正重复的两遍（`frame_plan()` 的第二次调用 + `validate_scene`
> 的独立循环），1024 节点 147.9 → 94.2 µs（−36%）。

第 1、3 遍检查的是「两个节点是否对同一个 resource 声明了不同 `(renderer, revision)`」。
第 2 遍与第 4 遍做的是同一组查找。

实验（把 `validate_plan_resources` 在记忆化路径上跳过 + 去掉 `validate_scene` 的
循环）。**这是上界，不是可落地的量**——上面的更正说明第一项删不得：

| 场景 | 今天 | 上界（连承重的检查一起删） | 省下 |
|---|---:|---:|---:|
| gpu-view × 64 | 9.46 | **6.04** | 36% |
| gpu-view × 256 | 35.88 | **18.54** | 48% |
| gpu-view × 1024 | 147.88 | **74.25** | **50%** |
| `host-textures-64` | 28.50 | 27.12 | 5% |

gpu-view 那三行省下的量与 `validate + frame_plan` 两列之和逐个吻合。
`host-textures-64` 只省 5%，因为它的主成本是命令循环（64 次 host texture draw，
10.0 µs），而且第一遍扫描本来在给第二遍预热 cache line——**三遍走同一份数据部分是
互相摊销的，各阶段计时之和不能直接当作可回收量**。

剩下那两遍是**不可回收的**：第 4 遍要每帧重新比对 N 个 `preparation_version` 才能
判断 display list 能不能复用；第 1 遍的冲突检查每帧都可能给出新答案。所以
「N 个 GPU 节点的静止帧是 O(N)」这条改不掉，能改掉的是系数——实测 1.6 倍
（147.9 → 94.2 µs），把第 1 遍也变成增量维护则可到 2 倍，代价是动 scene 的核心
变更路径，**没有做**。

### 线索里其余的每帧成本，实测都可以忽略

| 项 | 实测 | 判断 |
|---|---|---|
| `quads.poll_images()` + `host_textures.poll_images()` | 0.04–0.71 µs | 忽略 |
| `gpu_interleaved` 的 `commands.iter().any(...)` | 0.00–0.21 µs | 忽略（合并后 commands 很短，猜对了） |
| `self.painted` 的 `commands.iter().all(...)` | 0.00–0.17 µs | 忽略 |
| `PreparedBatch` 键比对 | ≤0.50 µs | 忽略 |
| `text.begin_frame` 的 `renderers.truncate`、`icons/meshes.begin_frame` | **0.00** | 命中缓存的帧上根本不执行（它们在 `build` 分支里） |

### 「任何一个 GPU 节点都会让整棵树每帧重新 encode」

是真的（`self.painted` 只在命令表里没有 `HostTexture` / `Custom` 时才设），
但**实测便宜，而且必要**。N 个纯色 quad + 1 个 host texture，第二帧（display list
已缓存，仍然全量重 encode）：

| quad 实例 | 1 | 65 | 257 | 1025 | 4097 |
|---|---:|---:|---:|---:|---:|
| paint total /µs | 3.17 | 4.75 | 3.33 | 4.67 | **9.50** |
| 其中命令循环 /µs | 1.71 | 2.00 | 1.29 | 2.67 | **3.75** |

去掉那个 host texture，同一棵树走「只 blit」快路径约 1.3 µs。**4096 个 quad 的
重 encode 代价约 8 µs**，没有值得回收的东西。

它也是必要的：host texture 的**内容**可以在场景不变时改变（painter 里那条注释就是
这个意思），而 dest 是一张纹理——重画 host texture 所在的区域，就必须把 document
order 上排在它之后、与它重叠的普通 UI 一起重画。要只重编码一部分，就得把 dest 按
「第一个 GPU 内容之前 / 之后」切成两段缓存，而可回收的量只有 8 µs。**不做。**

## 七、结论：做什么，不做什么

| 项 | 实测收益 | 判断 |
|---|---|---|
| **打包 icon atlas**（一张纹理 + shelf 打包器） | 真实 shell **34 → 15**（实测，与模拟一致）；顺带修掉「同字形不同尺寸各占一张纹理」和 128 条 LRU 抖动 | **✅ 已做**。这是唯一一条既常见又能压到 O(1) 的线 |
| **`frame_plan()` 每帧两次 + `validate_scene` 与 `paint` 各走一遍自定义节点** | 1024 节点 **147.9 → 94.2 µs/帧**（−36%）；256 节点 32.9 → 23.2；host-textures-64 21.5 → 17.5 | **✅ 已做**。是纯重复，不是权衡。记忆化路径上的冲突检查**不能**删（见第六节更正） |
| 文字跨 scissor 合并 | 微基准 34 → 3，**真实 shell 0**；重叠判定接手 | **不做**。安全、有像素证据，但不解决实际问题 |
| 共享同一张 host texture 时的 N 次 draw | 64 节点可从 66 → 3 | **记账**。今天没有这种产品形状，出现了再按 gpu-view 的实例化路子改 |
| 把轴对齐 scissor 搬进顶点数据以解放 quad/icon 合并 | 真实 shell 34 → 33 | **不做** |
| `quad.rs` 逐实例 `Option<String>` clone | 4097 实例 3.75 µs | **不做** |
| Affine 文字合并 | 32 条 → 34 次 draw，但真实 UI 是个位数 | **不做** |
| backdrop 面板共享 copy+blur | 需要「紧邻 + 无绘制间隔 + 同半径」 | **记账** |
| 不同纹理的 HostTexture / 不同 url 的背景图 / backdrop pass / 分组 pass | 一次 draw 一张纹理；一个面板一次 dest 读 | **是下限，不是缺陷** |
| 任何 GPU 节点触发整树重 encode | 4096 quad 约 8 µs | **不做**，且必要 |

两条「做」的验证方式（照 `scene_paint/tests.rs` 的既有写法）：

- icon atlas：一条 N 个**不同字形**相邻图标的 draw call 增量断言（与 1 个图标相同），
  加上每个字形仍然上墨的定点像素断言；反向验证是恢复 per-key 纹理，断言给出 N+2 vs 3。
  整帧字节哈希对照一张含多字形、多尺寸图标条的帧。
- `frame_plan` 去重：断言一帧里 `validate_plan_resources` 只跑一次（或直接断言
  `frame_plan()` 在记忆化路径上是 O(1)），以及冲突场景**仍然整帧拒绝**——那条语义
  不能丢，它有既有测试覆盖。

**不用快照套件做视觉门禁**（本机字体栅格化与基线不符）；像素证据用 `tests.rs` 的定点
读取，整帧对照用字节哈希。

## 八、落地结果（2026-09-10 同一次会话）

两条「做」都已落地在当前分支上，实现细节与前后对照写进了
[`gpu-node-scale.md`](gpu-node-scale.md) 的两节「已落地」。摘要：

| 项 | 结果 |
|---|---|
| 打包 icon atlas | 真实 shell 34 → **15** draw call；五个基准场景 5 / 4 / 68 / 16 / 16 **不变**（它们只用一个字形一个尺寸）；两张对照帧字节哈希不变（`db94505a59558ebc`、`99c98d511c739fb8`） |
| 静止帧不再重复扫描 GPU 节点 | gpu-view 64 / 256 / 1024 的 `paint` p50 从 0.0093 / 0.0329 / 0.1470 ms 降到 **0.0072 / 0.0232 / 0.0942 ms**；`host-textures-64` 0.0215 → **0.0175 ms** |

`nana-ui` 465 个测试通过（原 464，替换了 1 条、新增 2 条），`nana-ui-scene` /
`nana-ui-runtime` / `nana-ui-core` 全绿，`clippy --all-targets` 无告警。

新增 / 替换的回归测试，三条都做过反向验证：

- `adjacent_icons_batch_into_one_draw_whatever_their_glyphs`（`scene_paint/tests.rs`）
  ——12 个相邻图标在「同字形」「不同字形」「同字形不同尺寸」三种情况下都必须与 1 个
  图标 draw call 相同，且每个图标仍在自己的槽位上墨。把 `can_extend_run` 的
  `head.key == candidate.key` 加回去，它给出 14 vs 3。
- `a_full_sheet_keeps_this_frames_glyphs_and_drops_the_idle_ones`（`icon.rs`）——沉睡
  字形被回收、在用字形必须活过重排、工作集小时纹理不长大。去掉 repack 里保留
  `frame_keys` 的那一步，它在「必须活下来」上挂。
- `a_frame_larger_than_the_sheet_grows_it_instead_of_dropping_glyphs`（`icon.rs`）——
  一帧自己的字形装不下时纹理必须增长。去掉增长循环，它在「必须找到位置」上挂。

**第七节里判为「不做」的项一个都没有动**，包括文字跨 scissor 合并——它在真实形状上
收益为 0，理由见第四节第 2 小节。
