use crate::method::Method;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::quote;
use std::collections::HashSet;
use syn::LitStr;

pub(crate) fn parse_path_variables(path: &LitStr) -> (Vec<TokenStream2>, Vec<String>) {
    let mut path_vars = vec![];
    match portfu_common::router::route::Route::new(path.value()) {
        portfu_common::router::route::Route::Static(_, _) => (vec![quote! {}], vec![]),
        portfu_common::router::route::Route::Segmented(segments, _) => {
            let mut variables = vec![];
            for segment in segments.iter().filter_map(|v| match v {
                portfu_common::router::route::PathSegment::Static(_) => None,
                portfu_common::router::route::PathSegment::Variable(v) => {
                    Some(Ident::new(v.name.as_str(), Span::call_site()))
                }
            }) {
                let seg_lit = syn::LitStr::new(&segment.to_string(), segment.span());
                variables.push(
                    quote! {
                            #[allow(non_camel_case_types)]
                            pub enum #segment {}

                            impl ::portfu_updated::prelude::PathName for #segment {
                                const NAME: &'static str = #seg_lit;
                            }
                            let #segment: ::portfu_updated::prelude::PathImpl::<#segment> = ::portfu_updated::prelude::FromRequest::try_from(request).await?;
                            let #segment: ::portfu_updated::prelude::Path = #segment.into_path();

                        }
                );
                path_vars.push(format!("{segment}"));
            }
            (variables, path_vars)
        }
    }
}

pub(crate) fn extract_method_filters(methods: &HashSet<Method>) -> TokenStream2 {
    debug_assert!(!methods.is_empty(), "Args::methods should not be empty");
    let mut others = methods.iter();
    let first = others.next().unwrap();
    if methods.len() > 1 {
        let other_method_guards: Vec<TokenStream2> = others
            .map(|method| {
                quote! {
                    ::portfu_updated::prelude::filters::method::#method.clone()
                }
            })
            .collect();
        quote! {
            .filter(
                ::std::sync::Arc::new(::portfu_updated::prelude::filters::any(
                    String::new(),
                    &[
                        ::portfu_updated::prelude::filters::method::#first.clone(),
                        #(#other_method_guards),*
                    ]
                ))
            )
        }
    } else {
        quote! {
            .filter(::portfu_updated::prelude::filters::method::#first.clone())
        }
    }
}
