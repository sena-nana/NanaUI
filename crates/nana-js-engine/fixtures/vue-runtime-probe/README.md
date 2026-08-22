# Vue runtime-core Phase 2 probe

Build the shared IIFE consumed by V8 tests:

```bash
npm install
./node_modules/.bin/esbuild src/probe.js \
  --bundle --format=iife --platform=neutral --target=es2020 \
  --outfile=dist/vue-runtime-probe.iife.js \
  --alias:@vue/runtime-core=./node_modules/@vue/runtime-core/dist/runtime-core.esm-bundler.js \
  --alias:@vue/reactivity=./node_modules/@vue/reactivity/dist/reactivity.esm-bundler.js \
  --alias:@vue/shared=./node_modules/@vue/shared/dist/shared.esm-bundler.js \
  --define:process.env.NODE_ENV=\"production\"
```

`dist/vue-runtime-probe.iife.js` is checked in so Rust tests do not require npm.
