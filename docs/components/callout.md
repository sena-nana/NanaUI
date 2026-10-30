# callout

`callout(icon, tone, text)` 是按 `StatusTone` 着色的提示条：一枚图标、一段文字，可选一个行尾动作。它是 `view::marks` 里的视图，不是注册控件。

## 基本用法

```rust
use nana_ui::runtime::view::callout;
use nana_ui_core::{Icon, StatusTone};

callout(Icon::AlertTriangle, StatusTone::Warning, "模型还没有优化，首次加载会慢一些。")
```

- 图标由调用方给，颜色取色调的角色（`tone_role(tone)`）。
- `text` 可以绑定；为空时整条隐藏。`.visible(cond)` 与「文字为空时隐藏」同时生效。
- 默认带一层浅色调底色和自己的内边距，文字从顶端排起，长文字换行。

## 两种强度

| 写法 | 外观 | 用在 |
| --- | --- | --- |
| `callout(..)` | 浅色调底色，自带内边距，正文字号 | 需要注意但不打断操作：配置不完整、功能受限 |
| `callout(..).quiet()` | 没有底色，不带水平内边距，字小一号、偏淡，和图标居中 | 已在内容线上的块里的一行说明 |

`.quiet()` 不带水平内边距，因为它放进的块已经有内容线（见[版式与组合](../guide/composition.md#内容线)）。把它放在没有内边距的容器里，它会贴边。

## 行尾动作

`.trailing(view)` 在行尾放一个跳转链接或按钮：行尾不收缩，文字先换行。一行提示配较高的按钮时，写 `.css(css! { align-items: center; })` 让它们居中。

## 在状态层级里的位置

行内说明（`.quiet()`）→ 提示条（`callout`）→ 通知（`Toast`）→ 对话框（`ConfirmDialog`）。同一件事只用一层；操作结果走通知，不要再加提示条。空列表用 `EmptyState`。

## 接口

| 接口 | 说明 |
| --- | --- |
| `callout(icon, tone, text)` | 提示条 |
| `Callout::quiet()` | 无底色的一行说明 |
| `Callout::trailing(view)` | 行尾动作 |
| `.class` / `.css` / `.visible` | 作用在提示条的行上 |

## 参见

[版式与组合](../guide/composition.md) · [Toast](toast.md) · [EmptyState](empty-state.md) · [StatusBadge](status-badge.md)
