/**
 * Contract: createNanaRenderer patchProp mirrors Vue runtime-dom
 * (.prop / ^attr, boolean attrs, classList↔class, SVG attrs).
 * Behavioral coverage lives in `nana-ui-vue` bridge/renderer tests.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, test } from "node:test";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = readFileSync(join(root, "src/createNanaRenderer.js"), "utf8");

describe("patchProp Vue runtime-dom contracts", () => {
  test("strips .prop and ^attr modifiers", () => {
    assert.match(src, /propKey\[0\] === ["']\.["']/);
    assert.match(src, /propKey\[0\] === ["']\^["']/);
    assert.match(src, /propKey = propKey\.slice\(1\)/);
  });

  test("class patch syncs classList via __replace", () => {
    assert.match(src, /syncClassList\(el,\s*value\)/);
    assert.match(src, /__replace\(classValue/);
    assert.match(src, /node\.classList = createClassList\(nid,\s*node\)/);
  });

  test("classList mutations write attributes.class", () => {
    assert.match(src, /el\.attributes\.class = joined/);
  });

  test("SVG attrs prefer attribute path", () => {
    assert.match(src, /COMMON_SVG_ATTRS/);
    assert.match(src, /isSvgElement/);
    assert.match(src, /xlink:/);
    assert.match(src, /viewBox/);
    assert.match(src, /__isSVG/);
  });

  test("boolean false clears attribute locally", () => {
    assert.match(src, /value == null \|\| value === false/);
    assert.match(src, /delete el\.attributes\[propKey\]/);
  });

  test("createElement seeds __isSVG from namespace", () => {
    assert.match(src, /node\.__isSVG = ns === ["']svg["']/);
  });
});
