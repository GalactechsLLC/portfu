use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use http::{HeaderName, Request};
use hyper::body::Incoming;

#[derive(Ord, PartialOrd, PartialEq, Eq)]
pub enum FilterResult {
    Allow,
    Block,
}

impl From<bool> for FilterResult {
    fn from(value: bool) -> Self {
        if value {
            FilterResult::Allow
        } else {
            FilterResult::Block
        }
    }
}

pub trait FilterFn {
    fn name(&self) -> &str;
    fn filter(
        &self,
        request: &Request<Incoming>,
    ) -> FilterResult;
}

#[derive(Clone)]
pub struct Filter {
    name: String,
    filter_functions: Vec<Arc<dyn FilterFn + Sync + Send>>
}
impl Filter {
    pub fn or(&self, filter: Arc<dyn FilterFn + Sync + Send>) -> FilterOr {
        FilterOr {
            name: self.name().to_string(),
            filter_functions: self.filter_functions.iter().chain(&[filter]).cloned().collect::<Vec<Arc<dyn FilterFn + Sync + Send>>>()
        }
    }
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
        impl<'a> crate::filters::FilterFn for $object {
            fn name(&self) -> &str {
                stringify!($variant)
            }
            fn filter(
                &self,
                request: &Request<Incoming>
            ) -> FilterResult {
                (*request.method() == ::http::method::Method::$variant).into()
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

pub fn any(name: String, filter: &[Arc<dyn FilterFn + Sync + Send>]) -> Filter {
    Filter {
        name: name.clone(),
        filter_functions: vec![Arc::new(FilterOr {
            name,
            filter_functions: filter.iter().cloned().collect(),
        })]
    }
}

pub fn all(name: String, filter: &[Arc<dyn FilterFn + Sync + Send>]) -> Filter {
    Filter {
        name,
        filter_functions: filter.iter().cloned().collect(),
    }
}

pub struct FilterOr {
    name: String,
    filter_functions: Vec<Arc<dyn FilterFn + Sync + Send>>
}
impl FilterOr {
    pub fn or(&self, filter: Arc<dyn FilterFn + Sync + Send>) -> FilterOr {
        FilterOr {
            name: self.name().to_string(),
            filter_functions: self.filter_functions.iter().chain(&[filter]).cloned().collect::<Vec<Arc<dyn FilterFn + Sync + Send>>>()
        }
    }
}
impl FilterFn for FilterOr {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn filter(&self, request: &Request<Incoming>) -> FilterResult {
        if self.filter_functions.iter().cloned().any(|f| {
            f.filter(request) == FilterResult::Allow
        }) {
            FilterResult::Allow
        } else {
            FilterResult::Block
        }
    }
}
impl FilterFn for Filter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn filter(&self, request: &Request<Incoming>) -> FilterResult {
        if self.filter_functions.iter().cloned().all(|f| {
            f.filter(request) == FilterResult::Allow
        }) {
            FilterResult::Allow
        } else {
            FilterResult::Block
        }
    }
}

pub struct HasHeader(HeaderName);
impl FilterFn for HasHeader {
    fn name(&self) -> &str {
        self.0.as_str()
    }

    fn filter(&self, request: &Request<Incoming>) -> FilterResult {
        request.headers().contains_key(&self.0).into()
    }
}

pub fn has_header(header: HeaderName) -> Filter {
    Filter {
        name: format!("has_header_{header}"),
        filter_functions: vec![Arc::new(HasHeader(header))]
    }
}