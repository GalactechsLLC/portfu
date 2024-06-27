use crate::themes::{Theme};
use crate::themes::page::Page;
use crate::themes::template::Template;
use portfu::pfcore::routes::Route;
use portfu::prelude::once_cell::sync::Lazy;
use regex::Regex;
use std::borrow::Cow;
use std::sync::Arc;
use portfu::prelude::uuid::Uuid;

pub static DEFAULT_THEME: Lazy<Arc<Theme>> = Lazy::new(|| {
    Arc::new(Theme {
        name: "Default Theme".to_string(),
        metadata: Default::default(),
        template: Template {
            id: -1,
            uuid: Uuid::new_v4(),
            title: "".to_string(),
            tags: Default::default(),
            html: include_str!("../../front_end_dist/themes/default/template.html").to_string(),
            css: include_str!("../../front_end_dist/themes/default/template.css").to_string(),
            js: include_str!("../../front_end_dist/themes/default/template.js").to_string(),
        },
        pages: vec![
            (
                Arc::new(Route::Static(
                    Cow::Borrowed("/contact.html"),
                    Regex::new(r"/contact\.(css|html|js)")
                        .expect("Expected Theme to have Valid Regex"),
                )),
                Page {
                    path: "/contact.html".to_string(),
                    metadata: None,
                    html: include_str!(
                        "../../front_end_dist/themes/default/pages/contact/page.html"
                    )
                    .to_string(),
                    css: include_str!("../../front_end_dist/themes/default/pages/contact/page.css")
                        .to_string(),
                    js: include_str!("../../front_end_dist/themes/default/pages/contact/page.js")
                        .to_string(),
                    ..Default::default()
                },
            ),
            (
                Arc::new(Route::Static(
                    Cow::Borrowed("/index.html"),
                    Regex::new(r"/index\.(css|html|js)")
                        .expect("Expected Theme to have Valid Regex"),
                )),
                Page {
                    path: "/index.html".to_string(),
                    metadata: None,
                    html: include_str!(
                        "../../front_end_dist/themes/default/pages/contact/page.html"
                    )
                    .to_string(),
                    css: include_str!("../../front_end_dist/themes/default/pages/contact/page.css")
                        .to_string(),
                    js: include_str!("../../front_end_dist/themes/default/pages/contact/page.js")
                        .to_string(),
                    ..Default::default()
                },
            ),
        ],
    })
});
