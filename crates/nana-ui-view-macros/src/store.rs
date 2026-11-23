//! `#[derive(Store)]`.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::{Data, DataEnum, DeriveInput, Fields, GenericParam, Generics, Ident};

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
    let name = &input.ident;
    let vis = &input.vis;
    let fields_trait = format_ident!("{name}StoreFields");
    let path = quote!(#krate::view::StorePath);
    let (_, ty_generics, _) = input.generics.split_for_impl();
    let value = quote!(#name #ty_generics);
    let accessors = match &input.data {
        Data::Struct(data) => struct_accessors(name, &value, &data.fields, krate)?,
        Data::Enum(data) => enum_accessors(name, &value, data, krate)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "`#[derive(Store)]` takes a struct with named fields or an enum",
            ));
        }
    };
    // A store holds `'static` values: every parameter is bound by it, so a
    // field built from them is too.
    let mut where_clause = input
        .generics
        .where_clause
        .clone()
        .unwrap_or_else(|| syn::parse_quote!(where));
    for param in &input.generics.params {
        match param {
            GenericParam::Lifetime(lifetime) => {
                let lifetime = &lifetime.lifetime;
                where_clause
                    .predicates
                    .push(syn::parse_quote!(#lifetime: 'static));
            }
            GenericParam::Type(ty) => {
                let ty = &ty.ident;
                where_clause
                    .predicates
                    .push(syn::parse_quote!(#ty: 'static));
            }
            GenericParam::Const(_) => {}
        }
    }
    let trait_generics = &input.generics.params;
    let trait_args = generic_args(&input.generics);
    let path_param = format_ident!("__NanaStorePath");
    let declarations = accessors.iter().map(|accessor| &accessor.declaration);
    let definitions = accessors.iter().map(|accessor| &accessor.definition);
    let doc = format!("Accessors of `{name}` inside a store (`#[derive(Store)]`).");
    let impl_params = impl_generics_with(&input.generics, &path_param);
    let mut impl_where = where_clause.clone();
    impl_where
        .predicates
        .push(syn::parse_quote!(#path_param: #path<Value = #value>));
    Ok(quote! {
        #[doc = #doc]
        #vis trait #fields_trait<#trait_generics>: #path<Value = #value> + Sized
        #where_clause
        {
            #(#declarations)*
        }

        impl<#impl_params> #fields_trait<#trait_args> for #path_param
        #impl_where
        {
            #(#definitions)*
        }
    })
}

struct Accessor {
    declaration: TokenStream,
    definition: TokenStream,
}

fn struct_accessors(
    name: &Ident,
    value: &TokenStream,
    fields: &Fields,
    krate: &TokenStream,
) -> syn::Result<Vec<Accessor>> {
    let Fields::Named(fields) = fields else {
        return Err(syn::Error::new_spanned(
            name,
            "`#[derive(Store)]` takes a struct with named fields or an enum",
        ));
    };
    let subfield = quote!(#krate::view::Subfield);
    Ok(fields
        .named
        .iter()
        .enumerate()
        .map(|(segment, field)| {
            let ident = field.ident.as_ref().expect("named fields have names");
            let ty = &field.ty;
            let segment = segment as u32;
            let doc = format!("The `{ident}` field of the `{name}` at this path.");
            Accessor {
                declaration: quote! {
                    #[doc = #doc]
                    fn #ident(self) -> #subfield<Self, #ty>;
                },
                definition: quote! {
                    fn #ident(self) -> #subfield<Self, #ty> {
                        let get: for<'__v> fn(&'__v #value) -> &'__v #ty = |value| &value.#ident;
                        let get_mut: for<'__v> fn(&'__v mut #value) -> &'__v mut #ty =
                            |value| &mut value.#ident;
                        #subfield::__new(self, #segment, get, get_mut)
                    }
                },
            }
        })
        .collect())
}

/// Per variant, `is_<variant>()`, tracked so that it runs again only when
/// the variant may change, and a guarded path per field:
/// `<variant>_<field>()` for named fields, `<variant>()` for a single
/// unnamed one and `<variant>_<n>()` for several.
fn enum_accessors(
    name: &Ident,
    value: &TokenStream,
    data: &DataEnum,
    krate: &TokenStream,
) -> syn::Result<Vec<Accessor>> {
    let variant_path = quote!(#krate::view::Variant);
    let variant_is = quote!(#krate::view::__variant_is);
    let mut accessors = Vec::new();
    let mut segment = 0u32;
    for (index, variant) in data.variants.iter().enumerate() {
        let ident = &variant.ident;
        let snake = snake_case(&ident.to_string());
        let index = index as u32;
        let is = accessor_ident(&format!("is_{snake}"), ident.span());
        let pattern = match &variant.fields {
            Fields::Named(_) => quote!(#name::#ident { .. }),
            Fields::Unnamed(_) => quote!(#name::#ident(..)),
            Fields::Unit => quote!(#name::#ident),
        };
        let doc = format!(
            "Whether the `{name}` at this path is `{ident}`. Tracked: runs again only when \
             the value at this path, or above it, is written."
        );
        accessors.push(Accessor {
            declaration: quote! {
                #[doc = #doc]
                #[track_caller]
                fn #is(self) -> bool;
            },
            definition: quote! {
                #[track_caller]
                fn #is(self) -> bool {
                    let test: fn(&#value) -> bool = |value| matches!(value, #pattern);
                    #variant_is(self, #index, test)
                }
            },
        });
        let fields: Vec<_> = variant.fields.iter().collect();
        for (position, field) in fields.iter().enumerate() {
            let ty = &field.ty;
            let (method, binding, pattern) = match &field.ident {
                Some(field_ident) => (
                    accessor_ident(
                        &format!("{snake}_{}", field_ident.unraw()),
                        field_ident.span(),
                    ),
                    field_ident.clone(),
                    quote!(#name::#ident { #field_ident, .. }),
                ),
                None => {
                    let method = if fields.len() == 1 {
                        accessor_ident(&snake, ident.span())
                    } else {
                        accessor_ident(&format!("{snake}_{position}"), ident.span())
                    };
                    let binding = format_ident!("field");
                    let blanks = (0..fields.len()).map(|other| {
                        if other == position {
                            quote!(#binding)
                        } else {
                            quote!(_)
                        }
                    });
                    let pattern = quote!(#name::#ident(#(#blanks),*));
                    (method, binding, pattern)
                }
            };
            let field_name = match &field.ident {
                Some(field_ident) => format!("`{}`", field_ident.unraw()),
                None => format!("field {position}"),
            };
            let doc = format!(
                "{field_name} of `{name}::{ident}` at this path: leads somewhere only while \
                 the value is that variant."
            );
            let segment_now = segment;
            segment += 1;
            accessors.push(Accessor {
                declaration: quote! {
                    #[doc = #doc]
                    fn #method(self) -> #variant_path<Self, #ty>;
                },
                definition: quote! {
                    fn #method(self) -> #variant_path<Self, #ty> {
                        #[allow(unreachable_patterns)]
                        let get: for<'__v> fn(&'__v #value) -> ::core::option::Option<&'__v #ty> =
                            |value| match value {
                            #pattern => ::core::option::Option::Some(#binding),
                            _ => ::core::option::Option::None,
                        };
                        #[allow(unreachable_patterns)]
                        let get_mut: for<'__v> fn(
                            &'__v mut #value,
                        ) -> ::core::option::Option<&'__v mut #ty> = |value| match value {
                                #pattern => ::core::option::Option::Some(#binding),
                                _ => ::core::option::Option::None,
                            };
                        #variant_path::__new(self, #segment_now, get, get_mut)
                    }
                },
            });
        }
    }
    Ok(accessors)
}

/// `LoadingMore` → `loading_more`.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (at, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let after_lower =
                at > 0 && (chars[at - 1].is_lowercase() || chars[at - 1].is_numeric());
            let before_lower = chars.get(at + 1).is_some_and(|next| next.is_lowercase());
            let after_upper = at > 0 && chars[at - 1].is_uppercase();
            if after_lower || (after_upper && before_lower) {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// An accessor name, raw when it is a keyword.
fn accessor_ident(name: &str, span: Span) -> Ident {
    match syn::parse_str::<Ident>(name) {
        Ok(_) => Ident::new(name, span),
        Err(_) => Ident::new_raw(name, span),
    }
}

/// The struct's parameters as arguments: `'a, T, N`.
fn generic_args(generics: &Generics) -> TokenStream {
    let args = generics.params.iter().map(|param| match param {
        GenericParam::Lifetime(lifetime) => {
            let lifetime = &lifetime.lifetime;
            quote!(#lifetime)
        }
        GenericParam::Type(ty) => {
            let ident = &ty.ident;
            quote!(#ident)
        }
        GenericParam::Const(constant) => {
            let ident = &constant.ident;
            quote!(#ident)
        }
    });
    quote!(#(#args),*)
}

/// The struct's parameters, without defaults, then the path parameter
/// bound to the struct: what the blanket impl declares.
fn impl_generics_with(generics: &Generics, path_param: &Ident) -> TokenStream {
    let mut params: Vec<TokenStream> = Vec::new();
    for param in &generics.params {
        params.push(match param {
            GenericParam::Lifetime(lifetime) => quote!(#lifetime),
            GenericParam::Type(ty) => {
                let mut ty = ty.clone();
                ty.eq_token = None;
                ty.default = None;
                quote!(#ty)
            }
            GenericParam::Const(constant) => {
                let mut constant = constant.clone();
                constant.eq_token = None;
                constant.default = None;
                quote!(#constant)
            }
        });
    }
    params.push(quote!(#path_param));
    quote!(#(#params),*)
}
