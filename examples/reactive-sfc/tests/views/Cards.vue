<script setup lang="rust">
defineProps!(narrow: Signal<bool>);
</script>

<template>
  <Column class="card" class:narrow="narrow.get()">
    <Row key="row" class="row" />
  </Column>
</template>

<style scoped>
.card { container-type: inline-size; container-name: card; width: 400px; height: 300px; }
.card.narrow { width: 280px; }
.row { height: 20px; background: #ff0000; }
@container card (max-width: 300px) { .row { background: #0000ff; height: 40px; } }
</style>
