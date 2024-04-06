use http::{HeaderName, Request};
use hyper::body::Incoming;
use portfu_core::filters::{Filter, FilterFn, FilterMode, FilterResult};
use std::sync::Arc;

pub mod method;

pub fn any(name: String, filter: &[Arc<dyn FilterFn + Sync + Send>]) -> Filter {
    Filter {
        name: name.clone(),
        mode: FilterMode::Any,
        filter_functions: vec![Arc::new(Filter {
            name,
            mode: FilterMode::Any,
            filter_functions: filter.to_vec(),
        })],
    }
}

pub fn all(name: String, filter: &[Arc<dyn FilterFn + Sync + Send>]) -> Filter {
    Filter {
        name,
        mode: FilterMode::All,
        filter_functions: filter.to_vec(),
    }
}

struct HasHeader(HeaderName);
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
        mode: FilterMode::All,
        filter_functions: vec![Arc::new(HasHeader(header))],
    }
}
