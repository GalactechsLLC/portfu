use crate::ServiceData;
use async_trait::async_trait;
use std::fmt::{Debug, Formatter};
use std::io::Error;
use std::sync::Arc;

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum MiddlewareResult {
    Continue,
    Return,
}

impl From<bool> for MiddlewareResult {
    fn from(value: bool) -> Self {
        if value {
            MiddlewareResult::Continue
        } else {
            MiddlewareResult::Return
        }
    }
}

#[async_trait]
pub trait Middleware {
    fn name(&self) -> &str;
    async fn before(&self, data: &mut ServiceData) -> Result<MiddlewareResult, Error>;
    async fn after(&self, data: &mut ServiceData) -> Result<MiddlewareResult, Error>;
}
impl Debug for dyn Middleware + Send + Sync + 'static {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Debug)]
pub struct MiddlewareImpl {
    pub name: String,
    pub handlers: Vec<Arc<dyn Middleware + Sync + Send>>,
}
#[async_trait]
impl Middleware for MiddlewareImpl {
    fn name(&self) -> &str {
        &self.name
    }

    async fn before(&self, data: &mut ServiceData) -> Result<MiddlewareResult, Error> {
        for func in self.handlers.iter() {
            match func.before(data).await? {
                MiddlewareResult::Continue => continue,
                MiddlewareResult::Return => {
                    return Ok(MiddlewareResult::Return);
                }
            }
        }
        Ok(MiddlewareResult::Continue)
    }

    async fn after(&self, data: &mut ServiceData) -> Result<MiddlewareResult, Error> {
        for func in self.handlers.iter() {
            match func.after(data).await? {
                MiddlewareResult::Continue => continue,
                MiddlewareResult::Return => {
                    return Ok(MiddlewareResult::Return);
                }
            }
        }
        Ok(MiddlewareResult::Continue)
    }
}
