<script setup lang="rust">
defineProps!(title: Signal<String>, on_remove: impl Fn() + Send + 'static);
</script>

<template>
  <Row :gap="8">
    <Text>{{ title }}</Text>
    <Button @activate="on_remove">删除</Button>
  </Row>
</template>
