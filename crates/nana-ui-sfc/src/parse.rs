//! `.vue` source → blocks → template element tree and script statements.
//!
//! Expressions are parsed from text padded to their position in the file,
//! so every span — and every error — carries the file's own line and column.

use proc_macro2::Span;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Ident, Pat, PatType, Stmt, Token};

use nana_ui_view_codegen::{Attr, AttrName, AttrValue, Element, Node, TextPart};

use crate::Error;

/// A position in the file: 1-based line, 0-based column in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

pub struct Component {
    pub name: String,
    pub name_span: Span,
    pub props: Vec<PatType>,
    pub script: Vec<Stmt>,
    pub template: Vec<Node>,
    /// The `<style>` block's CSS, compiled against this view's elements.
    pub style: Option<Style>,
}

/// One `<style>` block slice inside [`Style::css`], recording its starting
/// offset in the combined CSS text and its starting position in the `.vue` file.
#[derive(Debug, Clone)]
pub struct StyleSegment {
    pub combined_start: usize,
    pub len: usize,
    /// 1-based.
    pub line: usize,
    /// 0-based, in characters.
    pub column: usize,
}

/// All `<style>` blocks of a `.vue` component combined in document order, with
/// per-block origin metadata so diagnostics map back to exact `.vue` positions.
pub struct Style {
    pub css: String,
    pub segments: Vec<StyleSegment>,
}

impl Style {
    /// `line:column` (1-based) of a byte offset into the combined CSS.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let clamped = offset.min(self.css.len());
        let seg = self
            .segments
            .iter()
            .rev()
            .find(|s| clamped >= s.combined_start)
            .or_else(|| self.segments.first());
        let Some(seg) = seg else {
            return (1, 1);
        };
        let local_end = clamped.min(seg.combined_start + seg.len);
        let before = &self.css[seg.combined_start..local_end];
        match before.rfind('\n') {
            None => (seg.line, seg.column + before.chars().count() + 1),
            Some(newline) => (
                seg.line + before.matches('\n').count(),
                before[newline + 1..].chars().count() + 1,
            ),
        }
    }
}

struct Source<'a> {
    file: &'a str,
    text: &'a str,
    /// Byte offset where each line starts.
    lines: Vec<usize>,
}

impl<'a> Source<'a> {
    fn new(file: &'a str, text: &'a str) -> Self {
        let mut lines = vec![0];
        lines.extend(text.match_indices('\n').map(|(index, _)| index + 1));
        Self { file, text, lines }
    }

    fn pos(&self, offset: usize) -> Pos {
        let line = self.lines.partition_point(|start| *start <= offset);
        let start = self.lines[line - 1];
        Pos {
            line,
            column: self.text[start..offset].chars().count(),
        }
    }

    fn error(&self, offset: usize, message: impl Into<String>) -> Error {
        let pos = self.pos(offset);
        Error {
            file: self.file.to_owned(),
            line: pos.line,
            column: pos.column + 1,
            message: message.into(),
        }
    }
}

/// Turn a syn error from a padded parse into a file error.
pub(crate) fn syn_error(file: &str, error: syn::Error) -> Error {
    let start = error.span().start();
    Error {
        file: file.to_owned(),
        line: start.line.max(1),
        column: start.column + 1,
        message: error.to_string(),
    }
}

/// Parse `text` as `T` with spans at `pos` in the file.
fn parse_at<T>(
    parser: impl Parser<Output = T>,
    text: &str,
    pos: Pos,
    file: &str,
) -> Result<T, Error> {
    let padded = format!(
        "{}{}{}",
        "\n".repeat(pos.line - 1),
        " ".repeat(pos.column),
        text
    );
    parser.parse_str(&padded).map_err(|error| {
        let mut error = syn_error(file, error);
        // "Unexpected end of input" has no span of its own.
        if error.line < pos.line {
            error.line = pos.line;
            error.column = pos.column + 1;
        }
        error
    })
}

fn ident_at(name: &str, pos: Pos, file: &str) -> Result<Ident, Error> {
    parse_at(<Ident as syn::ext::IdentExt>::parse_any, name, pos, file)
}

fn expr_at(text: &str, pos: Pos, file: &str) -> Result<Expr, Error> {
    parse_at(<Expr as syn::parse::Parse>::parse, text, pos, file)
}

/// `<slot/>` places the `children` argument, `<slot name="header"/>` the
/// `header` one.
fn slot(element: Element, file: &str) -> Result<Node, Error> {
    let span = element.name.span();
    let mut name = String::from("children");
    for attr in element.attrs {
        match (attr.name, attr.value) {
            (AttrName::Plain(ident), AttrValue::Lit(syn::Expr::Lit(lit))) if ident == "name" => {
                if let syn::Lit::Str(text) = lit.lit {
                    name = text.value().replace('-', "_");
                }
            }
            _ => {
                return Err(syn_error(
                    file,
                    syn::Error::new(span, "`<slot>` takes only `name=\"…\"`"),
                ));
            }
        }
    }
    if !element.children.is_empty() {
        return Err(syn_error(
            file,
            syn::Error::new(span, "`<slot>` has no fallback content"),
        ));
    }
    let ident = Ident::new(&name, span);
    Ok(Node::Expr(syn::parse_quote_spanned!(span=> #ident)))
}

/// `todo-row` → `TodoRow`; `TodoRow` stays.
fn pascal_case(tag: &str) -> String {
    if !tag.contains('-') {
        return tag.to_owned();
    }
    tag.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// `TodoItem.vue` → `TodoItem`.
fn component_name(file: &str) -> String {
    std::path::Path::new(file)
        .file_stem()
        .map(|stem| pascal_case(&stem.to_string_lossy()))
        .unwrap_or_default()
}

struct RawBlock<'t> {
    tag: &'t str,
    attrs: Vec<(String, Option<String>)>,
    body_start: usize,
    body_end: usize,
}

fn parse_tag_attrs(raw: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == name_start {
            i += 1;
            continue;
        }
        let name = raw[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                let val = raw[v_start..i].to_owned();
                if i < bytes.len() {
                    i += 1;
                }
                Some(val)
            } else {
                let v_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                Some(raw[v_start..i].to_owned())
            }
        } else {
            None
        };
        out.push((name, value));
    }
    out
}

/// Find the closing `>` of an opening tag starting at `tag_start`, respecting
/// `"..."` and `'...'` quoted attribute values.
fn find_open_tag_end(text: &str, tag_start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = tag_start;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i),
            None => {}
        }
        i += 1;
    }
    None
}

/// Check if `text[pos..]` starts with `</tag` followed by optional whitespace and `>`,
/// returning the byte index immediately after `>`.
fn match_closing_tag(text: &str, pos: usize, tag: &str) -> Option<usize> {
    let rest = text.get(pos..)?;
    let after_slash = rest.strip_prefix("</")?;
    let after_tag = after_slash.get(..tag.len())?;
    if !after_tag.eq_ignore_ascii_case(tag) {
        return None;
    }
    let tail = &after_slash[tag.len()..];
    let ws_len = tail.len() - tail.trim_start().len();
    if tail[ws_len..].starts_with('>') {
        Some(pos + 2 + tag.len() + ws_len + 1)
    } else {
        None
    }
}

/// Check if `text[pos..]` starts with `<tag` followed by whitespace, `>`, or `/`.
fn starts_open_tag(text: &str, pos: usize, tag: &str) -> bool {
    let Some(rest) = text.get(pos..) else {
        return false;
    };
    let Some(after_lt) = rest.strip_prefix('<') else {
        return false;
    };
    let Some(head) = after_lt.get(..tag.len()) else {
        return false;
    };
    if !head.eq_ignore_ascii_case(tag) {
        return false;
    }
    matches!(
        after_lt.as_bytes().get(tag.len()),
        Some(b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
    )
}

/// Scan `<template>` body until its matching `</template>`, tracking nested
/// `<template>` blocks (like `<template #header>` / `<template #default>`) and
/// skipping HTML comments and quoted tag attributes.
fn find_template_close(text: &str, body_start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = body_start;
    let mut depth = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if text[i..].starts_with("<!--") {
                let rel = text[i + 4..].find("-->")?;
                i = i + 4 + rel + 3;
                continue;
            }
            if let Some(after_close) = match_closing_tag(text, i, "template") {
                depth -= 1;
                if depth == 0 {
                    return Some((i, after_close));
                }
                i = after_close;
                continue;
            }
            if starts_open_tag(text, i, "template") {
                if let Some(gt) = find_open_tag_end(text, i + 1) {
                    let self_closing = text[i..gt].trim_end().ends_with('/');
                    if !self_closing {
                        depth += 1;
                    }
                    i = gt + 1;
                    continue;
                }
                return None;
            }
            // Skip any other element's opening tag so quoted attribute values
            // containing `</template>` or `<template` don't confuse nesting.
            if bytes.get(i + 1).is_some_and(|b| b.is_ascii_alphabetic())
                && let Some(gt) = find_open_tag_end(text, i + 1)
            {
                i = gt + 1;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Scan `<script>` (Rust) body until `</script>`, skipping Rust string literals,
/// raw strings `r#"..."#`, line comments `//`, and nested block comments `/* */`.
fn find_script_close(text: &str, body_start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = body_start;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && let Some(after_close) = match_closing_tag(text, i, "script")
        {
            return Some((i, after_close));
        }
        // Line comment `// ...`
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Nested Rust block comment `/* ... */`
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut cdepth = 1usize;
            while i < bytes.len() && cdepth > 0 {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    cdepth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    cdepth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Raw string literal: `r#"..."#`, `br#"..."#`, `cr#"..."#`
        if bytes[i] == b'r'
            && (i == body_start
                || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_'
                || matches!(bytes[i - 1], b'b' | b'c'))
        {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let hashes = j - (i + 1);
                j += 1;
                while j < bytes.len() {
                    if bytes[j] == b'"' {
                        let mut h = 0;
                        while h < hashes && bytes.get(j + 1 + h) == Some(&b'#') {
                            h += 1;
                        }
                        if h == hashes {
                            j = j + 1 + hashes;
                            break;
                        }
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        // Regular string literal `"..."`
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    None
}

/// Scan `<style>` (CSS) body until `</style>`, skipping CSS strings and `/* */` comments.
fn find_style_close(text: &str, body_start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = body_start;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && let Some(after_close) = match_closing_tag(text, i, "style")
        {
            return Some((i, after_close));
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    None
}

/// Scan top-level SFC blocks (`<template>`, `<script>`, `<style>`) cleanly
/// without being fooled by strings inside `<script>` or `<template>` or by
/// multiple `<style>` blocks.
fn scan_top_level_blocks<'t>(src: &Source<'t>) -> Result<Vec<RawBlock<'t>>, Error> {
    let text = src.text;
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut blocks = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if text[i..].starts_with("<!--") {
            let Some(rel) = text[i + 4..].find("-->") else {
                return Err(src.error(i, "comment is never closed"));
            };
            i = i + 4 + rel + 3;
            continue;
        }
        let open_offset = i;
        let name_start = i + 1;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'-')
        {
            name_end += 1;
        }
        let tag = &text[name_start..name_end];
        let Some(gt) = find_open_tag_end(text, name_end) else {
            return Err(src.error(open_offset, format!("`<{tag}` is never closed")));
        };
        let attrs_raw = &text[name_end..gt];
        let self_closing = attrs_raw.trim_end().ends_with('/');
        let attrs = parse_tag_attrs(attrs_raw.trim_end_matches('/'));
        let body_start = gt + 1;
        if self_closing {
            blocks.push(RawBlock {
                tag,
                attrs,
                body_start,
                body_end: body_start,
            });
            i = body_start;
            continue;
        }
        let close = match tag {
            "template" => find_template_close(text, body_start),
            "script" => find_script_close(text, body_start),
            "style" => find_style_close(text, body_start),
            _ => {
                // Unknown top-level tag: advance past opening tag.
                i = body_start;
                continue;
            }
        };
        let Some((body_end, next_i)) = close else {
            return Err(src.error(open_offset, format!("`<{tag}>` has no `</{tag}>`")));
        };
        blocks.push(RawBlock {
            tag,
            attrs,
            body_start,
            body_end,
        });
        i = next_i;
    }
    Ok(blocks)
}

pub fn component(file: &str, text: &str) -> Result<Component, Error> {
    let src = Source::new(file, text);
    let blocks = scan_top_level_blocks(&src)?;

    // Combine all top-level `<style>` blocks in document order. Each block is
    // component-scoped by default (`scoped` or bare); `<style global>` is also
    // accepted cleanly.
    let mut combined_css = String::new();
    let mut style_segments = Vec::new();
    let mut script_block: Option<&RawBlock<'_>> = None;
    let mut template_block: Option<&RawBlock<'_>> = None;

    for blk in &blocks {
        match blk.tag {
            "style" => {
                if let Some((_, lang)) = blk.attrs.iter().find(|(k, _)| k == "lang")
                    && !lang
                        .as_deref()
                        .is_some_and(|v| v.eq_ignore_ascii_case("css"))
                {
                    return Err(
                        src.error(blk.body_start, "`<style>` takes plain CSS (`lang=\"css\"`)")
                    );
                }
                if !combined_css.is_empty() {
                    combined_css.push('\n');
                }
                let combined_start = combined_css.len();
                let slice = &src.text[blk.body_start..blk.body_end];
                combined_css.push_str(slice);
                let pos = src.pos(blk.body_start);
                style_segments.push(StyleSegment {
                    combined_start,
                    len: slice.len(),
                    line: pos.line,
                    column: pos.column,
                });
            }
            "script" if script_block.is_none() => {
                script_block = Some(blk);
            }
            "template" if template_block.is_none() => {
                template_block = Some(blk);
            }
            _ => {}
        }
    }

    let style = if style_segments.is_empty() {
        None
    } else {
        Some(Style {
            css: combined_css,
            segments: style_segments,
        })
    };

    let (props, script, script_pos) = match script_block {
        Some(blk) => {
            let has_setup = blk.attrs.iter().any(|(k, _)| k == "setup");
            let has_rust = blk.attrs.iter().any(|(k, v)| {
                k == "lang" && v.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("rust"))
            });
            if !has_setup || !has_rust {
                return Err(src.error(
                    blk.body_start,
                    "the script block must be `<script setup lang=\"rust\">`",
                ));
            }
            let pos = src.pos(blk.body_start);
            let (props, script) = script(&src, blk.body_start, blk.body_end)?;
            (props, script, Some(pos))
        }
        None => (Vec::new(), Vec::new(), None),
    };

    let tpl = template_block.ok_or_else(|| src.error(0, "a `.vue` view needs a `<template>`"))?;
    let template_pos = src.pos(tpl.body_start);
    let mut parser = Template {
        src: &src,
        offset: tpl.body_start,
        end: tpl.body_end,
    };
    let template = parser.nodes(None)?;
    let name_pos = script_pos.unwrap_or(template_pos);
    let name_span = ident_at("__nana_comp", name_pos, file)
        .map(|id| id.span())
        .unwrap_or_else(|_| Span::call_site());
    Ok(Component {
        name: component_name(file),
        name_span,
        props,
        script,
        template,
        style,
    })
}

fn script(src: &Source, start: usize, end: usize) -> Result<(Vec<PatType>, Vec<Stmt>), Error> {
    let pos = src.pos(start);
    // The `{` that makes the body a block takes the column before it.
    let padded = format!(
        "{}{}{{{}}}",
        "\n".repeat(pos.line - 1),
        " ".repeat(pos.column.saturating_sub(1)),
        &src.text[start..end]
    );
    let block: syn::Block = syn::parse_str(&padded).map_err(|error| syn_error(src.file, error))?;
    let mut props = Vec::new();
    let mut stmts = Vec::new();
    for stmt in block.stmts {
        if let Stmt::Macro(mac) = &stmt
            && mac.mac.path.is_ident("defineProps")
        {
            let declared = Punctuated::<PatType, Token![,]>::parse_terminated
                .parse2(mac.mac.tokens.clone())
                .map_err(|error| syn_error(src.file, error))?;
            props.extend(declared);
            continue;
        }
        stmts.push(stmt);
    }
    Ok((props, stmts))
}

struct Template<'s, 't> {
    src: &'s Source<'t>,
    offset: usize,
    end: usize,
}

impl Template<'_, '_> {
    fn rest(&self) -> &str {
        &self.src.text[self.offset..self.end]
    }

    fn skip_whitespace(&mut self) {
        let rest = self.rest();
        self.offset += rest.len() - rest.trim_start().len();
    }

    /// Children until `</closing>` (or the end of the template).
    fn nodes(&mut self, closing: Option<&str>) -> Result<Vec<Node>, Error> {
        let mut nodes = Vec::new();
        loop {
            if self.rest().starts_with("<!--") {
                let close = self
                    .rest()
                    .find("-->")
                    .ok_or_else(|| self.src.error(self.offset, "comment is never closed"))?;
                self.offset += close + 3;
                continue;
            }
            if self.rest().starts_with("</") {
                let Some(closing) = closing else {
                    return Err(self
                        .src
                        .error(self.offset, "closing tag without an opening one"));
                };
                let tag_end = self
                    .rest()
                    .find('>')
                    .ok_or_else(|| self.src.error(self.offset, "unclosed `</`"))?;
                let name = self.rest()[2..tag_end].trim();
                if pascal_case(name) != closing {
                    return Err(self
                        .src
                        .error(self.offset, format!("`</{name}>` closes `<{closing}>`")));
                }
                self.offset += tag_end + 1;
                return Ok(nodes);
            }
            if self.rest().is_empty() {
                return match closing {
                    Some(closing) => Err(self
                        .src
                        .error(self.end, format!("`<{closing}>` is never closed"))),
                    None => Ok(nodes),
                };
            }
            if self.rest().starts_with('<') {
                let element = self.element()?;
                nodes.push(if element.name == "slot" {
                    slot(element, self.src.file)?
                } else {
                    Node::Element(element)
                });
            } else if let Some(text) = self.text()? {
                nodes.push(text);
            }
        }
    }

    fn element(&mut self) -> Result<Element, Error> {
        let start = self.offset;
        self.offset += 1;
        let name_len = self
            .rest()
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(self.rest().len());
        let raw_name = &self.rest()[..name_len];
        if raw_name.is_empty() {
            return Err(self.src.error(start, "expected a tag name"));
        }
        let tag = pascal_case(raw_name);
        let name = ident_at(&tag, self.src.pos(start + 1), self.src.file)?;
        self.offset += name_len;
        let mut attrs = Vec::new();
        loop {
            self.skip_whitespace();
            if self.rest().starts_with("/>") {
                self.offset += 2;
                return Ok(Element {
                    module: Vec::new(),
                    name,
                    attrs,
                    children: Vec::new(),
                });
            }
            if self.rest().starts_with('>') {
                self.offset += 1;
                break;
            }
            if self.rest().is_empty() {
                return Err(self.src.error(start, format!("`<{tag}` is never closed")));
            }
            attrs.push(self.attr()?);
        }
        let children = self.nodes(Some(&tag))?;
        Ok(Element {
            module: Vec::new(),
            name,
            attrs,
            children,
        })
    }

    fn attr(&mut self) -> Result<Attr, Error> {
        let start = self.offset;
        let name_len = self
            .rest()
            .find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(self.rest().len());
        let raw = self.rest()[..name_len].to_owned();
        self.offset += name_len;
        let value = if self.rest().starts_with('=') {
            self.offset += 1;
            let quote = self
                .rest()
                .chars()
                .next()
                .filter(|c| *c == '"' || *c == '\'');
            let Some(quote) = quote else {
                return Err(self.src.error(self.offset, "attribute values are quoted"));
            };
            let value_start = self.offset + 1;
            let len = self.src.text[value_start..self.end]
                .find(quote)
                .ok_or_else(|| self.src.error(start, "attribute value is never closed"))?;
            self.offset = value_start + len + 1;
            Some((
                decode(&self.src.text[value_start..value_start + len]),
                self.src.pos(value_start),
            ))
        } else {
            None
        };
        let pos = self.src.pos(start);
        let file = self.src.file;
        let expr = |value: &Option<(String, Pos)>| -> Result<AttrValue, Error> {
            match value {
                Some((text, pos)) => expr_at(text, *pos, file).map(AttrValue::Expr),
                None => Ok(AttrValue::None),
            }
        };
        let (name, value) = if let Some(event) = raw.strip_prefix('@') {
            let name = ident_at(&event.replace('-', "_"), pos, file)?;
            (AttrName::Event(name), expr(&value)?)
        } else if let Some(event) = raw.strip_prefix("on:") {
            let path = parse_at(<syn::Path as syn::parse::Parse>::parse, event, pos, file)?;
            (AttrName::On(path), expr(&value)?)
        } else if let Some(directive) = raw.strip_prefix("v-") {
            let value = match (directive, &value) {
                ("for", Some((text, pos))) => {
                    let (pattern, source) = text
                        .split_once(" in ")
                        .ok_or_else(|| self.src.error(start, "`v-for=\"item in items\"`"))?;
                    let pattern = parse_at(Pat::parse_single, pattern.trim(), *pos, file)?;
                    // Columns count characters, up to the trimmed source.
                    let skipped = text.len() - source.trim_start().len();
                    let source_pos = Pos {
                        line: pos.line,
                        column: pos.column + text[..skipped].chars().count(),
                    };
                    AttrValue::For(pattern, expr_at(source.trim(), source_pos, file)?)
                }
                _ => expr(&value)?,
            };
            let span = ident_at("v", pos, file)?.span();
            (AttrName::Directive(directive.to_owned(), span), value)
        } else if let Some(class) = raw.strip_prefix("class:") {
            // `class:active="condition"`: the class while the condition holds.
            (
                AttrName::Directive(format!("class:{class}"), ident_at("v", pos, file)?.span()),
                expr(&value)?,
            )
        } else if let Some(slot) = raw.strip_prefix('#') {
            // `#name` is `v-slot:name`.
            (
                AttrName::Directive(format!("slot:{slot}"), ident_at("v", pos, file)?.span()),
                expr(&value)?,
            )
        } else if let Some(bound) = raw.strip_prefix(':') {
            let name = ident_at(&bound.replace('-', "_"), pos, file)?;
            (AttrName::Plain(name), expr(&value)?)
        } else {
            // A plain attribute is a string, as in HTML.
            let name = ident_at(&raw.replace('-', "_"), pos, file)?;
            let value = match value {
                Some((text, _)) => {
                    let literal = syn::LitStr::new(&text, Span::call_site());
                    AttrValue::Lit(syn::parse_quote!(#literal))
                }
                None => AttrValue::None,
            };
            (AttrName::Plain(name), value)
        };
        Ok(Attr { name, value })
    }

    /// Text up to the next tag, with `{{ … }}` interpolation. Whitespace
    /// runs collapse to one space; whitespace-only text is dropped.
    fn text(&mut self) -> Result<Option<Node>, Error> {
        let start = self.offset;
        let mut parts = Vec::new();
        let mut literal = String::new();
        while !self.rest().is_empty() && !self.rest().starts_with('<') {
            if self.rest().starts_with("{{") {
                let open = self.offset;
                let close = self
                    .rest()
                    .find("}}")
                    .ok_or_else(|| self.src.error(open, "`{{` is never closed"))?;
                let inner = &self.rest()[2..close];
                let pos = self.src.pos(open + 2);
                let expr = expr_at(inner, pos, self.src.file)?;
                if !literal.is_empty() {
                    parts.push(TextPart::Literal(std::mem::take(&mut literal)));
                }
                parts.push(TextPart::Expr(expr));
                self.offset += close + 2;
                continue;
            }
            let ch = self.rest().chars().next().expect("rest is not empty");
            literal.push(ch);
            self.offset += ch.len_utf8();
        }
        if !literal.is_empty() {
            parts.push(TextPart::Literal(literal));
        }
        // Collapse whitespace like a browser, trimming the ends.
        let mut parts: Vec<TextPart> = parts
            .into_iter()
            .map(|part| match part {
                TextPart::Literal(text) => TextPart::Literal(collapse(&decode(&text))),
                expr => expr,
            })
            .collect();
        if let Some(TextPart::Literal(first)) = parts.first_mut() {
            *first = first.trim_start().to_owned();
        }
        if let Some(TextPart::Literal(last)) = parts.last_mut() {
            *last = last.trim_end().to_owned();
        }
        parts.retain(|part| !matches!(part, TextPart::Literal(text) if text.is_empty()));
        if parts.is_empty() {
            return Ok(None);
        }
        let span = expr_at("__nana_text", self.src.pos(start), self.src.file)?.span();
        Ok(Some(Node::Mixed(parts, span)))
    }
}

fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
    }
    if space {
        out.push(' ');
    }
    out
}

/// The few HTML entities templates use.
fn decode(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Component, Error> {
        component("Sample.vue", text)
    }

    #[test]
    fn blocks_props_and_the_element_tree_parse() {
        let parsed = parse(
            r#"<script setup lang="rust">
defineProps!(title: String, count: Signal<u32>);
let open = signal(false);
</script>
<template>
  <Column :gap="8">
    <Text>标题 {{ title }}  and   {{ count }}</Text>
    <todo-row v-for="t in items" :key="t.id" @done="open.set(true)" />
  </Column>
</template>"#,
        )
        .unwrap();
        assert_eq!(parsed.name, "Sample");
        assert_eq!(parsed.props.len(), 2);
        assert_eq!(parsed.script.len(), 1);
        let [Node::Element(column)] = parsed.template.as_slice() else {
            panic!("one root");
        };
        assert_eq!(column.name, "Column");
        let [Node::Element(text), Node::Element(row)] = column.children.as_slice() else {
            panic!("two children");
        };
        let [Node::Mixed(parts, _)] = text.children.as_slice() else {
            panic!("mixed text");
        };
        let shapes: Vec<String> = parts
            .iter()
            .map(|part| match part {
                TextPart::Literal(text) => format!("L({text})"),
                TextPart::Expr(_) => "E".to_owned(),
            })
            .collect();
        assert_eq!(shapes, ["L(标题 )", "E", "L( and )", "E"]);
        assert_eq!(row.name, "TodoRow");
        assert_eq!(row.attrs.len(), 3);
    }

    #[test]
    fn errors_point_at_the_file_position() {
        let error = parse("<template>\n  <Text>{{ a + }}</Text>\n</template>")
            .err()
            .expect("an error");
        assert_eq!((error.line, error.file.as_str()), (2, "Sample.vue"));
        let error = parse("<template>\n  <Column>\n</template>")
            .err()
            .expect("an error");
        assert!(error.message.contains("never closed"), "{error}");
        let error = parse("<template><A></B></template>")
            .err()
            .expect("an error");
        assert!(error.message.contains("`</B>` closes `<A>`"), "{error}");
        let error = parse("<template></template><style lang=\"scss\">a {}</style>")
            .err()
            .expect("an error");
        assert!(error.message.contains("plain CSS"), "{error}");
        let error = parse("<script lang=\"rust\">let a = 1;</script><template></template>")
            .err()
            .expect("an error");
        assert!(error.message.contains("<script setup lang"), "{error}");
    }
}
