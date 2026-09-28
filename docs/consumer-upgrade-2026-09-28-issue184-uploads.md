# Issue #184：帧上传与帧槽重写

b187fb48b 的 upload arena 让每次上传发生两遍（先写进一个没人读的 arena，再照常 queue 写目标），arena 在连续出帧时永不回绕、不断翻倍；`begin_frame` 槽满时忙等，调用方持有全部未提交槽时会卡死；`GpuContext::write_texture` 每个区域单独开一帧提交。说明见 [GPU](gpu.md) 的“帧上传”与“帧槽”。

## 删除的 API

| 删除 | 替代 |
| --- | --- |
| `GpuDeviceState::reserve_upload` / `set_upload_capacity` / `begin_frame`、`UploadReservation` | 无；帧上传由 `FrameContext` 管理 |
| `__framework::stage_upload` / `upload_backing_size` | `__framework::frame_uploads(&frame)` 返回的 `FrameUploadHandle::write_buffer` / `write_texture` / `write_texture_rows` |
| `GpuDeviceState::realize_texture`、`GpuPolicyStats::realization_hits` / `realization_misses` | 无；HostTexture 已是本设备纹理。`gpu.realization_*` 指标 id 保留不复用 |

## 行为变化

- `GpuContext::write_texture` 与 `GpuContext::write_buffer` 不再立即提交：写入在本设备的下一次提交（任何线程上的 `FrameContext::submit`，或 `GpuContext::flush_uploads()`）之前落地，与 queue 写入的“在下一次 submit 之前”语义一致。经 `wgpu-interop` 自己提交原始 command buffer、并读取这些上传结果的调用方，先调用 `flush_uploads()`。
- 被丢弃的 `FrameContext` 里的写入在下一次提交时落地，而不是丢失。
- `begin_frame()` 槽满时阻塞等待最早已提交的帧，不再忙等；全部槽都被未提交录制占用时不占槽直接开始，并报告 `gpu.frame_slots_exhausted`（之前是死锁）。`frame_slot_stalls` 现在只计这种情况。
- 同一 URL 图片被 quad 背景与 HostTexture mask 同时使用时只抓取、解码、上传一次：`SceneWgpuPainter` 持有唯一的 `UrlTextureCache`，两条管线共用（此前各有一个，各做一遍）。
- 新计数：`GpuPolicyStats::{upload_writes, upload_copies, upload_flushes, upload_ring_allocations, upload_ring_waits, frame_slot_waits}`，对应 `gpu.upload_*`、`gpu.frame_slot_waits` 指标（id 20–25）。

## 测量

同机交替 A/B，每场景 5 轮取 instructions retired 最小值（负载约 6.7）：host-textures-64 −20.2%，ui-dense-2k −5.9%，ui −5.5%，text-retained −3.4%，text-paint-color −0.1%；`frame_slot_stalls` 由每次运行数万到十几万次降为 0。

## 剩余

- 文本 glyph 块与绘制顺序保留文本自己的 ring（encoder 内按位置的 copy 是 #224 的合同）。
