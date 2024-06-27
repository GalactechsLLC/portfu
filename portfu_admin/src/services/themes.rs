use std::collections::HashMap;
use std::io::Error;
use std::sync::Arc;
use tokio::sync::RwLock;
use portfu::pfcore::{ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry};
use portfu::pfcore::editable::EditResult;
use portfu::pfcore::service::{Service, ServiceBuilder};
use portfu::prelude::async_trait::async_trait;
use portfu::prelude::http::Extensions;
use crate::themes::default::DEFAULT_THEME;
use crate::themes::Theme;
use crate::themes::token::Token;

#[derive(Clone)]
pub struct ThemeSelector {
    pub themes: Arc<RwLock<HashMap<String, Arc<Theme>>>>,
    pub tokens: Arc<RwLock<Vec<Token>>>,
    pub selected_theme: Arc<RwLock<Arc<Theme>>>,
}
impl Default for ThemeSelector {
    fn default() -> Self {
        Self {
            themes: Arc::new(RwLock::new(HashMap::from([(
                String::from("default"),
                DEFAULT_THEME.clone(),
            )]))),
            tokens: Default::default(),
            selected_theme: Arc::new(RwLock::new(DEFAULT_THEME.clone())),
        }
    }
}
#[async_trait]
impl ServiceHandler for ThemeSelector {
    fn name(&self) -> &str {
        "ThemeSelector"
    }

    async fn handle(&self, data: ServiceData) -> Result<ServiceData, (ServiceData, Error)> {
        let theme = self.selected_theme.read().await.clone();
        let tokens = self.tokens.clone();
        theme.render(data, tokens).await
    }
    fn is_editable(&self) -> bool {
        true
    }
    async fn current_value(&self) -> EditResult {
        EditResult::Success(self.selected_theme.read().await.name.as_bytes().to_vec())
    }
    async fn update_value(&self, new_value: Vec<u8>, _: Option<Vec<u8>>) -> EditResult {
        let theme_as_string = String::from_utf8_lossy(&new_value);
        if let Some(theme) = self
            .themes
            .read()
            .await
            .get(theme_as_string.as_ref())
            .cloned()
        {
            *self.selected_theme.write().await = theme;
            EditResult::Success(new_value)
        } else {
            EditResult::Failed(format!("Failed to find theme with name: {theme_as_string}"))
        }
    }
}
impl ServiceRegister for ThemeSelector {
    fn register(self, service_registry: &mut ServiceRegistry, _: Extensions) {
        service_registry.register(self.into())
    }
}
impl From<ThemeSelector> for Service {
    fn from(value: ThemeSelector) -> Self {
        ServiceBuilder::new("/*")
            .name(value.name())
            .handler(Arc::new(value))
            .build()
    }
}