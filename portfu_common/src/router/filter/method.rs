use crate::router::filter::traits::Filter as FilterFn;
use crate::router::filter::{Filter, FilterMode, FilterResult};
use crate::service::request::Request;
use std::sync::Arc;
macro_rules! method_macro {
    ($variant:ident, $object:ident) => {
        pub struct $object;
        impl FilterFn for $object {
            fn name(&self) -> &str {
                stringify!($variant)
            }
            fn filter<'a>(
                &'a self,
                request: &'a Request,
            ) -> std::pin::Pin<
                Box<dyn ::std::future::Future<Output = FilterResult> + 'a + Send + Sync>,
            > {
                Box::pin(
                    async move { (request.method() == &::http::method::Method::$variant).into() },
                )
            }
        }
        pub static $variant: ::once_cell::sync::Lazy<Arc<Filter>> =
            ::once_cell::sync::Lazy::new(|| {
                Arc::new(Filter {
                    name: stringify!($variant).to_string(),
                    mode: FilterMode::Any,
                    filter_functions: vec![Arc::new($object {})],
                })
            });
    };
}

method_macro!(GET, Get);
method_macro!(POST, Post);
method_macro!(PUT, Put);
method_macro!(DELETE, Delete);
method_macro!(HEAD, Head);
method_macro!(CONNECT, Connect);
method_macro!(OPTIONS, Options);
method_macro!(TRACE, Trace);
method_macro!(PATCH, Patch);
