<script setup lang="ts">
import { ref } from "vue";

const label = ref("socket:idle");
let socket: WebSocket;
const events: string[] = [];
const states: number[] = [];
let closed: { code: number; reason: string; wasClean: boolean } | null = null;
let denied = false;
const listenerMessages: string[] = [];
(globalThis as any).__nanaSocketProbe = {
  start(url: string, deniedUrl: string) {
    try { new WebSocket(deniedUrl); } catch { denied = true; }
    socket = new WebSocket(url);
    socket.addEventListener("message", (event) => listenerMessages.push(event.data));
    socket.onopen = () => {
      events.push("open");
      states.push(socket.readyState);
      label.value = "socket:open";
      socket.send("vue-loopback");
    };
    socket.onmessage = (event) => {
      events.push("message");
      states.push(socket.readyState);
      label.value = `socket:${event.data}`;
    };
    socket.onclose = (event) => {
      events.push("close");
      states.push(socket.readyState);
      closed = { code: event.code, reason: event.reason, wasClean: event.wasClean };
      label.value = `socket:closed:${event.code}:${event.reason}`;
    };
    socket.onerror = () => { events.push("error"); };
  },
  probe() {
    return { events: [...events], states: [...states], readyState: socket.readyState, closed, denied, listenerMessages: [...listenerMessages] };
  },
};
</script>

<template><p>{{ label }}</p></template>
