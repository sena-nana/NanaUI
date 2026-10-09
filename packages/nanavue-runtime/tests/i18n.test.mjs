/**
 * Behavior: `Nana.i18n` is the application's catalog and locale. It calls
 * host-global APIs (never through a window), sends a catalog as
 * `[locale, message, pattern]` triples so message names survive the host
 * bridge, and rejects a malformed catalog or locale before the host sees it.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { createTestRuntime } from "./load-runtime.mjs";

/** Arrays built in the sandbox realm: compare their shape, not prototypes. */
const plain = (value) => JSON.parse(JSON.stringify(value));

test("a catalog crosses as triples, nested names joined, through a host-global call", async () => {
  const { sandbox, calls } = await createTestRuntime();
  sandbox.__nanaActiveWindowId = 3;
  calls.length = 0;
  sandbox.Nana.i18n.setCatalog(
    {
      "en-US": { files: "{count, plural, one {# file} other {# files}}", onboarding: { title: "Welcome" } },
      ar: { key: "مفتاح" },
    },
    { fallback: "en-US", missing: "keep-previous" },
  );
  assert.deepEqual(plain(calls), [
    [
      "i18nSetCatalog",
      [
        [
          ["en-US", "files", "{count, plural, one {# file} other {# files}}"],
          ["en-US", "onboarding.title", "Welcome"],
          ["ar", "key", "مفتاح"],
        ],
        "en-US",
        "keep-previous",
      ],
    ],
  ]);
});

test("a malformed catalog or option throws before reaching the host", async () => {
  const { sandbox, calls } = await createTestRuntime();
  calls.length = 0;
  for (const catalog of [null, [], { en: "files" }, { en: { files: 3 } }]) {
    assert.throws(() => sandbox.Nana.i18n.setCatalog(catalog), { name: "TypeError" });
  }
  assert.throws(() => sandbox.Nana.i18n.setCatalog({}, { missing: "blank" }), { name: "TypeError" });
  assert.deepEqual(calls, []);
});

test("the application locale is set and read through the host", async () => {
  const { sandbox, calls } = await createTestRuntime();
  sandbox.__nanaActiveWindowId = 2;
  const host = sandbox.__nanaHost.call;
  sandbox.__nanaHost.call = (name, args) => (name === "i18nGetLocale" ? "ar" : host(name, args));
  calls.length = 0;
  sandbox.Nana.i18n.setLocale("ar");
  sandbox.Nana.i18n.setLocale({ messages: "ar", direction: "ltr", formatting: "ar-EG", language: null });
  sandbox.Nana.i18n.setLocale(null);
  sandbox.Nana.i18n.setLocale("");
  assert.equal(sandbox.Nana.i18n.locale, "ar");
  assert.deepEqual(plain(calls), [
    ["i18nSetLocale", ["ar"]],
    ["i18nSetLocale", [{ messages: "ar", direction: "ltr", formatting: "ar-EG" }]],
    ["i18nSetLocale", [null]],
    ["i18nSetLocale", [null]],
  ]);
  for (const bad of [3, ["ar"], { message: "ar" }, { messages: "" }, { messages: "ar", direction: "up" }]) {
    assert.throws(() => sandbox.Nana.i18n.setLocale(bad), { name: "TypeError" });
  }
});

test("a window takes a locale of its own when created and later", async () => {
  const { sandbox, calls } = await createTestRuntime();
  let request;
  sandbox.Nana.host = {
    async invoke(name, args) {
      assert.equal(name, "windowCreate");
      request = args[0];
      return { id: 2, mountRoot: 2 * 4294967296 + 2, ready: true, isolation: "isolated" };
    },
  };
  const handle = await sandbox.Nana.windows.create({ isolation: "isolated", locale: { messages: "he" } });
  assert.deepEqual(plain(request.locale), { messages: "he" });
  await assert.rejects(sandbox.Nana.windows.create({ locale: 7 }), { name: "TypeError" });

  calls.length = 0;
  handle.setLocale("de");
  handle.setLocale(null);
  assert.deepEqual(plain(calls), [
    ["windowSetLocale", [2, "de"]],
    ["windowSetLocale", [2, null]],
  ]);
});
