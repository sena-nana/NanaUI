# Vue SFC compatibility fixture

This is the smallest supported L1 source pipeline: Vue 3 SFC + TypeScript + CSS
is built through the Nana application entry and custom renderer. The checked-in
IIFE is loaded directly by V8 tests; it is not a general bundle
loader and does not include `@vue/runtime-dom`.

```sh
npm ci
npm run build
```
