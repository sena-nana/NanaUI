//! The `.vue` dialect compiler for `nana-ui-runtime`'s declarative views.
//!
//! A view file is a Vue single-file component whose script is Rust:
//!
//! ```vue
//! <script setup lang="rust">
//! defineProps!(todo: Todo, list: Signal<Vec<Todo>>);
//! let editing = signal(false);
//! </script>
//! <template>
//!   <Row :gap="8">
//!     <Text>{{ todo.title }}</Text>
//!     <Button @activate="list.update(|l| l.retain(|t| t.id != todo.id))">删除</Button>
//!   </Row>
//! </template>
//! ```
//!
//! `TodoItem.vue` becomes `pub fn todo_item(todo: Todo, list: Signal<Vec<Todo>>)
//! -> impl IntoView`. The template goes through the same code generator as
//! `view!`. Because the compiler sees the whole component, it also knows
//! which names are signals and how each is used:
//!
//! - a signal nothing writes or hands out becomes a `constant`, and a
//!   computed reading only constants becomes one too;
//! - a binding reading only constants is written once, with no effect;
//! - a binding reading known signals through code the compiler can see
//!   declares them, and debug builds check the claim on every run;
//! - computed cycles are errors; a signal written but never read, and a
//!   watcher writing what it reads, are warnings;
//! - every binding's class and dependencies go into a report.
//!
//! Use [`Compiler::build`] from a build script and `include!` the result.

mod analyze;
mod parse;

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use nana_ui_view_codegen::{Attr, AttrName, AttrValue, Element, Node, TextPart};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Expr, Ident, Pat, PatType, Stmt};

use analyze::{Analysis, Class, Kind, Use};
use parse::Component;

/// A compile error at a position in a `.vue` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub file: String,
    pub line: usize,
    /// 1-based.
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}: {}",
            self.file, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for Error {}

/// What a batch of view files compiles to.
pub struct Output {
    /// Rust source: one `pub fn` per view, formatted.
    pub code: String,
    /// Markdown: each view's signals and bindings.
    pub report: String,
    /// `file: message` lines.
    pub warnings: Vec<String>,
}

pub struct Compiler {
    runtime: TokenStream,
    hot: bool,
}

/// One view's hot-reloadable state: the hash of everything but its static
/// text, and that text in template order (see [`Compiler::hot`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotView {
    /// The view's name, the key the running application knows it by.
    pub name: String,
    pub shape: u64,
    pub literals: Vec<String>,
}

/// A compiled view: its item, report section, warnings, and in hot mode its
/// shape and static text.
type CompiledView = (TokenStream, String, Vec<String>, Option<(u64, Vec<String>)>);

/// FNV-1a: stable across builds and processes, which the shape needs.
fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Replace every text node that is only static text with a read of the
/// view's literal table, collecting the text in template order.
fn hot_literals(nodes: &mut [Node], runtime: &TokenStream, name: &str, out: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Element(element) => {
                hot_literals(&mut element.children, runtime, name, out);
                for attr in &mut element.attrs {
                    if let AttrValue::View(slot) = &mut attr.value {
                        hot_literals(slot, runtime, name, out);
                    }
                }
            }
            Node::Mixed(parts, _)
                if parts
                    .iter()
                    .all(|part| matches!(part, TextPart::Literal(_))) =>
            {
                let text: String = parts
                    .iter()
                    .map(|part| match part {
                        TextPart::Literal(text) => text.as_str(),
                        TextPart::Expr(_) => "",
                    })
                    .collect();
                let index = out.len();
                out.push(text);
                *node = Node::Verbatim(quote! {
                    move || #runtime::view::__hot_text(#name, #index, __NANA_HOT[#index])
                });
            }
            _ => {}
        }
    }
}

impl Compiler {
    /// `runtime` is the path of `nana-ui-runtime` in the generated code,
    /// e.g. `::nana_ui::runtime` or `::nana_ui_runtime`.
    pub fn new(runtime: &str) -> Self {
        Self {
            runtime: runtime.parse().expect("runtime path tokenizes"),
            hot: false,
        }
    }

    /// Development builds: static text reads a per-view table the running
    /// application can replace, so editing text in a `.vue` file shows
    /// without a rebuild (`nana-ui-dev`'s `watch_templates`). Each view
    /// carries a shape hash of everything else; a change there still needs
    /// a rebuild. Costs a tracked read per text node.
    pub fn hot(mut self, hot: bool) -> Self {
        self.hot = hot;
        self
    }

    /// Hot mode's view of `sources`: each view's shape and static text, as
    /// [`Self::compile`] embeds them. Compile the same batch the binary was
    /// built from, so shapes match.
    pub fn hot_views(&self, sources: &[(String, String)]) -> Result<Vec<HotView>, Error> {
        let components = sources
            .iter()
            .map(|(file, text)| parse::component(file, text).map(|c| (file.as_str(), c)))
            .collect::<Result<Vec<_>, _>>()?;
        let known: HashMap<String, Vec<PatType>> = components
            .iter()
            .map(|(_, c)| (c.name.clone(), c.props.clone()))
            .collect();
        let hot = Self {
            runtime: self.runtime.clone(),
            hot: true,
        };
        components
            .into_iter()
            .map(|(file, component)| {
                let name = component.name.clone();
                let (_, _, _, state) = hot.component(file, component, &known)?;
                let (shape, literals) = state.expect("hot mode records the state");
                Ok(HotView {
                    name,
                    shape,
                    literals,
                })
            })
            .collect()
    }

    /// Compile `(file name, source)` pairs as one batch: views may use each
    /// other by tag, with arguments matched by prop name.
    pub fn compile(&self, sources: &[(String, String)]) -> Result<Output, Error> {
        let components = sources
            .iter()
            .map(|(file, text)| parse::component(file, text).map(|c| (file.as_str(), c)))
            .collect::<Result<Vec<_>, _>>()?;
        let known: HashMap<String, Vec<PatType>> = components
            .iter()
            .map(|(_, c)| (c.name.clone(), c.props.clone()))
            .collect();
        let mut items = Vec::new();
        let mut report = String::new();
        let mut warnings = Vec::new();
        for (file, component) in components {
            let (item, section, found, _) = self.component(file, component, &known)?;
            items.push(item);
            report.push_str(&section);
            warnings.extend(found.into_iter().map(|w| format!("{file}: {w}")));
        }
        let runtime = &self.runtime;
        // Signatures (`Signal<T>`, `impl IntoView`) resolve at module scope.
        let file: syn::File = syn::parse2(quote! {
            #[allow(unused_imports)]
            use #runtime::view::*;
            #(#items)*
        })
        .map_err(|error| Error {
            file: "<generated>".into(),
            line: 0,
            column: 0,
            message: error.to_string(),
        })?;
        Ok(Output {
            code: prettyplease::unparse(&file),
            report,
            warnings,
        })
    }

    /// For build scripts: compile every `.vue` file in `dir` into
    /// `$OUT_DIR/nana_views.rs` (and the report into
    /// `$OUT_DIR/nana_views.deps.md`), print warnings through Cargo, and ask
    /// to rerun when a view changes.
    pub fn build(&self, dir: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
        let dir = dir.as_ref();
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "vue"))
            .collect();
        files.sort();
        let mut sources = Vec::new();
        for path in files {
            println!("cargo:rerun-if-changed={}", path.display());
            sources.push((path.display().to_string(), std::fs::read_to_string(&path)?));
        }
        let output = self.compile(&sources)?;
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
        std::fs::write(out.join("nana_views.rs"), output.code)?;
        std::fs::write(out.join("nana_views.deps.md"), output.report)?;
        for warning in output.warnings {
            println!("cargo:warning={warning}");
        }
        Ok(())
    }

    fn component(
        &self,
        file: &str,
        component: Component,
        known: &HashMap<String, Vec<PatType>>,
    ) -> Result<CompiledView, Error> {
        let Component {
            name,
            props,
            script,
            mut template,
            style,
        } = component;
        let mut literals = Vec::new();
        if self.hot {
            hot_literals(&mut template, &self.runtime, &name, &mut literals);
        }
        let mut analysis = Analysis::new(&props, &script);
        analysis.scan_script(&script);
        count(&mut template, &mut analysis);
        if let Some(cycle) = analysis.cycles() {
            let line = analysis
                .tracked
                .iter()
                .find(|t| t.name == cycle[0])
                .map_or(1, |t| t.line);
            return Err(Error {
                file: file.to_owned(),
                line,
                column: 1,
                message: format!("computeds read each other in a loop: {}", cycle.join(" → ")),
            });
        }
        analysis.fold();
        analysis.finish();
        // Classes against the view's `<style>`, after the analysis has seen
        // their conditions and before bindings are rewritten.
        let nana_ui_view_codegen::CompiledStyles {
            items: style_items,
            warnings: style_warnings,
        } = nana_ui_view_codegen::compile_styles(
            style.as_ref().map_or("", |style| style.css.as_str()),
            &mut template,
            &self.runtime,
        );
        let mut rows = Vec::new();
        let mut rewrite = Rewrite {
            file,
            runtime: &self.runtime,
            analysis: &analysis,
            known,
            rows: &mut rows,
        };
        rewrite.nodes(&mut template)?;
        // Named slots left on builtin elements (the known components' are
        // arguments now) become `.name(view)`, as in `view!`.
        nana_ui_view_codegen::lift_slots(&mut template)
            .map_err(|error| parse::syn_error(file, error))?;
        let (body, lints) = nana_ui_view_codegen::expand_checked(&self.runtime, &template)
            .map_err(|error| parse::syn_error(file, error))?;
        let mut warnings = analysis.warnings.clone();
        warnings.extend(style_warnings.into_iter().map(|warning| {
            let (line, column) = match warning.at {
                nana_ui_view_codegen::StyleAt::Sheet(range) => style
                    .as_ref()
                    .map_or((1, 1), |style| style.position(range.start)),
                nana_ui_view_codegen::StyleAt::Template(span) => {
                    let start = span.start();
                    (start.line, start.column + 1)
                }
            };
            format!("{line}:{column}: {}", warning.message)
        }));
        warnings.extend(lints.into_iter().map(|lint| {
            let start = lint.span.start();
            format!("{}:{}: {}", start.line, start.column + 1, lint.message)
        }));
        let runtime = &self.runtime;
        let script: Vec<Stmt> = script
            .into_iter()
            .enumerate()
            .map(|(index, stmt)| match analysis.is_folded(index) {
                Some(kind) => fold_stmt(stmt, kind, runtime),
                None => stmt,
            })
            .collect();
        let function = nana_ui_view_codegen::function_ident(&name, Span::call_site());
        let (hot, state) = if self.hot {
            // Everything the view is, but its text: equal shapes differ in
            // text only.
            // Sites name the file as the batch did; hash its name alone, so
            // a watcher reading the directory by another path agrees.
            let file_name = Path::new(file)
                .file_name()
                .map_or(file.into(), |name| name.to_string_lossy());
            let shape = stable_hash(
                &quote! {
                    fn #function(#(#props),*) { #(#style_items)* #(#script)* #body }
                }
                .to_string()
                .replace(file, &file_name),
            );
            let hot = quote! {
                const __NANA_HOT: &[&str] = &[#(#literals),*];
                #runtime::view::__hot_register(#name, #shape);
            };
            (hot, Some((shape, literals)))
        } else {
            (TokenStream::new(), None)
        };
        let item = quote! {
            #[allow(unused_imports, unused_variables, clippy::all)]
            pub fn #function(#(#props),*) -> impl #runtime::view::IntoView {
                use #runtime::view::*;
                #(#style_items)*
                #hot
                #(#script)*
                #body
            }
        };
        let section = report(file, &name, &analysis, &rows);
        Ok((item, section, warnings, state))
    }
}

/// `let x = signal(v)` → `let x = constant(v)`; a folded computed runs its
/// closure once: `constant((f)())`. An annotation `Signal<T>` becomes
/// `Const<T>`.
fn fold_stmt(stmt: Stmt, kind: Kind, runtime: &TokenStream) -> Stmt {
    let Stmt::Local(mut local) = stmt else {
        return stmt;
    };
    if let Pat::Type(typed) = &mut local.pat
        && let syn::Type::Path(path) = &mut *typed.ty
        && let Some(last) = path.path.segments.last_mut()
    {
        last.ident = Ident::new("Const", last.ident.span());
    }
    if let Some(init) = &mut local.init
        && let Expr::Call(call) = &*init.expr
    {
        let args = &call.args;
        *init.expr = if kind == Kind::Computed {
            syn::parse_quote!(#runtime::view::constant((#args)()))
        } else {
            syn::parse_quote!(#runtime::view::constant(#args))
        };
    }
    Stmt::Local(local)
}

fn tag(element: &Element) -> String {
    element.name.to_string()
}

/// A tag the code generator builds itself (its control table); anything
/// else is a component.
fn is_builtin(element: &Element) -> bool {
    nana_ui_view_codegen::is_builtin(&tag(element))
}

/// First pass: count how every template expression uses the tracked names.
fn count(nodes: &mut [Node], analysis: &mut Analysis) {
    for node in nodes {
        match node {
            Node::Element(element) => {
                let component = !is_builtin(element);
                for attr in &element.attrs {
                    match (&attr.name, &attr.value) {
                        (AttrName::Plain(name), AttrValue::Expr(expr)) => {
                            if component
                                || nana_ui_view_codegen::is_argument(
                                    &tag(element),
                                    &name.to_string(),
                                )
                            {
                                analysis.scan_use(expr, &[Use::Escape]);
                            } else {
                                analysis.count_binding(expr);
                            }
                        }
                        (AttrName::Event(_) | AttrName::On(_), AttrValue::Expr(expr)) => {
                            analysis.scan_use(expr, &[Use::Escape]);
                        }
                        (AttrName::Directive(directive, _), AttrValue::Expr(expr)) => {
                            if directive == "model" {
                                analysis.scan_use(expr, &[Use::Write, Use::Read]);
                            } else {
                                analysis.count_binding(expr);
                            }
                        }
                        (AttrName::Directive(_, _), AttrValue::For(_, source)) => {
                            analysis.scan_use(source, &[Use::Read]);
                        }
                        _ => {}
                    }
                }
                count(&mut element.children, analysis);
            }
            Node::Mixed(parts, _) => {
                for part in parts {
                    if let TextPart::Expr(expr) = part {
                        analysis.count_binding(expr);
                    }
                }
            }
            Node::Expr(expr) => analysis.scan_use(expr, &[Use::Escape]),
            Node::Text(_) | Node::Verbatim(_) => {}
        }
    }
}

struct Row {
    site: String,
    what: String,
    class: Class,
}

/// Second pass: rewrite bindings by class and match component arguments.
struct Rewrite<'a> {
    file: &'a str,
    runtime: &'a TokenStream,
    analysis: &'a Analysis,
    known: &'a HashMap<String, Vec<PatType>>,
    rows: &'a mut Vec<Row>,
}

fn site(file: &str, span: Span) -> String {
    let start = span.start();
    format!("{}:{}:{}", file, start.line, start.column + 1)
}

impl Rewrite<'_> {
    fn nodes(&mut self, nodes: &mut [Node]) -> Result<(), Error> {
        for node in nodes.iter_mut() {
            match node {
                Node::Element(element) => self.element(element)?,
                Node::Mixed(..) => {
                    if let Some(value) = self.text(node, "文本") {
                        *node = Node::Verbatim(value);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn element(&mut self, element: &mut Element) -> Result<(), Error> {
        let name = tag(element);
        if let Some(props) = self.known.get(&name) {
            self.component(element, props)?;
            return self.nodes(&mut element.children);
        }
        // A Rust function component's arguments pass as written; its
        // conditions are still bindings.
        let builtin = is_builtin(element);
        for attr in &mut element.attrs {
            let label = match &attr.name {
                AttrName::Plain(ident)
                    if builtin && !nana_ui_view_codegen::is_argument(&name, &ident.to_string()) =>
                {
                    format!("<{name} :{ident}>")
                }
                AttrName::Directive(directive, _)
                    if matches!(directive.as_str(), "if" | "else-if")
                        || (builtin && directive == "show") =>
                {
                    format!("<{name} v-{directive}>")
                }
                _ => continue,
            };
            if let AttrValue::Expr(expr) = &attr.value
                && let Some(value) = self.binding(expr, label)
            {
                attr.value = AttrValue::Verbatim(value);
            }
        }
        // A text-bearing element's one child is its value.
        if builtin
            && matches!(name.as_str(), "Text" | "Button" | "Checkbox")
            && let [child @ Node::Mixed(..)] = element.children.as_mut_slice()
        {
            if let Some(value) = self.text(child, &format!("<{name}> 文本")) {
                *child = Node::Verbatim(value);
            }
            return Ok(());
        }
        self.nodes(&mut element.children)
    }

    /// A binding value rewritten by class, or `None` to leave it to the
    /// code generator's own rule.
    fn binding(&mut self, expr: &Expr, what: String) -> Option<TokenStream> {
        let class = self.analysis.classify(expr);
        let runtime = self.runtime;
        let value = match &class {
            Class::Value if matches!(expr, Expr::Lit(_) | Expr::Path(_) | Expr::Field(_)) => None,
            Class::Value => Some(quote!(#runtime::view::Fixed(#expr))),
            Class::Direct(_) | Class::Dynamic => None,
            Class::Static(deps) => Some(self.checked(expr.span(), deps, quote!(#expr))),
        };
        self.rows.push(Row {
            site: site(self.file, expr.span()),
            what,
            class,
        });
        value
    }

    /// Mixed text rewritten by class; `None` keeps it dynamic.
    fn text(&mut self, node: &Node, what: &str) -> Option<TokenStream> {
        let Node::Mixed(parts, span) = node else {
            return None;
        };
        let exprs: Vec<&Expr> = parts
            .iter()
            .filter_map(|part| match part {
                TextPart::Expr(expr) => Some(expr),
                TextPart::Literal(_) => None,
            })
            .collect();
        let first = exprs.first()?;
        let class = self.analysis.classify_all(&exprs);
        let runtime = self.runtime;
        let format = nana_ui_view_codegen::format_parts(parts, *span);
        let value = match &class {
            Class::Value => Some(quote!(#runtime::view::Fixed(#format))),
            Class::Static(deps) => Some(self.checked(first.span(), deps, format)),
            // `classify_all` answers `Static` for a lone signal.
            Class::Direct(_) | Class::Dynamic => None,
        };
        self.rows.push(Row {
            site: site(self.file, first.span()),
            what: what.to_owned(),
            class,
        });
        value
    }

    fn checked(&self, span: Span, deps: &[String], value: TokenStream) -> TokenStream {
        let runtime = self.runtime;
        let at = site(self.file, span);
        let deps = deps.iter().map(|name| format_ident!("{}", name));
        quote!(#runtime::view::__checked(#at, [#(#deps.dep()),*], move || #value))
    }

    /// Match a known view's arguments by prop name: `@done` fills
    /// `on_done`, `<template #header>` fills `header`, the other children
    /// fill `children`.
    fn component(&mut self, element: &mut Element, props: &[PatType]) -> Result<(), Error> {
        let name = tag(element);
        let mut values: HashMap<String, AttrValue> = HashMap::new();
        let (slots, rest): (Vec<Node>, Vec<Node>) = std::mem::take(&mut element.children)
            .into_iter()
            .partition(|child| matches!(child, Node::Element(e) if e.name == "template"));
        element.children = rest;
        for slot in slots {
            let Node::Element(mut template) = slot else {
                unreachable!("partitioned on elements");
            };
            let Some(slot) = template.attrs.iter().find_map(|attr| match &attr.name {
                AttrName::Directive(directive, _) => directive.strip_prefix("slot:"),
                _ => None,
            }) else {
                return Err(parse::syn_error(
                    self.file,
                    syn::Error::new(template.name.span(), "`<template>` here needs `#slot-name`"),
                ));
            };
            let slot = match slot.replace('-', "_") {
                default if default == "default" => String::from("children"),
                other => other,
            };
            self.nodes(&mut template.children)?;
            values.insert(slot, AttrValue::View(template.children));
        }
        let error = |message: String| {
            parse::syn_error(self.file, syn::Error::new(element.name.span(), message))
        };
        let mut kept = Vec::new();
        for attr in std::mem::take(&mut element.attrs) {
            if matches!(&attr.name, AttrName::Plain(ident) if ident == "key")
                || matches!(attr.name, AttrName::Directive(..))
            {
                kept.push(attr);
                continue;
            }
            match attr.name {
                AttrName::Plain(ident) => {
                    let value = match attr.value {
                        AttrValue::Lit(lit) => {
                            AttrValue::Verbatim(quote!(::core::convert::Into::into(#lit)))
                        }
                        other => other,
                    };
                    values.insert(ident.to_string(), value);
                }
                AttrName::Event(event) => {
                    let handler = nana_ui_view_codegen::handler(&attr.value, event.span())
                        .map_err(|e| parse::syn_error(self.file, e))?;
                    values.insert(format!("on_{event}"), AttrValue::Verbatim(handler));
                }
                AttrName::On(_) => {
                    return Err(error(format!(
                        "`on:` is for builtin elements; declare an `on_…` prop on `{name}`"
                    )));
                }
                AttrName::Directive(..) => unreachable!("kept above"),
            }
        }
        let mut ordered = Vec::new();
        let mut wants_children = false;
        for (index, prop) in props.iter().enumerate() {
            let Pat::Ident(pat) = &*prop.pat else {
                return Err(error(format!("`{name}` has a prop that is not a name")));
            };
            let prop_name = pat.ident.to_string();
            if prop_name == "children" {
                if index + 1 != props.len() {
                    return Err(error(format!("`{name}`: `children` must be the last prop")));
                }
                wants_children = true;
                if let Some(AttrValue::View(nodes)) = values.remove("children") {
                    if !element.children.is_empty() {
                        return Err(error(format!(
                            "`<{name}>` has both `#default` and other children"
                        )));
                    }
                    element.children = nodes;
                }
                continue;
            }
            let value = values
                .remove(&prop_name)
                .ok_or_else(|| error(format!("`<{name}>` is missing `:{prop_name}`")))?;
            ordered.push(Attr {
                name: AttrName::Plain(Ident::new(&prop_name, element.name.span())),
                value,
            });
        }
        if let Some(extra) = values.keys().next() {
            return Err(error(format!("`{name}` has no prop `{extra}`")));
        }
        if wants_children && element.children.is_empty() {
            element.children.push(Node::Verbatim(quote!(())));
        }
        if !wants_children && !element.children.is_empty() {
            return Err(error(format!(
                "`{name}` takes no children; declare a `children` prop"
            )));
        }
        ordered.extend(kept);
        element.attrs = ordered;
        Ok(())
    }
}

fn report(file: &str, name: &str, analysis: &Analysis, rows: &[Row]) -> String {
    let mut out = format!("## {name}（`{file}`）\n\n");
    if !analysis.tracked.is_empty() {
        out.push_str(
            "| 名字 | 种类 | 读 | 写 | 传出 | 结果 |\n| --- | --- | --- | --- | --- | --- |\n",
        );
        for tracked in &analysis.tracked {
            let kind = match tracked.kind {
                Kind::Signal => "signal",
                Kind::Computed => "computed",
                Kind::Prop => "prop",
                Kind::Store => "store",
            };
            let result = if tracked.folded {
                "折叠为常量"
            } else {
                "保留"
            };
            out.push_str(&format!(
                "| `{}` | {kind} | {} | {} | {} | {result} |\n",
                tracked.name, tracked.usage.reads, tracked.usage.writes, tracked.usage.escapes
            ));
        }
        out.push('\n');
    }
    if !rows.is_empty() {
        out.push_str("| 位置 | 绑定 | 分类 |\n| --- | --- | --- |\n");
        for row in rows {
            out.push_str(&format!(
                "| {} | `{}` | {} |\n",
                row.site,
                row.what,
                row.class.label()
            ));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
