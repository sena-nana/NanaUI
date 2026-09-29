<script setup lang="rust">
use nana_ui::runtime::{LengthSpec, Stack};

// The top inset clears the title bar the window draws over its content.
let page = Stack::fill_column(16.0)
    .padding(16.0)
    .with_layout(|layout| layout.padding_top = Some(LengthSpec::Px(48.0)));
</script>

<template>
  <Widget :of="page">
    <Counter key="counter" />
    <Section key="todo-section">
      <template #header><Text>待办</Text></template>
      <TodoList key="todos" />
    </Section>
  </Widget>
</template>
