use std::fmt::{Debug, Formatter};
use std::io::{Error};
use std::sync::Arc;
use async_trait::async_trait;
use http::Extensions;

#[derive(Debug)]
pub struct Task{
    pub name: String,
    pub task_fn: Arc<dyn TaskFn + Sync + Send>
}

#[async_trait]
pub trait TaskFn {
    fn name(&self) -> &str;
    async fn run(&self, state: Extensions) -> Result<(), Error>;
}

impl Debug for (dyn TaskFn + Send + Sync + 'static) {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[async_trait]
impl TaskFn for Task {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    async fn run(&self, state: Extensions) -> Result<(), Error> {
        self.task_fn.run(state).await
    }
}