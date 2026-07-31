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

## 2026-07-30 LiliaUI 完整组件覆盖复测

完成组件覆盖矩阵中的控件、表面、反馈、菜单/浮层、日历、图片查看器、设置区块
与桌面/弹窗 Shell 后，在同一 Apple M4 / Metal 环境重新执行 10 次预热和 60 次
采样。完整中位数与 p95 保存在
`docs/performance/2026-07-30-macos-component-suite.json`。

| 场景 | 总计中位数 | 总计 p95 |
| --- | ---: | ---: |
| 100 项列表 | 0.527 ms | 0.845 ms |
| 500 项列表 | 0.707 ms | 0.765 ms |
| 1000 项列表 | 1.011 ms | 1.186 ms |
| 控件页 | 0.623 ms | 0.902 ms |
| 表面页 | 0.555 ms | 0.593 ms |
| 反馈页（日历指针） | 0.569 ms | 0.685 ms |
| 工作区页 | 0.577 ms | 0.710 ms |
| 三个设置页 | 0.535–0.551 ms | 0.650–0.657 ms |
| Dialog | 1.183 ms | 1.288 ms |
| Popover | 0.574 ms | 0.690 ms |
| Context Menu | 1.056 ms | 1.191 ms |
| ImageViewer（指针缩放） | 1.187 ms | 1.340 ms |
| 500 项 Dropdown | 0.309 ms | 0.399 ms |
| 200 项 SearchDropdown | 0.297 ms | 0.309 ms |
| 120 项 Context Menu | 0.342 ms | 0.400 ms |
| 20 Region + resize/collapse | 2.514 ms | 2.673 ms |
| 50 Region + resize/collapse | 6.058 ms | 6.284 ms |

相对同日只发送滚轮的 WGPU 30 列表基线，100 项总计中位数增加 0.001ms，500 项
增加 0.019ms，1000 项增加 0.255ms；本轮每次迭代同时发送指针移动、主键按下/
释放和滚轮输入，因此事件遍历成本更完整。1000 项列表 p95 为 1.186ms。常规
完整页面中位数低于 0.7ms；全屏 Dialog、Context Menu 和带裁剪、缩放、平移
变换的 ImageViewer 均低于 1.2ms。50 Region 压力场景同时执行连续
resize/collapse，总计 p95 为 6.284ms，仍低于 60Hz 的 16.67ms 帧预算。

`/usr/bin/time -l` 记录全部 19 个场景顺序运行时 `ui-benchmark` 最大 RSS 为
121,274,368 字节（约 115.66MiB），peak memory footprint 为 410,977,120
字节；该峰值包含单一 Renderer 累积的 20/50 Region Canvas 与 GPU pipeline
缓存。真实
`component-gallery` Metal 窗口静置 25 秒后为 0.0% CPU、106,864KiB RSS
（约 104.36MiB）和 sleeping 状态，没有持续重绘。

`component-gallery` Release 二进制为 22,457,328 字节（约 21.42MiB）。
链接映射确认其中 10,087,364 字节（约 9.62MiB）来自为中英文一致字重而注册
的 Noto Sans SC Regular、Medium、SemiBold、Bold 四个字体文件；图片查看器
接收宿主已渲染内容，不启用 Iced 图片编解码 feature，也没有引入第二套 GPU
上下文或 CPU 回读路径。因此当前体积增长来自明确的字体资产与完整组件实例化，
不是图片编解码器或重复 GPU 运行时。

## 尚未完成的基线

以下数据仍需在屏幕解锁、固定窗口尺寸及对应目标平台上采集，不能从当前结果外推：

- timestamp query 的纯 UI GPU Pass 时间；
- 可见窗口中的 100、500、1000 项连续滚动与输入延迟；
- 多面板连续 resize；
- 透明窗口与不透明窗口的差异；
- Windows、Linux 以及不同 macOS 设备；
- 用户可见冷启动、打包产物体积与签名后的应用体积。

后续变更应沿用同一构建配置和测量环境，或明确记录环境差异，不得用 Debug 与 Release、锁屏与可见窗口数据直接互相比较。

## 2026-07-31 模块化与按需构建复测

本轮仍使用 Apple M4 / Metal、900×640 离屏 Renderer、10 次预热与 60 次采样。
改动将 Gallery 页面、Workspace 注册合同和组合渲染按职责拆分；Gallery 的日历
模型与上下文菜单数据改为首次使用时初始化；Workspace 内容解析、主区域边界判断
和几何写入从重复线性扫描改为线性处理。

Cargo 默认最小特性集的 Release `nana_ui.rlib` 为 2,721,864 字节，显式全功能为
16,625,128 字节，减少 83.63%。相同 `transparent-window` 在显式
`full` 全功能构建中为 21,262,736 字节，在仅启用 `bundled-fonts` 时为
21,236,192 字节，减少 26,544 字节。两者差异较小，说明 Release 链接器原本已
移除大部分未引用组件；feature 的主要额外收益是让未选模块与字体不进入编译图。

改动前后各执行一次完整同参数基准的中位数：

| 场景 | 改动前总计 | 改动后总计 | 改动后 p95 |
| --- | ---: | ---: | ---: |
| 100 项列表 | 0.497 ms | 0.518 ms | 1.908 ms |
| 500 项列表 | 0.703 ms | 0.730 ms | 2.258 ms |
| 1000 项列表 | 0.932 ms | 0.895 ms | 1.980 ms |
| 控件页 | 0.843 ms | 0.650 ms | 2.322 ms |
| 表面页 | 0.569 ms | 0.552 ms | 2.222 ms |
| 反馈页 | 0.579 ms | 0.598 ms | 2.232 ms |
| 工作区页 | 0.597 ms | 0.598 ms | 2.136 ms |
| 50 Region + resize/collapse | 7.876 ms | 8.101 ms | 8.973 ms |

列表和常规页面中位数没有一致方向的回退，50 Region 仍低于 60Hz 的 16.67ms
预算。20 Region 场景在本轮连续复测中的中位数从 2.615ms 波动到 4.057ms，
变化几乎全部来自 Metal `submit+wait`（2.521–3.974ms）；同一实现的 50 Region
中位数为 7.828–8.238ms。CPU 的 view/layout/event/draw 阶段仍是微秒级，因此
不把这组 GPU 抖动归因为结构拆分，也不据此声称纯 GPU Pass 得到提升。目标平台
timestamp query 和可见窗口交互仍属于上文未完成基线。
