/**
 * `Nana.i18n` (Issue #267): the application's message catalog and locale.
 *
 * Messages resolve in the Runtime, not here. A `<nana-text>` names a message
 * and its arguments (`<T>` in @nanaui/nanavue-components renders one); the
 * Runtime resolves it in the locale of the scope it is in, and again on every
 * switch, so a switch re-renders nothing in Vue.
 */
import { hostCall } from "./layoutMetrics.js";

const MISSING = new Set(["key", "keep-previous"]);
const DIRECTIONS = new Set(["ltr", "rtl"]);
const LOCALE_FIELDS = new Set(["messages", "language", "direction", "formatting"]);

function isPlainObject(value) {
  return value != null && typeof value === "object" && !Array.isArray(value);
}

/**
 * `{ "en-US": { files: "…", settings: { title: "…" } } }` as
 * `[locale, message, pattern]` triples, nested names joined with ".".
 * Triples rather than the object: the host bridge drops object keys named
 * `key`, `ref` or `on*`, and `onboarding.title` is a message name.
 */
export function flattenCatalog(catalog) {
  if (!isPlainObject(catalog)) {
    throw new TypeError("Nana.i18n.setCatalog expects { locale: { message: pattern } }");
  }
  const entries = [];
  for (const [locale, messages] of Object.entries(catalog)) {
    if (!isPlainObject(messages)) {
      throw new TypeError(`Nana.i18n.setCatalog: the messages of ${locale} are not an object`);
    }
    flattenMessages(locale, messages, "", entries);
  }
  return entries;
}

function flattenMessages(locale, messages, prefix, entries) {
  for (const [name, pattern] of Object.entries(messages)) {
    const id = prefix ? `${prefix}.${name}` : name;
    if (typeof pattern === "string") entries.push([locale, id, pattern]);
    else if (isPlainObject(pattern)) flattenMessages(locale, pattern, id, entries);
    else throw new TypeError(`Nana.i18n.setCatalog: ${locale} ${id} is not a pattern`);
  }
}

/**
 * A locale as the host reads it: a language tag, `{ messages, language,
 * direction, formatting }`, or `null` for none. The parts name what changes
 * apart: the messages picked, the language text shapes in, the direction the
 * scope lays out in, and the locale numbers and dates format in.
 */
export function localeArg(locale) {
  if (locale == null || locale === "") return null;
  if (typeof locale === "string") return locale;
  if (!isPlainObject(locale)) {
    throw new TypeError("a locale is a language tag, { messages, language, direction, formatting } or null");
  }
  for (const key of Object.keys(locale)) {
    if (!LOCALE_FIELDS.has(key)) throw new TypeError(`a locale has no "${key}"`);
  }
  if (typeof locale.messages !== "string" || !locale.messages) {
    throw new TypeError("a locale names its messages with a language tag");
  }
  if (locale.direction != null && !DIRECTIONS.has(locale.direction)) {
    throw new TypeError('a locale direction is "ltr" or "rtl"');
  }
  const arg = { messages: locale.messages };
  for (const key of ["language", "direction", "formatting"]) {
    if (locale[key] != null) arg[key] = String(locale[key]);
  }
  return arg;
}

export const i18n = {
  /**
   * Install the application's catalog in every window. `fallback` is the
   * locale every lookup ends in; `missing` is what a message no locale has
   * shows: `"key"` (the default) or `"keep-previous"`. The same catalog
   * installed again does nothing, so an isolated window running the
   * application script again costs no work.
   */
  setCatalog(catalog, options) {
    const { fallback = null, missing = null } = options || {};
    if (missing != null && !MISSING.has(missing)) {
      throw new TypeError('missing is "key" or "keep-previous"');
    }
    hostCall("i18nSetCatalog", [
      flattenCatalog(catalog),
      fallback == null ? null : String(fallback),
      missing,
    ]);
  },
  /** The application's locale: every window without one of its own takes it. */
  setLocale(locale) {
    hostCall("i18nSetLocale", [localeArg(locale)]);
  },
  /** The tag of the application locale's messages, or `null`. */
  get locale() {
    return hostCall("i18nGetLocale", []) ?? null;
  },
};
