import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const skip = new Set(["node_modules", ".vitepress", "dist", "cache"]);

function markdownFiles(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (skip.has(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) markdownFiles(full, out);
    else if (entry.name.endsWith(".md")) out.push(full);
  }
  return out;
}

const blockPattern = /^:::api[ \t]*\n([\s\S]*?)^:::[ \t]*$/gm;
const viewFence = /^```rust view[ \t]*$/gm;
const rustFence = /^```rust rust[ \t]*$/gm;
let failed = false;

for (const file of markdownFiles(root)) {
  const text = fs.readFileSync(file, "utf8").replace(/\r\n/g, "\n");
  const blocks = [...text.matchAll(blockPattern)];
  let rest = text;
  for (const block of blocks) rest = rest.replace(block[0], "");
  if (/^```rust (?:view|rust)[ \t]*$/m.test(rest)) {
    console.error(`${path.relative(root, file)}: rust view/rust fence outside :::api`);
    failed = true;
  }
  blocks.forEach((block, index) => {
    const body = block[1];
    const view = (body.match(viewFence) || []).length;
    const rust = (body.match(rustFence) || []).length;
    if (view !== 1 || rust !== 1) {
      console.error(
        `${path.relative(root, file)}: :::api #${index + 1} has view=${view} rust=${rust}`,
      );
      failed = true;
    }
  });
}

if (failed) process.exit(1);
