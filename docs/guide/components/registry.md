# 扩展控件

多数界面用现有控件拼出来就够了。只有它会参与排版、点击和绘制，而且现有控件表达不了时，才加一种新的。内置控件和你的控件走同一张 `ComponentRegistry`。

## 登记

稳定身份是 `ComponentTypeId`，例如 `nana.button`、`app.preview-card`。下面这份把一张卡片登记进去，`AppContext::install` 在建树之前调用。

```rust
use nana_ui::runtime::{
    AppContext, ComponentView, ExtensionRegistrar, FrameworkError, MutationQueue, NodeKind,
    RegisterableComponent, StableNodeId, TextContent, UiExtension, UiWorld,
};

#[derive(Clone, PartialEq)]
struct PreviewCard {
    title: String,
}

impl ComponentView for PreviewCard {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "preview-card".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.title.as_str()) {
            mutations.set_text(id, TextContent::new(self.title.clone()));
        }
    }
}

impl RegisterableComponent for PreviewCard {
    const TYPE_ID: &'static str = "app.preview-card";
    const TAGS: &'static [&'static str] = &["preview-card"];
}

struct PreviewCards;

impl UiExtension for PreviewCards {
    fn name(&self) -> &'static str {
        "preview-cards"
    }

    fn install(&self, registrar: &mut ExtensionRegistrar) -> Result<(), FrameworkError> {
        registrar.register_component::<PreviewCard>()
    }
}

fn install(cx: &mut AppContext) -> Result<(), FrameworkError> {
    cx.install(&PreviewCards)
}
```

`ComponentView` 要求 `Clone + PartialEq + Send`。`project` 读到的每个字段都要参与相等比较：相等的写入会被跳过。`project` 如果读共享的内部可变状态，或者往别的组件的节点上打补丁，加上 `const ALWAYS_REPROJECT: bool = true;`。

`RegisterableComponent` 给出类型 id 与标签元数据。`install` 里调用 `ExtensionRegistrar::register_component`。额外标签用 `register_component_alias`。指针和键盘激活用 `register_activation`，不要去改 `activate_node`。

Rust 用 `create_component` 按类型建节点。tag 解析同一张表，等于 `ComponentTypeId` 去掉 `nana.` 前缀：`nana.preview-card` 对应 `preview-card`。和 HTML 同语义就用原生标签（`button`、`table` / `tr` / `td`）。语义不同就换名：`search-dropdown`，不是 HTML `<search>`。没登记、也不是已知 HTML 的 tag 会报错，不会当成布局盒。

## 事件名单和 GPU 槽

监听哪些名字，权威在 `UiWorld` 的 `EventListeners` 上，不在控件外面另存一份。投影时写：

```rust
mutations.set_event_listener(id, "activate", true);
```

GPU 槽是节点上的 `CustomRenderNode`，同样在这次提交里写上。`renderer` 和 `resource` 是不透明键，字段里不放 GPU 后端对象：

```rust
mutations.set_custom_render(
    id,
    Some(nana_ui::runtime::CustomRenderNode::new(renderer, resource, revision)),
);
```

普通实时画面仍然用 `GpuTextureView` 和同一条字符串 slot。那条路径的槽也落在 Runtime 的 `CustomRenderNode` 上。见 [实时画面](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md)。

## 限制

发布用的 `dist` 档是 `panic = "abort"`。工厂里的 panic 会直接终止进程，不要把它当成可恢复的错误。也不支持把控件做成动态加载的 dylib。

## 未用到的内置控件自动摇树裁剪

从 Rust 类型、`view!` 或 `.vue` 建树时，内置控件不再携带按标签构造的绑定器；`AppContext::new()` 默认只在首次创建某类控件时按需安装其行为。未创建的内置控件不会被链接进最终二进制（`AppContext::typed()` 与 `RuntimeDocument::typed(document)` 仅保留为兼容别名）。

在 `[profile.dist]`（`opt-level = "z"`、fat LTO、`panic = "abort"`、strip）与预编译着色器下，托管窗口 `examples/reactive-sfc` 的发布体积为 `5.56 MiB`（`5,835,088 B`）。剩余体积主要在 GPU、文字和内置字体。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/props">
    <p class="next-step-link">属性</p>
    <p class="next-step-caption">常量、信号和闭包三种值。</p>
  </a>
  <a class="next-step" href="/guide/components/events">
    <p class="next-step-link">事件</p>
    <p class="next-step-caption">控件发出什么，视图上怎么接。</p>
  </a>
  <a class="next-step" href="/reference/components">
    <p class="next-step-link">控件参考</p>
    <p class="next-step-caption">登记合同和已有控件的语义。</p>
  </a>
</div>
