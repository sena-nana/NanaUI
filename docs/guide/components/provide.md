# 依赖提供

`provide(value)` 把一个值挂在当前作用域上。下层作用域，包括之后才建出来的行和分支，用 `use_context::<T>()` 沿父链读取，最近的提供者优先。同一个作用域里再 `provide` 同一个类型，会换掉上一个。`T` 必须是 `Clone + 'static`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{use_context, IntoView, provide, signal};

fn page() -> impl IntoView {
    let user = signal(String::from("你"));
    provide(user);
    view! {
        <Column gap=8>
            {greeting()}
        </Column>
    }
}

fn greeting() -> impl IntoView {
    let user = use_context::<nana_ui::runtime::view::Signal<String>>().expect("page provides user");
    view! {
        <Text>"你好，{user}"</Text>
    }
}
```

```rust rust
use nana_ui::runtime::view::{use_context, IntoView, column, provide, signal, text};

fn page() -> impl IntoView {
    let user = signal(String::from("你"));
    provide(user);
    column().gap(8).children(greeting())
}

fn greeting() -> impl IntoView {
    let user = use_context::<nana_ui::runtime::view::Signal<String>>().expect("page provides user");
    text!("你好，{user}")
}
```

:::

只能在构建视图时读。事件处理器运行时不在任何作用域里，`use_context` 得到 `None`。

::: warning
在作用域之外调用 `provide` 什么也不做。事件处理里调用 `use_context` 也读不到。把信号放进 `provide`，子视图拿到的仍是那一个 `Signal`。
:::

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/props">
    <p class="next-step-link">属性</p>
    <p class="next-step-caption">把值传进一个视图函数。</p>
  </a>
  <a class="next-step" href="/guide/essentials/view-functions">
    <p class="next-step-link">组合视图</p>
    <p class="next-step-caption">一块界面是一个返回 IntoView 的函数。</p>
  </a>
</div>
