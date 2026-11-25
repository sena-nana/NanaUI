# TimelineEditor

`TimelineEditor` 是可编辑的时间底座，给动画、音频或数据编辑用。一个节点里画出时间标尺和播放头、左侧可折叠的轨道树、轨道上的关键点和片段，以及曲线视图里的贝塞尔曲线和切线手柄。内容归应用：应用填 `tracks`，编辑通过事件回报，应用改好数据再写回。它没有模板标签，用 `widget(TimelineEditor::new(轨道))` 放进视图。

> 动画里的 `Timeline`（动作描述）是另一样东西；这个组件叫 `TimelineEditor`，它的数据和事件类型以 `Timeline` 开头。

这个类型在 Cargo feature `timeline` 后面（`components` 包含它）。

## 基本用法

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{
    TimelineEditor, TimelineKey, TimelineKeysMoved, TimelinePlayheadChanged, TimelineTrack,
};

widget(
    TimelineEditor::new([
        TimelineTrack::new("head", "头", []).expanded(true),
        TimelineTrack::new("x", "角度 X", [TimelineKey::new(0.0, 0.0), TimelineKey::new(1.0, 30.0)])
            .depth(1),
        TimelineTrack::new("blink", "眨眼", [TimelineKey::new(0.8, 1.0)]),
    ])
    .fps(30.0)
    .playhead(0.5),
)
.on(|e: &TimelinePlayheadChanged| { /* e.time，e.committed */ })
.on(|e: &TimelineKeysMoved| { /* 松手（e.committed）时把 e.keys 移动 e.dt 秒、e.dv */ })
```

## 标尺与播放头

- 刻度单位是帧（`TimelineUnit::Frames`，按 `.fps()`）或秒。带数字的刻度至少相隔 64 px，取 1 / 2 / 5 × 10ⁿ 中第一个够宽的步长；其间的细刻度取步长的十分之一、五分之一或一半里第一个不小于 6 px 的（帧单位只取整帧）。缩放时刻度自动加密或变稀。
- 在标尺上按下并拖动移动播放头，拖动中发 `TimelinePlayheadChanged { committed: false }`，松手发一次 `committed: true`。开着 `.snap(true)`（默认）时落在整帧上。

## 轨道

- `TimelineTrack::depth(n)` 设嵌套深度：一条轨道属于它前面最近的深度更小的轨道。`.expanded(bool)` 让它成为分支。
- 点分支左边的三角折叠或展开它，被折叠的子轨道不显示，并发 `TimelineTrackToggled { track, expanded }`。
- 轨道和关键点区域一起纵向滚动（`view.scroll`）。

## 关键点

- 点一个关键点选中它，Shift 点选加入或移出选择；在空白处拖出一个框，框里的关键点被选中（Shift 时加入）。选择变化时发 `TimelineSelectionChanged { keys }`，选择保存在 `selection`（按 `TimelineKeyRef { track, key }` 排序）。
- 拖动已选中的关键点移动整个选择。开着吸附时，第一个选中的关键点落在整帧上。拖动中组件自己画出移动后的样子，并发 `TimelineKeysMoved { committed: false }`；松手发一次 `committed: true`，应用这时改数据。
- 删除、全选、适配视图这类按键属于应用的快捷键：应用读 `selection` 删除关键点，调用 `cx.fit_timeline(entity)` 适配视图。

## 片段

片段是轨道上的一段时间：`TimelineClip::new(id, start, length, label)`，用 `TimelineTrack::clips([...])` 放进轨道。`id` 在整条时间轴里唯一，事件按 `id` 指片段。同一轨道上的片段不重叠。

```rust
use nana_ui::runtime::{
    TimelineClip, TimelineClipSelectionChanged, TimelineClipTone, TimelineClipTrimmed,
    TimelineClipsDeleteRequested, TimelineClipsMoved,
};

widget(
    TimelineEditor::new([
        TimelineTrack::new("motion", "动作", []).clips([
            TimelineClip::new("wave", 0.0, 1.2, "挥手"),
            TimelineClip::new("nod", 1.5, 0.5, "点头").resizable(false),
        ]),
        TimelineTrack::new("sound", "音效", [])
            .clips([TimelineClip::new("chime", 0.4, 0.8, "铃声").tone(TimelineClipTone::Warning)]),
    ])
    .selected_clips(selected.clone()),
)
.on(|e: &TimelineClipSelectionChanged| { /* 保存 e.clips，下次传回 .selected_clips */ })
.on(|e: &TimelineClipsMoved| { /* e.committed 时把 e.clips 移动 e.dt 秒；e.track 是换到的轨道 */ })
.on(|e: &TimelineClipTrimmed| { /* e.committed 时把 e.clip 改成 e.start / e.length */ })
.on(|e: &TimelineClipsDeleteRequested| { /* 删除 e.clips */ })
```

- 片段只在关键点视图里画：圆角矩形占满轨道行（上下各留 2 px），底色是色调的淡色，选中时边框加深，名字在片段内单行显示、放不下省略，太窄时不显示。关键点画在片段上面，点到关键点时优先关键点。折叠起来的轨道不画片段。曲线视图忽略片段。
- 色调（`TimelineClipTone`）：`Accent` 用强调色，`Info` / `Success` / `Warning` / `Neutral` 跟随主题的状态色。
- 选择：点片段选中它，Shift 点选加入或移出；点已选中的片段保持整组选择，以便一起拖动；点关键点区域的空白处清空片段选择（Shift 框选时保留）。变化时发 `TimelineClipSelectionChanged { clips }`。选择由应用保存，用 `.selected_clips(ids)` 传回。
- 移动：拖动片段移动所有选中的片段，发 `TimelineClipsMoved { clips, dt, track, committed }`。只选了一个片段时，拖到另一条叶子轨道的行上会换轨道，`track` 是目标轨道在 `tracks` 里的下标；没有换轨道时为 `None`。目标轨道放不下时留在原轨道。
- 修剪：`resizable` 的片段，指针离边缘 6 px 以内（窄片段最多它宽度的三分之一）时按边缘算，悬停时显示修剪把手、光标变成左右调整。拖动起始边改 `start` 并保持结束时间，拖动结束边改 `length`，发 `TimelineClipTrimmed { clip, start, length, committed }`。修剪不会让片段短于 `min_clip_length`（默认 0.05 秒），起始不早于 0。修剪会单独选中这个片段（它已经在选择里时不变）。
- 吸附（`.snap(true)`）：移动或修剪的边缘在 6 px 内吸到其他片段的边缘和播放头；都不在范围内时，修剪的边缘、或移动时按下的片段起点落在整帧上。
- 不重叠：移动或修剪碰到同一轨道（换轨道时是目标轨道）上不动的片段时停在它的边缘；移动不早于 0。
- 拖动中组件自己画出移动或修剪后的样子，发 `committed: false`；松手时有变化才发一次 `committed: true`，应用这时改数据。拖动中不支持 Esc 取消。
- 时间轴获得焦点且有选中的片段时，Delete 或 Backspace 发 `TimelineClipsDeleteRequested { clips }`，由应用删除并更新选择。应用自己的快捷键先收到按键，可以截下来另行处理。
- 读屏值在有片段时附上片段总数和选中数。

## 曲线视图

- `.mode(TimelineMode::Curves)`：每条可见轨道的关键点用三次贝塞尔曲线相连，控制点是关键点的出切线和下一个关键点的入切线（`TimelineKey::out_handle` / `in_handle`，相对关键点的 `[dt, dv]`）。曲线在首末关键点之外保持水平。
- 选中的关键点显示两个切线手柄。拖动手柄发 `TimelineHandleMoved { key, out, handle, broken }`。手柄保持在关键点的同侧（出切线向后、入切线向前）。`broken` 为假时，另一侧手柄转到相反方向、保持原长度，组件在拖动中这样画，应用提交时也这样改；关键点本身 `broken`，或拖动时按住 Alt，两侧手柄各自独立。
- 曲线视图里拖动关键点同时改时间和值（`dv`）。
- `fit()` / `cx.fit_timeline()` 把选中的关键点（没有选中时是全部关键点）放进视图：时间铺满关键点区域，值和切线手柄铺满高度。

## 视图

- 滚轮以指针处的时间为中心缩放；Shift + 滚轮滚动轨道；Ctrl + 滚轮平移时间。中键拖动平移（曲线视图里同时平移值）。
- 视图改变时发 `TimelineViewChanged { view }`。应用可以保存 `view`，下次用 `.view(...)` 恢复。视图重新渲染时，用户缩放和平移过的视图不会被覆盖。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `TimelineEditor::new(轨道)` | `TimelineTrack` | 轨道和关键点 |
| `.fps(f64)` | `f64` | 每秒帧数，默认 30 |
| `.unit(TimelineUnit)` | `Frames` / `Seconds` | 标尺单位 |
| `.mode(TimelineMode)` | `DopeSheet` / `Curves` | 关键点视图或曲线视图 |
| `.view(TimelineView)` | `start`, `pixels_per_second`, `value_min`, `value_max`, `scroll` | 可见窗口 |
| `.playhead(f64)` | 秒 | 播放头 |
| `.snap(bool)` | `bool` | 拖动吸附到整帧（片段还吸附到其他片段边缘和播放头），默认开 |
| `.selected_clips(ids)` | `Arc<str>` | 选中的片段 |
| `.min_clip_length(秒)` | `f64` | 修剪后片段的最短长度，默认 0.05 |
| `.clips_editable(bool)` | `bool` | 默认开。关掉后片段仍能选中、播放头照常拖动，但片段不能移动、修剪，也不发删除请求；用于只读的编辑器 |
| `.header_width(px)` | `f32` | 轨道标题列宽，默认 140 |
| `.label(文字)` | `Arc<str>` | 读屏名称 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `TimelinePlayheadChanged` | `time`, `committed` | 播放头拖动中 / 松手 |
| `TimelineSelectionChanged` | `keys` | 选中的关键点变化 |
| `TimelineKeysMoved` | `keys`, `dt`, `dv`, `committed` | 关键点拖动中 / 松手 |
| `TimelineHandleMoved` | `key`, `out`, `handle`, `broken`, `committed` | 切线手柄拖动中 / 松手 |
| `TimelineClipSelectionChanged` | `clips` | 选中的片段变化 |
| `TimelineClipsMoved` | `clips`, `dt`, `track`, `committed` | 片段拖动中 / 松手 |
| `TimelineClipTrimmed` | `clip`, `start`, `length`, `committed` | 片段边缘拖动中 / 松手 |
| `TimelineClipsDeleteRequested` | `clips` | 聚焦时按 Delete / Backspace |
| `TimelineTrackToggled` | `track`, `expanded` | 分支折叠或展开 |
| `TimelineViewChanged` | `view` | 缩放、平移或滚动 |

## 参见

[PropertyGrid](https://github.com/sena-nana/NanaUI/blob/main/docs/components/property-grid.md) · [总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md)
