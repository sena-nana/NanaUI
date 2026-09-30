import container from "markdown-it-container";
import type { MarkdownRenderer } from "vitepress";

// `:::api` holds one ```rust view fence and one ```rust rust fence.
// The check script rejects any other shape. Unmarked fences stay visible.
export function apiStylePlugin(md: MarkdownRenderer) {
  md.use(container, "api", {
    render(tokens, idx) {
      return tokens[idx].nesting === 1 ? '<div class="api-pair">\n' : "</div>\n";
    },
  });

  const fence = md.renderer.rules.fence;
  md.renderer.rules.fence = (tokens, idx, options, env, self) => {
    const token = tokens[idx];
    const raw = token.info.trim();
    const kind = raw === "rust view" ? "api-view" : raw === "rust rust" ? "api-rust" : null;
    if (kind) token.info = "rust";
    const html = fence ? fence(tokens, idx, options, env, self) : self.renderToken(tokens, idx, options);
    token.info = raw;
    return kind ? `<div class="${kind}">\n${html}</div>\n` : html;
  };
}
