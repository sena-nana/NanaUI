//! The code generator behind `view!` and the `.vue` dialect compiler: a
//! template's element tree becomes the declarative view function calls of
//! `nana-ui-runtime` (`column`, `text`, `each`, `when`, …). Nothing here adds
//! a runtime concept; the output is the call chain a person would write.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Expr, Ident, Lit, LitStr, Pat};

/// Expand `nodes` into one view expression. `krate` is the path of
/// `nana-ui-runtime` as seen from the generated code.
pub fn expand(krate: &TokenStream, nodes: &[Node]) -> syn::Result<TokenStream> {
    Gen { krate }.nodes(nodes)
}

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
    Verbatim(TokenStream),
}

/// A piece of [`Node::Mixed`] text.
// Compile-time syntax, built once per template: variant size does not matter.
#[allow(clippy::large_enum_variant)]
pub enum TextPart {
    Literal(String),
    Expr(Expr),
}

pub struct Element {
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
}

pub struct Attr {
    pub name: AttrName,
    pub value: AttrValue,
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
        AttrValue::For(..) => unreachable!("only v-for takes a loop"),
    }
}

/// A numeric argument of type `ty`: a literal is written with that suffix
/// (`8` → `8_f32`), anything else is passed as is for the compiler to check.
fn number(value: &AttrValue, ty: &str, span: Span) -> syn::Result<TokenStream> {
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
    let literal = match value {
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
    };
    match literal {
        Some(tokens) => Ok(tokens),
        None => raw(value, span),
    }
}

/// The expression itself, for arguments and handlers.
fn raw(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Lit(expr) | AttrValue::Expr(expr) => Ok(quote!(#expr)),
        AttrValue::Verbatim(tokens) => Ok(tokens.clone()),
        AttrValue::None => Err(syn::Error::new(span, "this attribute needs a value")),
        AttrValue::For(..) => Err(syn::Error::new(span, "unexpected loop")),
    }
}

/// An event handler: a closure or function value as is, any other
/// expression run as a statement.
pub fn handler(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Expr(Expr::Closure(closure)) => Ok(quote!(#closure)),
        // A function or closure value: `@activate={add}`.
        AttrValue::Expr(expr @ (Expr::Path(_) | Expr::Field(_))) => Ok(quote!(#expr)),
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
}

impl Gen<'_> {
    fn nodes(&self, nodes: &[Node]) -> syn::Result<TokenStream> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < nodes.len() {
            match &nodes[index] {
                Node::Element(element) if element.directive("if").is_some() => {
                    // Collect the v-else-if / v-else siblings of this chain.
                    let mut chain = vec![element];
                    index += 1;
                    while let Some(Node::Element(next)) = nodes.get(index) {
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
                    out.push(self.chain(&chain)?);
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
        Ok(match node {
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
            Node::Verbatim(value) => quote!(#krate::view::text(#value)),
        })
    }

    /// `v-if` / `v-else-if` / `v-else` → nested `when(..).otherwise(..)`.
    fn chain(&self, chain: &[&Element]) -> syn::Result<TokenStream> {
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
                self.chain(rest)?
            };
            out = quote!(#out.otherwise(move || #otherwise));
        }
        Ok(out)
    }

    /// One element, with `v-for` wrapping it in `each` when present.
    fn element(&self, element: &Element) -> syn::Result<TokenStream> {
        let Some(each) = element.directive("for") else {
            return self.single(element);
        };
        let krate = self.krate;
        if element.directive("if").is_some() {
            return Err(syn::Error::new(
                element.name.span(),
                "put `v-if` on a wrapping element or inside the row, not next to `v-for`",
            ));
        }
        let AttrValue::For(pattern, source) = &each.value else {
            return Err(syn::Error::new(
                element.name.span(),
                "`v-for={item in items}`",
            ));
        };
        let key = element.plain("key").ok_or_else(|| {
            syn::Error::new(
                element.name.span(),
                "`v-for` needs `key={…}` naming each item",
            )
        })?;
        let key = raw(&key.value, element.name.span())?;
        let row = self.single(element)?;
        Ok(quote! {
            #krate::view::each(
                #source,
                move |__nana_item: &_| { let #pattern = __nana_item; #key },
                move |#pattern| #row,
            )
        })
    }

    /// The element itself: constructor, fields, directives, handlers.
    fn single(&self, element: &Element) -> syn::Result<TokenStream> {
        let krate = self.krate;
        let tag = element.name.to_string();
        let span = element.name.span();
        let children = &element.children;
        let (mut out, consumed): (TokenStream, &[&str]) = match tag.as_str() {
            "Column" | "Row" => {
                let make = format_ident!("{}", tag.to_lowercase(), span = span);
                let gap = match element.plain("gap") {
                    Some(gap) => number(&gap.value, "f32", span)?,
                    None => quote!(0.0_f32),
                };
                let body = self.nodes(children)?;
                (
                    quote_spanned!(span=> #krate::view::#make(#gap, #body)),
                    &["gap"],
                )
            }
            "Text" => {
                let value = self.string_child(element, "value")?;
                (
                    quote_spanned!(span=> #krate::view::text(#value)),
                    &["value"],
                )
            }
            "Button" | "Checkbox" => {
                let make = format_ident!("{}", tag.to_lowercase(), span = span);
                let label = self.string_child(element, "label")?;
                (
                    quote_spanned!(span=> #krate::view::#make(#label)),
                    &["label"],
                )
            }
            "Slider" => {
                let bound = |name: &str| -> syn::Result<TokenStream> {
                    let attr = element.plain(name).ok_or_else(|| {
                        syn::Error::new(span, format!("`<Slider>` needs `{name}=`"))
                    })?;
                    number(&attr.value, "f64", span)
                };
                let (min, max, step) = (bound("min")?, bound("max")?, bound("step")?);
                (
                    quote_spanned!(span=> #krate::view::slider(#min, #max, #step)),
                    &["min", "max", "step"],
                )
            }
            "TextInput" => (quote_spanned!(span=> #krate::view::text_input()), &[]),
            "Widget" => {
                let component = element
                    .plain("of")
                    .ok_or_else(|| syn::Error::new(span, "`<Widget of={component}>`"))?;
                let component = raw(&component.value, span)?;
                let body = self.nodes(children)?;
                (
                    quote_spanned!(span=> #krate::view::widget(#component).children(#body)),
                    &["of"],
                )
            }
            _ => return self.component(element),
        };
        for attr in &element.attrs {
            match &attr.name {
                AttrName::Plain(name) => {
                    let text = name.to_string();
                    if consumed.contains(&text.as_str()) {
                        continue;
                    }
                    if text == "key" {
                        if element.directive("for").is_none() {
                            let key = raw(&attr.value, name.span())?;
                            out = quote!(#out.key(#key));
                        }
                        continue;
                    }
                    let value = prop(&attr.value);
                    let setter = Ident::new(&text, name.span());
                    out = quote!(#out.#setter(#value));
                }
                AttrName::Event(event) => {
                    let method = format_ident!("on_{}", event, span = event.span());
                    let handler = handler(&attr.value, event.span())?;
                    out = quote!(#out.#method(#handler));
                }
                AttrName::On(event) => {
                    let handler = raw(&attr.value, event.span())?;
                    out = quote!(#out.on::<#event>(#handler));
                }
                AttrName::Directive(directive, span) => match directive.as_str() {
                    "show" => {
                        let value = prop(&attr.value);
                        out = quote_spanned!(*span=> #out.visible(#value));
                    }
                    "model" => {
                        let signal = raw(&attr.value, *span)?;
                        out = quote_spanned!(*span=> #out.model(#signal));
                    }
                    "if" | "else-if" | "else" | "for" => {}
                    other => {
                        return Err(syn::Error::new(
                            *span,
                            format!("unknown directive `v-{other}`"),
                        ));
                    }
                },
            }
        }
        Ok(out)
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
                AttrName::Plain(name) => args.push(raw(&attr.value, name.span())?),
                AttrName::Directive(directive, _)
                    if matches!(directive.as_str(), "if" | "else-if" | "else" | "for") => {}
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
        let call = quote_spanned!(span=> #function(#(#args),*));
        let krate = self.krate;
        Ok(match key {
            Some(key) => quote_spanned!(span=> #krate::view::keyed(#key, #call)),
            None => call,
        })
    }

    /// A text-bearing element's value: the `name=` attribute, or its one
    /// child (a string, interpolated or not, or an expression).
    fn string_child(&self, element: &Element, name: &str) -> syn::Result<TokenStream> {
        if let Some(attr) = element.plain(name) {
            return Ok(prop(&attr.value));
        }
        match element.children.as_slice() {
            [] => Ok(quote!("")),
            [Node::Text(text)] => Ok(string_prop(text)),
            [Node::Mixed(parts, span)] => Ok(mixed_prop(parts, *span)),
            [Node::Expr(expr)] => Ok(prop(&AttrValue::Expr(expr.clone()))),
            [Node::Verbatim(value)] => Ok(value.clone()),
            _ => Err(syn::Error::new(
                element.name.span(),
                format!("`<{}>` takes one string or `{{expression}}`", element.name),
            )),
        }
    }
}

impl Element {
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
}
