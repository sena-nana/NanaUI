# 文本

测量、整形和保留的文本布局，权威是 `nana-text`。你写节点上的文字和样式。字有多宽、在哪里断行、光标落在哪个簇，都由这一份引擎回答。

绘制走 `NanaRenderer::text`。测量和绘制问的是同一个进程里的引擎。测量在 `nana-ui` 的 `nana_text.rs`：`NanaTextShaper` 是进程级引擎的壳。绘制在 `scene_paint/text/`。引擎本体在 `text_engine.rs`。flush 调用的就是这个 shaper。

```text
UiWorld 里的文字
    │  NanaTextShaper
    ▼
nana-text          字体选择、shaping、TextLayout
    │  句柄留在节点上
    ▼
UiScene            ScenePrimitiveKind::Text
    │
    ▼
NanaRenderer::text 栅格、atlas，画进这一帧
```

产品路径仍是 `UiWorld` → `UiScene` → `SceneWgpuPainter` → 宿主 Surface。文本不另开一棵树，也不另开 Device。下面只写你要顺着走的路径。容差、语料和每一阶段的计数在 [文本引擎](../reference/text-engine.md)。

## 引擎拥有什么

`nana-text` 拥有文本 IR、稳定代际 ID，以及何时重做 shaping、行怎样编排。命中、光标和选区矩形是跑在这份 IR 上的纯函数。

它不重写 OpenType、复杂文字整形、Unicode BiDi 和 UAX #14 的断行表。那些留在既有的库里，由 `nana-text` 决定何时调用。字形轮廓的栅格化也不在 `nana-text`。那是画笔一侧的设备状态。

排版用词来自 `nana-ui-core`：变体轴、字距、断行、对齐、方向、书写模式、行高。不要在引擎旁边再做一套。

句柄是 `{ index, generation }`。`generation == 0` 是空。槽位回收后用更高的代际重发。旧句柄会被拒绝，不会指到新内容。

## 先选字体

字体层只做选择，不做 shaping。样式收成一次查询，选出 primary 和 fallback 链，再按覆盖率把一段文字分到具体的 face。

```text
TextStyle
    → FontQuery
    → FontSelection
    → FontAssignment
```

face 没有的变体轴不生效，也不会被改写成 `wght`。进程里这一份 `FontSystem` 由 `text_engine.rs` 持有。`@font-face`、`local()` 和 `sans-serif` 都落在它上面。

## 再整形

Shaping 把字符序列变成不可变的 `ShapedRun`。这一步不断行，不算光标。那是下一步的事。

分段按字素簇、script 和 BiDi。一个簇不会被样式边界切开。段落方向由样式里的 `direction` 固定，不按第一个强字符猜测。覆盖率选中了、整形仍然画出 `.notdef` 的簇，会换候选 face 再试。一个 face 都没有时，才不出 run。

相同的文本和塑形样式可以共用一份结果。颜色、透明度、transform、最大宽度和换行都不在这个缓存键里。

## 然后排版

`ShapedRun` 加上约束，得到不可变的 `TextLayout`。宽度、换行、对齐或最大行数变了，只重跑这一层，不重新整形。

单行、不换行、没有高度预算、而且只有一段的标签走 fast path：累出宽度，算一个行盒，需要时裁一次省略号。其余走段落路径。断行机会来自 UAX #14。到底断不断，看已经整形好的 advance，不按字数估算。

```text
ShapedText + TextConstraints
    → 单行 fast path，或按段断行
    → 每行的视觉顺序、行盒、对齐、省略号
    → TextLayout
```

颜色、透明度和 transform 同样不进入排版的缓存键。

## 留在树上的是句柄

Runtime 对文本只保存逻辑状态、revision 和句柄。整形和排版的结果由 `nana-text` 持有。

```text
TextNodeState
    revisions、stamp、TextLayoutId
        │
        ▼
进程级引擎的 shaping cache 与 layout cache
        │
        ▼
ExtractedNode.text_layout → UiScene 里的 Text
```

什么变了，在发生的地方分类：

| 变了什么 | 接下来 |
| --- | --- |
| 内容、字体、塑形样式 | 重新 shaping，再排版 |
| 宽度、换行、行高、对齐 | 只排版 |
| 颜色 | 不重新整形，也不重新排版 |
| transform、opacity | 不碰文本 |
| 逐字特效、揭示（`GLYPH_PRESENTATION`） | 不碰文本，只在合成器 |
| 富文本 span 的字体、字号、字重、字距 | 重新 shaping，再排版 |
| 富文本 span 的颜色、描边、阴影、装饰线 | 不重新整形，也不重新排版；画笔重建这段的字形实例 |
| 富文本 span 的特效索引 | 不碰文本 |
| 内联对象（贴纸、图片）的尺寸 | 只排版，不重新 shaping |
| 内联对象显示的内容 | 只重绘 |

富文本是应用持有的 `RichText`：一个字符串加按字节范围的稀疏样式。`SetRichText` 把它交给节点，按它改到的那一层分类。塑形层在 `TextNodeState` 里铺在节点的计算样式上，变成 `TextSource` 的 span，测量和绘制还是同一份 `TextLayout`。绘制层进场景的 `Text { rich }`，画笔把阴影、描边、填充和装饰线排成同一段落的实例。细节见 [文本引擎](../reference/text-engine.md#富文本-span)。

`transform` / `opacity` 不增加文本的 revision，所以稳态的合成动画碰不到 shaping。采样也不写回基础样式。呈现仍留在 overlay 上。

三个 revision 和字体代际都没变时，在复制文本、查缓存之前就跳过。这一跳过的代价跟文字有多长无关。

同一段文字、同一份约束，可以共享同一份 `TextLayout`。句柄交给另一棵 `UiWorld` 会被拒绝。节点删掉时句柄释放。换父节点不释放。

## 可编辑是附加层

标签不持有编辑会话。没有被编辑的文字不跑这里。

可编辑文本在换行处分段。每段一份 `TextLayout`。改一个字，只重排变化的段。只移动光标或选区，不重新 shaping，也不重新排版。

组字是暂时的 overlay，不是先提交再撤销。组字期间，已提交的文本不动。屏幕上排的是已提交文本加上 preedit。失焦会丢掉 preedit。选区颜色是绘制，不导致重新整形。

光标、命中和选区矩形都读已经留下的布局。几何过期时，移动退回逻辑顺序，而不是拿另一份会话的数字去对。

## 画的是留下的那一份

画笔优先用 Runtime 保留的 `TextLayout`。句柄就是身份。换行、省略号和 `white-space` 都是 Runtime 已经定好的，不再从场景描述里重排一遍。

没有句柄的文字，以及对齐盒对不上的节点，画笔才自己排一份。编辑器的呈现、EmptyState 和 Modal 的内建文本属于前一种。带尾随控件、或内容几何改写了文本盒的节点属于后一种。

栅格缓存的键里没有颜色、节点透明度和场景变换。同一个字两种颜色共用一张位图。布局、亚像素相位和字体代际都没变时，画笔复用已有的字形实例，不重新整形。

宿主换设备时，atlas 随旧的 painter 丢掉。新设备的第一帧把用到的字形再栅格、再上传一次。段落仍是 Runtime 留下的那一份，不因此重排，也不重新整形。

菜单提示、快捷键徽标这类装饰，仍可能按字符数估盒子有多宽。盒子里的字还是由引擎排、由画笔画。

## 接着读

<div class="next-steps">
  <a class="next-step" href="/architecture/window">
    <p class="next-step-link">窗口</p>
    <p class="next-step-caption">原生窗口、标题栏和系统材质。</p>
  </a>
  <a class="next-step" href="/reference/text-engine">
    <p class="next-step-link">文本引擎</p>
    <p class="next-step-caption">字体、整形、排版和可编辑路径的合同。</p>
  </a>
  <a class="next-step" href="/architecture/frame">
    <p class="next-step-link">一帧</p>
    <p class="next-step-caption">文字在 flush 里测量，不在你自己的布局里。</p>
  </a>
</div>
