use crate::stores::DataStoreEntry;
use crate::themes::page::Page;
use crate::themes::replace_tokens;
use crate::themes::token::Token;
use portfu::pfcore::service::BodyType;
use portfu::pfcore::{IntoStreamBody, ServiceData};
use portfu::prelude::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use portfu::prelude::http::HeaderValue;
use portfu::prelude::uuid::Uuid;
use std::collections::HashMap;
use std::io::Error;
use std::sync::Arc;
use struct_field_names_as_array::FieldNamesAsSlice;
use tokio::sync::RwLock;

#[derive(Clone, Eq, PartialEq, FieldNamesAsSlice)]
pub struct Template {
    pub id: isize,
    pub uuid: Uuid,
    pub title: String,
    pub tags: HashMap<String, String>,
    pub html: String,
    pub css: String,
    pub js: String,
}
impl DataStoreEntry<isize> for Template {
    fn key_name() -> &'static str {
        "id"
    }

    fn key_value(&self) -> isize {
        self.id
    }

    fn parameters() -> &'static [&'static str] {
        &["id", "uuid"]
    }

    fn matches(&self, name: &str, other: &str) -> bool {
        if !Self::FIELD_NAMES_AS_SLICE.contains(&name) {
            return false;
        }
        match name {
            "id" => self.id.to_string() == other,
            "uuid" => self.uuid.to_string() == other,
            _ => false,
        }
    }
}
impl Default for Template {
    fn default() -> Self {
        Self {
            id: -1,
            uuid: Default::default(),
            title: "Default Template".to_string(),
            tags: Default::default(),
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
        data.response
            .set_body(BodyType::Stream(template_html.stream_body()));
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
        data.response.set_body(BodyType::Stream(resp.stream_body()));
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
