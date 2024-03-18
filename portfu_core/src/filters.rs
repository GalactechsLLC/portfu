use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use http::{Response};
use http_body_util::Full;
use hyper::body::{Bytes};
use crate::service::ServiceRequest;
use crate::ServiceHandler;
pub struct Filter {
    name: String,
    filter_functions: Vec<Arc<dyn ServiceHandler + Sync + Send>>
}
impl<'a> Debug for Filter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for func in &self.filter_functions {
            f.write_str(func.name())?;
        }
        Ok(())
    }
}

macro_rules! method_macro {
    ($variant:ident, $object:ident, $method:ident) => {
        pub struct $object {}
        #[async_trait::async_trait]
        impl<'a> crate::ServiceHandler for $object {
            fn name(&self) -> &str {
                stringify!($variant)
            }
            async fn handle(
                &self,
                _address: &::std::net::SocketAddr,
                service_request: &ServiceRequest,
                response: ::http::Response<::http_body_util::Full<::hyper::body::Bytes>>
            ) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
                assert_eq!(service_request.request.method(), ::http::Method::$variant);
                Ok(response)
            }
        }
        pub static $variant: ::once_cell::sync::Lazy<::std::sync::Arc<Filter>> = ::once_cell::sync::Lazy::new( || {
            ::std::sync::Arc::new(Filter {
                name: stringify!($variant).to_string(),
                filter_functions: vec![
                    Arc::new($object {}),
                ]
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

pub fn any(name: String, filter: Box<&[Arc<dyn ServiceHandler + Sync + Send>]>) -> Filter {
    Filter {
        name,
        filter_functions: filter.iter().cloned().collect(),
    }
}

impl Filter {
    pub fn or(mut self, filter: Arc<dyn ServiceHandler + Sync + Send>) -> Self {
        self.filter_functions.push(filter);
        self
    }
}
#[async_trait]
impl ServiceHandler for Filter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    async fn handle(&self, address: &SocketAddr, request: &ServiceRequest, response: Response<Full<Bytes>>) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
        let mut response = response;
        for filter_fn in self.filter_functions.iter().cloned() {
            response = filter_fn.handle(address, request, response).await?;
        }
        Ok(response)
    }
}