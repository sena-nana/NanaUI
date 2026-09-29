//! `#[derive(Store)]`.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields};

/// The path the generated code names the runtime by: the crate itself, its
/// own tests and examples, or a dependent reaching it directly or through
/// `nana-ui`.
pub(crate) fn runtime_path() -> TokenStream {
    match crate_name("nana-ui-runtime") {
        Ok(FoundCrate::Itself) => {
            if std::env::var("CARGO_CRATE_NAME").as_deref() == Ok("nana_ui_runtime") {
                quote!(crate)
            } else {
                quote!(::nana_ui_runtime)
            }
        }
        Ok(FoundCrate::Name(name)) => {
            let name = syn::Ident::new(&name, Span::call_site());
            quote!(::#name)
        }
        Err(_) => match crate_name("nana-ui") {
            Ok(FoundCrate::Name(name)) => {
                let name = syn::Ident::new(&name, Span::call_site());
                quote!(::#name::runtime)
            }
            _ => quote!(::nana_ui::runtime),
        },
    }
}

pub(crate) fn expand(input: &DeriveInput, krate: &TokenStream) -> syn::Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`#[derive(Store)]` takes a struct with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`#[derive(Store)]` takes a struct with named fields",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`#[derive(Store)]` does not take generic structs yet",
        ));
    }
    let name = &input.ident;
    let vis = &input.vis;
    let fields_trait = format_ident!("{name}StoreFields");
    let path = quote!(#krate::view::StorePath);
    let subfield = quote!(#krate::view::Subfield);
    let mut declarations = Vec::new();
    let mut definitions = Vec::new();
    for (segment, field) in fields.named.iter().enumerate() {
        let ident = field.ident.as_ref().expect("named fields have names");
        let ty = &field.ty;
        let segment = segment as u32;
        let doc = format!("The `{ident}` field of the `{name}` at this path.");
        declarations.push(quote! {
            #[doc = #doc]
            fn #ident(self) -> #subfield<Self, #ty>;
        });
        definitions.push(quote! {
            fn #ident(self) -> #subfield<Self, #ty> {
                fn get(value: &#name) -> &#ty {
                    &value.#ident
                }
                fn get_mut(value: &mut #name) -> &mut #ty {
                    &mut value.#ident
                }
                #subfield::__new(self, #segment, get, get_mut)
            }
        });
    }
    let doc = format!("Field accessors of `{name}` inside a store (`#[derive(Store)]`).");
    Ok(quote! {
        #[doc = #doc]
        #vis trait #fields_trait: #path<Value = #name> + Sized {
            #(#declarations)*
        }

        impl<P: #path<Value = #name>> #fields_trait for P {
            #(#definitions)*
        }
    })
}
