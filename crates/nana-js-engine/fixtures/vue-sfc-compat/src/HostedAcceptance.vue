<script setup lang="ts">
import { h, nextTick, onBeforeUnmount, onMounted, ref } from "vue";

declare const Nana: any;

const props = defineProps<{ hybrid: boolean; autoWindows?: boolean }>();
const dialogOpen = ref(false);
const name = ref("NanaUI");
const nameInput = ref<any>(null);
const status = ref("starting");
const score = ref(0);
const canvas = ref<any>(null);
const gpuCanvas = ref<any>(null);
const nativeProbe = ref<any>(null);
const nativeVisible = ref(props.hybrid);
const probeStatus = ref("native:pending");
const pointerSample = ref<any>(null);
let frame = 0;
let auxiliaryWindow: any = null;
let inputProbe = false;
try {
  inputProbe = Boolean((globalThis as any).__nanaHost?.call?.("acceptanceInputProbe", []));
} catch {}

const AuxiliaryAcceptance = {
  render() {
    return h("main", {
      style: "display:flex;flex-direction:column;gap:12px;padding:24px;background:rgba(15,23,42,.86);color:white",
    }, [
      h("h2", null, "Vue auxiliary window"),
      h("p", null, "Same V8, isolated document, transparent modal surface."),
      h("button", { onClick: () => auxiliaryWindow?.close() }, "Close auxiliary"),
    ]);
  },
};

async function openAuxiliaryWindow() {
  if (auxiliaryWindow) return auxiliaryWindow;
  auxiliaryWindow = await Nana.windows.create({
    title: "NanaUI Vue auxiliary acceptance",
    width: 480,
    height: 300,
    transparent: true,
    modal: true,
    parentId: 0,
  });
  auxiliaryWindow.mount(AuxiliaryAcceptance);
  auxiliaryWindow.closed.then(() => { auxiliaryWindow = null; });
  return auxiliaryWindow;
}

function drawGame() {
  const target = canvas.value;
  const context = target?.getContext?.("2d");
  if (!context) return;
  const tick = () => {
    const x = (score.value * 9 + 12) % Math.max(24, target.width - 24);
    context.clearRect(0, 0, target.width, target.height);
    context.fillStyle = "#111827";
    context.fillRect(0, 0, target.width, target.height);
    context.fillStyle = "#60a5fa";
    context.beginPath();
    context.arc(x, target.height / 2, 10, 0, Math.PI * 2);
    context.fill();
    frame = requestAnimationFrame(tick);
  };
  tick();
}

async function drawGpu() {
  try {
    const gpu = (navigator as any).gpu;
    const adapter = await gpu?.requestAdapter();
    const device = await adapter?.requestDevice();
    const context = gpuCanvas.value?.getContext?.("webgpu");
    if (!device || !context) throw new Error("WebGPU unavailable");
    context.configure({
      device,
      format: gpu.getPreferredCanvasFormat(),
      alphaMode: "premultiplied",
    });
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({ colorAttachments: [{
      view: context.getCurrentTexture().createView(),
      loadOp: "clear",
      clearValue: { r: 0.08, g: 0.18, b: 0.32, a: 1 },
      storeOp: "store",
    }] });
    pass.end();
    device.queue.submit([encoder.finish()]);
    await device.queue.onSubmittedWorkDone();
    status.value = props.hybrid ? "Vue + Canvas + WebGPU + Iced" : "Vue + Canvas + WebGPU";
  } catch (error) {
    status.value = error instanceof Error ? error.message : "WebGPU error";
  }
}

async function pingNative() {
  if (!nativeProbe.value) return;
  const result = await Nana.components.call(nativeProbe.value, "ping", { score: score.value });
  probeStatus.value = `command:${result.score}:${result.calls}`;
}

function onNativeActivated(event: any) {
  probeStatus.value = `event:${event.score}`;
}

function onCanvasPointer(event: any) {
  const target = event.currentTarget;
  if (event.type === "pointerdown") target?.setPointerCapture?.(event.pointerId);
  if (event.type === "pointerup" || event.type === "pointercancel") {
    target?.releasePointerCapture?.(event.pointerId);
  }
  const width = Math.max(1, Number(target?.clientWidth || target?.width || 1));
  score.value = Math.max(0, Math.min(100, Math.round((Number(event.offsetX) / width) * 100)));
  pointerSample.value = {
    type: event.pointerType,
    pressure: event.pressure,
    tangentialPressure: event.tangentialPressure,
    tiltX: event.tiltX,
    tiltY: event.tiltY,
    twist: event.twist,
    captured: Boolean(target?.hasPointerCapture?.(event.pointerId)),
  };
  event.preventDefault();
}

async function setScore(value: number) {
  score.value = value;
  await nextTick();
}

async function setNativeVisible(value: boolean) {
  nativeVisible.value = value;
  await nextTick();
}

onMounted(async () => {
  await nextTick();
  if (inputProbe) nameInput.value?.focus?.();
  drawGame();
  if (props.hybrid) {
    await setScore(7);
    await pingNative();
  }
  requestAnimationFrame(() => void drawGpu());
  if (props.autoWindows) void openAuxiliaryWindow();
  (globalThis as any).__nanaHostedAcceptanceControl = {
    pingNative,
    setScore,
    setNativeVisible,
    openAuxiliaryWindow,
    state: () => ({
      score: score.value,
      probeStatus: probeStatus.value,
      pointerSample: pointerSample.value,
    }),
  };
});

onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  auxiliaryWindow?.close();
  delete (globalThis as any).__nanaHostedAcceptanceControl;
});
</script>

<template>
  <main class="acceptance-shell">
    <header class="acceptance-header">
      <div>
        <h1>NanaUI Vue hosted acceptance</h1>
        <p>{{ status }}</p>
      </div>
      <button @click="dialogOpen = true">Open dialog</button>
    </header>

    <section class="acceptance-grid">
      <article class="acceptance-card">
        <label>Name <input ref="nameInput" :value="name" @input="name = $event.target.value" /></label>
        <p>Hello, {{ name }}.</p>
        <button @click="score++">Score {{ score }}</button>
        <img
          alt="inline acceptance asset"
          src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='96' height='48'%3E%3Crect width='96' height='48' rx='12' fill='%2360a5fa'/%3E%3Ccircle cx='28' cy='24' r='12' fill='white'/%3E%3C/svg%3E"
        />
      </article>

      <article class="acceptance-card opacity-group">
        <h2>Canvas mini game</h2>
        <canvas
          ref="canvas"
          width="320"
          height="160"
          @pointerdown="onCanvasPointer"
          @pointermove="onCanvasPointer"
          @pointerup="onCanvasPointer"
          @pointercancel="onCanvasPointer"
        />
      </article>

      <article class="acceptance-card">
        <h2>WebGPU surface</h2>
        <canvas ref="gpuCanvas" width="320" height="160" />
      </article>

      <article v-if="hybrid" class="acceptance-card">
        <h2>Registered Iced component</h2>
        <nana-acceptance-probe
          v-if="nativeVisible"
          ref="nativeProbe"
          :label="name"
          :score="score"
          @activated="onNativeActivated"
        >
          <span>Vue slot content: {{ score }}</span>
        </nana-acceptance-probe>
        <p>{{ probeStatus }}</p>
        <button @click="pingNative">Call native command</button>
      </article>
    </section>

    <div v-if="dialogOpen" class="acceptance-modal" role="dialog" aria-modal="true">
      <section class="acceptance-dialog">
        <h2>Vue dialog</h2>
        <p>This overlay, its input, and both GPU surfaces share one composition tree.</p>
        <button @click="dialogOpen = false">Close</button>
      </section>
    </div>
  </main>
</template>

<style>
.acceptance-shell { display: flex; flex-direction: column; gap: 16px; padding: 20px; }
.acceptance-header { display: flex; flex-direction: row; justify-content: space-between; align-items: center; }
.acceptance-grid { display: flex; flex-direction: row; flex-wrap: wrap; gap: 14px; }
.acceptance-card { display: flex; flex-direction: column; gap: 8px; width: 340px; padding: 12px; border-radius: 12px; background: #f3f4f6; }
.opacity-group { opacity: 0.78; }
.acceptance-modal { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(15, 23, 42, 0.45); }
.acceptance-dialog { display: flex; flex-direction: column; gap: 12px; width: 360px; padding: 20px; border-radius: 14px; background: white; }
canvas { width: 320px; height: 160px; }
img { width: 96px; height: 48px; }
</style>
