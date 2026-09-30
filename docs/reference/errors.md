# 诊断码

写界面时会撞上的 fault。框架侧的登记、指标和禁止在帧路径上打印，见 [诊断与应用路径](diagnostics.md)。

| 代码 | 什么时候 | 怎么改 |
| --- | --- | --- |
| `runtime.reactive.disposed_access` | 作用域回收之后又读写了其中的信号。同时 panic。消息里有信号是在哪里创建的 | 列表行和条件分支里创建的信号，不要留到作用域外面用。清理和稍后才回来的任务用 `try_update`：信号还在才改，已经不在就返回 `false`。见 [挂载与卸载](../guide/essentials/lifecycle.md) |
| `runtime.reactive.did_not_settle` | 副作用互相触发，超过 64 轮还不停。剩下的队列被丢掉 | 不要在 `watch_effect` 里写它自己读的那个信号。见 [侦听](../guide/essentials/watch.md) |
| `runtime.reactive.static_deps_mismatch` | debug 构建里，`.vue` 编译器声明过静态依赖的绑定，读到了声明之外的信号。消息里有模板位置。release 不核对 | 让模板表达式读到的信号都出现在编译器看得见的依赖里。见 [.vue 方言](../guide/scaling/sfc.md) |
| `runtime.view.error_unhandled` | 视图返回 `Err`，上面没有 `error_boundary` 接住 | 用错误边界显示回退。没有边界就不要让视图返回 `Err`。见 [声明式视图](reactive-view.md) |

同一组里还有计数指标，不是 fault：`runtime.reactive.flushes`、`runtime.reactive.signal_writes`、`runtime.reactive.effects_run`、`runtime.reactive.nodes_patched`、`runtime.reactive.commits`，以及直方图 `runtime.reactive.flush`。高频数据走指标，不走每帧事件。
