# 无障碍

读屏要能叫出控件的名字。输入框的名字来自 `label`，占位文字不算。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <TextInput label="姓名" placeholder="怎么称呼" v-model={name} />
}
```

```rust rust
use nana_ui::runtime::view::text_input;

text_input().label("姓名").placeholder("怎么称呼").model(name)
```

:::

编译模板时，读屏叫不出名字的控件会得到一条警告，编译仍然成功。两条规则：`Button`、`Checkbox`、`Switch`、`ListItem` 没有文字；`TextInput`、`TextArea`、`NumberInput`、`Slider`、`Progress` 没写 `label`。`.vue` 的警告经 `cargo:warning` 带行列。`view!` 在稳定版上以「使用了已弃用常量」的形式出现在宏调用处，说明写在弃用提示里。

## 行标签

设置行自己建标签。控件没有名字时，读屏读这行的标签。装配时用 `MutationQueue::set_labelled_by` 把控件和标签节点关联起来。关联放在 `UiWorld` 里，控件重新投影自己的状态不会把它冲掉。任一端销毁，关联一起去掉。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::settings_row;

settings_row("静音").control(view! {
    <Switch></Switch>
})
```

```rust rust
use nana_ui::runtime::view::{settings_row, switch};

settings_row("静音").control(switch(""))
```

:::

控件自己有非空名字时用它自己的，例如 `switch("静音")`。空名字（`switch("")`、滑块、下拉选择）才用行标签。行标签改了，控件的无障碍节点跟着重新投影。`Select` 被这样命名之后，无障碍名字不是当前显示的选项；选项是它的值。没人给它起名时，仍按显示的文字命名。

`settings_row` 没有模板标签。分组里哪一行是首行、末行，由你按数据算好再绑定。

## 隐藏的还在不在树上

`UiWorld::project_accessibility` 保留连接可见控件所需的结构祖先。`visibility: hidden` 的容器输出中性的 Generic：它自己的标签、值、描述、原来的角色和交互状态都不对外提供。显式 `visibility: visible` 的孩子继续有完整的父子链和语义。隐藏的叶子，以及不生成布局盒的子树，不进入投影。重新可见之后，恢复的是当前业务状态里的语义。

启用 `accesskit-tree` 的嵌入式宿主用 `AccessTreeProjector` 消费完整树或 `AccessibilityDelta`。隐藏的结构容器是没有点击、没有聚焦操作的 GenericContainer，孩子的操作和焦点还在。`AccessTreeProjector::new` 只建立保留状态。第一次要完整树时调用 `full_update()`。

静态 `Text` 的可访问内容映射到 AccessKit 的 `Label.value`，于是 UIA 的 Name 里有这段文字。普通控件的 label 和 value 仍然分开。编辑控件的名称和输入值不混在一起。

## 窗口根和发布时机

hosted 适配器另有一个稳定的 Window 无障碍根。挂载、替换或清空文档，以及焦点进入普通控件，都不替换这个根的身份。Runtime 节点仍保留自己的角色和稳定 id。没有控件聚焦时，原生焦点回到窗口根。

变化在布局收敛后暂存，在成功呈现之后、应用的 `presented` 和窗口绑定回调之前发布。还没呈现的一批变化留着增量。攒了多批时，恢复后要一份当前文档的完整快照，避免把中间删掉又挂上的节点拼错。空闲重试不会盖掉已有变化。各窗口的队列分开，关掉一扇窗口不会改主窗口的队列。

新建的主窗口和辅助窗口先只有 Window 根。首次成功呈现之后才发布内容。挂载前调用 `flush`，也不会把还没呈现的控件送进原生树。冷启动的第一批局部增量会补一次当前文档的快照。

投影合同在 [应用 API](../../reference/application-api.md#隐藏层级中的无障碍语义)，帧的顺序在 [怎么工作](../../reference/how-it-works.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/fetch">
    <p class="next-step-link">Fetch</p>
    <p class="next-step-caption">页面请求和远程图片共用的白名单。</p>
  </a>
  <a class="next-step" href="/guide/components/props">
    <p class="next-step-link">属性</p>
    <p class="next-step-caption">label 这样的字段怎样绑定。</p>
  </a>
  <a class="next-step" href="/guide/scaling/performance">
    <p class="next-step-link">性能</p>
    <p class="next-step-caption">无障碍变化跟在呈现之后发布。</p>
  </a>
</div>
