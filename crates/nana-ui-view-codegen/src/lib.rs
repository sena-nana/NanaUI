//! The code generator behind `view!` and the `.vue` dialect compiler: a
//! template's element tree becomes the declarative view function calls of
//! `nana-ui-runtime` (`column`, `text`, `each`, `when`, …). Nothing here adds
//! a runtime concept; the output is the call chain a person would write.

mod style;

pub use style::{
    CompiledStyles, StyleAt, StyleWarning, compile_inline, compile_styles, compile_stylesheet,
};

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Ident, Lit, LitStr, Pat};

/// A built-in control, from the table in `nana-ui-view-schema`.
pub struct Control {
    pub tag: &'static str,
    pub function: &'static str,
    /// Element-function arguments: `text`, `f32` or `f64`.
    pub arguments: &'static [(&'static str, &'static str)],
    /// Bindable fields and their types.
    pub fields: &'static [(&'static str, &'static str)],
    /// Event methods (`on_activate`), and whether their handler takes the
    /// event.
    pub events: &'static [(&'static str, bool)],
    pub model: bool,
}

impl Control {
    fn field(&self, name: &str) -> Option<&'static str> {
        self.fields
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, ty)| *ty)
    }

    fn field_list(&self) -> String {
        let names: Vec<_> = self
            .fields
            .iter()
            .map(|(name, _)| format!("`{name}`"))
            .collect();
        if names.is_empty() {
            String::from("none")
        } else {
            names.join(", ")
        }
    }
}

macro_rules! table {
    ($(
        $tag:ident => $function:ident ($($argument:ident: $kind:ident),*) for $component:ident {
            $($field:ident: $ty:ty = $write:ident),* $(,)?
        }
        $(on { $($on:ident: $on_event:ident),* $(,)? })?
        $(with { $($with:ident: $with_event:ident),* $(,)? })?
        $(model $model:ident: $model_ty:ty => $model_event:ident |$event:ident| $from_event:expr)?
        ;
    )*) => {
        /// Every built-in control the templates know by tag.
        pub const CONTROLS: &[Control] = &[$(Control {
            tag: stringify!($tag),
            function: stringify!($function),
            arguments: &[$((stringify!($argument), stringify!($kind))),*],
            fields: &[$((stringify!($field), stringify!($ty))),*],
            events: &[$($((stringify!($on), false)),*)? $($((stringify!($with), true)),*)?],
            model: table!(@model $($model)?),
        }),*];
    };
    (@model) => { false };
    (@model $model:ident) => { true };
}

nana_ui_view_schema::for_each_control!(table);

/// The built-in control a tag names.
pub fn control(tag: &str) -> Option<&'static Control> {
    CONTROLS.iter().find(|control| control.tag == tag)
}

/// Whether `tag` is built in: a control, `T` (localized text), `Column`,
/// `Row`, `Widget`, `Block`, `Virtual`, `Transition`, `TransitionGroup`,
/// `KeepAlive`, `Suspense`, `Teleport` or `ErrorBoundary`.
pub fn is_builtin(tag: &str) -> bool {
    matches!(
        tag,
        "T" | "Column"
            | "Row"
            | "Widget"
            | "Block"
            | "Virtual"
            | "Transition"
            | "TransitionGroup"
            | "KeepAlive"
            | "Suspense"
            | "Teleport"
            | "ErrorBoundary"
    ) || control(tag).is_some()
}

/// Attributes of a built-in `tag` that are construction arguments rather
/// than bindings: numbers the element function takes, `gap`, `of`, `key`,
/// `ref`, `labelled_by`.
pub fn is_argument(tag: &str, attribute: &str) -> bool {
    matches!(attribute, "key" | "ref" | "labelled_by")
        || match tag {
            "Column" | "Row" => attribute == "gap",
            "Widget" => attribute == "of",
            "Block" | "Virtual" | "Transition" | "TransitionGroup" | "KeepAlive" | "Suspense"
            | "Teleport" => true,
            _ => control(tag).is_some_and(|control| {
                control
                    .arguments
                    .iter()
                    .any(|(name, kind)| *name == attribute && *kind != "text")
            }),
        }
}

/// What `<T>` gives its node rather than its message: what every element
/// takes, the class its `<style>` compiled, and `Text`'s `decorative`. The
/// theme roles ([`STYLE_ROLES`]) are its node's too.
const T_NODE_ATTRIBUTES: &[&str] = &["key", "ref", "labelled_by", "locale", "class", "decorative"];

/// Whether `attribute` of `tag` is part of the message `<T>` says: `id`,
/// which names the message, or one of its arguments, which is every
/// attribute its node does not take.
pub fn is_message_part(tag: &str, attribute: &str) -> bool {
    tag == "T" && !T_NODE_ATTRIBUTES.contains(&attribute) && !STYLE_ROLES.contains(&attribute)
}

/// What `<T id="files" count={n}>` says, as the expression
/// `LocalizedText::new("files").arg("count", n)`: `id` names the message and
/// every other part of it is an argument, in the order written. A string is
/// a constant. An expression goes through `read` first, which writes the
/// value a signal it names holds (the `.vue` compiler writes `n` as
/// `n.get()`); left as it is, a path or a field is cloned, so the expression
/// can run again in a closure.
pub fn localized_text(
    krate: &TokenStream,
    element: &Element,
    read: &dyn Fn(&Expr) -> Option<TokenStream>,
) -> syn::Result<TokenStream> {
    let span = element.name.span();
    let mut key = None;
    let mut args = Vec::new();
    let mut names = Vec::new();
    for attr in &element.attrs {
        let AttrName::Plain(name) = &attr.name else {
            continue;
        };
        let text = name.to_string();
        if !is_message_part("T", &text) {
            continue;
        }
        let at = name.span();
        if text == "id" {
            key = Some(match &attr.value {
                AttrValue::Lit(Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(key), ..
                })) if !key.value().trim().is_empty() => quote!(#key),
                AttrValue::Expr(expr) => {
                    let key = read(expr).unwrap_or_else(|| quote!(#expr));
                    quote_spanned!(expr.span()=> &(#key))
                }
                AttrValue::Verbatim(key) => quote!(&(#key)),
                _ => {
                    return Err(syn::Error::new(
                        at,
                        "`id` is the key of the message: a string that is not empty, or an \
                         expression choosing one",
                    ));
                }
            });
            continue;
        }
        if names.contains(&text) {
            return Err(syn::Error::new(
                at,
                format!("`<T>` has the argument `{text}` twice"),
            ));
        }
        let value = match &attr.value {
            AttrValue::Lit(literal) => quote!(#literal),
            AttrValue::Expr(expr) => read(expr).unwrap_or_else(|| argument(krate, expr)),
            AttrValue::Verbatim(value) => value.clone(),
            AttrValue::None | AttrValue::For(..) | AttrValue::View(_) => {
                return Err(syn::Error::new(
                    at,
                    format!("the argument `{text}` of `<T>` needs a value: `{text}=\"…\"`"),
                ));
            }
        };
        let name = LitStr::new(&text, at);
        args.push(quote_spanned!(at=> .arg(#name, #value)));
        names.push(text);
    }
    let key = key.ok_or_else(|| {
        syn::Error::new(
            span,
            "`<T>` needs `id=\"…\"`, the key of the message it says",
        )
    })?;
    Ok(quote_spanned!(span=> #krate::LocalizedText::new(#key) #(#args)*))
}

/// A message argument as a value the closure it may sit in can give again:
/// a path or a field cloned (`count`, `todo.title`), anything else as it is.
fn argument(krate: &TokenStream, expr: &Expr) -> TokenStream {
    match expr {
        Expr::Path(_) | Expr::Field(_) => {
            quote_spanned!(expr.span()=> #krate::view::__arg(&#expr))
        }
        _ => quote!(#expr),
    }
}

/// `locale=` as `El::locale` takes it: a language tag, checked here
/// (`locale="ar"`), or a `Locale`, a signal or a closure, as any prop.
/// Errors are at `span`, the attribute's name.
fn locale(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Lit(Expr::Lit(syn::ExprLit {
            lit: Lit::Str(tag), ..
        })) => {
            if is_language_tag(tag.value().trim()) {
                Ok(quote!(#tag))
            } else {
                Err(syn::Error::new(
                    span,
                    format!(
                        "`locale=\"{}\"` is not a language tag such as `ar` or `zh-CN`",
                        tag.value()
                    ),
                ))
            }
        }
        AttrValue::None => Err(syn::Error::new(
            span,
            "`locale` needs a language tag: `locale=\"ar\"`",
        )),
        value => Ok(prop(value)),
    }
}

/// Whether `tag` is shaped like a BCP 47 language tag: subtags of one to
/// eight ASCII letters or digits joined by `-` (or `_`), the first of them
/// letters.
fn is_language_tag(tag: &str) -> bool {
    let fits = |subtag: &str, digits: bool| {
        (1..=8).contains(&subtag.len())
            && subtag
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || (digits && byte.is_ascii_digit()))
    };
    let mut subtags = tag.split(['-', '_']);
    subtags.next().is_some_and(|first| fits(first, false))
        && subtags.all(|subtag| fits(subtag, true))
}

/// `locale` makes an element a locale scope. The blocks a template wraps
/// around a chain or a list, and the views it builds from one, are not
/// elements: refuse it on them rather than drop it.
fn reject_locale(element: &Element) -> syn::Result<()> {
    let Some(Attr {
        name: AttrName::Plain(name),
        ..
    }) = element.plain("locale")
    else {
        return Ok(());
    };
    Err(syn::Error::new(
        name.span(),
        format!(
            "`<{}>` takes no `locale`: it is not an element. Put `locale` on the element it \
             holds, or on one around it (`<Column locale=\"ar\">`)",
            element.name
        ),
    ))
}

/// Expand `nodes` into one view expression. `krate` is the path of
/// `nana-ui-runtime` as seen from the generated code.
pub fn expand(krate: &TokenStream, nodes: &[Node]) -> syn::Result<TokenStream> {
    Ok(expand_checked(krate, nodes)?.0)
}

/// A problem in a template that still compiles: where, and what.
#[derive(Debug)]
pub struct Warning {
    pub span: Span,
    pub message: String,
}

/// [`expand`], with the warnings the template earns: controls a screen
/// reader could not name.
pub fn expand_checked(
    krate: &TokenStream,
    nodes: &[Node],
) -> syn::Result<(TokenStream, Vec<Warning>)> {
    expand_checked_at(krate, nodes, None)
}

/// [`expand_checked`] with an optional source file for SFC-generated nodes.
/// The source is metadata only; ordinary Rust/view! callers keep the
/// `Location::caller()` path.
pub fn expand_checked_at(
    krate: &TokenStream,
    nodes: &[Node],
    source_file: Option<&str>,
) -> syn::Result<(TokenStream, Vec<Warning>)> {
    let generator = Gen {
        krate,
        source_file,
        warnings: std::cell::RefCell::new(Vec::new()),
    };
    let tokens = generator.nodes(nodes)?;
    Ok((tokens, generator.warnings.into_inner()))
}

/// Theme roles every element may bind (`El::foreground` and the rest):
/// colours a stylesheet cannot give, since it is compiled for one theme.
pub const STYLE_ROLES: &[&str] = &["foreground", "background", "border", "radius"];

/// Controls whose accessible name is their `label` field and that have no
/// other text to fall back on.
const NAMED_BY_LABEL: &[&str] = &["TextInput", "TextArea", "NumberInput", "Slider", "Progress"];

/// One template node.
pub enum Node {
    Element(Element),
    /// A string in `view!` syntax: `{name}` interpolates like `format!`.
    Text(LitStr),
    /// Text with `{{ expression }}` parts (`.vue` templates).
    Mixed(Vec<TextPart>, Span),
    /// `{expression}`: any view.
    Expr(Expr),
    /// Text whose value a front end already wrote as a prop (a constant, a
    /// closure, a checked binding).
    Verbatim(TokenStream, Span),
}

/// A piece of [`Node::Mixed`] text.
// Compile-time syntax, built once per template: variant size does not matter.
#[allow(clippy::large_enum_variant)]
pub enum TextPart {
    Literal(String),
    Expr(Expr),
}

fn node_span(node: &Node) -> Span {
    match node {
        Node::Element(element) => element.name.span(),
        Node::Text(text) => text.span(),
        Node::Mixed(_, span) | Node::Verbatim(_, span) => *span,
        Node::Expr(expr) => expr.span(),
    }
}

pub struct Element {
    /// The path before the name: `kit` in `<kit::EmptyState>`. A tag with a
    /// path always calls the component function it names, even when its
    /// last segment is also a built-in tag.
    pub module: Vec<Ident>,
    pub name: Ident,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

pub enum AttrName {
    /// `name=value`
    Plain(Ident),
    /// `@event={…}`
    Event(Ident),
    /// `on:Type={|e| …}`
    On(syn::Path),
    /// `v-if`, `v-else-if`, `v-else`, `v-for`, `v-show`, `v-model`
    Directive(String, Span),
}

#[allow(clippy::large_enum_variant)]
pub enum AttrValue {
    None,
    Lit(Expr),
    Expr(Expr),
    For(Pat, Expr),
    /// A value a front end already wrote as a prop.
    Verbatim(TokenStream),
    /// A view argument of a component (a `.vue` named slot's content), or
    /// under `slot:name` a named slot's content ([`lift_slots`]).
    View(Vec<Node>),
}

pub struct Attr {
    pub name: AttrName,
    pub value: AttrValue,
}

/// Move each `<template #name>…</template>` child onto its element as a
/// `slot:name` directive holding that content, which expands to
/// `.name(view)`: the call the Rust spelling writes (`.navigation(…)`,
/// `.control(…)`). `#default` content joins the other children.
/// `<Suspense>` keeps its templates: its `#fallback` is not a method.
pub fn lift_slots(nodes: &mut [Node]) -> syn::Result<()> {
    for node in nodes {
        let Node::Element(element) = node else {
            continue;
        };
        for attr in &mut element.attrs {
            if let AttrValue::View(slot) = &mut attr.value {
                lift_slots(slot)?;
            }
        }
        lift_slots(&mut element.children)?;
        if element.module.is_empty() && element.name == "Suspense" {
            continue;
        }
        let mut children = Vec::new();
        for child in std::mem::take(&mut element.children) {
            let Node::Element(template) = child else {
                children.push(child);
                continue;
            };
            if template.name != "template" {
                children.push(Node::Element(template));
                continue;
            }
            let slot = template.attrs.iter().find_map(|attr| match &attr.name {
                AttrName::Directive(directive, span) => directive
                    .strip_prefix("slot:")
                    .map(|name| (name.to_owned(), *span)),
                _ => None,
            });
            match slot {
                Some((name, _)) if name == "default" => children.extend(template.children),
                Some((name, span)) => element.attrs.push(Attr {
                    name: AttrName::Directive(format!("slot:{name}"), span),
                    value: AttrValue::View(template.children),
                }),
                None => {
                    return Err(syn::Error::new(
                        template.name.span(),
                        "`<template>` here needs `#slot-name`",
                    ));
                }
            }
        }
        element.children = children;
    }
    Ok(())
}

/// `.name(view)` for each named slot of `element`, after `out`.
fn slot_calls(
    generator: &Gen<'_>,
    element: &Element,
    mut out: TokenStream,
) -> syn::Result<TokenStream> {
    for attr in &element.attrs {
        if let (AttrName::Directive(directive, span), AttrValue::View(nodes)) =
            (&attr.name, &attr.value)
            && let Some(name) = directive.strip_prefix("slot:")
        {
            let method = Ident::new(&name.replace('-', "_"), *span);
            let body = generator.nodes(nodes)?;
            out = quote_spanned!(*span=> #out.#method(#body));
        }
    }
    Ok(out)
}

/// A closure literal is passed through; a path or field (a signal, a value)
/// is passed as is; anything else is re-evaluated when what it reads
/// changes.
fn prop(value: &AttrValue) -> TokenStream {
    match value {
        AttrValue::None => quote!(true),
        AttrValue::Lit(lit) => quote!(#lit),
        AttrValue::Expr(Expr::Closure(closure)) => quote!(#closure),
        AttrValue::Expr(expr @ (Expr::Path(_) | Expr::Lit(_) | Expr::Field(_))) => quote!(#expr),
        AttrValue::Expr(expr) => quote_spanned!(expr.span()=> move || #expr),
        AttrValue::Verbatim(tokens) => tokens.clone(),
        AttrValue::For(..) | AttrValue::View(_) => unreachable!("only a component takes these"),
    }
}

/// A numeric argument of type `ty`: a literal is written with that suffix
/// (`8` → `8_f32`), anything else is passed as is for the compiler to check.
fn number(value: &AttrValue, ty: &str, span: Span) -> syn::Result<TokenStream> {
    match typed_number(value, ty) {
        Some(tokens) => Ok(tokens),
        None => raw(value, span),
    }
}

/// A numeric literal (or a string holding one) written with the suffix of
/// `ty`; `None` for anything else.
fn typed_number(value: &AttrValue, ty: &str) -> Option<TokenStream> {
    fn digits(lit: &Lit) -> Option<String> {
        match lit {
            Lit::Int(int) => Some(int.base10_digits().to_owned()),
            Lit::Float(float) => Some(float.base10_digits().to_owned()),
            _ => None,
        }
    }
    let typed = |lit: &Lit, negative: bool| -> Option<TokenStream> {
        let literal = syn::LitFloat::new(&format!("{}_{ty}", digits(lit)?), lit.span());
        Some(if negative {
            quote!(-#literal)
        } else {
            quote!(#literal)
        })
    };
    match value {
        // A plain `.vue` attribute is a string: `max="100"`.
        AttrValue::Lit(Expr::Lit(syn::ExprLit {
            lit: Lit::Str(text),
            ..
        })) => text.value().trim().parse::<f64>().ok().map(|number| {
            let literal = syn::LitFloat::new(&format!("{}_{ty}", number.abs()), text.span());
            if number.is_sign_negative() {
                quote!(-#literal)
            } else {
                quote!(#literal)
            }
        }),
        AttrValue::Lit(Expr::Lit(expr)) | AttrValue::Expr(Expr::Lit(expr)) => {
            typed(&expr.lit, false)
        }
        AttrValue::Lit(Expr::Unary(unary)) | AttrValue::Expr(Expr::Unary(unary))
            if matches!(unary.op, syn::UnOp::Neg(_)) =>
        {
            match &*unary.expr {
                Expr::Lit(expr) => typed(&expr.lit, true),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A `NodeRef` as `ref` and `labelled_by` take it: a string (`ref="input"`
/// in `.vue`) names the variable, anything else is the expression itself.
fn named_ref(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Lit(Expr::Lit(syn::ExprLit {
            lit: Lit::Str(name),
            ..
        })) => Ok(Ident::new(&name.value(), name.span()).into_token_stream()),
        value => raw(value, span),
    }
}

/// The expression itself, for arguments and handlers.
fn raw(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Lit(expr) | AttrValue::Expr(expr) => Ok(quote!(#expr)),
        AttrValue::Verbatim(tokens) => Ok(tokens.clone()),
        AttrValue::None => Err(syn::Error::new(span, "this attribute needs a value")),
        AttrValue::For(..) => Err(syn::Error::new(span, "unexpected loop")),
        AttrValue::View(_) => Err(syn::Error::new(span, "a slot fills a component argument")),
    }
}

/// An event handler: a closure or function value as is, any other
/// expression run as a statement.
pub fn handler(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    event_handler(value, false, span)
}

/// [`handler`] for an event whose handler takes the event: a statement
/// ignores it.
fn event_handler(value: &AttrValue, takes_event: bool, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Expr(Expr::Closure(closure)) => Ok(quote!(#closure)),
        // A function or closure value: `@activate={add}`.
        AttrValue::Expr(expr @ (Expr::Path(_) | Expr::Field(_))) => Ok(quote!(#expr)),
        AttrValue::Expr(expr) if takes_event => {
            Ok(quote_spanned!(expr.span()=> move |_| { #expr; }))
        }
        AttrValue::Expr(expr) => Ok(quote_spanned!(expr.span()=> move || { #expr; })),
        AttrValue::Verbatim(tokens) => Ok(tokens.clone()),
        _ => Err(syn::Error::new(span, "an event handler is `{expression}`")),
    }
}

/// A string with `{…}` interpolation re-formats when a signal it names
/// changes; a plain string is a constant.
fn string_prop(text: &LitStr) -> TokenStream {
    if interpolates(&text.value()) {
        quote_spanned!(text.span()=> move || ::std::format!(#text))
    } else {
        quote!(#text)
    }
}

/// Whether a `view!` string has `{…}` interpolation (`{{` is an escape).
fn interpolates(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' if bytes.get(index + 1) == Some(&b'{') => index += 2,
            b'{' => return true,
            _ => index += 1,
        }
    }
    false
}

/// `.vue` text: constant when it has no `{{ … }}`, otherwise a closure
/// formatting each expression with `Display` (a signal reads and tracks).
fn mixed_prop(parts: &[TextPart], span: Span) -> TokenStream {
    if let [TextPart::Literal(plain)] = parts {
        let text = LitStr::new(plain, span);
        return quote!(#text);
    }
    let format = format_parts(parts, span);
    quote_spanned!(span=> move || #format)
}

/// `::std::format!(…)` over mixed text: literal parts with braces escaped,
/// each expression formatted with `Display`.
pub fn format_parts(parts: &[TextPart], span: Span) -> TokenStream {
    let mut format = String::new();
    let mut args = Vec::new();
    for part in parts {
        match part {
            TextPart::Literal(text) => format.push_str(&text.replace('{', "{{").replace('}', "}}")),
            TextPart::Expr(expr) => {
                format.push_str("{}");
                args.push(expr);
            }
        }
    }
    let format = LitStr::new(&format, span);
    quote_spanned!(span=> ::std::format!(#format, #(#args),*))
}

/// The function a component tag calls, as an identifier: `TodoRow` →
/// `todo_row`, a keyword such as `Dyn` → `r#dyn`.
pub fn function_ident(tag: &str, span: Span) -> Ident {
    let name = snake_case(tag);
    match syn::parse_str::<Ident>(&name) {
        Ok(_) => Ident::new(&name, span),
        Err(_) => Ident::new_raw(&name, span),
    }
}

/// `TodoRow` → `todo_row`: the function a component tag calls.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// The pattern, source and key of a `v-for` element.
fn loop_parts(element: &Element) -> syn::Result<(&Pat, &Expr, TokenStream)> {
    let span = element.name.span();
    if element.directive("if").is_some() {
        return Err(syn::Error::new(
            span,
            "put `v-if` on a wrapping element or inside the row, not next to `v-for`",
        ));
    }
    let Some(AttrValue::For(pattern, source)) = element.directive("for").map(|each| &each.value)
    else {
        return Err(syn::Error::new(span, "`v-for={item in items}`"));
    };
    let key = element
        .plain("key")
        .ok_or_else(|| syn::Error::new(span, "`v-for` needs `key={…}` naming each item"))?;
    Ok((pattern, source, raw(&key.value, span)?))
}

/// Whether an attribute is a class the view's `<style>` compiled onto the
/// element (`class`, `class:name`).
fn is_class_directive(name: &AttrName) -> bool {
    matches!(name, AttrName::Directive(directive, _)
        if directive == "class" || directive == "class_when")
}

/// A block's compiled classes as calls on the structural view it builds:
/// they style that view's container.
fn container_classes(element: &Element) -> syn::Result<Vec<TokenStream>> {
    element
        .attrs
        .iter()
        .filter_map(|attr| match &attr.name {
            AttrName::Directive(directive, span) if directive == "class" => {
                Some(raw(&attr.value, *span).map(|value| quote_spanned!(*span=> .class(#value))))
            }
            AttrName::Directive(directive, span) if directive == "class_when" => Some(
                raw(&attr.value, *span).map(|value| quote_spanned!(*span=> .class_when(#value))),
            ),
            _ => None,
        })
        .collect()
}

/// Children as one view: nothing, one view, or tuples of at most twelve.
fn fragment(children: Vec<TokenStream>) -> TokenStream {
    match children.len() {
        0 => quote!(()),
        1 => children.into_iter().next().expect("one child"),
        n if n <= 12 => quote!((#(#children,)*)),
        _ => {
            let chunks: Vec<TokenStream> = children
                .chunks(12)
                .map(|chunk| quote!((#(#chunk,)*)))
                .collect();
            fragment(chunks)
        }
    }
}

struct Gen<'a> {
    krate: &'a TokenStream,
    source_file: Option<&'a str>,
    warnings: std::cell::RefCell<Vec<Warning>>,
}

impl Gen<'_> {
    fn nodes(&self, nodes: &[Node]) -> syn::Result<TokenStream> {
        self.node_list(&nodes.iter().collect::<Vec<_>>())
    }

    fn node_list(&self, nodes: &[&Node]) -> syn::Result<TokenStream> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < nodes.len() {
            match nodes[index] {
                Node::Element(element) if element.directive("if").is_some() => {
                    // Collect the v-else-if / v-else siblings of this chain.
                    let mut chain = vec![element];
                    index += 1;
                    while let Some(Node::Element(next)) = nodes.get(index).copied() {
                        let last = next.directive("else").is_some();
                        if !last && next.directive("else-if").is_none() {
                            break;
                        }
                        chain.push(next);
                        index += 1;
                        if last {
                            break;
                        }
                    }
                    out.push(self.chain(&chain, &[], &[])?);
                    continue;
                }
                Node::Element(element)
                    if element.directive("else").is_some()
                        || element.directive("else-if").is_some() =>
                {
                    return Err(syn::Error::new(
                        element.name.span(),
                        "`v-else` must follow a `v-if` or `v-else-if` sibling",
                    ));
                }
                node => out.push(self.node(node)?),
            }
            index += 1;
        }
        Ok(fragment(out))
    }

    fn node(&self, node: &Node) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let value = match node {
            Node::Element(element) => self.element(element)?,
            Node::Text(text) if interpolates(&text.value()) => {
                quote_spanned!(text.span()=> #krate::text!(#text))
            }
            Node::Text(text) => quote_spanned!(text.span()=> #krate::view::text(#text)),
            Node::Mixed(parts, span) => {
                let value = mixed_prop(parts, *span);
                quote_spanned!(*span=> #krate::view::text(#value))
            }
            Node::Expr(expr) => quote!(#expr),
            Node::Verbatim(value, _) => quote!(#krate::view::text(#value)),
        };
        let needs_marker = match node {
            Node::Element(element) => {
                element
                    .attrs
                    .iter()
                    .any(|attr| matches!(&attr.value, AttrValue::Expr(_) | AttrValue::For(_, _)))
                    // A `<T>` whose message a front end wrote holds the
                    // expressions of its arguments there.
                    || (element.name == "T"
                        && matches!(element.children.as_slice(), [Node::Verbatim(..)]))
            }
            Node::Text(_) => false,
            Node::Mixed(..) | Node::Verbatim(..) => true,
            Node::Expr(..) => false,
        };
        Ok(if needs_marker {
            self.marker(node_span(node), value)
        } else {
            value
        })
    }

    /// SFC builds use compile-time-only markers to recover the source range
    /// after prettyplease has rendered tokens into plain text. The markers
    /// are removed by nana-ui-sfc before the generated file is written.
    fn marker(&self, span: Span, value: TokenStream) -> TokenStream {
        let Some(file) = self.source_file else {
            return value;
        };
        let start = span.start();
        let file = file
            .replace('%', "%25")
            .replace('|', "%7C")
            .replace('\\', "%5C");
        let marker = syn::LitStr::new(
            &format!(
                "__NANA_SFC_MARKER__|{file}|{}|{}",
                start.line,
                start.column + 1
            ),
            span,
        );
        quote!({
            const _: &str = #marker;
            #value
        })
    }

    fn source_site(&self, span: Span) -> Option<TokenStream> {
        let file = self.source_file?;
        let start = span.start();
        let file = syn::LitStr::new(file, span);
        let line = start.line as u32;
        let column = (start.column + 1) as u32;
        let krate = self.krate;
        Some(quote_spanned!(span=> #krate::view::SourceLocation::new(#file, #line, #column)))
    }

    /// `v-if` / `v-else-if` / `v-else` → nested `when(..).otherwise(..)`,
    /// each with the `modifiers` of the blocks around the chain
    /// (`.transition(..)`, `.keep_alive()`); the outermost `when`, whose
    /// container holds the chain, also with the `container` styles.
    fn chain(
        &self,
        chain: &[&Element],
        modifiers: &[TokenStream],
        container: &[TokenStream],
    ) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let (first, rest) = chain.split_first().expect("a chain has its v-if");
        let condition = first
            .directive("if")
            .or_else(|| first.directive("else-if"))
            .expect("chain links carry a condition");
        let condition = prop(&condition.value);
        let body = self.element(first)?;
        let mut out = quote!(#krate::view::when(#condition, move || #body));
        if let Some(next) = rest.first() {
            let otherwise = if next.directive("else").is_some() {
                self.element(next)?
            } else {
                self.chain(rest, modifiers, &[])?
            };
            out = quote!(#out.otherwise(move || #otherwise));
        }
        Ok(self.marker(
            first.name.span(),
            quote!(#out #(#modifiers)* #(#container)*),
        ))
    }

    /// One element, with `v-for` wrapping it in `each` when present.
    fn element(&self, element: &Element) -> syn::Result<TokenStream> {
        if element.directive("for").is_none() {
            return self.single(element);
        }
        let krate = self.krate;
        // `v-virtual="row height"`: build only the rows in view;
        // `v-virtual.measured`: rows size to their content.
        if let Some((directive, attr)) = element.attrs.iter().find_map(|attr| match &attr.name {
            AttrName::Directive(directive, _) if directive.split('.').next() == Some("virtual") => {
                Some((directive, attr))
            }
            _ => None,
        }) {
            let height = number(&attr.value, "f32", element.name.span())?;
            let measured = match directive.split_once('.') {
                None => quote!(),
                Some((_, "measured")) => quote!(.measured()),
                Some((_, other)) => {
                    return Err(syn::Error::new(
                        element.name.span(),
                        format!("unknown modifier `v-virtual.{other}`"),
                    ));
                }
            };
            let rows = self.virtual_rows(element, height)?;
            return Ok(quote!(#rows #measured));
        }
        let (pattern, source, key) = loop_parts(element)?;
        let row = self.single(element)?;
        Ok(quote! {
            #krate::view::each(
                #source,
                move |__nana_item: &_| { let #pattern = __nana_item; #key },
                move |#pattern| #row,
            )
        })
    }

    /// `each_virtual` over a `v-for` element's loop, rows `height` tall.
    fn virtual_rows(&self, element: &Element, height: TokenStream) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let (pattern, source, key) = loop_parts(element)?;
        let row = self.single(element)?;
        Ok(quote! {
            #krate::view::each_virtual(
                #source,
                move |__nana_item: &_| { let #pattern = __nana_item; #key },
                #height,
                move |#pattern| #row,
            )
        })
    }

    /// `<Virtual row-height=… height=… measured>` around one `v-for`
    /// element: the list and the scroll area it lives in.
    fn virtual_list(&self, element: &Element) -> syn::Result<TokenStream> {
        let span = element.name.span();
        let [Node::Element(rows)] = element.children.as_slice() else {
            return Err(syn::Error::new(
                span,
                "`<Virtual>` holds one element with `v-for`",
            ));
        };
        if rows.directive("for").is_none() {
            return Err(syn::Error::new(
                rows.name.span(),
                "the element inside `<Virtual>` needs `v-for`",
            ));
        }
        let height = element.plain("row_height").ok_or_else(|| {
            syn::Error::new(
                span,
                "`<Virtual>` needs `row-height=`, the row height or its estimate",
            )
        })?;
        let mut out = self.virtual_rows(rows, number(&height.value, "f32", span)?)?;
        if let Some(grid) = element.plain("grid") {
            let at = span;
            let min_width = number(&grid.value, "f32", at)?;
            let gap = match element.plain("gap") {
                Some(gap) => number(&gap.value, "f32", at)?,
                None => quote!(0_f32),
            };
            out = quote!(#out.grid(#min_width, #gap));
        } else if element.plain("gap").is_some() {
            return Err(syn::Error::new(
                span,
                "`gap=` goes with `grid=` on `<Virtual>`",
            ));
        }
        let classes = container_classes(element)?;
        out = quote!(#out #(#classes)*);
        for attr in &element.attrs {
            if is_class_directive(&attr.name) {
                continue;
            }
            // `v-show` keeps the list (its scroll area, or the list itself
            // under `within`) and hides it, as on an element.
            if let AttrName::Directive(directive, at) = &attr.name
                && directive == "show"
            {
                let value = prop(&attr.value);
                out = quote_spanned!(*at=> #out.visible(#value));
                continue;
            }
            let AttrName::Plain(name) = &attr.name else {
                return Err(syn::Error::new(
                    span,
                    "`<Virtual>` takes attributes, `class` and `v-show` only",
                ));
            };
            let at = name.span();
            out = match name.to_string().as_str() {
                "row_height" | "grid" | "gap" => out,
                "measured" => quote!(#out.measured()),
                "grow" => quote!(#out.grow()),
                method @ ("height" | "width" | "overscan") => {
                    let method = Ident::new(method, at);
                    let value = number(&attr.value, "f32", at)?;
                    quote!(#out.#method(#value))
                }
                "scroll" => {
                    let scroll = raw(&attr.value, at)?;
                    quote!(#out.scroll_view(#scroll))
                }
                "within" => {
                    let within = raw(&attr.value, at)?;
                    quote!(#out.within(#within))
                }
                "list_ref" => {
                    let list_ref = raw(&attr.value, at)?;
                    quote!(#out.list_ref(#list_ref))
                }
                "key" => {
                    let key = raw(&attr.value, at)?;
                    quote!(#out.key(#key))
                }
                other => {
                    return Err(syn::Error::new(
                        at,
                        format!(
                            "`<Virtual>` has no attribute `{other}`; it has `row-height`, \
                             `measured`, `height`, `width`, `grow`, `overscan`, `scroll`, \
                             `within`, `grid`, `gap`, `list-ref`, `key`, `class`, `v-show`"
                        ),
                    ));
                }
            };
        }
        Ok(out)
    }

    /// `<Block>`, `<Transition>`, `<TransitionGroup>` or `<KeepAlive>`
    /// around a `v-if` chain, one `v-for` element (not under `<KeepAlive>`),
    /// or another of these blocks: the `when` / `each` inside with each
    /// block's method. A block's `class` and `class:name` style the
    /// container the chain or the list is built in.
    fn block(
        &self,
        element: &Element,
        mut modifiers: Vec<TokenStream>,
        mut container: Vec<TokenStream>,
        lists: bool,
    ) -> syn::Result<TokenStream> {
        let span = element.name.span();
        let tag = element.name.to_string();
        // Nested blocks come here without passing `single`.
        reject_locale(element)?;
        container.extend(container_classes(element)?);
        let lists = match tag.as_str() {
            "Block" => {
                if let Some(attr) = element
                    .attrs
                    .iter()
                    .find(|attr| !is_class_directive(&attr.name))
                {
                    let at = match &attr.name {
                        AttrName::Plain(name) => name.span(),
                        _ => span,
                    };
                    return Err(syn::Error::new(
                        at,
                        "`<Block>` takes `class` and `class:name` only: they style the \
                         container of the `v-if` chain or the `v-for` list it holds",
                    ));
                }
                lists
            }
            "KeepAlive" => {
                if let Some(attr) = element
                    .attrs
                    .iter()
                    .find(|attr| !is_class_directive(&attr.name))
                {
                    let at = match &attr.name {
                        AttrName::Plain(name) => name.span(),
                        _ => span,
                    };
                    return Err(syn::Error::new(
                        at,
                        "`<KeepAlive>` takes no attributes but `class`; around a `v-if` chain \
                         it keeps the branches not shown alive (use `dynamic(..).max(n)` for a \
                         limit)",
                    ));
                }
                modifiers.push(quote!(.keep_alive()));
                false
            }
            _ => {
                let transition = self.transition(element)?;
                modifiers.push(quote!(.transition(#transition)));
                lists
            }
        };
        let children: Vec<&Element> = element
            .children
            .iter()
            .map(|node| match node {
                Node::Element(child) => Ok(child),
                _ => Err(syn::Error::new(
                    span,
                    format!("`<{tag}>` holds a `v-if` chain or one `v-for` element"),
                )),
            })
            .collect::<syn::Result<_>>()?;
        match children.as_slice() {
            [inner]
                if inner.module.is_empty()
                    && matches!(
                        inner.name.to_string().as_str(),
                        "Block" | "Transition" | "TransitionGroup" | "KeepAlive"
                    ) =>
            {
                self.block(inner, modifiers, container, lists)
            }
            [rows] if rows.directive("for").is_some() => {
                if !lists {
                    return Err(syn::Error::new(
                        rows.name.span(),
                        "`<KeepAlive>` holds a `v-if` chain: list rows are kept by their key",
                    ));
                }
                if !modifiers.is_empty()
                    && rows.attrs.iter().any(|attr| {
                        matches!(&attr.name, AttrName::Directive(directive, _)
                            if directive.split('.').next() == Some("virtual"))
                    })
                {
                    return Err(syn::Error::new(
                        rows.name.span(),
                        "a virtual list does not animate its rows",
                    ));
                }
                let list = self.element(rows)?;
                Ok(quote!(#list #(#modifiers)* #(#container)*))
            }
            [first, ..] if first.directive("if").is_some() => {
                for (index, link) in children.iter().enumerate().skip(1) {
                    let last = link.directive("else").is_some();
                    if (!last && link.directive("else-if").is_none())
                        || (last && index + 1 != children.len())
                    {
                        return Err(syn::Error::new(
                            link.name.span(),
                            format!("`<{tag}>` holds one `v-if` chain"),
                        ));
                    }
                }
                self.chain(&children, &modifiers, &container)
            }
            _ => Err(syn::Error::new(
                span,
                format!("`<{tag}>` holds a `v-if` chain or one `v-for` element"),
            )),
        }
    }

    /// The `Transition` a `<Transition name="fade" duration="150"
    /// move="200">` describes; `:transition={value}` gives one directly.
    fn transition(&self, element: &Element) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let span = element.name.span();
        let tag = element.name.to_string();
        let mut name = None;
        let mut duration = quote!(150_f64);
        let mut moves = None;
        let mut given = None;
        for attr in &element.attrs {
            if is_class_directive(&attr.name) {
                continue;
            }
            let AttrName::Plain(attribute) = &attr.name else {
                return Err(syn::Error::new(
                    span,
                    format!("`<{tag}>` takes attributes only"),
                ));
            };
            let at = attribute.span();
            match attribute.to_string().as_str() {
                "name" => match &attr.value {
                    AttrValue::Lit(Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(value),
                        ..
                    })) => name = Some((value.value(), value.span())),
                    _ => return Err(syn::Error::new(at, "`name=\"fade\"`: a preset name")),
                },
                "duration" => duration = number(&attr.value, "f64", at)?,
                "move" => moves = Some(number(&attr.value, "f64", at)?),
                "transition" => given = Some(raw(&attr.value, at)?),
                other => {
                    return Err(syn::Error::new(
                        at,
                        format!(
                            "`<{tag}>` has no attribute `{other}`; it has `name`, `duration`, \
                             `move`, `transition`"
                        ),
                    ));
                }
            }
        }
        let millis =
            |ms: &TokenStream| quote!(::std::time::Duration::from_secs_f64(#ms as f64 / 1000.0));
        let length = millis(&duration);
        let transition = match (given, name) {
            (Some(_), Some((_, at))) => {
                return Err(syn::Error::new(
                    at,
                    "`name=` and `transition=` exclude each other",
                ));
            }
            (Some(given), None) => given,
            (None, name) => {
                let preset = match name.as_ref().map(|(name, at)| (name.as_str(), *at)) {
                    None | Some(("fade", _)) => quote!(fade(#length)),
                    Some(("slide-up", _)) => quote!(slide(0.0, 12.0, #length)),
                    Some(("slide-down", _)) => quote!(slide(0.0, -12.0, #length)),
                    Some(("slide-left", _)) => quote!(slide(12.0, 0.0, #length)),
                    Some(("slide-right", _)) => quote!(slide(-12.0, 0.0, #length)),
                    Some(("scale", _)) => quote!(scale(0.95, #length)),
                    Some((other, at)) => {
                        return Err(syn::Error::new(
                            at,
                            format!(
                                "no transition named `{other}`; there are `fade`, `slide-up`, \
                                 `slide-down`, `slide-left`, `slide-right`, `scale`"
                            ),
                        ));
                    }
                };
                quote!(#krate::view::Transition::#preset)
            }
        };
        Ok(match moves {
            Some(ms) => {
                let length = millis(&ms);
                quote!(#transition.moves(#length))
            }
            None => transition,
        })
    }

    /// `<Suspense fallback={view}>content</Suspense>`; in a `.vue` file the
    /// fallback is `<template #fallback>`.
    fn suspense(&self, element: &Element) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let span = element.name.span();
        let mut fallback = None;
        for attr in &element.attrs {
            match &attr.name {
                AttrName::Plain(name) if name == "fallback" => {
                    fallback = Some(match &attr.value {
                        AttrValue::View(nodes) => self.nodes(nodes)?,
                        value => raw(value, name.span())?,
                    });
                }
                AttrName::Plain(name) => {
                    return Err(syn::Error::new(
                        name.span(),
                        format!("`<Suspense>` has no attribute `{name}`; it has `fallback`"),
                    ));
                }
                _ => return Err(syn::Error::new(span, "`<Suspense>` takes `fallback` only")),
            }
        }
        let mut content = Vec::new();
        for node in &element.children {
            match node {
                Node::Element(slot) if slot.module.is_empty() && slot.name == "template" => {
                    let named = slot.attrs.iter().find_map(|attr| match &attr.name {
                        AttrName::Directive(directive, _) => directive.strip_prefix("slot:"),
                        _ => None,
                    });
                    match named {
                        Some("fallback") if fallback.is_none() => {
                            fallback = Some(self.nodes(&slot.children)?);
                        }
                        Some("default") => content.extend(slot.children.iter()),
                        _ => {
                            return Err(syn::Error::new(
                                slot.name.span(),
                                "`<Suspense>` takes `<template #fallback>` and its content",
                            ));
                        }
                    }
                }
                other => content.push(other),
            }
        }
        let fallback = fallback.ok_or_else(|| {
            syn::Error::new(span, "`<Suspense>` needs a fallback to show while loading")
        })?;
        let content = self.node_list(&content)?;
        Ok(quote_spanned! {span=>
            #krate::view::suspense(move || #fallback, move || #content)
        })
    }

    /// What `t` takes for `<T>`: the message a front end already wrote as a
    /// prop (its one child), else the one its attributes give
    /// ([`localized_text`]), a constant while they are literals and
    /// re-evaluated when what it reads changes otherwise.
    fn message(&self, element: &Element) -> syn::Result<TokenStream> {
        match element.children.as_slice() {
            [Node::Verbatim(message, _)] => return Ok(message.clone()),
            [] => {}
            _ => {
                return Err(syn::Error::new(
                    element.name.span(),
                    "`<T>` takes no children: `id` names the message it says, and its other \
                     attributes are the message's arguments",
                ));
            }
        }
        let message = localized_text(self.krate, element, &|_| None)?;
        let reads = element.attrs.iter().any(|attr| {
            matches!(&attr.name, AttrName::Plain(name) if is_message_part("T", &name.to_string()))
                && matches!(attr.value, AttrValue::Expr(_))
        });
        Ok(if reads {
            quote_spanned!(element.name.span()=> move || #message)
        } else {
            message
        })
    }

    /// The element itself: constructor, fields, directives, handlers.
    fn single(&self, element: &Element) -> syn::Result<TokenStream> {
        if !element.module.is_empty() {
            return self.component(element);
        }
        let krate = self.krate;
        let tag = element.name.to_string();
        let span = element.name.span();
        let children = &element.children;
        if matches!(
            tag.as_str(),
            "Block"
                | "Virtual"
                | "Transition"
                | "TransitionGroup"
                | "KeepAlive"
                | "Suspense"
                | "Teleport"
                | "ErrorBoundary"
        ) {
            reject_locale(element)?;
        }
        // `<T>` is a `Text`: its node takes `Text`'s fields.
        let control = control(if tag == "T" { "Text" } else { &tag });
        let (mut out, consumed): (TokenStream, Vec<&str>) = match (tag.as_str(), control) {
            ("T", _) => {
                let message = self.message(element)?;
                (quote_spanned!(span=> #krate::view::t(#message)), Vec::new())
            }
            ("Column" | "Row", _) => {
                let make = format_ident!("{}", tag.to_lowercase(), span = span);
                let gap = match element.plain("gap") {
                    Some(gap) => {
                        let gap = number(&gap.value, "f32", span)?;
                        quote_spanned!(span=> .gap(#gap))
                    }
                    None => TokenStream::new(),
                };
                let body = self.nodes(children)?;
                (
                    quote_spanned!(span=> #krate::view::#make() #gap .children(#body)),
                    vec!["gap"],
                )
            }
            ("Virtual", _) => return self.virtual_list(element),
            ("Block" | "Transition" | "TransitionGroup" | "KeepAlive", _) => {
                return self.block(element, Vec::new(), Vec::new(), true);
            }
            ("Suspense", _) => return self.suspense(element),
            ("ErrorBoundary", _) => {
                let krate = self.krate;
                let fallback = element
                    .attrs
                    .iter()
                    .find_map(|attr| match &attr.name {
                        AttrName::Plain(name) if name == "fallback" => Some(raw(&attr.value, span)),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        syn::Error::new(
                            span,
                            "`<ErrorBoundary>` needs `fallback={|errors| view}`, the view shown \
                             while an error stands",
                        )
                    })??;
                let body = self.nodes(children)?;
                (
                    quote_spanned!(span=> #krate::view::error_boundary(#fallback, move || #body)),
                    vec!["fallback"],
                )
            }
            ("Teleport", _) => {
                let krate = self.krate;
                let mut to = None;
                for attr in &element.attrs {
                    match &attr.name {
                        AttrName::Plain(name) if name == "to" => to = Some(prop(&attr.value)),
                        AttrName::Plain(name) if name == "key" => {}
                        // They style the anchor the content is built in.
                        name if is_class_directive(name) => {}
                        _ => {
                            return Err(syn::Error::new(
                                span,
                                "`<Teleport>` takes `to={node}` (a node ref, a node id or an \
                                 expression choosing one), and `class` / `class:name` for the \
                                 anchor the content is built in",
                            ));
                        }
                    }
                }
                let to =
                    to.ok_or_else(|| syn::Error::new(span, "`<Teleport>` needs `to={node}`"))?;
                let body = self.nodes(children)?;
                (
                    quote_spanned!(span=> #krate::view::teleport(#to, #body)),
                    vec!["to"],
                )
            }
            ("Widget", _) => {
                let component = element
                    .plain("of")
                    .ok_or_else(|| syn::Error::new(span, "`<Widget of={component}>`"))?;
                let component = raw(&component.value, span)?;
                let body = self.nodes(children)?;
                (
                    quote_spanned!(span=> #krate::view::widget(#component).children(#body)),
                    vec!["of"],
                )
            }
            (_, Some(control)) => {
                self.check_name(element, control);
                let function = Ident::new(control.function, span);
                let mut args = Vec::new();
                for &(name, kind) in control.arguments {
                    args.push(match kind {
                        "text" => self.string_child(element, name)?,
                        "expr" => {
                            let attr = element.plain(name).ok_or_else(|| {
                                syn::Error::new(span, format!("`<{tag}>` needs `{name}=`"))
                            })?;
                            raw(&attr.value, span)?
                        }
                        _ => {
                            let attr = element.plain(name).ok_or_else(|| {
                                syn::Error::new(span, format!("`<{tag}>` needs `{name}=`"))
                            })?;
                            number(&attr.value, kind, span)?
                        }
                    });
                }
                if !children.is_empty()
                    && !control.arguments.iter().any(|(_, kind)| *kind == "text")
                {
                    return Err(syn::Error::new(span, format!("`<{tag}>` has no children")));
                }
                (
                    quote_spanned!(span=> #krate::view::#function(#(#args),*)),
                    control.arguments.iter().map(|(name, _)| *name).collect(),
                )
            }
            _ => return self.component(element),
        };
        for attr in &element.attrs {
            match &attr.name {
                AttrName::Plain(name) => {
                    let text = name.to_string();
                    // `<T>`'s message is its constructor's argument.
                    if consumed.contains(&text.as_str()) || is_message_part(&tag, &text) {
                        continue;
                    }
                    if text == "key" {
                        if element.directive("for").is_none() {
                            let key = raw(&attr.value, name.span())?;
                            out = quote!(#out.key(#key));
                        }
                        continue;
                    }
                    if text == "ref" {
                        let node_ref = named_ref(&attr.value, name.span())?;
                        out = quote!(#out.node_ref(#node_ref));
                        continue;
                    }
                    if text == "labelled_by" {
                        // The caption's `ref`, as `aria-labelledby` names an
                        // id; or a node id, or a closure choosing one.
                        let label = named_ref(&attr.value, name.span())?;
                        out = quote!(#out.labelled_by(#label));
                        continue;
                    }
                    if text == "locale" {
                        let locale = locale(&attr.value, name.span())?;
                        out = quote!(#out.locale(#locale));
                        continue;
                    }
                    if STYLE_ROLES.contains(&text.as_str()) {
                        let value = prop(&attr.value);
                        let setter = Ident::new(&text, name.span());
                        out = quote!(#out.#setter(#value));
                        continue;
                    }
                    let value = match control.and_then(|control| control.field(&text)) {
                        Some(ty @ ("f32" | "f64")) => {
                            typed_number(&attr.value, ty).unwrap_or_else(|| prop(&attr.value))
                        }
                        Some(_) => prop(&attr.value),
                        None => {
                            return Err(syn::Error::new(
                                name.span(),
                                match control {
                                    Some(control) => format!(
                                        "`<{tag}>` has no attribute `{text}`; it has {}",
                                        control.field_list()
                                    ),
                                    None => format!("`<{tag}>` takes no attribute `{text}`"),
                                },
                            ));
                        }
                    };
                    let setter = Ident::new(&text, name.span());
                    out = quote!(#out.#setter(#value));
                }
                AttrName::Event(event) => {
                    let method = format_ident!("on_{}", event, span = event.span());
                    let takes_event = match control {
                        Some(control) => control
                            .events
                            .iter()
                            .find(|(name, _)| method == name)
                            .map(|(_, takes_event)| *takes_event)
                            .ok_or_else(|| {
                                syn::Error::new(
                                    event.span(),
                                    format!("`<{tag}>` has no event `@{event}`"),
                                )
                            })?,
                        None => false,
                    };
                    let handler = event_handler(&attr.value, takes_event, event.span())?;
                    out = quote!(#out.#method(#handler));
                }
                AttrName::On(event) => {
                    let handler = raw(&attr.value, event.span())?;
                    // `|component, event, cx| …` also gets the component
                    // and its context, as `on_cx` does in Rust.
                    let with_context = matches!(
                        &attr.value,
                        AttrValue::Expr(Expr::Closure(closure)) if closure.inputs.len() == 3
                    );
                    out = if with_context {
                        quote!(#out.on_cx::<#event>(#handler))
                    } else {
                        quote!(#out.on::<#event>(#handler))
                    };
                }
                AttrName::Directive(directive, span) => match directive.as_str() {
                    "show" => {
                        let value = prop(&attr.value);
                        out = quote_spanned!(*span=> #out.visible(#value));
                    }
                    "model" => {
                        if !control.is_some_and(|control| control.model) {
                            return Err(syn::Error::new(
                                *span,
                                format!("`<{tag}>` has no `v-model`"),
                            ));
                        }
                        let signal = raw(&attr.value, *span)?;
                        out = quote_spanned!(*span=> #out.model(#signal));
                    }
                    // Written from `class` and `class:name` against the
                    // view's `<style>`: the calls the Rust spelling writes.
                    "class" => {
                        let value = raw(&attr.value, *span)?;
                        out = quote_spanned!(*span=> #out.class(#value));
                    }
                    "class_when" => {
                        let value = raw(&attr.value, *span)?;
                        out = quote_spanned!(*span=> #out.class_when(#value));
                    }
                    "if" | "else-if" | "else" | "for" => {}
                    slot if slot.starts_with("slot:") => {}
                    virtual_rows if virtual_rows.split('.').next() == Some("virtual") => {
                        if element.directive("for").is_none() {
                            return Err(syn::Error::new(*span, "`v-virtual` goes with `v-for`"));
                        }
                    }
                    other => {
                        return Err(syn::Error::new(
                            *span,
                            format!("unknown directive `v-{other}`"),
                        ));
                    }
                },
            }
        }
        let out = slot_calls(self, element, out)?;
        if !matches!(tag.as_str(), "Column" | "Row" | "Widget") && control.is_none() {
            return Ok(out);
        }
        let Some(source) = self.source_site(span) else {
            return Ok(out);
        };
        let mut fields = Vec::new();
        for attr in &element.attrs {
            let (name, at) = match &attr.name {
                AttrName::Plain(name)
                    if !is_message_part(&tag, &name.to_string())
                        && control
                            .is_some_and(|control| control.field(&name.to_string()).is_some()) =>
                {
                    (name.to_string(), name.span())
                }
                AttrName::Plain(_) => continue,
                AttrName::Directive(name, at) => (
                    match name.as_str() {
                        "show" => "style.layout.hidden",
                        "class" | "class_when" => "style.layout",
                        "model" => match tag.as_str() {
                            "Switch" | "Checkbox" => "checked",
                            _ => "value",
                        },
                        _ => continue,
                    }
                    .to_owned(),
                    *at,
                ),
                _ => continue,
            };
            let at = match &attr.value {
                AttrValue::Expr(expr) => expr.span(),
                _ => at,
            };
            let at = self.source_site(at).expect("source file is set");
            fields.push(quote!((#name, #at)));
        }
        if let Some(control) = control {
            for (name, kind) in control.arguments {
                if *kind != "text" || element.plain(name).is_some() {
                    continue;
                }
                let at = match element.children.as_slice() {
                    [Node::Verbatim(_, at)] => *at,
                    [Node::Mixed(parts, at)] => parts
                        .iter()
                        .find_map(|part| match part {
                            TextPart::Expr(expr) => Some(expr.span()),
                            _ => None,
                        })
                        .unwrap_or(*at),
                    _ => continue,
                };
                // `<T>`'s child is its message, which `t` binds.
                let name = if tag == "T" { "localized" } else { name };
                let at = self.source_site(at).expect("source file is set");
                fields.push(quote!((#name, #at)));
            }
        }
        if fields.is_empty() {
            return Ok(out);
        }
        Ok(
            quote!(#out.source_site(const { &#krate::view::ViewSource { element: #source, fields: &[#(#fields),*] } })),
        )
    }

    /// `<TodoRow todo={t} />` → `todo_row(t)`; attribute values are the
    /// arguments in order, children the last argument.
    fn component(&self, element: &Element) -> syn::Result<TokenStream> {
        let span = element.name.span();
        let function = function_ident(&element.name.to_string(), span);
        let mut args = Vec::new();
        let mut key = None;
        for attr in &element.attrs {
            match &attr.name {
                AttrName::Plain(name) if name == "key" && element.directive("for").is_some() => {}
                // A key on a component names its first root.
                AttrName::Plain(name) if name == "key" => {
                    key = Some(raw(&attr.value, name.span())?)
                }
                AttrName::Plain(name) => args.push(match &attr.value {
                    AttrValue::View(nodes) => self.nodes(nodes)?,
                    value => raw(value, name.span())?,
                }),
                AttrName::Directive(directive, _)
                    if matches!(directive.as_str(), "if" | "else-if" | "else" | "for")
                        || directive.starts_with("slot:")
                        || directive.split('.').next() == Some("virtual") => {}
                _ => {
                    return Err(syn::Error::new(
                        span,
                        "a component takes values only; bind events and fields inside it",
                    ));
                }
            }
        }
        if !element.children.is_empty() {
            args.push(self.nodes(&element.children)?);
        }
        let module = &element.module;
        let call = slot_calls(
            self,
            element,
            quote_spanned!(span=> #(#module::)* #function(#(#args),*)),
        )?;
        let krate = self.krate;
        Ok(match key {
            Some(key) => quote_spanned!(span=> #krate::view::keyed(#key, #call)),
            None => call,
        })
    }

    /// A text-bearing element's value: the `name=` attribute, or its one
    /// child (a string, interpolated or not, or an expression).
    /// Warn when a control would have no accessible name: an empty text
    /// argument (`<Button></Button>`), or no `label` on a control named only
    /// by it.
    fn check_name(&self, element: &Element, control: &Control) {
        let tag = control.tag;
        // Another node's text names it.
        if element.plain("labelled_by").is_some() {
            return;
        }
        let empty_text = |name: &str| match element.plain(name) {
            Some(attr) => matches!(&attr.value, AttrValue::Lit(Expr::Lit(syn::ExprLit {
                lit: Lit::Str(text), ..
            })) if text.value().trim().is_empty()),
            None => match element.children.as_slice() {
                [] => true,
                [Node::Text(text)] => text.value().trim().is_empty(),
                _ => false,
            },
        };
        let message = if let Some((name, _)) =
            control.arguments.iter().find(|(_, kind)| *kind == "text")
            && *name == "label"
            && empty_text(name)
        {
            Some(format!(
                "`<{tag}>` has no text: a screen reader cannot name it; give it a label"
            ))
        } else if NAMED_BY_LABEL.contains(&tag)
            && !element
                .attrs
                .iter()
                .any(|attr| matches!(&attr.name, AttrName::Plain(name) if name == "label"))
        {
            Some(format!(
                "`<{tag}>` has no `label`: a screen reader cannot say what it is for"
            ))
        } else {
            None
        };
        if let Some(message) = message {
            self.warnings.borrow_mut().push(Warning {
                span: element.name.span(),
                message,
            });
        }
    }

    fn string_child(&self, element: &Element, name: &str) -> syn::Result<TokenStream> {
        if let Some(attr) = element.plain(name) {
            return Ok(prop(&attr.value));
        }
        match element.children.as_slice() {
            [] => Ok(quote!("")),
            [Node::Text(text)] => Ok(string_prop(text)),
            [Node::Mixed(parts, span)] => Ok(mixed_prop(parts, *span)),
            [Node::Expr(expr)] => Ok(prop(&AttrValue::Expr(expr.clone()))),
            [Node::Verbatim(value, _)] => Ok(value.clone()),
            _ => Err(syn::Error::new(
                element.name.span(),
                format!("`<{}>` takes one string or `{{expression}}`", element.name),
            )),
        }
    }
}

impl Element {
    /// The tag as written: `EmptyState`, or `kit::EmptyState` with a path.
    pub fn tag(&self) -> String {
        let mut tag = String::new();
        for segment in &self.module {
            tag.push_str(&segment.to_string());
            tag.push_str("::");
        }
        tag.push_str(&self.name.to_string());
        tag
    }

    fn directive(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(
            |attr| matches!(&attr.name, AttrName::Directive(directive, _) if directive == name),
        )
    }

    fn plain(&self, name: &str) -> Option<&Attr> {
        self.attrs
            .iter()
            .find(|attr| matches!(&attr.name, AttrName::Plain(ident) if ident == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_is_detected_and_doubled_braces_are_not() {
        assert!(interpolates("计数 {count}"));
        assert!(!interpolates("literal {{braces}}"));
        assert!(!interpolates("plain"));
        assert_eq!(snake_case("TodoRow"), "todo_row");
    }

    #[test]
    fn mixed_text_is_constant_without_expressions_and_escapes_braces() {
        let span = Span::call_site();
        let constant = mixed_prop(&[TextPart::Literal("a {b}".into())], span).to_string();
        assert_eq!(constant, "\"a {b}\"");
        let count: Expr = syn::parse_quote!(count);
        let dynamic = mixed_prop(
            &[TextPart::Literal("n {".into()), TextPart::Expr(count)],
            span,
        )
        .to_string();
        assert!(dynamic.contains("\"n {{{}\""), "{dynamic}");
        assert!(dynamic.starts_with("move ||"), "{dynamic}");
    }

    #[test]
    fn locale_takes_language_tags_and_refuses_the_rest() {
        for tag in ["ar", "zh-CN", "zh_Hant_TW", "en-US-x-twain", "x-klingon"] {
            assert!(is_language_tag(tag), "{tag}");
        }
        for tag in [
            "",
            "a r",
            "ar-",
            "-ar",
            "1ar",
            "zh--cn",
            "abcdefghi",
            "ar.b",
        ] {
            assert!(!is_language_tag(tag), "{tag}");
        }
    }

    /// `<T>`'s `id` and arguments are its message; what any element takes
    /// stays the node's.
    #[test]
    fn t_tells_its_message_from_its_node() {
        for part in ["id", "count", "value", "name"] {
            assert!(is_message_part("T", part), "{part}");
        }
        for node in ["key", "ref", "locale", "decorative", "class", "foreground"] {
            assert!(!is_message_part("T", node), "{node}");
        }
        assert!(!is_message_part("Text", "count"));
        assert!(is_builtin("T"));
    }
}
