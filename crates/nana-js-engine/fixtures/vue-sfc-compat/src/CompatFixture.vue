<script setup lang="ts">
import { onMounted, ref } from "vue";

interface FixturePayload {
  message: string;
  sequence: number;
}

const status = ref("loading");
const byteLength = ref(0);

onMounted(async () => {
  try {
    const fixtureBase = String(
      (globalThis as typeof globalThis & { __NANA_FIXTURE_URL__?: string }).__NANA_FIXTURE_URL__ ??
        "",
    );
    const response = await fetch(`${fixtureBase}/fixture`, {
      headers: { "x-nana-fixture": "sfc" },
    });
    const copy = response.clone();
    const payload = (await response.json()) as FixturePayload;
    byteLength.value = (await copy.arrayBuffer()).byteLength;
    status.value = response.ok ? `${payload.message}:${payload.sequence}` : `http:${response.status}`;
  } catch (error) {
    status.value = error instanceof Error ? error.name : "FetchError";
  }
});
</script>

<template>
  <main class="compat-fixture">
    <h1>Vue SFC compatibility</h1>
    <p data-testid="fetch-status">{{ status }}</p>
    <p data-testid="byte-length">{{ byteLength }}</p>
  </main>
</template>

<style>
.compat-fixture {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px;
}

.compat-fixture h1 {
  font-size: 20px;
  font-weight: 600;
}
</style>
