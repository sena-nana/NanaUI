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
