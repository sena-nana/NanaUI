# Vue SFC compatibility fixture

This is the smallest supported L1 source pipeline: Vue 3 SFC + TypeScript + CSS
is built through the Nana application entry and custom renderer. The checked-in
IIFE is loaded directly by V8 tests; it is not a general bundle
loader and does not include `@vue/runtime-dom`.

```sh
npm ci
npm run build
```


Virtual navigation acceptance fixture (one million logical rows, five visible rows,
and one retained business key):

```sh
node build-virtual.mjs
```

The bundle is written to `target/virtual-navigation/app.js` at the repository root.
Load it with `nana-agent-session --js target/virtual-navigation/app.js --stdio`.
Use agent IDs `jump`, `row-500000`, and `release` to verify navigation, hit testing,
and release of `row-2`. This is a functional fixture, not a refresh-rate benchmark.

After building the Agent, run `python verify-virtual.py` to assert the native
operation sequence and write raw replies, a report, and a screenshot.


Frozen table acceptance:

```sh
node build-virtual.mjs table
python verify-frozen-table.py
```

Uses one million logical rows and ten thousand logical columns without allocating
that data matrix. Verifies frozen corner, header, column, body hit targets and
release of retained row/column keys. Outputs `target/virtual-table/` at repo root.


Automatic focus retention (no explicit retained keys):

```sh
node build-virtual.mjs activity
python verify-activity.py
node --test virtual-window.test.mjs
```

The native Agent checks focus, stable input identity, offscreen text entry and
release after blur. Component tests additionally exercise IME event lifetime and
frozen table retention. These do not test OS IME candidate windows or 120Hz.
