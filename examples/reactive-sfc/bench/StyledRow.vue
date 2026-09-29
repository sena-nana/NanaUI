<script setup lang="rust">
defineProps!(index: usize, selected: Signal<usize>);
</script>

<template>
  <Row class="item" class:active="selected.get() == index" :gap="8">
    <Text class="title">第 {{ index }} 行</Text>
    <Button>打开</Button>
  </Row>
</template>

<style scoped>
.item { padding: 4px 8px; }
.item.active { opacity: 0.6; }
.title { flex-grow: 1; }
</style>
