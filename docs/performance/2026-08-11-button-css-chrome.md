# Button CSS chrome（border / padding / radius / bg）

日期：2026-08-11  
对照：Lilia Home `.overview-actions__btn` / `__btn--primary`（`LiliaGithub/src/styles/page.css`）  
MDN：[box model](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_box_model) · [border](https://developer.mozilla.org/en-US/docs/Web/CSS/border)

交叉引用：

| 文档 | 关系 |
|------|------|
| [`compatibility-roadmap.md`](../compatibility-roadmap.md) **A-06** | 阶段 Todo（已勾选 done） |
| [`css-layout-parity.md`](../css-layout-parity.md) | 宣称闭合表 / 已支持矩阵中的 Button chrome 行 |
| [`vue-nana-renderer-system.md`](../vue-nana-renderer-system.md) §布局映射 | pad/surface → Button 内层消费说明 |

## 缺口表（修复前 → 后）

| CSS 子集 | 解析 → LayoutStyle | 修复前 iced 消费 | 缺口根因 | 修复后 |
|----------|-------------------|------------------|----------|--------|
| `padding` / `padding-*` | ✓ | 外层 container + 内层 `ControlSize.padding_x` | **双层 padding**；`padding:0` 仍被 Medium 10px 覆盖 | Button/IconButton 消费显式 padding；外层 `consume.padding` |
| `margin` | ✓ | 外层 container | 已 OK | 仍外层 |
| `gap`（icon+text） | ✓ | 硬编码 `spacing(6)` | 解析了但不读 | `layout.gap_or(6)` |
| `width` / `height` px | ✓ | 外层 Fixed；内层 ControlSize 高 | 外层有、内层可能不一致（22px sort） | 内层 `height`/`width` 覆盖；外层保留 flex/min |
| `min-width` / `min-height` | ✓ | 外层 floor | 已 OK（如 primary 72） | 仍外层 |
| `border` / `border-width` / `border-color` | ✓ | 外层 surface | 内层 `button_style` 忽略（Subtle 才 1px） | `ButtonPaintOverride` |
| `border-radius` | ✓ | 外层 radius；内层固定 `radius_sm=6` | **圆角画在错层**；Lilia `--radius-sm≈12` 不生效 | 控件 Active 用 CSS radius |
| `background` / `background-color` | ✓ | 外层 surface | 内层 ButtonKind 再画一层 | override Active bg |
| `color` | ✓ | Text 路径读；Button **忽略** | toolbar muted 色丢失 | override + icon/label tint |
| `font-weight` / `font-size` | ✓ | Button::label 固定 Medium | primary `font-weight:700` 丢 | 有 CSS 时自建 text content |
| `ButtonKind`（Semantics） | prop/class | 内层主题 | **未改**：无业务 class 特判发明样式 | 仍 prop/`nana-btn--*` Semantics |

## 改动

- `nana-ui`：`ButtonPaintOverride` + `button_style_overridden`；`Button`/`IconButton` 支持 `padding`/`height`/`width`/`paint`
- `nana-ui-vue` `iced_app`：`button_view*` 从 `LayoutStyle` 应用 chrome；`apply_widget_box_model(..., consume)` 对 Button/Chip 跳过已消费 pad/paint
- 测：`button_layout_chrome_*` / `button_without_css_padding_*`

## 验证

```bash
cargo test -p nana-ui-vue --features iced-view --lib
# 335 passed
cargo test -p nana-ui --lib control_sizes
```

未跑 Android；未 commit。

## 宣称

- **done（A-06）**：Button/IconButton（及 Chip consume 路径）消费上述 CSS chrome 子集；无 CSS 时仍 `ControlSize`/`ButtonKind`。
- **仍非目标**：业务 class 特判样式；Hover/Pressed 走 kind（非全态 CSS paint）；非 Button 通用节点仍外层 surface。