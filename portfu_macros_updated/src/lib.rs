mod client_websocket;
mod endpoint;
mod files;
mod interval;
mod method;
mod static_files;
mod task;
mod utils;
mod websocket;

use crate::client_websocket::WebSocketClient;
use crate::files::Files;
use crate::interval::Interval;
use crate::static_files::StaticFiles;
use crate::task::Task;
use crate::websocket::WebSocketRoute;
use proc_macro::TokenStream;
use quote::ToTokens;

use crate::endpoint::Endpoint;

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

#[proc_macro_attribute]
pub fn endpoint(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match Endpoint::new(args, ast, vec![]) {
        Ok(route) => route.into_token_stream().into(),
        Err(err) => input_and_compile_error(input, err),
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
pub fn task(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = match syn::parse(args) {
        Ok(args) => args,
        Err(err) => return input_and_compile_error(input, err),
    };
    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        Err(err) => return input_and_compile_error(input, err),
    };
    match Task::new(args, ast) {
        Ok(route) => route.into_token_stream().into(),
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
