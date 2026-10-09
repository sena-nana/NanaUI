/**
 * `<T>`'s arguments as the `message-args` attribute of a `<nana-text>`
 * (Issue #267). Vue-free: the encoding is a contract with the Rust host.
 *
 * JSON text rather than an object, because the host bridge drops object keys
 * named `key`, `ref` or `on*` and turns a `Date` into `{}`. Names are sorted,
 * so the same arguments are the same text and Vue patches nothing.
 */

/**
 * Encode `args` for `message-args`; `undefined` when there are none.
 *
 * - a number stays a number (`plural`, `{n, number}`); NaN and infinities
 *   are left out, and the message names them as missing;
 * - a string stays a string;
 * - a boolean becomes `"true"` / `"false"`, which `select` reads;
 * - a `Date` becomes its local civil date and time of day;
 * - `{ year, month, day }`, with `hour`, `minute`, `second` optional, is a
 *   civil date (month 1–12), for a date with no time of day;
 * - `{ amount, currency }` is an amount of an ISO 4217 currency;
 * - `null` and `undefined` leave the argument out.
 *
 * Any other value is sent as it is, and the host reports it.
 */
export function encodeMessageArgs(args) {
  if (args == null) return undefined;
  const out = {};
  let any = false;
  for (const name of Object.keys(args).sort()) {
    const value = encodeArg(args[name]);
    if (value === undefined) continue;
    out[name] = value;
    any = true;
  }
  return any ? JSON.stringify(out) : undefined;
}

function encodeArg(value) {
  switch (typeof value) {
    case "number":
      return Number.isFinite(value) ? value : undefined;
    case "bigint":
      return Number(value);
    case "string":
      return value;
    case "boolean":
      return value ? "true" : "false";
    case "object":
      if (value === null) return undefined;
      if (value instanceof Date || Object.prototype.toString.call(value) === "[object Date]") {
        if (Number.isNaN(value.getTime())) return undefined;
        return {
          $date: [
            value.getFullYear(),
            value.getMonth() + 1,
            value.getDate(),
            value.getHours(),
            value.getMinutes(),
            value.getSeconds(),
          ],
        };
      }
      if ("currency" in value && "amount" in value) {
        return { $currency: [Number(value.amount), String(value.currency)] };
      }
      if ("year" in value && "month" in value && "day" in value) {
        const date = [value.year, value.month, value.day].map(Number);
        const time = ["hour", "minute", "second"].map((part) => value[part]);
        return {
          $date: time.some((part) => part != null)
            ? [...date, ...time.map((part) => Number(part ?? 0))]
            : date,
        };
      }
      return value;
    default:
      // Functions and symbols are no arguments.
      return undefined;
  }
}

/** What `<T>` passes to its `<nana-text>` rather than to the message. */
function isElementAttr(name, value) {
  return (
    name === "class" ||
    name === "style" ||
    name === "lang" ||
    name === "dir" ||
    name === "locale" ||
    name.startsWith("data-") ||
    name.startsWith("aria-") ||
    // Listeners: `@click` on `<T>` listens on the text.
    (/^on[A-Z]/.test(name) && (typeof value === "function" || Array.isArray(value)))
  );
}

/**
 * The props of the `<nana-text>` that `<T id="…" …>` renders: the element's
 * own attributes, the message's name, and the remaining attributes, with
 * `args` over them, as its arguments.
 */
export function messageTextProps(id, attrs, args) {
  const element = {};
  const fromAttrs = {};
  for (const [name, value] of Object.entries(attrs || {})) {
    if (isElementAttr(name, value)) element[name] = value;
    else fromAttrs[name] = value;
  }
  return {
    ...element,
    "message-id": String(id),
    "message-args": encodeMessageArgs(args ? { ...fromAttrs, ...args } : fromAttrs),
  };
}
