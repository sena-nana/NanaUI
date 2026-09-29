<script setup lang="rust">
defineProps!(index: usize);
// Nothing writes `label`: the compiler folds it to a constant.
let label = signal(String::from("打开"));
</script>

<template>
  <Row :gap="8">
    <Text>{{ label }} 第 {{ index }} 行</Text>
    <Button>{{ label }}</Button>
  </Row>
</template>
