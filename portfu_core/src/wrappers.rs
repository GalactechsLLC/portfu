use crate::ServiceData;
use async_trait::async_trait;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

pub enum WrapperResult {
    Continue,
    Return,
}

impl From<bool> for WrapperResult {
    fn from(value: bool) -> Self {
        if value {
            WrapperResult::Continue
        } else {
            WrapperResult::Return
        }
    }
}

#[async_trait]
pub trait WrapperFn {
    fn name(&self) -> &str;
    async fn before(&self, data: &mut ServiceData) -> WrapperResult;
    async fn after(&self, data: &mut ServiceData) -> WrapperResult;
}
impl Debug for (dyn WrapperFn + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Debug)]
pub struct Wrapper {
    pub name: String,
    pub wrapper_functions: Vec<Arc<dyn WrapperFn + Sync + Send>>,
}
#[async_trait]
impl WrapperFn for Wrapper {
    fn name(&self) -> &str {
        &self.name
    }

    async fn before(&self, data: &mut ServiceData) -> WrapperResult {
        for func in self.wrapper_functions.iter() {
            match func.before(data).await {
                WrapperResult::Continue => continue,
                WrapperResult::Return => {
                    return WrapperResult::Return;
                }
            }
        }
        WrapperResult::Continue
    }

    async fn after(&self, data: &mut ServiceData) -> WrapperResult {
        for func in self.wrapper_functions.iter() {
            match func.after(data).await {
                WrapperResult::Continue => continue,
                WrapperResult::Return => {
                    return WrapperResult::Return;
                }
            }
        }
        WrapperResult::Continue
    }
}
