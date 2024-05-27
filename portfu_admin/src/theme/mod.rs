pub mod default;

use crate::theme::default::DEFAULT_THEME;
use crate::DataStoreEntry;
use portfu::pfcore::editable::EditResult;
use portfu::pfcore::routes::Route;
use portfu::pfcore::service::{Service, ServiceBuilder};
use portfu::pfcore::{
    IntoStreamBody, ServiceData, ServiceHandler, ServiceRegister, ServiceRegistry,
};
use portfu::prelude::async_trait::async_trait;
use portfu::prelude::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use portfu::prelude::http::{Extensions, HeaderValue, StatusCode};
use std::collections::HashMap;
use std::io::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Default, Clone)]
pub struct Token {
    pub key: String,
    pub value: String,
}
impl Token {
    pub const TITLE: &'static str = "<!--{title-token}-->";
    pub const PAGE: &'static str = "<!--{page-token}-->";
    pub const META: &'static str = "<!--{meta-token}-->";
    pub const CSS_PATH: &'static str = "<!--{css-path-token}-->";
    pub const JS_PATH: &'static str = "<!--{js-path-token}-->";
}
impl DataStoreEntry for Token {
    fn key(&self) -> String {
        self.key.clone()
    }

    fn parameters() -> &'static [&'static str] {
        &["key", "value"]
    }

    fn matches(&self, name: &str, other: &str) -> bool {
        match name {
            "key" => self.key == other,
            "value" => self.value == other,
            _ => false,
        }
    }
}

#[derive(Default, Clone)]
pub struct PageMetadata {
    pub title: String,
    pub tags: HashMap<String, String>,
}

#[derive(Default, Clone)]
pub struct Page {
    pub path: String,
    pub metadata: Option<PageMetadata>,
    pub html: String,
    pub css: String,
    pub js: String,
}

#[derive(Clone)]
pub struct Template {
    pub html: String,
    pub css: String,
    pub js: String,
}
impl Default for Template {
    fn default() -> Self {
        Self {
            html: r#"<html><head></head><body><!--{page-token}--></body></html>"#.to_string(),
            css: Default::default(),
            js: Default::default(),
        }
    }
}
impl Template {
    pub async fn render_html(
        &self,
        page: &Page,
        mut data: ServiceData,
        tokens: Arc<RwLock<Vec<Token>>>,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let page_html = replace_tokens(&page.html, tokens.read().await.as_slice()).await;
        let template_html = replace_tokens(
            &self.html,
            &[
                tokens.read().await.as_slice(),
                &[Token {
                    key: Token::PAGE.to_string(),
                    value: page_html,
                }],
            ]
            .concat(),
        )
        .await;
        data.response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("text/html"));
        data.response
            .headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from(template_html.len()));
        *data.response.body_mut() = template_html.stream_body();
        Ok(data)
    }
    pub async fn render_css(
        &self,
        page: &Page,
        mut data: ServiceData,
        tokens: Arc<RwLock<Vec<Token>>>,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let template_css = replace_tokens(&self.css, tokens.read().await.as_slice()).await;
        let page_css = replace_tokens(&page.css, tokens.read().await.as_slice()).await;
        let resp = format!("{template_css}\r\n{page_css}");
        data.response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("text/css"));
        data.response
            .headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from(resp.len()));
        *data.response.body_mut() = resp.stream_body();
        Ok(data)
    }
    pub async fn render_js(
        &self,
        page: &Page,
        mut data: ServiceData,
        tokens: Arc<RwLock<Vec<Token>>>,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let template_js = replace_tokens(&self.js, tokens.read().await.as_slice()).await;
        let page_js = replace_tokens(&page.js, tokens.read().await.as_slice()).await;
        let resp = format!("{template_js}\r\n{page_js}");
        data.response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("text/javascript"));
        data.response
            .headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from(resp.len()));
        Ok(data)
    }
}

#[derive(Default, Clone)]
pub struct ThemeMetadata {
    pub description: String,
    pub demo_image: String,
    pub demo_link: String,
    pub source_link: String,
}

#[derive(Default, Clone)]
pub struct Theme {
    pub name: String,
    pub metadata: ThemeMetadata,
    pub template: Template,
    pub pages: Vec<(Arc<Route>, Page)>,
}
impl Theme {
    async fn render(
        &self,
        mut data: ServiceData,
        tokens: Arc<RwLock<Vec<Token>>>,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let path = if data.request.request.uri().path().ends_with('/') {
            format!("{}index.html", data.request.request.uri().path())
        } else {
            data.request.request.uri().path().to_string()
        };
        for page in &self.pages {
            if page.0.matches(&path) {
                return if path.ends_with(".css") {
                    self.template.render_css(&page.1, data, tokens).await
                } else if path.ends_with(".js") {
                    self.template.render_js(&page.1, data, tokens).await
                } else {
                    self.template.render_html(&page.1, data, tokens).await
                };
            }
        }
        *data.response.status_mut() = StatusCode::NOT_FOUND;
        *data.response.body_mut() = format!("{path} Not Found").stream_body();
        Ok(data)
    }
}

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

async fn replace_tokens(source: &str, tokens: &[Token]) -> String {
    let mut current_value = String::from(source);
    let mut rtn = current_value.clone();
    loop {
        for token in tokens {
            current_value = current_value.replace(&token.key, &token.value);
        }
        if current_value == rtn {
            return rtn;
        } else {
            rtn.clone_from(&current_value);
        }
    }
}
