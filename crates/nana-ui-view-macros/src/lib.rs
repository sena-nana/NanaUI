//! `view!`: a Vue-shaped template that expands to the declarative view
//! function API of `nana-ui-runtime` (`column`, `text`, `each`, `when`, …).
//! It adds no runtime concept; the expansion is exactly what a hand-written
//! call chain would be.
//!
//! Use it through `nana_ui_runtime::view!` (feature `view-macro`), which
//! supplies the crate path.

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Expr, Ident, Lit, LitStr, Pat, Token, braced};

#[proc_macro]
pub fn view(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match syn::parse::<Template>(input) {
        Ok(template) => template.expand().into(),
        Err(error) => error.to_compile_error().into(),
    }
}

struct Template {
    krate: TokenStream,
    nodes: Vec<Node>,
}

enum Node {
    Element(Element),
    Text(LitStr),
    Expr(Expr),
}

struct Element {
    name: Ident,
    attrs: Vec<Attr>,
    children: Vec<Node>,
}

enum AttrName {
    /// `name=value`
    Plain(Ident),
    /// `@event={…}`
    Event(Ident),
    /// `on:Type={|e| …}`
    On(syn::Path),
    /// `v-if`, `v-else-if`, `v-else`, `v-for`, `v-show`, `v-model`
    Directive(String, Span),
}

enum AttrValue {
    None,
    Lit(Expr),
    Expr(Expr),
    For(Pat, Expr),
}

struct Attr {
    name: AttrName,
    value: AttrValue,
}

impl Parse for Template {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut krate = quote!(::nana_ui_runtime);
        if input.peek(Token![crate]) && input.peek2(Token![=]) {
            input.parse::<Token![crate]>()?;
            input.parse::<Token![=]>()?;
            let mut path = TokenStream::new();
            while !input.peek(Token![;]) {
                path.extend([input.parse::<TokenTree>()?]);
            }
            input.parse::<Token![;]>()?;
            krate = path;
        }
        let mut nodes = Vec::new();
        while !input.is_empty() {
            nodes.push(input.parse()?);
        }
        Ok(Self { krate, nodes })
    }
}

impl Parse for Node {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![<]) {
            return input.parse().map(Node::Element);
        }
        if input.peek(LitStr) {
            return input.parse().map(Node::Text);
        }
        if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            return content.parse().map(Node::Expr);
        }
        Err(input.error("expected `<Tag>`, a string, or `{expression}`"))
    }
}

impl Parse for Element {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![<]>()?;
        let name = Ident::parse_any(input)?;
        let mut attrs = Vec::new();
        loop {
            if input.peek(Token![/]) {
                input.parse::<Token![/]>()?;
                input.parse::<Token![>]>()?;
                return Ok(Self {
                    name,
                    attrs,
                    children: Vec::new(),
                });
            }
            if input.peek(Token![>]) {
                input.parse::<Token![>]>()?;
                break;
            }
            attrs.push(input.parse()?);
        }
        let mut children = Vec::new();
        while !(input.peek(Token![<]) && input.peek2(Token![/])) {
            if input.is_empty() {
                return Err(syn::Error::new(
                    name.span(),
                    format!("`<{name}>` is never closed"),
                ));
            }
            children.push(input.parse()?);
        }
        input.parse::<Token![<]>()?;
        input.parse::<Token![/]>()?;
        let closing = Ident::parse_any(input)?;
        if closing != name {
            return Err(syn::Error::new(
                closing.span(),
                format!("`</{closing}>` closes `<{name}>`"),
            ));
        }
        input.parse::<Token![>]>()?;
        Ok(Self {
            name,
            attrs,
            children,
        })
    }
}

impl Parse for Attr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            AttrName::Event(Ident::parse_any(input)?)
        } else {
            let first = Ident::parse_any(input)?;
            if first == "on" && input.peek(Token![:]) && !input.peek(Token![::]) {
                input.parse::<Token![:]>()?;
                AttrName::On(input.parse()?)
            } else if first == "v" && input.peek(Token![-]) {
                input.parse::<Token![-]>()?;
                let mut directive = Ident::parse_any(input)?.to_string();
                if directive == "else" && input.peek(Token![-]) {
                    input.parse::<Token![-]>()?;
                    directive.push('-');
                    directive.push_str(&Ident::parse_any(input)?.to_string());
                }
                AttrName::Directive(directive, first.span())
            } else {
                AttrName::Plain(first)
            }
        };
        if !input.peek(Token![=]) {
            return Ok(Self {
                name,
                value: AttrValue::None,
            });
        }
        input.parse::<Token![=]>()?;
        let value = if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            if matches!(&name, AttrName::Directive(directive, _) if directive == "for") {
                let pattern = Pat::parse_single(&content)?;
                content.parse::<Token![in]>()?;
                AttrValue::For(pattern, content.parse()?)
            } else {
                AttrValue::Expr(content.parse()?)
            }
        } else if input.peek(Token![-]) {
            let minus = input.parse::<Token![-]>()?;
            let lit: Lit = input.parse()?;
            AttrValue::Lit(syn::parse_quote_spanned!(minus.span=> -#lit))
        } else {
            let lit: Lit = input.parse()?;
            AttrValue::Lit(syn::parse_quote!(#lit))
        };
        Ok(Self { name, value })
    }
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
        AttrValue::None => Err(syn::Error::new(span, "this attribute needs a value")),
        AttrValue::For(..) => Err(syn::Error::new(span, "unexpected loop")),
    }
}

fn handler(value: &AttrValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        AttrValue::Expr(Expr::Closure(closure)) => Ok(quote!(#closure)),
        // A function or closure value: `@activate={add}`.
        AttrValue::Expr(expr @ (Expr::Path(_) | Expr::Field(_))) => Ok(quote!(#expr)),
        AttrValue::Expr(expr) => Ok(quote_spanned!(expr.span()=> move || { #expr; })),
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
                        if next.directive("else-if").is_some() || next.directive("else").is_some() {
                            let last = next.directive("else").is_some();
                            chain.push(next);
                            index += 1;
                            if last {
                                break;
                            }
                        } else {
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
            Node::Expr(expr) => quote!(#expr),
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
        let function = Ident::new(&snake_case(&element.name.to_string()), span);
        let mut args = Vec::new();
        for attr in &element.attrs {
            match &attr.name {
                AttrName::Plain(name) if name == "key" && element.directive("for").is_some() => {}
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
        Ok(quote_spanned!(span=> #function(#(#args),*)))
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
            [Node::Expr(expr)] => Ok(prop(&AttrValue::Expr(expr.clone()))),
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

impl Template {
    fn expand(&self) -> TokenStream {
        let generator = Gen { krate: &self.krate };
        match generator.nodes(&self.nodes) {
            Ok(tokens) => tokens,
            Err(error) => error.to_compile_error(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(source: &str) -> String {
        let tokens: TokenStream = source.parse().expect("test source tokenizes");
        match syn::parse2::<Template>(tokens) {
            Ok(template) => template.expand().to_string(),
            Err(error) => error.to_compile_error().to_string(),
        }
    }

    #[test]
    fn mistakes_name_what_is_wrong() {
        for (source, message) in [
            ("<Column>", "`<Column>` is never closed"),
            ("<Column></Row>", "`</Row>` closes `<Column>`"),
            ("<Text v-else>\"x\"</Text>", "`v-else` must follow"),
            (
                "<Text v-for={t in items}>\"x\"</Text>",
                "`v-for` needs `key={…}`",
            ),
            ("<Slider min=0 max=1 />", "`<Slider>` needs `step=`"),
            ("<Text v-bogus>\"x\"</Text>", "unknown directive `v-bogus`"),
            (
                "<Row v-if={a} v-for={t in b} key={t}/>",
                "not next to `v-for`",
            ),
        ] {
            let expanded = expand(source);
            assert!(expanded.contains("compile_error"), "{source}: {expanded}");
            assert!(expanded.contains(message), "{source}: {expanded}");
        }
    }

    #[test]
    fn literals_stay_constant_paths_pass_through_expressions_become_closures() {
        let expanded = expand("<Button disabled={busy} loading={a && b} label=\"x\"/>");
        assert!(expanded.contains(". disabled (busy)"), "{expanded}");
        assert!(
            expanded.contains(". loading (move || a && b)"),
            "{expanded}"
        );
        assert!(expanded.contains("button (\"x\")"), "{expanded}");
    }

    #[test]
    fn interpolation_is_detected_and_doubled_braces_are_not() {
        assert!(interpolates("计数 {count}"));
        assert!(!interpolates("literal {{braces}}"));
        assert!(!interpolates("plain"));
        assert_eq!(snake_case("TodoRow"), "todo_row");
    }
}
