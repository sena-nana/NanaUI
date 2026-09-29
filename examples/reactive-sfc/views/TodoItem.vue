<script setup lang="rust">
defineProps!(todo: Todo, on_remove: impl Fn() + Send + 'static);
</script>

<template>
  <Row class="item" :gap="8">
    <Text class="title">{{ todo.title }}</Text>
    <Button key="remove" @activate="on_remove">删除</Button>
  </Row>
</template>

<style scoped>
.item { padding: 4px 8px; }
.title { flex-grow: 1; }
</style>
