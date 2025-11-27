use async_trait::async_trait;
use http::Extensions;
use std::fmt::{Debug, Formatter};
use std::io::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct ServerThreadImpl {
    pub name: String,
    pub handle: Arc<dyn ServerThread + Sync + Send>,
}

#[async_trait]
pub trait ServerThread {
    fn name(&self) -> &str;
    async fn run(&self, state: Arc<RwLock<Extensions>>) -> Result<(), Error>;
}

impl Debug for dyn ServerThread + Send + Sync + 'static {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[async_trait]
impl ServerThread for ServerThreadImpl {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    async fn run(&self, state: Arc<RwLock<Extensions>>) -> Result<(), Error> {
        self.handle.run(state).await
    }
}
