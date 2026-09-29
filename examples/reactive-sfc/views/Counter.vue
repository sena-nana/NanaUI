<script setup lang="rust">
let count = signal(0u64);
// Nothing writes `step`: the compiler folds it to a constant.
let step = signal(1u64);
let doubled = computed(move || count.get() * 2);
</script>

<template>
  <Row :gap="8">
    <Text key="value">计数 {{ count }}（双倍 {{ doubled }}）</Text>
    <Button key="inc" @activate="count.update(|c| *c += step.get())">加 {{ step }}</Button>
  </Row>
</template>
