/**
 * Behavior: `<T>`'s arguments as the `message-args` attribute the Rust host
 * reads, and how `<T>` splits its attributes between the element and the
 * message. Vue-free, like the encoding itself.
 */
import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { encodeMessageArgs, messageTextProps } from "../src/message-args.js";

describe("message-args", () => {
  test("numbers, text, booleans, dates and currencies have one wire form each", () => {
    const at = new Date(2026, 9, 9, 14, 5, 7);
    assert.deepEqual(
      JSON.parse(
        encodeMessageArgs({
          count: 3,
          name: "Nana",
          on: true,
          off: false,
          at,
          day: { year: 2026, month: 10, day: 9 },
          noon: { year: 2026, month: 10, day: 9, hour: 12 },
          price: { amount: 9.5, currency: "EUR" },
          big: 7n,
        }),
      ),
      {
        at: { $date: [2026, 10, 9, 14, 5, 7] },
        big: 7,
        count: 3,
        day: { $date: [2026, 10, 9] },
        name: "Nana",
        noon: { $date: [2026, 10, 9, 12, 0, 0] },
        off: "false",
        on: "true",
        price: { $currency: [9.5, "EUR"] },
      },
    );
  });

  test("absent and unrepresentable values are left out for the message to name", () => {
    assert.equal(encodeMessageArgs(null), undefined);
    assert.equal(encodeMessageArgs({}), undefined);
    assert.equal(
      encodeMessageArgs({ gone: null, missing: undefined, nan: NaN, far: Infinity, bad: new Date(NaN), fn() {} }),
      undefined,
    );
    // Not an argument the host knows: sent as it is, for the host to report.
    assert.equal(encodeMessageArgs({ list: [1, 2] }), '{"list":[1,2]}');
  });

  test("the same arguments in any order are the same text, so Vue patches nothing", () => {
    assert.equal(encodeMessageArgs({ b: 1, a: "x" }), encodeMessageArgs({ a: "x", b: 1 }));
    assert.equal(encodeMessageArgs({ b: 1, a: "x" }), '{"a":"x","b":1}');
  });
});

describe("<T> attributes", () => {
  test("the element keeps its own attributes and the rest are arguments", () => {
    const onClick = () => {};
    const props = messageTextProps(
      "files",
      {
        count: 2,
        "file-kind": "image",
        class: "caption",
        style: { color: "red" },
        lang: "ja",
        dir: "rtl",
        locale: "ar",
        "data-agent-id": "files.count",
        "aria-live": "polite",
        onClick,
        title: "Files",
      },
      null,
    );
    assert.deepEqual(Object.keys(props).sort(), [
      "aria-live",
      "class",
      "data-agent-id",
      "dir",
      "lang",
      "locale",
      "message-args",
      "message-id",
      "onClick",
      "style",
    ]);
    assert.equal(props["message-id"], "files");
    assert.equal(props.onClick, onClick);
    assert.deepEqual(JSON.parse(props["message-args"]), {
      count: 2,
      "file-kind": "image",
      title: "Files",
    });
  });

  test("args gives arguments by name, over attributes and names the element keeps", () => {
    const props = messageTextProps("greeting", { count: 1, class: "x" }, { count: 5, class: "admin" });
    assert.equal(props.class, "x");
    assert.deepEqual(JSON.parse(props["message-args"]), { class: "admin", count: 5 });
    assert.equal(messageTextProps("plain", {}, null)["message-args"], undefined);
  });
});
