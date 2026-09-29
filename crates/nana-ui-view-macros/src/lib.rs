//! `view!`: a Vue-shaped template that expands to the declarative view
//! function API of `nana-ui-runtime` (`column`, `text`, `each`, `when`, …).
//! This crate only parses the template; `nana-ui-view-codegen`, shared with
//! the `.vue` dialect compiler, writes the calls.
//!
//! Use it through `nana_ui_runtime::view!` (feature `view-macro`), which
//! supplies the crate path.

use nana_ui_view_codegen::{Attr, AttrName, AttrValue, Element, Node};
use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::{Ident, Lit, LitStr, Pat, Token, braced};

#[proc_macro]
pub fn view(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_tokens(input.into()).into()
}

fn expand_tokens(input: TokenStream) -> TokenStream {
    let expanded = parse_template
        .parse2(input)
        .and_then(|template| nana_ui_view_codegen::expand(&template.krate, &template.nodes));
    expanded.unwrap_or_else(|error| error.to_compile_error())
}

struct Template {
    krate: TokenStream,
    nodes: Vec<Node>,
}

fn parse_template(input: ParseStream) -> syn::Result<Template> {
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
        nodes.push(parse_node(input)?);
    }
    Ok(Template { krate, nodes })
}

fn parse_node(input: ParseStream) -> syn::Result<Node> {
    if input.peek(Token![<]) {
        return parse_element(input).map(Node::Element);
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

fn parse_element(input: ParseStream) -> syn::Result<Element> {
    input.parse::<Token![<]>()?;
    let name = Ident::parse_any(input)?;
    let mut attrs = Vec::new();
    loop {
        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(Element {
                name,
                attrs,
                children: Vec::new(),
            });
        }
        if input.peek(Token![>]) {
            input.parse::<Token![>]>()?;
            break;
        }
        attrs.push(parse_attr(input)?);
    }
    let mut children = Vec::new();
    while !(input.peek(Token![<]) && input.peek2(Token![/])) {
        if input.is_empty() {
            return Err(syn::Error::new(
                name.span(),
                format!("`<{name}>` is never closed"),
            ));
        }
        children.push(parse_node(input)?);
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
    Ok(Element {
        name,
        attrs,
        children,
    })
}

fn parse_attr(input: ParseStream) -> syn::Result<Attr> {
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
        return Ok(Attr {
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
    Ok(Attr { name, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(source: &str) -> String {
        let tokens: TokenStream = source.parse().expect("test source tokenizes");
        expand_tokens(tokens).to_string()
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
}
