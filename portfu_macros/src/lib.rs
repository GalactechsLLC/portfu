#[cfg(feature = "client")]
mod client_websocket;
#[cfg(feature = "endpoint")]
mod endpoint;
#[cfg(feature = "files")]
mod files;
#[cfg(feature = "tasks")]
mod interval;
#[cfg(any(feature = "endpoint", feature = "websocket"))]
mod method;
#[cfg(feature = "files")]
mod static_files;
#[cfg(feature = "tasks")]
mod task;
#[cfg(any(feature = "endpoint", feature = "websocket"))]
mod utils;
#[cfg(feature = "websocket")]
mod websocket;

#[cfg(feature = "client")]
use crate::client_websocket::WebSocketClient;
#[cfg(feature = "files")]
use crate::files::Files;
#[cfg(feature = "tasks")]
use crate::interval::Interval;
#[cfg(feature = "files")]
use crate::static_files::StaticFiles;
#[cfg(feature = "tasks")]
use crate::task::Task;
#[cfg(feature = "websocket")]
use crate::websocket::WebSocketRoute;
#[cfg(any(
    feature = "client",
    feature = "endpoint",
    feature = "files",
    feature = "tasks",
    feature = "websocket"
))]
use proc_macro::TokenStream;
#[cfg(any(
    feature = "client",
    feature = "endpoint",
    feature = "files",
    feature = "tasks",
    feature = "websocket"
))]
use quote::ToTokens;

#[cfg(feature = "endpoint")]
use crate::endpoint::Endpoint;

/// Converts the error to a token stream and appends it to the original input.
///
/// Returning the original input in addition to the error is good for IDEs which can gracefully
/// recover and show more precise errors within the macro body.
///
/// See <https://github.com/rust-analyzer/rust-analyzer/issues/10468> for more info.
#[cfg(any(
    feature = "client",
    feature = "endpoint",
    feature = "files",
    feature = "tasks",
    feature = "websocket"
))]
fn input_and_compile_error(mut item: TokenStream, err: syn::Error) -> TokenStream {
    let compile_err = TokenStream::from(err.to_compile_error());
    item.extend(compile_err);
    item
}

#[cfg(feature = "endpoint")]
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
        #[cfg(feature = "endpoint")]
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

#[cfg(feature = "files")]
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

#[cfg(feature = "files")]
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

#[cfg(feature = "tasks")]
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

#[cfg(feature = "tasks")]
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

#[cfg(feature = "websocket")]
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

#[cfg(feature = "client")]
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
