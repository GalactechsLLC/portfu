mod client;
mod method;
mod server;

use crate::client::websocket::WebSocketClient;
use crate::method::Method;
use crate::server::endpoints::Endpoint;
use crate::server::files::Files;
use crate::server::interval::Interval;
use crate::server::static_files::StaticFiles;
use crate::server::task::Task;
use crate::server::websocket::WebSocketRoute;
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
            match Endpoint::new(
                args,
                ast,
                vec![method::Method::$variant, method::Method::Options],
            ) {
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
pub fn static_files(args: TokenStream, input: TokenStream) -> TokenStream {
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
pub fn files(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemStruct>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match Files::new(args, ast.ident) {
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
pub fn client_websocket(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match WebSocketClient::new(args, ast) {
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
                            let #segment: ::portfu::prelude::Path = ::portfu::pfcore::FromRequest::from_request(&mut handle_data.request, stringify!(#segment)).await.unwrap();
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
                    ::portfu::filters::method::#method.clone()
                }
            })
            .collect();
        quote! {
            .filter(
                ::std::sync::Arc::new(::portfu::filters::any(
                    String::new(),
                    &[
                        ::portfu::filters::method::#first.clone(),
                        #(#other_method_guards),*
                    ]
                ))
            )
        }
    } else {
        quote! {
            .filter(::portfu::filters::method::#first.clone())
        }
    }
}
