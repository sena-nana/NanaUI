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
    <TodoList key="todos" />
  </Widget>
</template>
