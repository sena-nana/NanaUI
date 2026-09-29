<script setup lang="rust">
defineProps!(header: impl IntoView, children: impl IntoView);
</script>

<template>
  <Column :gap="8">
    <slot name="header" />
    <slot />
  </Column>
</template>
