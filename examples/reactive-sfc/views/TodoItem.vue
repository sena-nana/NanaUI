<script setup lang="rust">
defineProps!(todo: Todo, on_remove: impl Fn() + Send + 'static);
</script>

<template>
  <Row :gap="8">
    <Text>{{ todo.title }}</Text>
    <Button key="remove" @activate="on_remove">删除</Button>
  </Row>
</template>
