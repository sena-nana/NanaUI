use super::*;

fn compile(files: &[(&str, &str)]) -> Result<Output, Error> {
    let sources: Vec<(String, String)> = files
        .iter()
        .map(|(name, text)| (name.to_string(), text.to_string()))
        .collect();
    Compiler::new("::nana_ui_runtime").compile(&sources)
}

/// Code without whitespace or trailing commas, so assertions do not depend
/// on the formatter.
fn squash(code: &str) -> String {
    let joined: String = code.split_whitespace().collect();
    joined.replace(",)", ")")
}

#[test]
fn never_written_signals_fold_and_their_bindings_are_written_once() {
    let out = compile(&[(
        "Title.vue",
        r#"<script setup lang="rust">
let title = signal(String::from("标题"));
let count = signal(0u32);
let label = computed(move || format!("{} 项", title.get()));
</script>
<template>
  <Column>
    <Text>{{ title }}</Text>
    <Text>{{ label }}</Text>
    <Text>计数 {{ count }}</Text>
    <Button @activate="count.update(|c| *c += 1)" :disabled="count.get() > 3">加一</Button>
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash(
            "let title = ::nana_ui_runtime::view::constant(String::from(\"标题\"))"
        )),
        "{code}"
    );
    assert!(
        code.contains(&squash("let label = ::nana_ui_runtime::view::constant((move || format!(\"{} 项\", title.get()))())")),
        "{code}"
    );
    assert!(
        code.contains(&squash("let count = signal(0u32)")),
        "count is written: {code}"
    );
    assert!(
        code.contains(&squash("Fixed(::std::format!(\"{}\", title))")),
        "{code}"
    );
    assert!(
        code.contains(&squash("__checked(\"Title.vue:10:17\", [count.dep()]")),
        "{code}"
    );
    assert!(
        code.contains(&squash(
            "__checked(\"Title.vue:11:62\", [count.dep()], move || count.get() > 3)"
        )),
        "{code}"
    );
    assert!(
        out.report
            .contains("| `title` | signal | 2 | 0 | 0 | 折叠为常量 |"),
        "{}",
        out.report
    );
    assert!(
        out.report.contains("| `count` | signal |"),
        "{}",
        out.report
    );
    assert!(out.report.contains("静态依赖（count）"), "{}", out.report);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn a_signal_handed_out_or_modeled_is_kept() {
    let out = compile(&[(
        "Keep.vue",
        r#"<script setup lang="rust">
let name = signal(String::new());
let shared = signal(1u8);
</script>
<template>
  <Column>
    <TextInput v-model="name" />
    <Other :value="shared" />
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash("let name = signal(String::new())")),
        "{code}"
    );
    assert!(code.contains(&squash("let shared = signal(1u8)")), "{code}");
    assert!(
        code.contains(&squash("other(shared)")),
        "a Rust component gets its arguments in order: {code}"
    );
}

#[test]
fn unseen_calls_stay_dynamic() {
    let out = compile(&[(
        "Dyn.vue",
        r#"<script setup lang="rust">
let n = signal(1u32);
let bump = move || n.update(|v| *v += 1);
</script>
<template>
  <Button @activate="bump" :label="describe(n.get())">x</Button>
</template>"#,
    )])
    .unwrap();
    assert!(out.report.contains("| 动态 |"), "{}", out.report);
    assert!(
        squash(&out.code).contains(&squash("button(move || describe(n.get()))")),
        "{}",
        out.code
    );
}

#[test]
fn known_views_take_arguments_by_prop_name_events_and_children() {
    let out = compile(&[
        (
            "Card.vue",
            r#"<script setup lang="rust">
defineProps!(title: String, on_close: impl Fn() + Send + Clone + 'static, children: impl IntoView);
</script>
<template><Column><Text>{{ title }}</Text>{{ "x" }}</Column></template>"#,
        ),
        (
            "Page.vue",
            r#"<script setup lang="rust">
let open = signal(true);
</script>
<template>
  <Card @close="open.set(false)" title="设置"><Text>内容</Text></Card>
</template>"#,
        ),
    ])
    .unwrap();
    let code = squash(&out.code);
    assert!(code.contains(&squash("pub fn card(title: String, on_close: impl Fn() + Send + Clone + 'static, children: impl IntoView)")), "{code}");
    assert!(code.contains(&squash("card(::core::convert::Into::into(\"设置\"), move || { open.set(false); }, ::nana_ui_runtime::view::text(\"内容\"))")), "{code}");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("`open` is written but nothing reads it")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn mistakes_are_reported_with_their_position() {
    let missing = compile(&[
        ("Card.vue", "<script setup lang=\"rust\">defineProps!(title: String);</script><template><Text>x</Text></template>"),
        ("Page.vue", "<template>\n  <Card />\n</template>"),
    ])
    .err().expect("an error");
    assert_eq!((missing.file.as_str(), missing.line), ("Page.vue", 2));
    assert!(missing.message.contains("missing `:title`"), "{missing}");

    let cycle = compile(&[(
        "Loop.vue",
        "<script setup lang=\"rust\">\nlet a = computed(move || b.get());\nlet b = computed(move || a.get());\n</script><template><Text>{{ a }}</Text></template>",
    )])
    .err().expect("an error");
    assert!(
        cycle
            .message
            .contains("computeds read each other in a loop"),
        "{cycle}"
    );
    assert_eq!(cycle.line, 2);

    let key = compile(&[(
        "K.vue",
        "<template>\n<Column>\n  <Text v-for=\"t in items\">x</Text>\n</Column>\n</template>",
    )])
    .err()
    .expect("an error");
    assert_eq!(key.line, 3, "{key}");
    assert!(key.message.contains("`v-for` needs `key"), "{key}");
}

#[test]
fn a_watcher_writing_what_it_reads_is_a_warning() {
    let out = compile(&[(
        "W.vue",
        r#"<script setup lang="rust">
let n = signal(0u32);
watch_effect(move || { let v = n.get(); n.set(v + 1); });
</script>
<template><Text>{{ n }}</Text></template>"#,
    )])
    .unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("reads and writes `n`")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn a_folded_signal_keeps_its_annotation_as_const() {
    let out = compile(&[(
        "Typed.vue",
        r#"<script setup lang="rust">
let items: Signal<Vec<u32>> = signal(Vec::new());
</script>
<template><Text>{{ items.with(|v| v.len()) }}</Text></template>"#,
    )])
    .unwrap();
    assert!(
        squash(&out.code).contains(&squash(
            "let items: Const<Vec<u32>> = ::nana_ui_runtime::view::constant(Vec::new())"
        )),
        "{}",
        out.code
    );
}

#[test]
fn named_slots_fill_view_arguments() {
    let card = (
        "Card.vue",
        r#"<script setup lang="rust">
defineProps!(header: impl IntoView, children: impl IntoView);
</script>
<template><Column><slot name="header"/><slot/></Column></template>"#,
    );
    let out = compile(&[
        card,
        (
            "Page.vue",
            "<template><Card><template #header><Text>标题</Text></template><template #default><Text>内容</Text></template></Card></template>",
        ),
    ])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash("column(0.0_f32, (header, children))")),
        "{code}"
    );
    assert!(
        code.contains(&squash(
            "card(::nana_ui_runtime::view::text(\"标题\"), ::nana_ui_runtime::view::text(\"内容\"))"
        )),
        "{code}"
    );

    let unnamed = compile(&[
        card,
        (
            "Page.vue",
            "<template>\n<Card><template><Text>x</Text></template></Card>\n</template>",
        ),
    ])
    .err()
    .expect("an error");
    assert!(unnamed.message.contains("needs `#slot-name`"), "{unnamed}");
    assert_eq!(unnamed.line, 2);
}

#[test]
fn the_control_table_types_attributes_and_rejects_unknown_ones() {
    let out = compile(&[(
        "Form.vue",
        r#"<script setup lang="rust">
let on = signal(false);
let amount = signal(3.0f64);
let choice: Signal<Option<Arc<str>>> = signal(None);
</script>
<template>
  <Column>
    <Switch v-model="on">通知</Switch>
    <NumberInput v-model="amount" placeholder="数量" />
    <Select v-model="choice" />
    <Progress max="100" :value="amount.get()" />
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash("switch(\"通知\").model(on)")),
        "{code}"
    );
    assert!(code.contains(&squash("progress(100_f64)")), "{code}");

    let typo = compile(&[(
        "Typo.vue",
        "<template>\n<Column>\n  <Button lable=\"x\">保存</Button>\n</Column>\n</template>",
    )])
    .err()
    .expect("an error");
    assert_eq!(typo.line, 3, "{typo}");
    assert!(
        typo.message.contains("`<Button>` has no attribute `lable`"),
        "{typo}"
    );

    let model = compile(&[(
        "Model.vue",
        "<script setup lang=\"rust\">\nlet n = signal(1u32);\n</script>\n<template>\n<Divider v-model=\"n\" />\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(
        model.message.contains("`<Divider>` has no `v-model`"),
        "{model}"
    );
}

#[test]
fn a_statement_handler_of_a_value_event_ignores_the_value() {
    let out = compile(&[(
        "Events.vue",
        r#"<script setup lang="rust">
let seen = signal(0u32);
</script>
<template>
  <Column>
    <Switch @change="seen.update(|n| *n += 1)">开关</Switch>
    <Button @activate="seen.set(0)">清零</Button>
    <Text>{{ seen }}</Text>
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash(
            ".on_change(move |_| { seen.update(|n| *n += 1); })"
        )),
        "{code}"
    );
    assert!(
        code.contains(&squash(".on_activate(move || { seen.set(0); })")),
        "{code}"
    );
}

#[test]
fn v_virtual_builds_only_the_rows_in_view() {
    let out = compile(&[(
        "Long.vue",
        r#"<script setup lang="rust">
let rows: Signal<Vec<u32>> = signal((0..10_000).collect());
</script>
<template>
  <Text v-for="n in rows" :key="*n" v-virtual.measured="24">行 {{ n }}</Text>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(code.contains(&squash("each_virtual(rows,")), "{code}");
    assert!(code.contains(&squash("24_f32,")), "{code}");
    assert!(code.contains(&squash(".measured()")), "{code}");

    let alone = compile(&[(
        "Alone.vue",
        "<template>\n<Text v-virtual=\"24\">x</Text>\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(
        alone.message.contains("`v-virtual` goes with `v-for`"),
        "{alone}"
    );
}

#[test]
fn a_virtual_element_sizes_the_list_it_holds() {
    let out = compile(&[(
        "Sized.vue",
        r#"<script setup lang="rust">
let rows: Signal<Vec<u32>> = signal((0..10_000).collect());
</script>
<template>
  <Virtual row-height="24" height="400" measured grow>
    <Text v-for="n in rows" :key="*n">行 {{ n }}</Text>
  </Virtual>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(code.contains(&squash("24_f32,")), "{code}");
    assert!(code.contains(&squash(".measured()")), "{code}");
    assert!(code.contains(&squash(".grow()")), "{code}");
    assert!(code.contains(&squash(".height(400_f32)")), "{code}");

    let empty = compile(&[(
        "Empty.vue",
        "<template>\n<Virtual row-height=\"24\"><Text>x</Text></Virtual>\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(empty.message.contains("needs `v-for`"), "{empty}");
}

#[test]
fn bindings_through_a_store_are_tracked_at_run_time() {
    let out = compile(&[(
        "Board.vue",
        r#"<script setup lang="rust">
let board = store(Board { tasks: Vec::new(), title: String::new() });
</script>
<template>
  <Column>
    <Text>{{ board.title().get() }}</Text>
    <Text>{{ board.tasks().len() }} 项</Text>
    <Text :value="board.title()" />
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(!code.contains("constant("), "a store never folds: {code}");
    assert!(
        !code.contains("Fixed("),
        "no store read is written once: {code}"
    );
    assert!(!code.contains("__checked("), "{code}");
    assert!(out.report.contains("| `board` | store |"), "{}", out.report);
}

#[test]
fn a_transition_element_animates_the_chain_or_list_it_holds() {
    let out = compile(&[(
        "Panel.vue",
        r#"<script setup lang="rust">
let open = signal(true);
let items: Signal<Vec<u32>> = signal(vec![1, 2]);
</script>
<template>
  <Column>
    <Transition name="slide-up" duration="200">
      <Text v-if="open.get()">开</Text>
      <Text v-else>关</Text>
    </Transition>
    <TransitionGroup duration="120" move="200">
      <Text v-for="n in items" :key="*n">{{ n }}</Text>
    </TransitionGroup>
    <Button @activate="open.update(|o| *o = !*o)">切换</Button>
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash(
            ".transition(::nana_ui_runtime::view::Transition::slide(0.0,12.0,"
        )),
        "{code}"
    );
    assert!(code.contains(&squash("200_f64")), "{code}");
    assert!(code.contains(&squash(".moves(")), "{code}");

    for (template, message) in [
        (
            "<Transition name=\"spin\"><Text v-if=\"true\">x</Text></Transition>",
            "no transition named `spin`",
        ),
        (
            "<Transition><Text>x</Text></Transition>",
            "holds a `v-if` chain or one `v-for` element",
        ),
        (
            "<Transition bogus=\"1\"><Text v-if=\"true\">x</Text></Transition>",
            "has no attribute `bogus`",
        ),
    ] {
        let error = compile(&[("Bad.vue", &format!("<template>\n{template}\n</template>"))])
            .err()
            .expect("an error");
        assert!(error.message.contains(message), "{template}: {error}");
    }
}

#[test]
fn suspense_takes_its_fallback_from_a_named_template() {
    let out = compile(&[(
        "Profile.vue",
        r#"<script setup lang="rust">
let id = signal(1u32);
</script>
<template>
  <Suspense>
    <template #fallback><Text>加载中</Text></template>
    <Text>{{ id }}</Text>
  </Suspense>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash(
            "::nana_ui_runtime::view::suspense(move || ::nana_ui_runtime::view::text(\"加载中\"), move ||"
        )),
        "{code}"
    );
    let missing = compile(&[(
        "Bad.vue",
        "<template>\n<Suspense><Text>x</Text></Suspense>\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(missing.message.contains("needs a fallback"), "{missing}");
}
