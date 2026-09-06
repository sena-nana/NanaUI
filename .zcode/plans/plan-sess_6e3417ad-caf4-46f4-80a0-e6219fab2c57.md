# 补全三处 HTML-AAM landmark 偏差

范围：全部在 Vue 树语义层（`crates/nana-ui-vue/src/tree.rs`），不动 Runtime 合同。`<search>` role 与 `<video>` 已确认无需改动。

## 1. `accessibility_role` 判定重构（行为保持）

`crates/nana-ui-vue/src/tree.rs:4478` 现在的顺序是：显式 role → `kind.is_layout()` 时 tag landmark → kind 兜底。改为语义化顺序：

1. 显式 `role`（不变）；
2. kind 兜底角色；**若结果为 `Generic`**（即元素未被 hints 改用途成具体控件），再查 `landmark_role_from_tag`；
3. 否则保持控件角色（`<nav class="nana-tabs">` → TabList、`<footer class="nana-search">` → ComboBox 等现行为全部不变——layout kind 的兜底本来就是 Generic，所以现测试全过）。

同时把 `has_accessible_name: bool` 参数升级为 `accessible_name: Option<&str>`。

## 2. 可访问名判定放宽（section/form）

在 `sync_semantic_styles`（tree.rs:1563 附近）为每个 widget 计算可访问名，优先级：

1. `props.label`（现状）；
2. `aria-labelledby`：attrs 里已有原始字符串；解析时按需从 Vue document `nodes` 的 attrs 建一次 `id → 节点` 映射（仅当存在 labelledby 时），引用目标的 accessible name 递归解析（带 visited 集防环），找不到目标则忽略。若 `id` 属性未持久化，在 `apply_prop` 的持久化臂（bridge.rs:1248）补 `"id"`；
3. 仅当 `element_tag` 为 `section`/`form` 时：`self.text_content(NodeHandle(widget.id))`（tree.rs:2404）trim 非空作为 name-from-content（与 Chrome 的 region 命名一致；不放开到其它 tag，避免改变现有 a11y 输出面）。

计算出的名字用于 section/form 的 Region/Form 升级门槛，**并同时写入 `AccessibilityState.label`**（若原本为空）——否则会产出无名 region，比 Generic 更糟。

## 3. header/footer 仅顶层映射 Banner/ContentInfo

按 ARIA in HTML：`<header>`/`<footer>` 若为 `article|aside|main|nav|section` 的后代则不出 landmark。在 tree.rs 新增小助手，沿 `parent_element`（tree.rs:1743）+ `element_tag`（tree.rs:2390）向上走到 `mount_root`/`html_root`，命中上述祖先 tag 则 `landmark_role_from_tag` 对 header/footer 返回 None（落到 Generic）。`landmark_role_from_tag`（tree.rs:4575）签名加 `is_top_level` 参数，只门 header/footer 两个分支。

## 4. 测试（renderer.rs 现有 harness 扩展）

复用 `html_landmark_tags_project_landmark_roles`（renderer.rs:2261）的 `createElement`/`insert`/`sync_semantic_styles` 模式，新增：

- `<section>` 由内部文本命名（插入 `<h2>` 或文本子节点，不设 aria-label）→ Region，且 a11y 快照带出该 label；
- `<section aria-labelledby="t">` + `<h2 id="t">Usage</h2>` → Region，名来自引用目标；
- `<header>`/`<footer>` 插在 `<main>`/`<article>` 内 → Generic；顶层（mountRoot 直系）仍为 Banner/ContentInfo（现有用例继续覆盖）；
- 回归：`<nav class="nana-tabs">` → TabList（非 Navigation），`<nav role="tablist">` → TabList。

现有测试（renderer.rs:2261、accessibility.rs:782、framework.rs 组件排除回归）必须全过。

## 5. 文档

更新 `docs/vue.md:35`：移除"header/footer 无条件映射"的已知偏差记录，改为新合同描述。

## 6. 验证

`cargo test -p nana-ui-vue --lib`（landmark/role/bridge 回归）+ `cargo test -p nana-ui --lib accessibility`（accesskit 投射）。纯逻辑改动，无 GPU/窗口面，不需 GPU 证据；改完跑全量 `cargo test --workspace -q` 确认无意外回归。

## 风险与已确认点

- revision 失效：祖先增删、label/text mutation 均走 `MessageBridge::bump()`，新增的判定输入天然被 `synced_semantic_revision` 守卫失效，无需额外处理。
- 文本滞后：`text_content` 读的是已 commit 的 Vue 文本节点（`runtime_text`），不依赖同帧未 commit 的 mutation。
- 不改变任何控件映射、tag 解析、HostTexture 合同。