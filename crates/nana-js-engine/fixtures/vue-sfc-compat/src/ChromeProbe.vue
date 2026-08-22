<script setup lang="ts">
import { NanaAppShell } from "@nanaui/nanavue-components";
</script>

<template>
  <NanaAppShell title="Chrome probe">
    <div class="chrome-probe-body">
      <p>Type in the field below.</p>
      <label>Text <input data-agent-id="chrome-probe-input" /></label>
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
