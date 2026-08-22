<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { NanaAppShell } from "@nanaui/nanavue-components/NanaAppShell";

const input = ref<any>(null);

onMounted(async () => {
  await nextTick();
  input.value?.focus?.();
});
</script>

<template>
  <NanaAppShell title="Chrome probe">
    <div class="chrome-probe-body" data-slot="body">
      <p>Type in the field below.</p>
      <label>Text <input ref="input" data-agent-id="chrome-probe-input" /></label>
    </div>
  </NanaAppShell>
</template>

<style>
.chrome-probe-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 20px;
}
</style>
