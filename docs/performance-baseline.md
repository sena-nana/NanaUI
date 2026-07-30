# NanaUI 性能基线

## 2026-07-29 macOS 初始基线

测量环境：

- macOS 26.5.2（25F84），Apple M4，10 核 GPU，16 GiB；
- Metal 4，DELL U2424H 1920×1080@120Hz；
- Rust/Cargo 1.97.0，Iced/iced_wgpu/iced_winit 0.14.0；
- 测量时用户会话锁屏、显示器休眠；
- Release 示例通过 `cargo build --release --workspace --examples --locked` 构建，未额外执行二进制压缩。

### Release 可执行文件

| 示例 | 字节 | 约值 |
| --- | ---: | ---: |
| `workspace-demo` | 11,209,296 | 10.69 MiB |
| `component-gallery` | 11,254,272 | 10.73 MiB |
| `transparent-window` | 10,663,120 | 10.17 MiB |
| `gpu-view-demo` | 10,688,432 | 10.19 MiB |
| `hosted-gpu-demo` | 9,685,344 | 9.24 MiB |

### 宿主纹理 Demo

`hosted-gpu-demo` 的 `--measure-first-frame` 模式从进程入口开始计时，在第一帧 `SurfaceTexture::present` 后输出耗时并退出。直接运行已构建的 Release 二进制 5 次：

| 次数 | 首帧耗时 |
| ---: | ---: |
| 1 | 152.884 ms |
| 2 | 98.848 ms |
| 3 | 102.604 ms |
| 4 | 100.333 ms |
| 5 | 98.666 ms |

中位数为 100.333 ms，范围为 98.666–152.884 ms；5 次均确认实际材质为 macOS Vibrancy。该数据包含 Winit、原生材质、Surface、适配器、唯一 Device/Queue、直接 WGPU 场景、Iced renderer 和首帧合成初始化，不包含 Cargo 编译；第一次运行可见缓存预热差异。由于本轮会话锁屏，它是“进程入口到首帧提交”数据，不等同于用户可见窗口完成显示时间。

Release 进程静置 44 秒和 70 秒时，`ps` 两次均记录：

- CPU：0.0%；
- RSS：90,960 KiB（约 88.83 MiB）；
- 状态：sleeping。

事件循环使用 `ControlFlow::Wait`。场景和 Iced 共用唯一 Device/Queue；场景写入宿主 `TextureView`，NanaUI 直接采样，不进行 CPU 回读或图片编码。Component Gallery 仅在真实 loading 动画进行期间启用 100ms 定时订阅，动画结束后移除订阅。

### 100 / 500 / 1000 项列表

`ui-benchmark` 在 900×640、scale factor 1.0 的离屏 Iced WGPU renderer 中运行真实 NanaUI `ListItem` 与 `Scrollable`。每轮都会重建列表、改变选中项并发送滚轮事件；每个规模预热 10 次后采样 60 次。完整结果保存在 `docs/performance/2026-07-29-macos-ui-list.json`。

下表为中位数，括号内为 p95，单位均为毫秒：

| 项数 | View 构建 | Layout/Diff | 滚轮 Update | Draw CPU | GPU submit+wait | 总计 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 0.061 (0.067) | 0.036 (0.044) | 0.003 (0.003) | 0.004 (0.004) | 1.569 (1.619) | 1.666 (1.713) |
| 500 | 0.198 (0.237) | 0.152 (0.181) | 0.016 (0.020) | 0.005 (0.007) | 1.576 (1.605) | 1.952 (2.027) |
| 1000 | 0.373 (0.447) | 0.289 (0.341) | 0.030 (0.036) | 0.007 (0.008) | 1.581 (1.599) | 2.276 (2.422) |

`GPU submit+wait` 包含 Iced prepare/render、命令提交和阻塞等待 GPU 完成，是端到端上界，不等同于 timestamp query 得到的纯 GPU Pass 时间。列表内容超出视口后，布局仍处理全部条目，但裁剪使实际绘制量保持接近可见区域。

### LiliaUI 表现对齐后复测

完成语义色、工作区 region、resize handle、标题栏和控件间距对齐后，沿用相同 Release 配置重新执行 10 次预热与 60 次采样。完整分布保存在 `docs/performance/2026-07-29-macos-ui-list-lilia-parity.json`：

| 项数 | 总计中位数 | 总计 p95 |
| ---: | ---: | ---: |
| 100 | 1.622 ms | 1.901 ms |
| 500 | 1.728 ms | 1.823 ms |
| 1000 | 1.927 ms | 2.094 ms |

三组总计中位数均未高于初始基线；视觉对齐没有引入可测的列表路径性能回退。p95 属于单轮端到端观测值，继续保留完整样本范围，不据此外推未测平台。

复测命令：

```bash
cargo build --release --workspace --examples --locked
./target/release/examples/hosted-gpu-demo --measure-first-frame
./target/release/examples/hosted-gpu-demo
pgrep -x hosted-gpu-demo
ps -p <PID> -o pid=,pcpu=,pmem=,rss=,etime=,state=
cargo run --release -p nana-ui --example ui-benchmark --locked
```

## 2026-07-30 WGPU 30 与动态工作区框架复测

NanaUI、Iced 与 Cryoglyph 收敛到 WGPU `30.0.0` 后，在同一 Apple M4 / Metal
设备上完成动态 Region 框架后单独运行 `ui-benchmark`。完整结果保存在
`docs/performance/2026-07-30-macos-wgpu30-ui-list.json`。

Release 全部示例重新构建后的当前体积：

| 示例 | 字节 | 约值 |
| --- | ---: | ---: |
| `workspace-demo` | 11,367,408 | 10.84 MiB |
| `component-gallery` | 11,346,464 | 10.82 MiB |
| `transparent-window` | 10,761,600 | 10.26 MiB |
| `gpu-view-demo` | 10,795,648 | 10.30 MiB |
| `hosted-gpu-demo` | 9,589,440 | 9.15 MiB |

`hosted-gpu-demo --measure-first-frame` 在 Release 构建后的首次冷缓存运行耗时
468.473ms；随后连续五次为 204.459、145.320、168.125、188.091 和
174.247ms，中位数 174.247ms。六次均完成真实 Surface present，并返回
macOS `vibrancy` 材质。普通模式静置 15 秒时为 0.0% CPU、99,168KiB RSS
（约 96.84MiB）和 sleeping 状态；`transparent-window` 也已在相同构建中
成功创建真实窗口。

| 项数 | View 构建中位数 | Layout/Diff 中位数 | 总计中位数 | 总计 p95 |
| ---: | ---: | ---: | ---: | ---: |
| 100 | 0.019 ms | 0.031 ms | 0.526 ms | 2.597 ms |
| 500 | 0.100 ms | 0.129 ms | 0.688 ms | 3.457 ms |
| 1000 | 0.168 ms | 0.204 ms | 0.756 ms | 2.829 ms |

按钮改为先测量自然内容尺寸、再执行双轴对齐后，三组总计中位数相对前次分别为
-0.016、+0.003 和 +0.007ms，没有表现出列表路径回退。p95 仍包含 Metal 提交
与阻塞等待抖动，不作为纯 GPU Pass 时间。

## 尚未完成的基线

以下数据仍需在屏幕解锁、固定窗口尺寸及对应目标平台上采集，不能从当前结果外推：

- timestamp query 的纯 UI GPU Pass 时间；
- 可见窗口中的 100、500、1000 项连续滚动与输入延迟；
- 多面板连续 resize；
- 透明窗口与不透明窗口的差异；
- Windows、Linux 以及不同 macOS 设备；
- 用户可见冷启动、打包产物体积与签名后的应用体积。

后续变更应沿用同一构建配置和测量环境，或明确记录环境差异，不得用 Debug 与 Release、锁屏与可见窗口数据直接互相比较。
