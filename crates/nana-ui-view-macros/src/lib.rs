//! `view!`: a Vue-shaped template that expands to the declarative view
//! function API of `nana-ui-runtime` (`column`, `text`, `each`, `when`, …).
//! This crate only parses the template; `nana-ui-view-codegen`, shared with
//! the `.vue` dialect compiler, writes the calls.
//!
//! Use it through `nana_ui_runtime::view!` (feature `view-macro`), which
//! supplies the crate path.

use nana_ui_view_codegen::{Attr, AttrName, AttrValue, Element, Node};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::{Ident, Lit, LitStr, Pat, Token, braced};

#[proc_macro]
pub fn view(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_tokens(input.into()).into()
}

/// Internal compile-time verification macro that emits code invoking all 242
/// `StyleOp` Rust setters from `nana-ui-css` against `nana_ui_runtime::view::__patch`.
#[doc(hidden)]
#[proc_macro]
pub fn __test_all_style_ops(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let krate = (|input: ParseStream| parse_crate(input))
        .parse2(input.into())
        .unwrap_or_else(|_| quote!(::nana_ui_runtime));
    nana_ui_view_codegen::emit_all_style_ops_verification_fn(&krate).into()
}

/// `#[derive(Store)]` on a struct with named fields or an enum, generic or
/// not: a `<Name>StoreFields` trait, with the type's parameters, giving
/// every path to the value inside a store an accessor per field. An enum
/// gets `is_<variant>()` per variant and a guarded `Variant` path per
/// field. Import the trait where the accessors are used.
#[proc_macro_derive(Store)]
pub fn derive_store(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    syn::parse2::<syn::DeriveInput>(input.into())
        .and_then(|input| store::expand(&input, &store::runtime_path()))
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

/// `#[component]`: lets a template pass this function's arguments by name
/// and leave out the ones marked `#[prop(default)]` /
/// `#[prop(default = expression)]`. Use it through
/// `nana_ui_runtime::view::component`.
#[proc_macro_attribute]
pub fn component(
    args: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    syn::parse::<syn::ItemFn>(item)
        .and_then(|item| component::expand(args.into(), item, &store::runtime_path()))
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

mod component;
mod css_tokens;
mod store;

/// `css! { padding: 12px; transition: opacity 150ms }` or
/// `css!(r#"padding: 12px; color: #9ecafe;"#)`: one declaration block written
/// as CSS tokens or a raw string literal, compiled with `nana-ui-css` at build
/// time, for `El::css`. Use it through `nana_ui_runtime::css!`, which supplies
/// the crate path.
#[proc_macro]
pub fn css(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let parsed = (|input: ParseStream| -> syn::Result<(TokenStream, css_tokens::CssText)> {
        let krate = parse_crate(input)?;
        let fork = input.fork();
        if let Ok(lit) = fork.parse::<LitStr>()
            && fork.is_empty()
        {
            let _: LitStr = input.parse()?;
            return Ok((krate, css_tokens::CssText::from_lit(&lit)));
        }
        let declarations: TokenStream = input.parse()?;
        Ok((
            krate,
            css_tokens::css(declarations, true, Span::call_site()),
        ))
    })
    .parse2(input.into());
    match parsed {
        Ok((krate, css)) => {
            let compiled = nana_ui_view_codegen::compile_inline(&css.text, &krate);
            if !compiled.errors.is_empty() {
                return style_errors_to_compile_error(&css, compiled.errors).into();
            }
            let warnings = locate(&css, compiled.warnings);
            if warnings.is_empty() {
                compiled.tokens
            } else {
                with_warnings("css!", compiled.tokens, warnings)
            }
        }
        Err(error) => error.to_compile_error(),
    }
    .into()
}

/// `stylesheet! { mod styles; .card { padding: 12px; } … }` or
/// `stylesheet! { mod styles; r#" .card { padding: 12px; } "# }`: a stylesheet
/// written as CSS tokens or a raw string literal, compiled at build time into a
/// module holding the sheet and one `view::Class` per class (`styles::card`;
/// `-` becomes `_`). With `pub mod` the classes are public; otherwise the
/// parent module sees them. Use it through `nana_ui_runtime::stylesheet!`.
#[proc_macro]
pub fn stylesheet(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let parsed = (|input: ParseStream| -> syn::Result<_> {
        let krate = parse_crate(input)?;
        let public = input.peek(Token![pub]);
        if public {
            input.parse::<syn::Visibility>()?;
        }
        input.parse::<Token![mod]>()?;
        let module = input.parse::<Ident>()?;
        input.parse::<Token![;]>()?;
        let fork = input.fork();
        let css = if let Ok(lit) = fork.parse::<LitStr>()
            && fork.is_empty()
        {
            let _: LitStr = input.parse()?;
            css_tokens::CssText::from_lit(&lit)
        } else {
            let sheet: TokenStream = input.parse()?;
            css_tokens::css(sheet, false, module.span())
        };
        Ok((krate, public, module, css))
    })
    .parse2(input.into());
    match parsed {
        Ok((krate, public, module, css)) => {
            let compiled = nana_ui_view_codegen::compile_stylesheet(
                &css.text,
                &module,
                public,
                &krate,
                &|class| css.class_span(class),
            );
            if !compiled.errors.is_empty() {
                return style_errors_to_compile_error(&css, compiled.errors).into();
            }
            let warnings = locate(&css, compiled.warnings);
            if warnings.is_empty() {
                compiled.tokens
            } else {
                // Items cannot hold the warning's `let`: put it in a const.
                let warned = with_warnings("stylesheet!", quote!(()), warnings);
                let tokens = compiled.tokens;
                quote! {
                    #tokens
                    const _: () = #warned;
                }
            }
        }
        Err(error) => error.to_compile_error(),
    }
    .into()
}

/// `crate = path;`, which the runtime's `macro_rules!` wrappers pass first.
fn parse_crate(input: ParseStream) -> syn::Result<TokenStream> {
    if !(input.peek(Token![crate]) && input.peek2(Token![=])) {
        return Ok(quote!(::nana_ui_runtime));
    }
    input.parse::<Token![crate]>()?;
    input.parse::<Token![=]>()?;
    let mut path = TokenStream::new();
    while !input.peek(Token![;]) {
        path.extend([input.parse::<TokenTree>()?]);
    }
    input.parse::<Token![;]>()?;
    Ok(path)
}

/// Style warnings at the tokens they are about.
fn locate(
    css: &css_tokens::CssText,
    warnings: Vec<nana_ui_view_codegen::StyleWarning>,
) -> Vec<nana_ui_view_codegen::Warning> {
    warnings
        .into_iter()
        .map(|warning| nana_ui_view_codegen::Warning {
            span: match warning.at {
                nana_ui_view_codegen::StyleAt::Sheet(range) => css.span(&range),
                nana_ui_view_codegen::StyleAt::Template(span) => span,
            },
            message: warning.message,
        })
        .collect()
}

/// Combine CSS errors into one `syn::Error` pointing at each diagnostic's token.
fn style_errors_to_syn(
    css: &css_tokens::CssText,
    errors: Vec<nana_ui_view_codegen::StyleWarning>,
) -> syn::Error {
    let mut combined: Option<syn::Error> = None;
    for err in errors {
        let span = match err.at {
            nana_ui_view_codegen::StyleAt::Sheet(range) => css.span(&range),
            nana_ui_view_codegen::StyleAt::Template(span) => span,
        };
        let syn_err = syn::Error::new(span, err.message);
        match &mut combined {
            Some(existing) => existing.combine(syn_err),
            None => combined = Some(syn_err),
        }
    }
    combined.expect("errors is non-empty")
}

fn style_errors_to_compile_error(
    css: &css_tokens::CssText,
    errors: Vec<nana_ui_view_codegen::StyleWarning>,
) -> TokenStream {
    style_errors_to_syn(css, errors).to_compile_error()
}

fn expand_tokens(input: TokenStream) -> TokenStream {
    let expanded = parse_template.parse2(input).and_then(|mut template| {
        nana_ui_view_codegen::lift_slots(&mut template.nodes)?;
        let empty_css = css_tokens::CssText::empty(Span::call_site());
        let style_ref = template.style.as_ref().unwrap_or(&empty_css);
        let compiled = nana_ui_view_codegen::compile_styles(
            &style_ref.text,
            &mut template.nodes,
            &template.krate,
        );
        if !compiled.errors.is_empty() {
            return Err(style_errors_to_syn(style_ref, compiled.errors));
        }
        let style_warnings = locate(style_ref, compiled.warnings);
        let items = compiled.items;
        let (tokens, mut warnings) =
            nana_ui_view_codegen::expand_checked(&template.krate, &template.nodes)?;
        warnings.extend(style_warnings);
        if items.is_empty() {
            Ok((tokens, warnings))
        } else {
            Ok((quote! {{ #(#items)* #tokens }}, warnings))
        }
    });
    match expanded {
        Ok((tokens, warnings)) if warnings.is_empty() => tokens,
        Ok((tokens, warnings)) => with_warnings("view!", tokens, warnings),
        Err(error) => error.to_compile_error(),
    }
}

/// Stable proc macros cannot warn, so each warning is the use of a
/// deprecated constant whose note is the message, at the element's span.
fn with_warnings(
    macro_name: &str,
    tokens: TokenStream,
    warnings: Vec<nana_ui_view_codegen::Warning>,
) -> TokenStream {
    let uses = warnings.into_iter().enumerate().map(|(index, warning)| {
        let name = quote::format_ident!("__nana_view_warning_{index}");
        let note = format!("{macro_name}: {}", warning.message);
        // The use carries the warning's span, so the lint lands there;
        // `quote_spanned!` would keep the interpolated name's own span.
        let used = Ident::new(&name.to_string(), warning.span);
        quote! {
            #[deprecated(note = #note)]
            #[allow(non_upper_case_globals)]
            const #name: () = ();
            let () = #used;
        }
    });
    quote! {{
        #(#uses)*
        #tokens
    }}
}

struct Template {
    krate: TokenStream,
    /// `<style>…</style>`: CSS for the template's `class` attributes.
    style: Option<css_tokens::CssText>,
    nodes: Vec<Node>,
}

fn parse_template(input: ParseStream) -> syn::Result<Template> {
    let krate = parse_crate(input)?;
    if input.peek(Ident) && input.peek2(Token![=]) && input.fork().parse::<Ident>()? == "style" {
        return Err(input.error(
            "`style = \"…\";` is gone: write the CSS as `<style> .a { padding: 4px; } </style>` \
             at the top of the template",
        ));
    }
    let style = parse_style(input)?;
    let mut nodes = Vec::new();
    while !input.is_empty() {
        nodes.push(parse_node(input)?);
    }
    Ok(Template {
        krate,
        style,
        nodes,
    })
}

/// Whether `input` starts with `<name>` or, with `close`, `</name>`.
fn peek_tag(input: ParseStream, name: &str, close: bool) -> bool {
    let fork = input.fork();
    fork.parse::<Token![<]>().is_ok()
        && (!close || fork.parse::<Token![/]>().is_ok())
        && fork.parse::<Ident>().is_ok_and(|ident| ident == name)
        && fork.parse::<Token![>]>().is_ok()
}

/// `<style>…</style>` at the top of a template: CSS as tokens or a raw string literal.
fn parse_style(input: ParseStream) -> syn::Result<Option<css_tokens::CssText>> {
    if !peek_tag(input, "style", false) {
        return Ok(None);
    }
    input.parse::<Token![<]>()?;
    let open = input.parse::<Ident>()?;
    input.parse::<Token![>]>()?;
    let mut tokens = TokenStream::new();
    while !peek_tag(input, "style", true) {
        if input.is_empty() {
            return Err(syn::Error::new(open.span(), "`<style>` has no `</style>`"));
        }
        tokens.extend([input.parse::<TokenTree>()?]);
    }
    for _ in 0..4 {
        input.parse::<TokenTree>()?;
    }
    if tokens.clone().into_iter().count() == 1
        && let Ok(lit) = syn::parse2::<LitStr>(tokens.clone())
    {
        return Ok(Some(css_tokens::CssText::from_lit(&lit)));
    }
    Ok(Some(css_tokens::css(tokens, false, open.span())))
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
    let (module, name) = parse_tag(input)?;
    let tag = tag_text(&module, &name);
    let mut attrs = Vec::new();
    loop {
        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(Element {
                module,
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
                format!("`<{tag}>` is never closed"),
            ));
        }
        children.push(parse_node(input)?);
    }
    input.parse::<Token![<]>()?;
    input.parse::<Token![/]>()?;
    let (closing_module, closing) = parse_tag(input)?;
    let closing_tag = tag_text(&closing_module, &closing);
    if closing_tag != tag {
        return Err(syn::Error::new(
            closing.span(),
            format!("`</{closing_tag}>` closes `<{tag}>`"),
        ));
    }
    input.parse::<Token![>]>()?;
    Ok(Element {
        module,
        name,
        attrs,
        children,
    })
}

/// `Name`, or `path::to::Name`: the path segments and the name.
fn parse_tag(input: ParseStream) -> syn::Result<(Vec<Ident>, Ident)> {
    let mut module = Vec::new();
    let mut name = Ident::parse_any(input)?;
    while input.peek(Token![::]) {
        input.parse::<Token![::]>()?;
        module.push(std::mem::replace(&mut name, Ident::parse_any(input)?));
    }
    Ok((module, name))
}

fn tag_text(module: &[Ident], name: &Ident) -> String {
    let mut tag = String::new();
    for segment in module {
        tag.push_str(&segment.to_string());
        tag.push_str("::");
    }
    tag.push_str(&name.to_string());
    tag
}

fn parse_attr(input: ParseStream) -> syn::Result<Attr> {
    let name = if input.peek(Token![@]) {
        input.parse::<Token![@]>()?;
        AttrName::Event(Ident::parse_any(input)?)
    } else if input.peek(Token![#]) {
        // `<template #name>`: a named slot, `#title-trailing` spelled either
        // way.
        let hash = input.parse::<Token![#]>()?;
        let mut name = Ident::parse_any(input)?.to_string();
        while input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            name.push('-');
            name.push_str(&Ident::parse_any(input)?.to_string());
        }
        AttrName::Directive(format!("slot:{name}"), hash.span)
    } else {
        let first = Ident::parse_any(input)?;
        if first == "on" && input.peek(Token![:]) && !input.peek(Token![::]) {
            input.parse::<Token![:]>()?;
            AttrName::On(input.parse()?)
        } else if first == "class" && input.peek(Token![:]) && !input.peek(Token![::]) {
            // `class:active={condition}` or `class:is-active={condition}`: the class while it holds.
            input.parse::<Token![:]>()?;
            let mut class = Ident::parse_any(input)?.to_string();
            while input.peek(Token![-]) {
                input.parse::<Token![-]>()?;
                class.push('-');
                if input.peek(syn::LitInt) {
                    let num: syn::LitInt = input.parse()?;
                    class.push_str(&num.to_string());
                } else {
                    class.push_str(&Ident::parse_any(input)?.to_string());
                }
            }
            AttrName::Directive(format!("class:{class}"), first.span())
        } else if first == "v" && input.peek(Token![-]) {
            input.parse::<Token![-]>()?;
            let mut directive = Ident::parse_any(input)?.to_string();
            if directive == "else" && input.peek(Token![-]) {
                input.parse::<Token![-]>()?;
                directive.push('-');
                directive.push_str(&Ident::parse_any(input)?.to_string());
            }
            // Modifiers: `v-virtual.measured`.
            while input.peek(Token![.]) {
                input.parse::<Token![.]>()?;
                directive.push('.');
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

    fn flat(source: &str) -> String {
        expand(source).split_whitespace().collect()
    }

    #[test]
    fn mistakes_name_what_is_wrong() {
        for (source, token) in [
            ("<Column>", "Column"),
            ("<Column></Row>", "Row"),
            ("<Text v-else>\"x\"</Text>", "v-else"),
            ("<Text v-for={t in items}>\"x\"</Text>", "key"),
            ("<Slider min=0 max=1 />", "step"),
            ("<Text v-bogus>\"x\"</Text>", "v-bogus"),
            ("<Row v-if={a} v-for={t in b} key={t}/>", "v-if"),
        ] {
            let expanded = expand(source);
            assert!(expanded.contains("compile_error"), "{source}: {expanded}");
            assert!(expanded.contains(token), "{source}: {expanded}");
        }
    }

    #[test]
    fn an_unnamed_control_expands_to_a_deprecation_warning_at_it() {
        let expanded =
            expand("crate = x; <Column><TextInput/><TextInput label=\"名字\"/></Column>");
        assert_eq!(expanded.matches("deprecated").count(), 1, "{expanded}");
        assert!(expanded.contains("label"), "{expanded}");
        let clean = expand("crate = x; <Button>\"保存\"</Button>");
        assert!(!clean.contains("deprecated"), "{clean}");
    }

    #[test]
    fn literals_stay_constant_paths_pass_through_expressions_become_closures() {
        let expanded = flat("<Button disabled={busy} loading={a && b} label=\"x\"/>");
        assert!(
            expanded.contains(".disabled(busy)"),
            "a path stays a path: {expanded}"
        );
        assert!(
            expanded.contains(".loading(move||a&&b)"),
            "an expression becomes a closure: {expanded}"
        );
        assert!(
            expanded.contains("button(\"x\")"),
            "a literal stays a literal: {expanded}"
        );
    }

    fn sheet(source: &str) -> css_tokens::CssText {
        let tokens: TokenStream = source.parse().expect("test CSS tokenizes");
        css_tokens::css(tokens, false, Span::call_site())
    }

    #[test]
    fn css_tokens_come_back_as_the_css_they_were_written_as() {
        let css = sheet(
            r##".a.b .c { padding: 4px 8px; margin: 0 -1px; font-size: "1.5em"; }
               .d > .e { color: "#9ecafe"; background: url("a//b.png"); font-family: "Noto Sans SC", sans-serif; }
               a[href^="x"] { opacity: 0.5 !important; }
               @media (min-width: 600px) { .f { flex-grow: 1; } }"##,
        );
        assert_eq!(
            css.text,
            ".a.b .c { padding: 4px 8px; margin: 0 -1px; font-size: 1.5em; }\n\
             .d > .e { color: #9ecafe; background: url(\"a//b.png\"); font-family: \"Noto Sans SC\", sans-serif; }\n\
             a[href^=\"x\"] { opacity: 0.5 !important; }\n\
             @media (min-width: 600px) { .f { flex-grow: 1; } }"
        );
    }

    /// An `@container` prelude written as tokens comes back as CSS: `<=`
    /// stays one operator, and the space between tokens follows the source.
    #[test]
    fn container_queries_come_back_as_they_were_written() {
        let source = "@container card (400px <= width < 800px) { .g { container-type: inline-size; } }\n\
                      @container (width<480px) { .h { opacity: 0.5; } }";
        assert_eq!(sheet(source).text, source);
    }

    #[test]
    fn container_warnings_point_at_their_tokens() {
        let source = "@container style(--x: 1) { .a { opacity: 0.5; } }\n\
                      @container (width > 1px) { div.b { opacity: 1; } .a { opacity: 0.5; } }";
        let css = sheet(source);
        let template = "crate = x; <Column class=\"a\"/>";
        let mut nodes = parse_template
            .parse2(template.parse().unwrap())
            .unwrap()
            .nodes;
        let compiled = nana_ui_view_codegen::compile_styles(&css.text, &mut nodes, &quote!(x));
        let warnings = locate(&css, compiled.warnings);
        let at = |needle: &str| {
            let warning = warnings
                .iter()
                .find(|w| w.message.contains(needle))
                .unwrap_or_else(|| panic!("no warning about {needle}: {warnings:?}"));
            let start = warning.span.start();
            (start.line, start.column)
        };
        assert_eq!(at("`style()`"), (1, 27));
        assert_eq!(at("class selectors"), (2, 27));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn css_tokens_preserve_quotes_on_string_properties_and_allow_escape_hatch() {
        let css = sheet(
            r##".grid {
                grid-template-areas: "head head" "nav main";
                font-feature-settings: "liga" 1, "tnum" 1;
                font-variation-settings: "wght" 600;
                font-family: "Inter", sans-serif;
                color: "#334455";
                font-size: "1.5em";
            }"##,
        );
        assert!(
            css.text
                .contains(r#"grid-template-areas: "head head" "nav main";"#),
            "{}",
            css.text
        );
        assert!(
            css.text
                .contains(r#"font-feature-settings: "liga" 1, "tnum" 1;"#),
            "{}",
            css.text
        );
        assert!(
            css.text.contains(r#"font-variation-settings: "wght" 600;"#),
            "{}",
            css.text
        );
        assert!(
            css.text.contains(r#"font-family: "Inter", sans-serif;"#),
            "{}",
            css.text
        );
        assert!(css.text.contains("color: #334455;"), "{}", css.text);
        assert!(css.text.contains("font-size: 1.5em;"), "{}", css.text);
    }

    #[test]
    fn hyphenated_class_inline_style_and_css_errors_expand_cleanly() {
        let expanded = expand(
            r##"crate = x;
            <style>
                .card.is-active { opacity: 0.5; }
            </style>
            <Column class="card" class:is-active={active} style="padding: 8px;" />"##,
        );
        assert!(!expanded.contains("compile_error"), "{expanded}");
        assert!(expanded.contains("class_when"), "{expanded}");
        assert!(expanded.contains("inline_patch"), "{expanded}");

        // Unknown CSS property inside `<style>` becomes a compile_error!
        let bad = expand(
            r##"crate = x; <style>.card { nonexistent_prop: 1px; }</style> <Column class="card"/>"##,
        );
        assert!(bad.contains("compile_error"), "{bad}");
        assert!(bad.contains("nonexistent_prop"), "{bad}");
    }

    #[test]
    fn a_quoted_value_compiles_as_the_bare_one() {
        let compile = |css: &str| {
            let template = "crate = x; <Column class=\"a\"/>";
            let mut nodes = parse_template
                .parse2(template.parse().unwrap())
                .unwrap()
                .nodes;
            let compiled = nana_ui_view_codegen::compile_styles(css, &mut nodes, &quote!(x));
            let items = compiled.items;
            quote!(#(#items)*).to_string()
        };
        let quoted = sheet(r#".a { padding: "4px" "8px"; font-size: "1.5em"; }"#);
        let bare = ".a { padding: 4px 8px; font-size: 1.5em; }";
        assert_eq!(compile(&quoted.text), compile(bare));
    }

    #[test]
    fn style_diagnostics_point_at_their_tokens() {
        let source = ".a { padding: 4px; frobnicate: 3; -webkit-font-smoothing: antialiased; }\n.b:hover .c { opacity: 0.5; }\ndiv .a { opacity: 1; }";
        let css = sheet(source);
        let template = "crate = x; <Column class=\"a ghost\"/>";
        let mut nodes = parse_template
            .parse2(template.parse().unwrap())
            .unwrap()
            .nodes;
        let compiled = nana_ui_view_codegen::compile_styles(&css.text, &mut nodes, &quote!(x));
        let warnings = locate(&css, compiled.warnings);
        let errors = locate(&css, compiled.errors);
        let at = |list: &[nana_ui_view_codegen::Warning], needle: &str| {
            let item = list
                .iter()
                .find(|w| w.message.contains(needle))
                .unwrap_or_else(|| panic!("no diagnostic about {needle}: {list:?}"));
            let start = item.span.start();
            (start.line, start.column)
        };
        // Lines and columns of the test source (1-based lines, 0-based columns).
        assert_eq!(at(&errors, "frobnicate"), (1, 19));
        assert_eq!(at(&warnings, "-webkit-font-smoothing"), (1, 34));
        assert_eq!(at(&warnings, "class selectors"), (3, 0));
    }

    #[test]
    fn raw_string_style_is_accepted_and_legacy_style_assign_is_refused() {
        let expanded =
            expand(r##"crate = x; <style>r#".a { padding: 4px; }"#</style> <Column class="a"/>"##);
        assert!(!expanded.contains("compile_error"), "{expanded}");
        assert!(expanded.contains("__NANA_SHEET"), "{expanded}");
        let expanded = expand("crate = x; style = \".a {}\"; <Column/>");
        assert!(expanded.contains("compile_error"), "{expanded}");
        assert!(expanded.contains("style ="), "{expanded}");
    }

    /// The lint lands where the use of the warning's constant is spanned:
    /// that must be the element, not the macro call.
    #[test]
    fn a_warning_is_used_at_the_span_it_is_about() {
        fn uses(tokens: TokenStream, out: &mut Vec<proc_macro2::LineColumn>) {
            for token in tokens {
                match token {
                    TokenTree::Group(group) => uses(group.stream(), out),
                    TokenTree::Ident(ident) if ident == "__nana_view_warning_0" => {
                        out.push(ident.span().start());
                    }
                    _ => {}
                }
            }
        }
        let source = "crate = x;\n<Column>\n  <TextInput/>\n</Column>";
        let mut found = Vec::new();
        uses(expand_tokens(source.parse().unwrap()), &mut found);
        // The declaration, then the use at `TextInput` (line 3, column 3).
        assert!(
            found.iter().any(|at| (at.line, at.column) == (3, 3)),
            "{found:?}"
        );
    }

    #[test]
    fn named_slots_are_method_calls_and_default_is_children() {
        let expanded = flat(
            "crate = x; <Widget of={shell}><template #title-trailing>\"t\"</template>\
             <template #default>\"d\"</template></Widget>",
        );
        assert!(
            expanded.contains(".title_trailing("),
            "a named slot is its method: {expanded}"
        );
        assert!(
            expanded.contains(".children("),
            "the default slot is children: {expanded}"
        );
        let unnamed = expand("crate = x; <Column><template>\"a\"</template></Column>");
        assert!(unnamed.contains("#slot-name"), "{unnamed}");
    }

    #[test]
    fn a_component_sets_its_properties_by_name_on_its_builder() {
        let expanded = flat(
            "crate = x; <kit::SwitchRow on={f} label=\"静音\" compact>\
             <Text>\"内容\"</Text></kit::SwitchRow>",
        );
        assert!(
            expanded
                .contains("kit::switch_row::Props::__new().on(f).label(\"静音\").compact(true)"),
            "attributes in the order written, a bare one is true: {expanded}"
        );
        assert!(
            expanded.contains(".children(") && expanded.ends_with(".__build()"),
            "children are the `children` property: {expanded}"
        );
        for (source, token) in [
            ("<Badge label=\"a\" label=\"b\"/>", "given twice"),
            ("<Badge children={x}>\"y\"</Badge>", "given twice"),
        ] {
            let expanded = expand(source);
            assert!(expanded.contains("compile_error"), "{source}: {expanded}");
            assert!(expanded.contains(token), "{source}: {expanded}");
        }
    }

    /// `<T>` is `t(…)`: literals make a constant message, any expression a
    /// closure building it again; `locale` scopes an element.
    #[test]
    fn t_says_a_message_and_locale_scopes_an_element() {
        let constant = flat("crate = x; <T id=\"files\" count=3 />");
        assert!(
            constant.contains("x::view::t(x::LocalizedText::new(\"files\").arg(\"count\",3))"),
            "literals are a constant: {constant}"
        );
        let bound = flat("crate = x; <T id=\"files\" count={n.get()} who={name} decorative />");
        assert!(
            bound.contains(
                "x::view::t(move||x::LocalizedText::new(\"files\").arg(\"count\",n.get())\
                 .arg(\"who\",x::view::__arg(&name)))"
            ),
            "an expression is read again, a path cloned: {bound}"
        );
        assert!(bound.contains(".decorative(true)"), "{bound}");
        let scoped = flat("crate = x; <Column locale=\"ar\"><T id=\"title\" /></Column>");
        assert!(scoped.contains(".locale(\"ar\")"), "{scoped}");
        let chosen = flat("crate = x; <Row locale={chosen} />");
        assert!(chosen.contains(".locale(chosen)"), "{chosen}");
    }

    #[test]
    fn t_and_locale_mistakes_name_what_is_wrong() {
        for (source, token) in [
            ("<T count={n} />", "id="),
            ("<T id=3 />", "key of the message"),
            ("<T id=\"a\">\"x\"</T>", "no children"),
            ("<T id=\"a\" n=1 n=2 />", "twice"),
            ("<Column locale=\"a b\" />", "language tag"),
            ("<Column locale />", "needs a language tag"),
            (
                "<Block locale=\"ar\"><Text v-if={a}>\"x\"</Text></Block>",
                "takes no `locale`",
            ),
            (
                "<Transition><Block locale=\"ar\"><Text v-if={a}>\"x\"</Text></Block></Transition>",
                "takes no `locale`",
            ),
            (
                "<Suspense fallback={f} locale=\"ar\">\"x\"</Suspense>",
                "takes no `locale`",
            ),
            (
                "<ErrorBoundary fallback={f} locale=\"ar\">\"x\"</ErrorBoundary>",
                "takes no `locale`",
            ),
        ] {
            let expanded = expand(&format!("crate = x; {source}"));
            assert!(expanded.contains("compile_error"), "{source}: {expanded}");
            assert!(expanded.contains(token), "{source}: {expanded}");
        }
    }
}
