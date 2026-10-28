# 移动宿主

移动宿主是 iOS 和 Android 两个**实验性**宿主共用的那一层（`platform/mobile-core`，crate `nana-mobile-host`）。目前它只用来验证一个问题：NanaUI 加上混合原生，能不能达到原生 App 的性能和手感。它不是产品交付目标，不要据此对外承诺移动端支持。

两个宿主的平台细节分别见 [iOS](ios.md) 和 [Android](android.md)。iOS 宿主是 rlib，App 通过 `IosApp` 把自己的视图放进去（见 [iOS](ios.md#在宿主上做一个-app)）。

## 分层

```text
原生外壳（UIKit 视图 / GameActivity）
  显示链、触摸与按键、文本代理、相机层、生命周期
    ↓ 降级成平台无关的调用
nana-mobile-host
  MobileRuntime：一份 RuntimeDocument、输入路由、IME 生命周期、安全区、FrameDemand
  MobileRenderer：一个 Device/Queue、SceneWgpuPainter、HostTextureRegistry
    ↓
nana-ui（只开 gpu，不开 hosted）
```

移动端不走 `run_runtime`、`RuntimeProgram` 和 `EmbeddedRuntime`，这些都在 `hosted` 里，依赖 winit。外壳拥有窗口和事件循环；`MobileRuntime` 只做和平台无关的部分。

`MobileRuntime` 默认装 NanaUI 内置的浅色、深色主题，度量换成 `TOUCH_METRICS`。应用有自己的设计系统时，在挂载视图时调用 `set_theme(MobileTheme { light, dark, page })`：两份定义按原样编译（编不过就拒绝，原主题不变），所以给触屏用的定义要从 `TOUCH_METRICS` 出发，只改自己的设计决定的部分；`page` 是根节点在安全区内外画的底色角色，默认 `Background`，整屏是一张卡片、分组是卡上浅色块的应用用 `Surface`。`background_rgba()` 报的也是这个角色的颜色。

## 混合原生的规则

原生只以两种方式参与：

1. **提供行为**：像素由 NanaUI 画，行为交给平台。例如文本输入走平台的文本系统（iOS 的 `UITextInput` 加 `UITextInteraction`，Android 的 GameTextInput），平台因此负责键盘、组字、候选、放大镜、选区手柄和编辑菜单。无障碍走平台的原生树。
2. **整块原生层**：放在 NanaUI 表面的上方或下方，不进入布局。例如相机预览层垫在透明表面下面，页面在对应区域留透明；系统面板（权限、分享、扫码）以模态方式盖在上面。

垫在下面的原生层要真的透得出来，需要两件事：

- **表面不能是不透明的。** 透明表面（`SurfaceOptions { transparent: true }`）依次选 `PreMultiplied`、`PostMultiplied`、`Inherit`。Metal 只提供 `Opaque` 和 `PostMultiplied`，只有后者会把 `CAMetalLayer.opaque` 清掉；退回 `Opaque` 的话，所有透明像素都会显示成黑色。
- **根节点不画页面底色。** `MobileRuntime::set_background_passthrough(true)` 之后，根节点不画背景，外壳在原生层下面用 `background_rgba()` 画同样的主题底色，所以页面看上去和不透明时一样。

页面里的 `GpuTextureView` 只能在它的宿主纹理已经注册时显示：一帧里只要有一个采样不到的宿主纹理，`paint` 就会整帧失败。测量页的预览区因此是一个固定大小的透明容器，里面的纹理视图只在 CPU 预览槽注册后才显示（`Probe::sync_preview`，两个宿主都在准备每一帧之前调用）。

**不把原生控件嵌进 NanaUI 的布局和滚动内容。** 原生层在系统合成线程上移动，NanaUI 按自己的帧移动，两者在滚动时对不齐；裁剪和层级还要拆层合成。

文本代理需要知道编辑器的几何，所以 runtime 提供了 `AppContext::focused_text_offset_at`（点落在哪个字节偏移）和 `focused_text_caret_rect`（某个偏移的光标框）。`MobileRuntime::text_offset_at`、`text_caret_rect` 和 `text_input_bounds` 用逻辑像素转发这些查询；`set_text_selection` 把平台改动后的选区写回编辑器。

## 帧：按需出帧

`MobileRuntime::frame_demand()` 告诉外壳显示链下一次什么时候需要触发，有三种答案：

| 值 | 含义 | 外壳的做法 |
| --- | --- | --- |
| `Now` | 有未呈现的变化：输入、视口、外观、待应用的响应式写入、合成器动画、宿主纹理更新 | 下一个 vsync 出帧 |
| `At(t)` | 下一个动画、光标闪烁或计时器的截止时间 | 暂停显示链，到 `t` 再恢复 |
| `Idle` | 没有任何事要做 | 暂停显示链 |

每次显示链触发时调用 `prepare_frame(now)`。它依次轮询就绪的视图任务、推进到期的动画和合成器时钟，然后排版，返回这一帧有没有新内容；返回 `false` 时外壳不画。视图任务在别的线程就绪时调用 `set_task_wake` 装的唤醒函数，外壳在主线程请求一帧。

**静止的文档不出帧**，也没有心跳帧。`a_settled_document_asks_for_no_frames` 钉住这一点。

## 生命周期

`MobileRenderer` 从第一块 surface 起就持有 Device/Queue、painter 和宿主纹理表，一直到 App 退出。进入后台时外壳只调 `detach()` 丢掉 surface；回到前台时用 `attach()` 接上新的 surface，然后 `request_present()`。文档和它的状态在这期间一直保留。

## 几何

`MobileViewport` 包括物理尺寸、缩放（UIKit 的 `contentScaleFactor`，Android 的 `densityDpi / 160`）和四边安全区（`SafeArea`，单位是物理像素）。`MobileRuntime` 的根节点铺满整个 surface，按安全区给应用视图留内边距。横屏时刘海和导航栏所在的左右两边同样避让。

移动宿主默认安装触屏密度 `TOUCH_METRICS`（见 [theme.md](theme.md)）。

## 输入

`push_touch` 只跟随主触点，第二根手指不参与。触摸拖动滚动、甩动和回弹由 runtime 完成（见 [runtime-scene.md](runtime-scene.md#触摸滚动与-scrollphysics)）。`set_scroll_physics` 选择物理档位：iOS 宿主用 `Ios`，Android 宿主用 `Android`。甩动和回弹走动画 deadline，所以进行期间 `frame_demand` 返回 `At`，停下后回到 `Idle`。`push_composition` 接平台文本系统的组字事件。点击编辑器以外的位置时，先提交未完成的组字，再移动焦点。

## 测量页

`platform/mobile-probe`（`nana-mobile-probe`）是两个宿主都挂载的测量页面：一个搜索框、一个开关和一个滑条、一块预览区，以及一个纵向滚动的 200 个按钮的网格。预览区没有背景，原生相机层会从下面透出来；打开「CPU 预览」后，宿主改为把帧上传到 `PREVIEW_SLOT`，用来和原生预览层做对比。

## 验收

- `cargo test -p nana-mobile-host -p nana-mobile-probe`：帧需求、安全区、触屏密度、组字、选区回写，以及光标框与点命中的互逆；另外，两种物理档位下，网格都要能拖动、甩动，停下后回到 Idle（`a_swipe_scrolls_the_grid_flings_and_settles_idle`）。`every_preview_path_paints` 在本机 GPU 上真的画一遍原生预览和 CPU 预览两条路径（没有 GPU 时跳过），宿主纹理缺失这类整帧失败只有这样才测得出来。
- `scripts/check-ios-arm64.sh` 和 `scripts/check-android-arm64.sh`：交叉编译。
- 性能和手感只认真机数据，模拟器的数字不能作为证据。达标线和对照方法见 [iOS](ios.md#验收)。
