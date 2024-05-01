use async_trait::async_trait;
use http::Request;
use hyper::body::Incoming;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

#[derive(Clone, Copy, Ord, PartialOrd, PartialEq, Eq)]
pub enum FilterResult {
    Allow,
    Block,
}
#[derive(Clone, Copy, Ord, PartialOrd, PartialEq, Eq)]
pub enum FilterMode {
    Any,
    All,
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

#[async_trait]
pub trait FilterFn {
    fn name(&self) -> &str;
    async fn filter(&self, request: &Request<Incoming>) -> FilterResult;
}
impl Debug for (dyn FilterFn + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone)]
pub struct Filter {
    pub name: String,
    pub mode: FilterMode,
    pub filter_functions: Vec<Arc<dyn FilterFn + Sync + Send>>,
}
impl Filter {
    pub fn or(self, filter: Arc<dyn FilterFn + Sync + Send>) -> Filter {
        Filter {
            name: self.name().to_string(),
            mode: FilterMode::Any,
            filter_functions: vec![Arc::new(self), filter],
        }
    }
}
impl Debug for Filter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for func in &self.filter_functions {
            f.write_str(func.name())?;
        }
        Ok(())
    }
}

#[async_trait]
impl FilterFn for Filter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    async fn filter(&self, request: &Request<Incoming>) -> FilterResult {
        match self.mode {
            FilterMode::Any => {
                for f in self.filter_functions.iter() {
                    if f.filter(request).await == FilterResult::Allow {
                        return FilterResult::Allow;
                    }
                }
                FilterResult::Block
            }
            FilterMode::All => {
                for f in self.filter_functions.iter() {
                    if f.filter(request).await != FilterResult::Allow {
                        return FilterResult::Block;
                    }
                }
                FilterResult::Allow
            }
        }
    }
}
