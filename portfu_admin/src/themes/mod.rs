pub mod default;
pub mod page;
pub mod template;
pub mod token;

use crate::themes::page::Page;
use crate::themes::template::Template;
use crate::themes::token::Token;
use portfu::pfcore::routes::Route;
use portfu::pfcore::{IntoStreamBody, ServiceData};
use portfu::prelude::http::StatusCode;
use std::io::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    pub async fn render(
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
