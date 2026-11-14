# nana-ui-devtools

[English](README.md) · 简体中文

NanaUI 的无头会话与离屏快照，给开发工具和测试用。

它能在不开窗的情况下运行一个 Rust 视图文档，注入指针、键盘和滚动输入，导出无障碍树，截图并做像素比对。输入走 Runtime 的规范输入路由，没有旁路。会话通过 JSON-lines 协议对外，`nana-runtime-agent` 二进制可以驱动内置夹具；应用要驱动自己的文档，在 dev 二进制里调用 `agent::cli::runtime_main`，或为自己的会话实现 `AgentSession`。

截图需要 CPU 回读，这条路径只存在于本 crate 的 `offscreen` 模块，产品呈现不经过这里。热重载的开发入口在 [`nana-ui-dev`](../nana-ui-dev)。

## Features

不开 feature 时本 crate 是空的。各 feature 逐层包含：

- `offscreen`：离屏绘制与 CPU 回读，读写 PNG。
- `runtime-agent`：在 `offscreen` 之上加 JSON 协议、`RuntimeAgentSession` 和命令行。
- `headless-host`：在 `runtime-agent` 之上加 `HeadlessHost`，按原生宿主的钩子顺序运行整个 `RuntimeProgram`（并提供可控时钟与稳态推进、目标定位、命中测试、场景探针、视图检查、直通 PNG 导出与像素比对）。
- `reactive-trace`：视图检查结果附带节点最近一次变化的原因。

## 示例

```bash
cargo run -p nana-ui-devtools --features runtime-agent --bin nana-runtime-agent -- \
  --fixture counter --a11y
```

## 测试

```bash
cargo test -p nana-ui-devtools --features runtime-agent
```

没有可用 GPU 适配器时，像素类测试直接返回。其余检查见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [开发期热重载合同](../../docs/reference/hot-reload.md) 和 [证据清单](../../docs/learn/evidence-checklist.md)。
