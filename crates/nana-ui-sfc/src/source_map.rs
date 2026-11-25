//! Bidirectional token-aligned source map between `.vue` views and
//! generated `nana_views.rs` Rust code (`nana-sfc-source-map/1`).

use std::path::Path;

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use serde::{Deserialize, Serialize};

pub const SOURCE_MAP_SCHEMA: &str = "nana-sfc-source-map/1";

/// 1-based line and character-column range in the generated `nana_views.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedRange {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

/// 1-based line and character-column point (or span) in a `.vue` source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoint {
    pub file: String,
    pub line: usize,
    pub column: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<usize>,
}

/// One segment linking a generated Rust span to a `.vue` source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSegment {
    pub generated: GeneratedRange,
    pub source: SourcePoint,
    #[serde(default)]
    pub exact: bool,
}

/// A resolved `.vue` span returned by reverse lookups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: String,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

/// A resolved 1-based `(line, column)` position in `nana_views.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedPos {
    pub line: usize,
    pub column: usize,
}

/// Complete bidirectional source map for a compiled `.vue` batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMap {
    pub schema: String,
    pub generated: String,
    pub segments: Vec<SourceSegment>,
}

impl SourceMap {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("source map serializes") + "\n"
    }

    /// Map a 1-based `(line, char_column)` in a `.vue` file to `nana_views.rs`.
    pub fn vue_to_generated(&self, file: &str, line: usize, column: usize) -> Option<GeneratedPos> {
        let matching: Vec<&SourceSegment> = self
            .segments
            .iter()
            .filter(|seg| paths_match(&seg.source.file, file))
            .collect();
        if matching.is_empty() {
            return None;
        }

        // 1. Exact token containing `(line, column)` (half-open on end_column for
        //    multi-char tokens so cursor at end_column hits the adjacent token first).
        let exact_inside = matching
            .iter()
            .copied()
            .filter(|seg| seg.exact && source_contains(&seg.source, line, column, false))
            .min_by_key(|seg| source_span_weight(&seg.source));
        if let Some(seg) = exact_inside {
            let gen_col = if line == seg.source.line {
                let delta = column.saturating_sub(seg.source.column);
                (seg.generated.start_column + delta).min(seg.generated.end_column)
            } else {
                seg.generated.start_column
            };
            return Some(GeneratedPos {
                line: seg.generated.start_line,
                column: gen_col,
            });
        }

        // 2. Exact token whose end touches `(line, column)` (e.g. cursor right after `.` or ident).
        let exact_touching = matching
            .iter()
            .copied()
            .filter(|seg| seg.exact && source_contains(&seg.source, line, column, true))
            .min_by_key(|seg| source_span_weight(&seg.source));
        if let Some(seg) = exact_touching {
            return Some(GeneratedPos {
                line: seg.generated.end_line,
                column: seg.generated.end_column,
            });
        }

        // 3. Non-exact span containing `(line, column)` (e.g. `@activate` -> `on_activate`).
        let non_exact = matching
            .iter()
            .copied()
            .filter(|seg| {
                !seg.exact
                    && seg.source.end_line.is_some()
                    && source_contains(&seg.source, line, column, true)
            })
            .min_by_key(|seg| source_span_weight(&seg.source));
        if let Some(seg) = non_exact {
            return Some(GeneratedPos {
                line: seg.generated.start_line,
                column: seg.generated.start_column,
            });
        }

        // 4. Nearest preceding exact token on the same line (helpful during typing after `. `).
        let preceding = matching
            .iter()
            .copied()
            .filter(|seg| seg.exact && seg.source.line == line && seg.source.column <= column)
            .max_by_key(|seg| seg.source.end_column.unwrap_or(seg.source.column));
        if let Some(seg) = preceding {
            return Some(GeneratedPos {
                line: seg.generated.end_line,
                column: seg.generated.end_column,
            });
        }

        // 5. Fallback statement-level segment on the same line (or closest earlier line).
        matching
            .iter()
            .copied()
            .filter(|seg| seg.source.line <= line)
            .max_by_key(|seg| (seg.source.line, seg.source.column))
            .map(|seg| GeneratedPos {
                line: seg.generated.start_line,
                column: seg.generated.start_column,
            })
    }

    /// Map a 1-based `(line, char_column)` in `nana_views.rs` back to `.vue`.
    pub fn generated_to_vue(&self, line: usize, column: usize) -> Option<SourceSpan> {
        self.generated_range_to_vue(line, column, line, column)
    }

    /// Map a 1-based range in `nana_views.rs` back to `.vue`.
    pub fn generated_range_to_vue(
        &self,
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Option<SourceSpan> {
        // Prefer exact token segments overlapping the target point/range.
        let pick = |exact_only: bool, token_only: bool| {
            self.segments
                .iter()
                .filter(|seg| !exact_only || seg.exact)
                .filter(|seg| !token_only || seg.source.end_line.is_some())
                .filter(|seg| {
                    generated_overlaps(
                        &seg.generated,
                        start_line,
                        start_column,
                        end_line,
                        end_column,
                    )
                })
                .min_by_key(|seg| {
                    let exact_rank = usize::from(!seg.exact);
                    let contains_start = usize::from(!generated_contains(
                        &seg.generated,
                        start_line,
                        start_column,
                    ));
                    (
                        exact_rank,
                        contains_start,
                        generated_span_weight(&seg.generated),
                    )
                })
        };

        let seg = pick(true, true)
            .or_else(|| pick(false, true))
            .or_else(|| pick(false, false))?;
        Some(SourceSpan {
            file: seg.source.file.clone(),
            start_line: seg.source.line,
            start_column: seg.source.column,
            end_line: seg.source.end_line.unwrap_or(seg.source.line),
            end_column: seg.source.end_column.unwrap_or(seg.source.column),
        })
    }
}

/// Convert a 0-based UTF-16 LSP column on `line_text` into a 1-based char column.
pub fn lsp_to_char_col(line_text: &str, utf16_col: u32) -> usize {
    let mut utf16 = 0u32;
    let mut chars = 1usize;
    for ch in line_text.chars() {
        if utf16 >= utf16_col {
            break;
        }
        utf16 += ch.len_utf16() as u32;
        chars += 1;
    }
    chars
}

/// Convert a 1-based char column on `line_text` into a 0-based UTF-16 LSP column.
pub fn char_col_to_lsp(line_text: &str, char_col_1based: usize) -> u32 {
    line_text
        .chars()
        .take(char_col_1based.saturating_sub(1))
        .map(|ch| ch.len_utf16() as u32)
        .sum()
}

/// Build the formatted code (with marker lines blanked) and bidirectional [`SourceMap`].
pub(crate) fn finalize_source_map(
    rendered: &str,
    header: &TokenStream,
    components: &[(&str, &str, &TokenStream)],
) -> (String, SourceMap) {
    let mut lines: Vec<String> = rendered.lines().map(str::to_owned).collect();
    let mut markers = Vec::new();
    let mut marker_lines = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(start) = line.find("__NANA_SFC_MARKER__|") else {
            continue;
        };
        let value = &line[start..];
        let value = value
            .split('"')
            .next()
            .unwrap_or(value)
            .trim_end_matches(';');
        let mut parts = value.split('|');
        let _ = parts.next();
        let Some(file) = parts.next() else { continue };
        let Some(line_no) = parts.next().and_then(|v| v.parse().ok()) else {
            continue;
        };
        let Some(column) = parts.next().and_then(|v| v.parse().ok()) else {
            continue;
        };
        let file = file
            .replace("%5C", "\\")
            .replace("%7C", "|")
            .replace("%25", "%");
        markers.push((index + 1, file, line_no, column));
        marker_lines.push(index);
    }
    for index in marker_lines {
        lines[index].clear();
    }
    let code = lines.join("\n") + if rendered.ends_with('\n') { "\n" } else { "" };

    let mut segments = align_token_segments(&code, header, components);

    for (index, (line, file, source_line, source_column)) in markers.iter().enumerate() {
        let end_line = markers
            .get(index + 1)
            .map_or(lines.len() + 1, |next| next.0);
        segments.push(SourceSegment {
            generated: GeneratedRange {
                start_line: *line,
                start_column: 1,
                end_line,
                end_column: 1,
            },
            source: SourcePoint {
                file: file.clone(),
                line: *source_line,
                column: *source_column,
                end_line: None,
                end_column: None,
            },
            exact: false,
        });
    }

    let map = SourceMap {
        schema: SOURCE_MAP_SCHEMA.to_owned(),
        generated: "nana_views.rs".to_owned(),
        segments,
    };
    (code, map)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokKind {
    Ident,
    Punct(char),
    Literal,
    Delim(char),
}

struct PreToken {
    text: String,
    kind: TokKind,
    file_idx: Option<usize>,
    span: Span,
}

struct PostToken {
    text: String,
    kind: TokKind,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

fn flatten_pre(tokens: TokenStream, file_idx: Option<usize>, out: &mut Vec<PreToken>) {
    let list: Vec<TokenTree> = tokens.into_iter().collect();
    let mut i = 0;
    while i < list.len() {
        // Skip `const _: &str = "__NANA_SFC_MARKER__|...";` (8 tokens).
        if i + 7 < list.len()
            && matches!(&list[i], TokenTree::Ident(id) if id == "const")
            && matches!(&list[i + 1], TokenTree::Ident(id) if id == "_")
            && matches!(&list[i + 2], TokenTree::Punct(p) if p.as_char() == ':')
            && matches!(&list[i + 3], TokenTree::Punct(p) if p.as_char() == '&')
            && matches!(&list[i + 4], TokenTree::Ident(id) if id == "str")
            && matches!(&list[i + 5], TokenTree::Punct(p) if p.as_char() == '=')
            && matches!(&list[i + 6], TokenTree::Literal(lit) if lit.to_string().contains("__NANA_SFC_MARKER__|"))
            && matches!(&list[i + 7], TokenTree::Punct(p) if p.as_char() == ';')
        {
            i += 8;
            continue;
        }
        match &list[i] {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => (Some('('), Some(')')),
                    Delimiter::Brace => (Some('{'), Some('}')),
                    Delimiter::Bracket => (Some('['), Some(']')),
                    Delimiter::None => (None, None),
                };
                if let Some(ch) = open {
                    out.push(PreToken {
                        text: ch.to_string(),
                        kind: TokKind::Delim(ch),
                        file_idx,
                        span: group.span_open(),
                    });
                }
                flatten_pre(group.stream(), file_idx, out);
                if let Some(ch) = close {
                    out.push(PreToken {
                        text: ch.to_string(),
                        kind: TokKind::Delim(ch),
                        file_idx,
                        span: group.span_close(),
                    });
                }
            }
            TokenTree::Ident(ident) => {
                out.push(PreToken {
                    text: ident.to_string(),
                    kind: TokKind::Ident,
                    file_idx,
                    span: ident.span(),
                });
            }
            TokenTree::Punct(punct) => {
                out.push(PreToken {
                    text: punct.as_char().to_string(),
                    kind: TokKind::Punct(punct.as_char()),
                    file_idx,
                    span: punct.span(),
                });
            }
            TokenTree::Literal(lit) => {
                out.push(PreToken {
                    text: lit.to_string(),
                    kind: TokKind::Literal,
                    file_idx,
                    span: lit.span(),
                });
            }
        }
        i += 1;
    }
}

fn flatten_post(tokens: TokenStream, out: &mut Vec<PostToken>) {
    for tt in tokens {
        match tt {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => (Some('('), Some(')')),
                    Delimiter::Brace => (Some('{'), Some('}')),
                    Delimiter::Bracket => (Some('['), Some(']')),
                    Delimiter::None => (None, None),
                };
                if let Some(ch) = open {
                    let s = group.span_open();
                    out.push(PostToken {
                        text: ch.to_string(),
                        kind: TokKind::Delim(ch),
                        start_line: s.start().line,
                        start_column: s.start().column + 1,
                        end_line: s.end().line,
                        end_column: s.end().column + 1,
                    });
                }
                flatten_post(group.stream(), out);
                if let Some(ch) = close {
                    let s = group.span_close();
                    out.push(PostToken {
                        text: ch.to_string(),
                        kind: TokKind::Delim(ch),
                        start_line: s.start().line,
                        start_column: s.start().column + 1,
                        end_line: s.end().line,
                        end_column: s.end().column + 1,
                    });
                }
            }
            TokenTree::Ident(ident) => {
                let s = ident.span();
                out.push(PostToken {
                    text: ident.to_string(),
                    kind: TokKind::Ident,
                    start_line: s.start().line,
                    start_column: s.start().column + 1,
                    end_line: s.end().line,
                    end_column: s.end().column + 1,
                });
            }
            TokenTree::Punct(punct) => {
                let s = punct.span();
                out.push(PostToken {
                    text: punct.as_char().to_string(),
                    kind: TokKind::Punct(punct.as_char()),
                    start_line: s.start().line,
                    start_column: s.start().column + 1,
                    end_line: s.end().line,
                    end_column: s.end().column + 1,
                });
            }
            TokenTree::Literal(lit) => {
                let s = lit.span();
                out.push(PostToken {
                    text: lit.to_string(),
                    kind: TokKind::Literal,
                    start_line: s.start().line,
                    start_column: s.start().column + 1,
                    end_line: s.end().line,
                    end_column: s.end().column + 1,
                });
            }
        }
    }
}

fn tokens_compatible(pre: &PreToken, post: &PostToken) -> bool {
    if pre.kind != post.kind {
        return false;
    }
    match pre.kind {
        TokKind::Punct(_) | TokKind::Delim(_) => true,
        TokKind::Ident => {
            pre.text == post.text
                || pre.text.strip_prefix("r#") == Some(post.text.as_str())
                || post.text.strip_prefix("r#") == Some(pre.text.as_str())
        }
        TokKind::Literal => true,
    }
}

fn align_token_segments(
    code: &str,
    header: &TokenStream,
    components: &[(&str, &str, &TokenStream)],
) -> Vec<SourceSegment> {
    let Ok(parsed_post) = code.parse::<TokenStream>() else {
        return Vec::new();
    };
    let mut pre = Vec::new();
    flatten_pre(header.clone(), None, &mut pre);
    for (idx, (_, _, item)) in components.iter().enumerate() {
        flatten_pre((*item).clone(), Some(idx), &mut pre);
    }
    let mut post = Vec::new();
    flatten_post(parsed_post, &mut post);

    let file_lines: Vec<Vec<&str>> = components
        .iter()
        .map(|(_, text, _)| text.lines().collect())
        .collect();

    let mut segments = Vec::new();
    let mut pi = 0usize;
    let mut qi = 0usize;
    const WINDOW: usize = 16;

    while pi < pre.len() && qi < post.len() {
        if tokens_compatible(&pre[pi], &post[qi]) {
            if let Some(seg) = build_token_segment(&pre[pi], &post[qi], components, &file_lines) {
                segments.push(seg);
            }
            pi += 1;
            qi += 1;
            continue;
        }
        // Look ahead for the closest pair match (prettyplease adds/removes commas/semicolons).
        let mut best: Option<(usize, usize)> = None;
        'search: for dist in 1..=WINDOW {
            for dp in 0..=dist {
                let dq = dist - dp;
                if pi + dp < pre.len()
                    && qi + dq < post.len()
                    && tokens_compatible(&pre[pi + dp], &post[qi + dq])
                {
                    best = Some((dp, dq));
                    break 'search;
                }
            }
        }
        match best {
            Some((dp, dq)) => {
                pi += dp;
                qi += dq;
            }
            None => {
                pi += 1;
                qi += 1;
            }
        }
    }
    segments
}

fn build_token_segment(
    pre: &PreToken,
    post: &PostToken,
    components: &[(&str, &str, &TokenStream)],
    file_lines: &[Vec<&str>],
) -> Option<SourceSegment> {
    let file_idx = pre.file_idx?;
    let s = pre.span.start();
    let e = pre.span.end();
    // Synthetic spans (`Span::call_site()`) have zero width at 1:0.
    if s.line == e.line && e.column <= s.column {
        return None;
    }
    if e.line < s.line {
        return None;
    }
    let file = components[file_idx].0;
    let lines = &file_lines[file_idx];
    let exact = if s.line == e.line {
        lines
            .get(s.line.saturating_sub(1))
            .is_some_and(|line_text| {
                let slice: String = line_text
                    .chars()
                    .skip(s.column)
                    .take(e.column - s.column)
                    .collect();
                slice == post.text
                    || slice.replace('-', "_") == post.text
                    || pascal_to_snake(&slice) == post.text
            })
    } else {
        false
    };
    Some(SourceSegment {
        generated: GeneratedRange {
            start_line: post.start_line,
            start_column: post.start_column,
            end_line: post.end_line,
            end_column: post.end_column,
        },
        source: SourcePoint {
            file: file.to_owned(),
            line: s.line,
            column: s.column + 1,
            end_line: Some(e.line),
            end_column: Some(e.column + 1),
        },
        exact,
    })
}

fn pascal_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (idx, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if idx > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn paths_match(segment_file: &str, query_file: &str) -> bool {
    if segment_file == query_file {
        return true;
    }
    let norm = |p: &str| p.replace('\\', "/");
    let a = norm(segment_file);
    let b = norm(query_file);
    if a == b {
        return true;
    }
    let a_path = Path::new(&a);
    let b_path = Path::new(&b);
    a_path.ends_with(b_path) || b_path.ends_with(a_path)
}

fn source_contains(source: &SourcePoint, line: usize, column: usize, inclusive_end: bool) -> bool {
    let end_line = source.end_line.unwrap_or(source.line);
    let end_col = source.end_column.unwrap_or(source.column);
    let after_start = (line, column) >= (source.line, source.column);
    let before_end = if inclusive_end {
        (line, column) <= (end_line, end_col)
    } else {
        (line, column) < (end_line, end_col)
    };
    after_start && before_end
}

fn source_span_weight(source: &SourcePoint) -> (usize, usize) {
    let end_line = source.end_line.unwrap_or(source.line);
    let end_col = source.end_column.unwrap_or(source.column);
    (
        end_line.saturating_sub(source.line),
        end_col.saturating_sub(source.column),
    )
}

fn generated_contains(range: &GeneratedRange, line: usize, column: usize) -> bool {
    (line, column) >= (range.start_line, range.start_column)
        && (line, column) <= (range.end_line, range.end_column)
}

fn generated_overlaps(
    range: &GeneratedRange,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
) -> bool {
    (end_line, end_column) >= (range.start_line, range.start_column)
        && (start_line, start_column) <= (range.end_line, range.end_column)
}

fn generated_span_weight(range: &GeneratedRange) -> (usize, usize) {
    (
        range.end_line.saturating_sub(range.start_line),
        range.end_column.saturating_sub(range.start_column),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Compiler;

    #[test]
    fn maps_script_and_template_tokens_bidirectionally() {
        let src = r#"<script setup lang="rust">
let draft = signal(String::new());
let add = move || {
    let len = draft.with(|d| d.len());
    let _ = len;
};
</script>
<template>
  <Column :gap="8">
    <Text>{{ draft }}</Text>
    <Button @activate="add">添加</Button>
  </Column>
</template>"#;
        let output = Compiler::new("::nana_ui_runtime")
            .compile(&[("views/Todo.vue".into(), src.into())])
            .unwrap();
        let map = SourceMap::from_json(&output.source_map).unwrap();
        let gen_lines: Vec<&str> = output.code.lines().collect();

        // Script identifier `draft` on line 2 (`let draft = signal(...)`), col 5..10.
        let pos = map.vue_to_generated("views/Todo.vue", 2, 6).unwrap();
        let line_text = gen_lines[pos.line - 1];
        assert!(
            line_text.contains("let draft"),
            "expected `let draft` at line {}: {line_text}",
            pos.line
        );
        let back = map.generated_to_vue(pos.line, pos.column).unwrap();
        assert_eq!(back.file, "views/Todo.vue");
        assert_eq!(back.start_line, 2);

        // Template interpolation `{{ draft }}` on line 10, col 14.
        let line10 = src.lines().nth(9).unwrap();
        let col10 = line10.find("draft").unwrap() + 2;
        let gen_tpl = map.vue_to_generated("views/Todo.vue", 10, col10).unwrap();
        assert!(gen_lines[gen_tpl.line - 1].contains("draft"));
        let back_tpl = map.generated_to_vue(gen_tpl.line, gen_tpl.column).unwrap();
        assert_eq!(back_tpl.start_line, 10);

        // Event handler `add` on line 11 (`@activate="add"`).
        let line11 = src.lines().nth(10).unwrap();
        let col11 = line11.find("add").unwrap() + 1;
        let gen_evt = map.vue_to_generated("views/Todo.vue", 11, col11).unwrap();
        assert!(gen_lines[gen_evt.line - 1].contains("add"));
        let back_evt = map.generated_to_vue(gen_evt.line, gen_evt.column).unwrap();
        assert_eq!(back_evt.start_line, 11);
    }

    #[test]
    fn maps_cross_component_calls_and_utf16_columns() {
        let item = r#"<script setup lang="rust">
defineProps!(title: String);
</script>
<template><Text>{{ title }}</Text></template>"#;
        let list = r#"<script setup lang="rust">
let label = signal(String::from("你好😀"));
</script>
<template><TodoItem :title="label.get()" /></template>"#;
        let output = Compiler::new("::nana_ui_runtime")
            .compile(&[
                ("views/TodoItem.vue".into(), item.into()),
                ("views/List.vue".into(), list.into()),
            ])
            .unwrap();
        let map = SourceMap::from_json(&output.source_map).unwrap();
        // `<TodoItem` in List.vue maps to `todo_item` call in generated code.
        let pos = map.vue_to_generated("List.vue", 4, 12).unwrap();
        assert!(
            output
                .code
                .lines()
                .nth(pos.line - 1)
                .unwrap()
                .contains("todo_item")
        );
        let back = map.generated_to_vue(pos.line, pos.column).unwrap();
        assert_eq!(back.file, "views/List.vue");
        assert_eq!(back.start_line, 4);

        // UTF-16 column conversion on CJK + emoji line.
        let line = "let x = \"你好😀\";";
        let utf16_end = line.encode_utf16().count() as u32;
        let char_col = lsp_to_char_col(line, utf16_end);
        assert_eq!(char_col, line.chars().count() + 1);
        assert_eq!(char_col_to_lsp(line, char_col), utf16_end);
    }
}
