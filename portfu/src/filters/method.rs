use async_trait::async_trait;
use http::Request;
use hyper::body::Incoming;
use std::sync::Arc;

macro_rules! method_macro {
    ($variant:ident, $object:ident, $method:ident) => {
        pub struct $object {}
        #[async_trait]
        impl<'a> portfu_core::router::filters::FilterFn for $object {
            fn name(&self) -> &str {
                stringify!($variant)
            }
            async fn filter(
                &self,
                request: &Request<Incoming>,
            ) -> ::portfu_core::router::filters::FilterResult {
                (*request.method() == ::http::method::Method::$variant).into()
            }
        }
        pub static $variant: ::once_cell::sync::Lazy<
            ::std::sync::Arc<::portfu_core::router::filters::Filter>,
        > = ::once_cell::sync::Lazy::new(|| {
            ::std::sync::Arc::new(::portfu_core::router::filters::Filter {
                name: stringify!($variant).to_string(),
                mode: ::portfu_core::router::filters::FilterMode::Any,
                filter_functions: vec![Arc::new($object {})],
            })
        });
    };
}

method_macro!(GET, Get, get);
method_macro!(POST, Post, post);
method_macro!(PUT, Put, put);
method_macro!(DELETE, Delete, delete);
method_macro!(HEAD, Head, head);
method_macro!(CONNECT, Connect, connect);
method_macro!(OPTIONS, Options, options);
method_macro!(TRACE, Trace, trace);
method_macro!(PATCH, Patch, patch);
