use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::{parse::Parse, Expr, Token};

pub struct FilesArgs {
    path: Expr,
    options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl Parse for FilesArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path: Expr = input.parse()?;
        if !input.peek(Token![,]) {
            return Ok(Self {
                path,
                options: Punctuated::new(),
            });
        }
        input.parse::<Token![,]>()?;
        let options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;
        Ok(Self { path, options })
    }
}

pub struct Files {
    name: Ident,
    path_expr: Expr,
    parsed: ParsedArgs,
}

impl Files {
    pub fn new(args: FilesArgs, name: Ident) -> syn::Result<Self> {
        let parsed = ParsedArgs::new(args.options)?;
        Ok(Self {
            name,
            path_expr: args.path,
            parsed,
        })
    }
}

impl ToTokens for Files {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let name = &self.name;
        let path_expr = &self.path_expr;
        let resource_name = self
            .parsed
            .resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), syn::LitStr::value);
        let scope = self
            .parsed
            .scope
            .as_ref()
            .map_or_else(|| "default".to_string(), syn::LitStr::value);
        let mount = self
            .parsed
            .mount
            .as_ref()
            .map_or_else(|| "/".to_string(), syn::LitStr::value);
        let domains = &self.parsed.domains;
        let cache_limit = self.parsed.cache_limit;
        let route = mount_route(mount.as_str());
        let handler_name = format_ident!("__portfu_dynamic_files_handler_{}", name);
        let factory_name = format_ident!("__portfu_make_dynamic_files_{}", name);
        let mime_name = format_ident!("__portfu_dynamic_files_mime_{}", name);
        let response_name = format_ident!("__portfu_dynamic_files_response_{}", name);

        let out = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;

            #[allow(non_snake_case)]
            fn #mime_name(path: &::std::path::Path) -> &'static str {
                let ext = path
                    .extension()
                    .and_then(|v| v.to_str())
                    .map(|v| v.to_ascii_lowercase())
                    .unwrap_or_default();
                match ext.as_str() {
                    "html" => "text/html; charset=utf-8",
                    "css" => "text/css; charset=utf-8",
                    "js" => "application/javascript; charset=utf-8",
                    "mjs" => "application/javascript; charset=utf-8",
                    "json" => "application/json",
                    "txt" => "text/plain; charset=utf-8",
                    "xml" => "application/xml",
                    "svg" => "image/svg+xml",
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    "ico" => "image/x-icon",
                    "wasm" => "application/wasm",
                    "pdf" => "application/pdf",
                    _ => "application/octet-stream",
                }
            }

            #[allow(non_snake_case)]
            fn #response_name(
                bytes: ::std::vec::Vec<u8>,
                content_type: &'static str,
            ) -> ::portfu::prelude::Response {
                let mut response = ::portfu::prelude::Response::from(bytes);
                if let Ok(header) = ::portfu::prelude::http::HeaderValue::from_str(content_type) {
                    response
                        .headers_mut()
                        .insert(::portfu::prelude::http::header::CONTENT_TYPE, header);
                }
                response
            }

            #[allow(non_camel_case_types)]
            struct #handler_name {
                root: ::std::path::PathBuf,
                canonical_root: ::std::path::PathBuf,
                mount: &'static str,
                cache_limit: usize,
                cache: ::std::sync::Arc<
                    ::tokio::sync::RwLock<
                        ::std::collections::HashMap<::std::path::PathBuf, ::std::sync::Arc<::std::vec::Vec<u8>>>
                    >
                >,
            }

            impl ::portfu::prelude::ServiceTrait for #handler_name {
                fn name(&self) -> &str {
                    #resource_name
                }
                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError>> + 'a + Send + Sync>> {
                    Box::pin(async move {
                        use ::portfu::prelude::http::Method;
                        if request.method() == Method::OPTIONS {
                            return Ok(::portfu::prelude::Response::ok(""));
                        }

                        let req_path = request.uri().path();
                        let stripped = req_path
                            .strip_prefix(self.mount)
                            .unwrap_or(req_path)
                            .trim_start_matches('/');
                        if stripped.is_empty() {
                            return Ok(::portfu::prelude::Response::not_found("File path not provided"));
                        }

                        let candidate = self.root.join(stripped);
                        let canonical = match ::tokio::fs::canonicalize(&candidate).await {
                            Ok(path) => path,
                            Err(_) => {
                                return Ok(::portfu::prelude::Response::not_found("File not found"));
                            }
                        };
                        if !canonical.starts_with(&self.canonical_root) {
                            return Ok(::portfu::prelude::Response::not_found("Invalid file path"));
                        }
                        let mime = #mime_name(canonical.as_path());

                        if request.method() == Method::HEAD {
                            return match ::tokio::fs::metadata(&canonical).await {
                                Ok(metadata) => {
                                    let mut response = ::portfu::prelude::Response::new();
                                    if let Ok(header) = ::portfu::prelude::http::HeaderValue::from_str(mime) {
                                        response
                                            .headers_mut()
                                            .insert(::portfu::prelude::http::header::CONTENT_TYPE, header);
                                    }
                                    response.headers_mut().insert(
                                        ::portfu::prelude::http::header::CONTENT_LENGTH,
                                        ::portfu::prelude::http::HeaderValue::from(metadata.len()),
                                    );
                                    Ok(response)
                                }
                                Err(_) => Ok(::portfu::prelude::Response::not_found("File not found")),
                            };
                        }

                        if self.cache_limit > 0 {
                            if let Some(cached) = self.cache.read().await.get(&canonical).cloned() {
                                return Ok(#response_name(cached.as_ref().clone(), mime));
                            }
                        }

                        match ::tokio::fs::read(&canonical).await {
                            Ok(bytes) => {
                                if self.cache_limit > 0 && bytes.len() <= self.cache_limit {
                                    self.cache
                                        .write()
                                        .await
                                        .insert(canonical.clone(), ::std::sync::Arc::new(bytes.clone()));
                                }
                                Ok(#response_name(bytes, mime))
                            }
                            Err(_) => Ok(::portfu::prelude::Response::not_found("File not found")),
                        }
                    })
                }
            }

            #[allow(non_snake_case)]
            fn #factory_name(_registry: &mut ::portfu::prelude::ServiceRegistry) -> ::portfu::prelude::Service {
                let mut root_path = (#path_expr).to_string();
                if !(root_path.ends_with('/') || root_path.ends_with('\\')) {
                    root_path.push(::std::path::MAIN_SEPARATOR);
                }
                let root = ::std::path::PathBuf::from(root_path);
                let canonical_root = root
                    .canonicalize()
                    .unwrap_or_else(|_| root.clone());
                ::portfu::prelude::ServiceBuilder::new(#route)
                    .name(#resource_name)
                    .scope(#scope)
                    #(.domain(#domains))*
                    .filter(::std::sync::Arc::new(::portfu::prelude::filters::any(
                        String::new(),
                        &[
                            ::portfu::prelude::filters::method::GET.clone(),
                            ::portfu::prelude::filters::method::HEAD.clone(),
                            ::portfu::prelude::filters::method::OPTIONS.clone(),
                        ]
                    )))
                    .handler(::std::sync::Arc::new(#handler_name {
                        root,
                        canonical_root,
                        mount: #mount,
                        cache_limit: #cache_limit as usize,
                        cache: ::std::sync::Arc::new(::tokio::sync::RwLock::new(::std::collections::HashMap::new())),
                    }))
                    .build()
            }

            ::portfu::prelude::inventory::submit! {
                ::portfu::prelude::ServiceRegistration {
                    register: #factory_name
                }
            }
        };
        output.extend(out);
    }
}

struct ParsedArgs {
    resource_name: Option<syn::LitStr>,
    scope: Option<syn::LitStr>,
    mount: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
    cache_limit: u64,
}

impl ParsedArgs {
    fn new(options: Punctuated<syn::MetaNameValue, Token![,]>) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut scope = None;
        let mut mount = None;
        let mut domains = Vec::new();
        let mut cache_limit = 65536;

        for nv in options {
            if nv.path.is_ident("name") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    resource_name = Some(lit);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute name expects literal string",
                    ));
                }
            } else if nv.path.is_ident("scope") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    scope = Some(lit);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute scope expects literal string",
                    ));
                }
            } else if nv.path.is_ident("mount") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    mount = Some(lit);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute mount expects literal string",
                    ));
                }
            } else if nv.path.is_ident("domain") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    domains.push(lit);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute domain expects literal string",
                    ));
                }
            } else if nv.path.is_ident("cache_limit") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(lit),
                    ..
                }) = nv.value
                {
                    cache_limit = lit.base10_parse::<u64>()?;
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute cache_limit expects literal u64",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: name, scope, mount, domain and cache_limit",
                ));
            }
        }

        Ok(Self {
            resource_name,
            scope,
            mount,
            domains,
            cache_limit,
        })
    }
}

fn mount_route(mount: &str) -> String {
    let mount = mount.trim_end_matches('/');
    if mount.is_empty() || mount == "/" {
        "/{file_path}*".to_string()
    } else if mount.starts_with('/') {
        format!("{mount}/{{file_path}}*")
    } else {
        format!("/{mount}/{{file_path}}*")
    }
}
