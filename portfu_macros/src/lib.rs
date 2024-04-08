mod endpoints;
mod static_files;
mod websocket;

mod interval;
mod postgres;
mod task;

use crate::endpoints::Endpoint;
use crate::interval::Interval;
use crate::method::Method;
use crate::static_files::StaticFiles;
use crate::task::Task;
use crate::websocket::WebSocketRoute;
use portfu_core::routes::PathSegment;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use std::collections::HashSet;
use syn::LitStr;

/// Converts the error to a token stream and appends it to the original input.
///
/// Returning the original input in addition to the error is good for IDEs which can gracefully
/// recover and show more precise errors within the macro body.
///
/// See <https://github.com/rust-analyzer/rust-analyzer/issues/10468> for more info.
fn input_and_compile_error(mut item: TokenStream, err: syn::Error) -> TokenStream {
    let compile_err = TokenStream::from(err.to_compile_error());
    item.extend(compile_err);
    item
}

mod method {
    use proc_macro2::{Span, TokenStream as TokenStream2};
    use quote::{ToTokens, TokenStreamExt};
    use syn::Ident;
    macro_rules! standard_method_type {
        (
            $($variant:ident, $upper:ident, $lower:ident,)+
        ) => {
            #[derive(Debug, Clone, PartialEq, Eq, Hash)]
            pub enum Method {
                $(
                    $variant,
                )+
            }
            impl Method {
                pub fn as_str(&self) -> &'static str {
                    match self {
                        $(Self::$variant => stringify!($upper),)+
                    }
                }
                fn parse(method: &str) -> Result<Self, String> {
                    match method {
                        $(stringify!($upper) => Ok(Self::$variant),)+
                        _ => Err(format!("HTTP method must be uppercase: `{}`", method)),
                    }
                }
            }
        };
    }

    standard_method_type! {
        Get,       GET,     get,
        Post,      POST,    post,
        Put,       PUT,     put,
        Delete,    DELETE,  delete,
        Head,      HEAD,    head,
        Connect,   CONNECT, connect,
        Options,   OPTIONS, options,
        Trace,     TRACE,   trace,
        Patch,     PATCH,   patch,
    }

    impl TryFrom<&syn::LitStr> for Method {
        type Error = syn::Error;
        fn try_from(value: &syn::LitStr) -> Result<Self, Self::Error> {
            Self::parse(value.value().as_str())
                .map_err(|message| syn::Error::new_spanned(value, message))
        }
    }
    impl ToTokens for Method {
        fn to_tokens(&self, stream: &mut TokenStream2) {
            let ident = Ident::new(self.as_str(), Span::call_site());
            stream.append(ident);
        }
    }
}

macro_rules! method_macro {
    ($variant:ident, $method:ident) => {
        #[proc_macro_attribute]
        pub fn $method(args: TokenStream, input: TokenStream) -> TokenStream {
            let args = match syn::parse(args) {
                Ok(args) => args,
                Err(err) => return input_and_compile_error(input, err),
            };
            let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
                Ok(ast) => ast,
                Err(err) => return input_and_compile_error(input, err),
            };
            match Endpoint::new(args, ast, Some(method::Method::$variant)) {
                Ok(route) => route.into_token_stream().into(),
                Err(err) => input_and_compile_error(input, err),
            }
        }
    };
}

method_macro!(Get, get);
method_macro!(Post, post);
method_macro!(Put, put);
method_macro!(Delete, delete);
method_macro!(Head, head);
method_macro!(Connect, connect);
method_macro!(Options, options);
method_macro!(Trace, trace);
method_macro!(Patch, patch);

#[proc_macro_attribute]
pub fn files(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemStruct>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match StaticFiles::new(args, ast.ident) {
        Ok(route) => route.into_token_stream().into(),
        Err(err) => input_and_compile_error(input, err),
    }
}

#[proc_macro_attribute]
pub fn websocket(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match WebSocketRoute::new(args, ast) {
        Ok(route) => route.into_token_stream().into(),
        Err(err) => input_and_compile_error(input, err),
    }
}

#[proc_macro_attribute]
pub fn task(_: TokenStream, input: TokenStream) -> TokenStream {
    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match Task::new(ast) {
        Ok(task) => task.into_token_stream().into(),
        Err(err) => input_and_compile_error(input, err),
    }
}

#[proc_macro_attribute]
pub fn interval(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match Interval::new(args, ast) {
        Ok(task) => task.into_token_stream().into(),
        Err(err) => input_and_compile_error(input, err),
    }
}

fn parse_path_variables(path: &LitStr) -> (Vec<TokenStream2>, Vec<String>) {
    let mut path_vars = vec![];
    match portfu_core::routes::Route::new(path.value()) {
        portfu_core::routes::Route::Static(_, _) => (vec![quote! {}], vec![]),
        portfu_core::routes::Route::Segmented(segments, _) => {
            let mut variables = vec![];
            for segment in segments.iter().filter_map(|v| match v {
                PathSegment::Static(_) => None,
                PathSegment::Variable(v) => Some(Ident::new(v.name.as_str(), Span::call_site())),
            }) {
                variables.push(
                    quote! {
                            let #segment: ::portfu::prelude::Path = ::portfu::pfcore::FromRequest::from_request(&mut data.request, stringify!(#segment)).await.unwrap();
                        }
                );
                path_vars.push(format!("{segment}"));
            }
            (variables, path_vars)
        }
    }
}

fn extract_method_filters(methods: &HashSet<Method>) -> TokenStream2 {
    debug_assert!(!methods.is_empty(), "Args::methods should not be empty");
    let mut others = methods.iter();
    let first = others.next().unwrap();
    if methods.len() > 1 {
        let other_method_guards: Vec<TokenStream2> = others
            .map(|method| {
                quote! {
                    .or(::portfu::filters::method::#method.clone())
                }
            })
            .collect();
        quote! {
            .filter(
                ::portfu::filters::any(::portfu::filters::method::#first.clone())
                    #(#other_method_guards)*
            )
        }
    } else {
        quote! {
            .filter(::portfu::filters::method::#first.clone())
        }
    }
}
