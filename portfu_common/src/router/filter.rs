use crate::service::request::Request;
use std::pin::Pin;
use std::sync::Arc;

#[cfg(feature = "oauth")]
pub mod auth;
pub mod method;
pub use method::*;

#[derive(Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq)]
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

impl From<FilterResult> for bool {
    fn from(value: FilterResult) -> bool {
        value == FilterResult::Allow
    }
}
#[derive(Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq)]
pub enum FilterMode {
    Any,
    All,
}

pub mod traits {
    use crate::service::request::Request;
    use std::pin::Pin;
    pub trait Filter {
        fn name(&self) -> &str;
        fn filter<'a>(
            &'a self,
            request: &'a Request,
        ) -> Pin<Box<dyn Future<Output = super::FilterResult> + 'a + Send + Sync>>;
    }
}

#[derive(Clone)]
pub struct Filter {
    pub name: String,
    pub mode: FilterMode,
    pub filter_functions: Vec<Arc<dyn traits::Filter + Sync + Send>>,
}
impl Filter {
    pub fn or(self, filter: Arc<dyn traits::Filter + Sync + Send>) -> Filter {
        Filter {
            name: traits::Filter::name(&self).to_string(),
            mode: FilterMode::Any,
            filter_functions: vec![Arc::new(self), filter],
        }
    }
}
impl traits::Filter for Filter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn filter<'a>(
        &'a self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move {
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
        })
    }
}

pub fn any(name: String, filter: &[Arc<dyn traits::Filter + Sync + Send>]) -> Filter {
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

pub fn all(name: String, filter: &[Arc<dyn traits::Filter + Sync + Send>]) -> Filter {
    Filter {
        name,
        mode: FilterMode::All,
        filter_functions: filter.to_vec(),
    }
}
