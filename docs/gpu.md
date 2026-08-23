# 实时画面

角色、特效、着色器、游戏视口和按钮一样，是树上的一块内容：有位置、会被裁切、点得到。不是盖在界面上的一层，也不是抠出来的洞。

新应用先把一扇窗口和一棵普通界面跑通，见 [开始](start.md)，再把画面挂上去。

## 怎么放进去

应用先画出一帧画面（角色、场景、预览都可以），再把它当作界面上的一块内容交给 NanaUI。这块内容和按钮一样占据布局里的位置：旁边是面板，上面可以是对话框，滚动时它跟着走，点到它就是点到它。

Live2D 也走这条路。后景、角色、前景是界面上相邻的几层画面，由你的应用去映射，而不是在 NanaUI 里出现一种「Live2D 控件」去直接画整个窗口。

仓库里可以对照：

```bash
cargo run -p nana-ui --example gpu-view-demo --features bundled-fonts,gpu
cargo run -p nana-ui --example hosted-gpu-demo --features bundled-fonts,gpu
```

## 内部如何工作

窗口、Surface、Device、Queue 由宿主唯一拥有。`SceneWgpuPainter` 被注入这一套上下文，把 `UiScene` 画进当前窗口的 Surface。不要再 `request_device` 一次，也不要把画面读回 CPU、编码成图片再贴回去。

```text
宿主 Window / 事件循环
        │
        └── 唯一 Device / Queue / Surface
                  ├── 应用自己的画面（NanaShader、Live2D、预览……）
                  └── NanaUI 的界面与 HostTexture 节点
```

`run_runtime` 是完整宿主入口。应用持有 `RuntimeDocument`，按窗口 viewport 调用 `flush`，框架完成文字 shaping 和布局。业务通过稳定的 `WindowId` 和 `WindowCommand` 开关工具窗口，不自己握着 Winit 窗口或 Surface。已经有外部事件循环时，可以用更低层的 `HostedGpuContext`；附加窗口只新建 Surface，继续共享同一份 Adapter / Device / Queue。

**HostTexture** 用稳定 ID 和 generation 包住一份可采样的纹理。宿主替换或改尺寸时加 generation，NanaUI 只重建对应的绑定，不拆布局。实时画面在 `prepare_window_frame` 里拿到最新纹理；被换掉的那份必须等到这一窗真正 present 之后，再在 `window_frame_presented` 里释放。

**合成顺序就是文档顺序。** `nana.host-texture` 在主 pass 里、在这个节点该出现的位置采样，不攒到帧尾另开一趟。能加入当前 dest pass 的自定义绘制走 `GpuView` 的 `draw_in_pass`；必须先画到纹理上的内容（例如 Live2D）注册 `SceneResourceProducer`，由编译图的 `PrepareExternal` 先提交，再被界面采样。同一资源在一帧里出现冲突的 revision 会整帧拒绝，不会偷偷选其中一个。

没有 GPU 节点的帧可以用 4x MSAA 画方块和网格，文字在 resolve 之后画。一旦这一帧里出现 HostTexture 或自定义 GPU 节点，整帧改为单采样，在同一 pass 里按顺序切 pipeline，不为每个插槽单独开 pass，也不在自定义节点两侧反复 resolve。

设备丢失后，runtime 为现有窗口重建唯一 GPU 上下文、所有 Surface 和 painter，再调用 `RuntimeProgram::rebuild_gpu`。业务程序实例不会被换掉，你只需要把依赖旧 Device 的资源重新绑上。

## 不要做的

- 让角色框架绕过界面，直接画到窗口表面
- 在界面画完之后，再单独贴一层「游戏」
- 为了省事把 GPU 内容攒到帧尾一次性画
- 为界面另开一套 Device / Queue
