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
    let mut joined: String = code.split_whitespace().collect();
    // Existing lowering assertions concern construction, not source metadata.
    while let Some(start) = joined.find(".source_site(") {
        let mut depth = 1;
        let body = start + ".source_site(".len();
        let end = joined[body..]
            .char_indices()
            .find_map(|(index, ch)| {
                match ch {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(body + index + 1)
            })
            .expect("balanced source metadata call");
        joined.replace_range(start..end, "");
    }
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
        code.contains(&squash("let title = ::nana_ui_runtime::view::constant")),
        "an unread signal folds: {code}"
    );
    assert!(
        code.contains(&squash("let label = ::nana_ui_runtime::view::constant")),
        "an unread computed folds: {code}"
    );
    assert!(
        code.contains(&squash("let count = signal(0u32)")),
        "a written signal stays a signal: {code}"
    );
    assert!(
        code.contains("count.dep()"),
        "the written signal is the binding's dependency: {code}"
    );
    assert!(
        out.code.contains("ViewSource") && out.code.contains("SourceLocation::new"),
        "SFC nodes retain their template source location: {code}"
    );
    assert!(!out.code.contains("__NANA_SFC_MARKER__"));
    assert!(out.source_map.contains("nana-sfc-source-map/1"));
    assert!(out.source_map.contains("Title.vue"));
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
    assert!(
        squash(&out.code).contains(&squash("move || describe(n.get())")),
        "an unseen call stays a closure: {}",
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
    assert!(code.contains(&squash("pub fn card(")), "{code}");
    assert!(code.contains(&squash("on_close:")), "{code}");
    assert!(code.contains(&squash("children:")), "{code}");
    assert!(
        code.contains(&squash("open.set(false)")),
        "the event is passed through: {code}"
    );
    assert!(
        out.warnings.iter().any(|w| w.contains("`open`")),
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
    assert!(missing.message.contains(":title"), "{missing}");

    let cycle = compile(&[(
        "Loop.vue",
        "<script setup lang=\"rust\">\nlet a = computed(move || b.get());\nlet b = computed(move || a.get());\n</script><template><Text>{{ a }}</Text></template>",
    )])
    .err().expect("an error");
    assert!(cycle.message.contains("a → b"), "{cycle}");
    assert_eq!(cycle.line, 2);

    let key = compile(&[(
        "K.vue",
        "<template>\n<Column>\n  <Text v-for=\"t in items\">x</Text>\n</Column>\n</template>",
    )])
    .err()
    .expect("an error");
    assert_eq!(key.line, 3, "{key}");
    assert!(key.message.contains("v-for"), "{key}");
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
        out.warnings.iter().any(|w| w.contains("`n`")),
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
    let code = squash(&out.code);
    assert!(
        code.contains(&squash("let items: Const<Vec<u32>>")),
        "{code}"
    );
    assert!(code.contains("view::constant"), "{code}");
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
        code.contains(&squash("column().children((header, children))")),
        "{code}"
    );
    assert!(code.contains("card("), "{code}");

    let unnamed = compile(&[
        card,
        (
            "Page.vue",
            "<template>\n<Card><template><Text>x</Text></template></Card>\n</template>",
        ),
    ])
    .err()
    .expect("an error");
    assert!(unnamed.message.contains("#slot-name"), "{unnamed}");
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
    assert!(code.contains(&squash(".model(on)")), "{code}");
    assert!(code.contains(&squash("progress(100_f64)")), "{code}");

    let typo = compile(&[(
        "Typo.vue",
        "<template>\n<Column>\n  <Button lable=\"x\">保存</Button>\n</Column>\n</template>",
    )])
    .err()
    .expect("an error");
    assert_eq!(typo.line, 3, "{typo}");
    assert!(typo.message.contains("lable"), "{typo}");

    let model = compile(&[(
        "Model.vue",
        "<script setup lang=\"rust\">\nlet n = signal(1u32);\n</script>\n<template>\n<Divider v-model=\"n\" />\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(model.message.contains("v-model"), "{model}");
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
        code.contains(&squash(".on_change(move |_|")),
        "a value event drops the value: {code}"
    );
    assert!(
        code.contains(&squash(".on_activate(move ||")),
        "a unit event stays unit: {code}"
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
        alone.message.contains("v-virtual") && alone.message.contains("v-for"),
        "{alone}"
    );
}

#[test]
fn a_virtual_element_sizes_the_list_it_holds() {
    let out = compile(&[(
        "Sized.vue",
        r#"<script setup lang="rust">
let rows: Signal<Vec<u32>> = signal((0..10_000).collect());
let shown = signal(true);
</script>
<template>
  <Virtual row-height="24" height="400" measured grow v-show="shown">
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
    assert!(code.contains(&squash(".visible(shown)")), "{code}");

    let empty = compile(&[(
        "Empty.vue",
        "<template>\n<Virtual row-height=\"24\"><Text>x</Text></Virtual>\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(empty.message.contains("v-for"), "{empty}");
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
    assert!(code.contains(".transition("), "{code}");
    assert!(code.contains(&squash(".moves(")), "{code}");

    for (template, token) in [
        (
            "<Transition name=\"spin\"><Text v-if=\"true\">x</Text></Transition>",
            "spin",
        ),
        ("<Transition><Text>x</Text></Transition>", "v-if"),
        (
            "<Transition bogus=\"1\"><Text v-if=\"true\">x</Text></Transition>",
            "bogus",
        ),
    ] {
        let error = compile(&[("Bad.vue", &format!("<template>\n{template}\n</template>"))])
            .err()
            .expect("an error");
        assert!(error.message.contains(token), "{template}: {error}");
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
        code.contains("::nana_ui_runtime::view::suspense(move||"),
        "{code}"
    );
    let missing = compile(&[(
        "Bad.vue",
        "<template>\n<Suspense><Text>x</Text></Suspense>\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(missing.message.contains("fallback"), "{missing}");
}

#[test]
fn keep_alive_and_transition_nest_around_a_chain() {
    let out = compile(&[(
        "Tabs.vue",
        r#"<script setup lang="rust">
let tab = signal(0u32);
</script>
<template>
  <Transition name="fade">
    <KeepAlive>
      <Text v-if="tab.get() == 0">一</Text>
      <Text v-else-if="tab.get() == 1">二</Text>
      <Text v-else>三</Text>
    </KeepAlive>
  </Transition>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert_eq!(
        code.matches(&squash(".keep_alive()")).count(),
        2,
        "both levels of the chain keep their branches: {code}"
    );
    assert_eq!(code.matches(".transition(").count(), 2, "{code}");

    let list = compile(&[(
        "List.vue",
        "<script setup lang=\"rust\">\nlet items: Signal<Vec<u32>> = signal(vec![]);\n</script>\n<template>\n<KeepAlive><Text v-for=\"n in items\" :key=\"*n\">x</Text></KeepAlive>\n</template>",
    )])
    .err()
    .expect("an error");
    assert!(
        list.message.contains("KeepAlive") && list.message.contains("v-if"),
        "{list}"
    );
}

#[test]
fn teleport_moves_its_children_under_the_node_it_names() {
    let out = compile(&[(
        "Dialog.vue",
        r#"<script setup lang="rust">
let layer = node_ref();
</script>
<template>
  <Column>
    <Column ref="layer" />
    <Teleport :to="layer"><Text>浮层</Text></Teleport>
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash("::nana_ui_runtime::view::teleport(layer")),
        "{code}"
    );
}

#[test]
fn an_error_boundary_takes_its_fallback_as_a_closure() {
    let out = compile(&[(
        "Guard.vue",
        r#"<template>
  <ErrorBoundary :fallback='|errors| text(errors.join("；"))'>
    <Text>内容</Text>
  </ErrorBoundary>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash("::nana_ui_runtime::view::error_boundary(|errors|")),
        "{code}"
    );
}

#[test]
fn controls_a_screen_reader_cannot_name_are_warnings() {
    let out = compile(&[(
        "Form.vue",
        r#"<script setup lang="rust">
let name = signal(String::new());
</script>
<template>
  <Column>
    <TextInput v-model="name" />
    <TextInput label="名字" v-model="name" />
    <Button></Button>
    <Button>保存</Button>
    <Slider min="0" max="1" step="0.1" />
  </Column>
</template>"#,
    )])
    .unwrap();
    let at = |line: &str, token: &str| {
        assert!(
            out.warnings
                .iter()
                .any(|warning| warning.contains(line) && warning.contains(token)),
            "{line} {token}: {:?}",
            out.warnings
        );
    };
    at("Form.vue: 6:", "label");
    at("Form.vue: 8:", "Button");
    at("Form.vue: 10:", "label");
    assert_eq!(out.warnings.len(), 3, "{:?}", out.warnings);
}

/// A control named by a caption (`labelled-by` naming the caption's `ref`,
/// as `aria-labelledby` names an id) has a name: no warning, and the ref is
/// passed as written.
#[test]
fn a_control_named_by_a_caption_is_not_a_warning() {
    let out = compile(&[(
        "Field.vue",
        r#"<script setup lang="rust">
let caption = node_ref();
let on = signal(false);
</script>
<template>
  <Column>
    <Text ref="caption">静音</Text>
    <Switch labelled-by="caption" v-model="on" />
    <Slider min="0" max="1" step="0.1" :labelled-by="caption" />
  </Column>
</template>"#,
    )])
    .unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let code = squash(&out.code);
    assert_eq!(
        code.matches(&squash(".labelled_by(caption)")).count(),
        2,
        "{code}"
    );
}

#[test]
fn hot_mode_moves_static_text_into_a_table_and_hashes_the_rest() {
    let view = r#"<script setup lang="rust">
let count = signal(0u32);
</script>
<template>
  <Column>
    <Text>标题</Text>
    <Button @activate="count.update(|c| *c += 1)">加一</Button>
    <Text>计数 {{ count }}</Text>
  </Column>
</template>"#;
    let hot = Compiler::new("::nana_ui_runtime").hot(true);
    let shape =
        |file: &str, text: &str| hot.hot_views(&[(file.into(), text.into())]).unwrap()[0].clone();
    let first = shape("views/Page.vue", view);
    assert_eq!(first.literals, ["标题", "加一"]);
    let moved = shape("/elsewhere/views/Page.vue", view);
    assert_eq!(moved.shape, first.shape, "the directory does not matter");
    let retitled = shape("views/Page.vue", &view.replace("标题", "新标题"));
    assert_eq!(retitled.shape, first.shape);
    assert_eq!(retitled.literals, ["新标题", "加一"]);
    let rewired = shape("views/Page.vue", &view.replace("*c += 1", "*c += 2"));
    assert_ne!(rewired.shape, first.shape, "code changed");
    // Text that moves what follows it to other lines and columns is still
    // only text.
    let longer = shape(
        "views/Page.vue",
        &view.replace("<Text>标题</Text>", "<Text>一个长得多的标题</Text>\n"),
    );
    assert_eq!(longer.shape, first.shape, "positions are not shape");
}

#[test]
fn a_style_block_compiles_into_patches_matched_per_element() {
    let out = compile(&[(
        "Card.vue",
        r#"<script setup lang="rust">
let done = signal(false);
</script>
<template>
  <Column class="card" class:done="done">
    <Text class="title">标题</Text>
    <Button class="ghost" @activate="done.set(true)">完成</Button>
  </Column>
</template>
<style scoped>
.card { padding: 12px; opacity: 1; transition: opacity 150ms linear; }
.card.done { opacity: 0.5; }
.title { opacity: 0.9 !important; }
.title { opacity: 0.2; }
.card:hover { opacity: 0.7; }
.card > .title { padding: 2px; }
.card { frobnicate: 3; }
</style>"#,
    )])
    .unwrap();
    let at = |line: &str, token: &str| {
        assert!(
            out.warnings
                .iter()
                .any(|warning| warning.contains(line) && warning.contains(token)),
            "{line} {token}: {:?}",
            out.warnings
        );
    };
    at("Card.vue: 15:", ":hover");
    at("Card.vue: 16:", ".card > .title");
    at("Card.vue: 17:", "frobnicate");
    at("Card.vue: 7:", "ghost");
}

/// The class rules of an `@container` block compile with the block's query
/// as data; a query the engine does not evaluate is a warning at its line,
/// and the container properties are fields like any other.
#[test]
fn container_rules_in_a_style_block_compile_with_their_query() {
    let out = compile(&[(
        "Panel.vue",
        r#"<template>
  <Column class="card">
    <Row class="row" />
  </Column>
</template>
<style scoped>
.card { container-type: inline-size; container-name: card; }
.row { height: 20px; }
@container card (max-width: 300px) { .row { height: 40px; } }
@container (aspect-ratio > 1) { .row { opacity: 0.5; } }
</style>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    assert!(
        code.contains(&squash(
            r#"::nana_ui_runtime::view::SheetQuery::new(::core::option::Option::Some("card"),
            ::nana_ui_runtime::ResponsiveAxis::Width"#
        )),
        "{code}"
    );
    assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
    assert!(
        out.warnings[0].contains("Panel.vue: 10:") && out.warnings[0].contains("aspect-ratio"),
        "{:?}",
        out.warnings
    );
}

/// `<T>` is `t(…)`: `id` names the message and every other attribute is an
/// argument. The arguments are one binding, classified as `{{ … }}` text is:
/// a lone signal is read with `.get()` and declared, a folded one is written
/// once, a call nobody sees into is tracked at run time.
#[test]
fn localized_text_is_one_binding_over_its_arguments() {
    let out = compile(&[(
        "Files.vue",
        r#"<script setup lang="rust">
defineProps!(owner: String);
let count = signal(1u64);
let fixed = signal(7u64);
</script>
<template>
  <Column>
    <T id="title" decorative />
    <T id="files" :count="count" />
    <T id="seven" :count="fixed" :who="owner" label="x" />
    <T :id="choose(count.get())" />
    <Button @activate="count.update(|c| *c += 1)">加一</Button>
  </Column>
</template>"#,
    )])
    .unwrap();
    let code = squash(&out.code);
    let has = |needle: &str| assert!(code.contains(&squash(needle)), "{needle}: {code}");
    has(r#"view::t(::nana_ui_runtime::LocalizedText::new("title")).decorative(true)"#);
    has(r#"view::t(::nana_ui_runtime::view::__checked("Files.vue:9:27", [count.dep()]"#);
    has(r#"::nana_ui_runtime::LocalizedText::new("files").arg("count", count.get())"#);
    has(
        r#"view::t(::nana_ui_runtime::view::Fixed(::nana_ui_runtime::LocalizedText::new("seven")
        .arg("count", fixed.get())
        .arg("who", ::nana_ui_runtime::view::__arg(&owner))
        .arg("label", "x")))"#,
    );
    has(r#"view::t(move || ::nana_ui_runtime::LocalizedText::new(&(choose(count.get()))))"#);
    assert!(
        out.report.contains("<T> 消息"),
        "each message is a row of the report: {}",
        out.report
    );
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);

    for (template, token) in [
        ("<T :count=\"1\" />", "`id=\"…\"`"),
        ("<T id=\"\" />", "not empty"),
        ("<T id=\"a\">文字</T>", "no children"),
        ("<T id=\"a\" :n=\"1\" :n=\"2\" />", "twice"),
        ("<T id=\"a\" n />", "needs a value"),
    ] {
        let error = compile(&[("Bad.vue", &format!("<template>\n{template}\n</template>"))])
            .err()
            .expect("an error");
        assert!(error.message.contains(token), "{template}: {error}");
        assert_eq!(error.line, 2, "{template}: {error}");
    }
}

/// Static text is a prop already in hot mode; a child of `<T>` is still an
/// error, not taken for its message.
#[test]
fn a_child_of_t_is_an_error_in_hot_mode_too() {
    let error = Compiler::new("::nana_ui_runtime")
        .hot(true)
        .compile(&[(
            "Hot.vue".into(),
            "<template>\n<T id=\"a\">文字</T>\n</template>".into(),
        )])
        .err()
        .expect("an error");
    assert!(error.message.contains("no children"), "{error}");
}

/// A built-in tag builds the built-in. A view named like one is still a
/// function Rust can call, but using its tag is an error, not a silent swap
/// for the built-in.
#[test]
fn a_tag_naming_a_built_in_never_reaches_a_view_of_that_name() {
    let own = (
        "T.vue",
        "<script setup lang=\"rust\">\ndefineProps!(id: &'static str);\n</script>\n\
         <template><Text>{{ id }}</Text></template>",
    );
    let alone = compile(&[own]).unwrap();
    assert!(
        squash(&alone.code).contains("pubfnt(id:&'staticstr)"),
        "{}",
        alone.code
    );
    for compiler in [
        Compiler::new("::nana_ui_runtime"),
        Compiler::new("::nana_ui_runtime").hot(true),
    ] {
        let used = compiler
            .compile(&[
                (own.0.into(), own.1.into()),
                (
                    "Page.vue".into(),
                    "<template>\n<Column>\n  <T id=\"files\" />\n</Column>\n</template>".into(),
                ),
            ])
            .err()
            .expect("an error");
        assert_eq!((used.file.as_str(), used.line), ("Page.vue", 3), "{used}");
        assert!(
            used.message.contains("`<T>` is built in") && used.message.contains("T.vue"),
            "{used}"
        );
    }
}

/// `locale` on an element makes it a scope, a tag checked at build time or
/// any binding; a block is not an element and refuses it; a view takes it as
/// the prop it declares.
#[test]
fn locale_scopes_an_element_and_is_refused_on_a_block() {
    let out = compile(&[
        (
            "Panel.vue",
            r#"<script setup lang="rust">
defineProps!(locale: &'static str);
</script>
<template><Column :locale="locale"><T id="title" /></Column></template>"#,
        ),
        (
            "Page.vue",
            r#"<script setup lang="rust">
let chosen = signal(None);
let rtl = signal(false);
let hebrew = Locale::parse("he");
</script>
<template>
  <Column locale="ar">
    <Row :locale="chosen" />
    <Text :locale="if rtl.get() { hebrew.clone() } else { None }">x</Text>
    <Panel locale="zh-CN" />
    <Button @activate="{ chosen.set(None); rtl.set(true) }">切换</Button>
  </Column>
</template>"#,
        ),
    ])
    .unwrap();
    let code = squash(&out.code);
    let has = |needle: &str| assert!(code.contains(&squash(needle)), "{needle}: {code}");
    has(r#".locale("ar")"#);
    has(".locale(chosen)");
    has(".locale(locale)");
    has("[rtl.dep()]");
    has(r#"panel(::core::convert::Into::into("zh-CN"))"#);

    for (template, token) in [
        ("<Column locale=\"a r\" />", "language tag"),
        ("<Column locale=\"\" />", "language tag"),
        ("<Column locale />", "needs a language tag"),
        (
            "<Block locale=\"ar\"><Text v-if=\"true\">x</Text></Block>",
            "takes no `locale`",
        ),
        (
            "<Transition><KeepAlive locale=\"ar\"><Text v-if=\"true\">x</Text></KeepAlive></Transition>",
            "takes no `locale`",
        ),
        (
            "<Virtual row-height=\"24\" locale=\"ar\"><Text v-for=\"n in rows\" :key=\"*n\">x</Text></Virtual>",
            "takes no `locale`",
        ),
        (
            "<Teleport :to=\"layer\" locale=\"ar\"><Text>x</Text></Teleport>",
            "takes no `locale`",
        ),
    ] {
        let error = compile(&[("Bad.vue", &format!("<template>\n{template}\n</template>"))])
            .err()
            .expect("an error");
        assert!(error.message.contains(token), "{template}: {error}");
        assert_eq!(error.line, 2, "{template}: {error}");
    }
}
