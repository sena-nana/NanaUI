# 架构

这一页是写给写应用的人看的。crate 之间的约定全文在 [架构合同](../reference/architecture.md)。一帧里逐步发生了什么，在 [框架如何运行](../reference/how-it-works.md)。

## 一条路径

```text
应用状态
    │  视图挂载 / 绑定 / 换纹理
    ▼
UiWorld          树、样式、布局、命中、焦点
    │  flush
    ▼
UiScene          与后端无关的绘制图
    │  SceneWgpuPainter
    ▼
宿主 Surface     Window / Device / Queue
```

`view!`、函数调用和 Vue 都写进这棵 `UiWorld`。它们是输入，不是第二棵树，也不是第二套 painter。

CSS 只有 `nana-ui-css` 这一处。Vue 路径在运行时解析样式。`view!` 的 `<style>` 和 `css!` 在构建时解析，运行时拿到的是 Style Model。

路由、每个区域里放什么、业务状态存在哪里，由你的应用持有。框架提供控件语义、布局、命中、焦点、输入法、无障碍增量，以及把树画进宿主的 Surface。

## 谁拥有哪一层

```text
nana-ui-core         样式、主题、几何、Workspace、运动合同
nana-ui-runtime      UiWorld、控件、布局、输入、焦点、Shell、Workspace、Dock
nana-ui-scene        绘制图。依赖 runtime，不依赖 WGPU
nana-ui              宿主适配和 SceneWgpuPainter
nana-ui-css          样式表解析和级联。不进 core / runtime / scene
nana-text            测量、整形、保留文本布局
nana-ui-input        输入事件和 HostServices。runtime 里的路由是唯一消费方
nana-gpu             GpuContext、FrameContext、GpuTexture。WGPU 是唯一后端
nana-window          原生窗口、系统材质、标题栏、缩放
```

普通控件拿不到窗口句柄。系统材质走 `nana-window`。材质失败时，返回实际应用的结果，或一次明确的回退。

你的应用拥有 Window、Surface、`GpuContext`，以及每一帧的 `FrameContext`。`SceneWgpuPainter` 画进这些帧。实时画面是 `GpuTextureView` 或 `CustomRenderNode`，仍然留在布局、裁剪、命中和文档顺序里。

## 逻辑状态和呈现

`UiWorld` 保存的是逻辑状态。过渡中的透明度和变换留在呈现 overlay 上，按节点和属性去查。你的业务读到的是逻辑值。绘制、命中、焦点和无障碍在需要时，按同一个时间戳取呈现值。

::: warning
不要把合成采样写回基础样式。那样会把过渡中的瞬时值当成逻辑状态。呈现停在 overlay 上。
:::

动画意图编译进 `nana-ui-core` 的运动合同。产品路径不把画面读回 CPU。

## 这一层不提供的

文档不把浏览器 DOM、应用内路由或服务端渲染写成框架能力。应用内网页只有明确的 `BrowserView` 宿主例外，当前后端是 macOS。区域里放什么、导航栈怎么走，由你的应用自己放进壳层槽位。

## 接着读

<div class="next-steps">
  <a class="next-step" href="/architecture/frame">
    <p class="next-step-link">一帧</p>
    <p class="next-step-caption">从输入到画上 Surface。</p>
  </a>
  <a class="next-step" href="/architecture/gpu">
    <p class="next-step-link">实时画面</p>
    <p class="next-step-caption">纹理怎样作为节点留在树上。</p>
  </a>
  <a class="next-step" href="/architecture/text">
    <p class="next-step-link">文本</p>
    <p class="next-step-caption">测量、整形和保留的文本布局。</p>
  </a>
  <a class="next-step" href="/architecture/window">
    <p class="next-step-link">窗口</p>
    <p class="next-step-caption">原生窗口、标题栏和缩放。</p>
  </a>
</div>
