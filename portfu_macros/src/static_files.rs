use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use syn::punctuated::Punctuated;
use syn::{parse::Parse, Token};

pub struct StaticFilesArgs {
    root: syn::LitStr,
    options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl Parse for StaticFilesArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let root = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid static_files definition, expected #[static_files("<root_path>", options...)]"#,
            ));
            err
        })?;

        if !input.peek(Token![,]) {
            if input.is_empty() {
                return Ok(Self {
                    root,
                    options: Punctuated::new(),
                });
            }
            return Err(syn::Error::new(
                Span::call_site(),
                "Expected comma after root path",
            ));
        }

        input.parse::<Token![,]>()?;
        let options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;
        Ok(Self { root, options })
    }
}

pub struct StaticFiles {
    name: Ident,
    resource_name: Option<syn::LitStr>,
    scope: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
    files: Vec<(String, String)>,
}

impl StaticFiles {
    pub fn new(args: StaticFilesArgs, name: Ident) -> syn::Result<Self> {
        let parsed = ParsedArgs::new(args)?;
        let files = collect_static_files(parsed.root_path.as_path())?;
        Ok(Self {
            name,
            resource_name: parsed.resource_name,
            scope: parsed.scope,
            domains: parsed.domains,
            files,
        })
    }
}

impl ToTokens for StaticFiles {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let name = &self.name;
        let resource_name = self
            .resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), syn::LitStr::value);
        let scope = self
            .scope
            .as_ref()
            .map_or_else(|| "default".to_string(), syn::LitStr::value);
        let domains = &self.domains;
        let name_tag = sanitize(&name.to_string());

        let mut static_defs = Vec::new();
        let mut inventory_defs = Vec::new();
        let mut service_index: usize = 0;

        for (route, source_path) in &self.files {
            let mut routes = vec![route.clone()];
            if let Some(prefix) = route.strip_suffix("index.html") {
                let trimmed = prefix.trim_end_matches('/');
                routes.push(if trimmed.is_empty() {
                    "/".to_string()
                } else {
                    format!("{trimmed}/")
                });
            }

            for route in routes {
                let mime = guess_content_type(source_path.as_str());
                let static_name = format_ident!(
                    "STATIC_FILE_{}_{}_{}",
                    name_tag,
                    sanitize(route.as_str()),
                    service_index
                );
                static_defs.push(quote! {
                    static #static_name: &'static [u8] = include_bytes!(#source_path);
                });

                let handler_name =
                    format_ident!("__portfu_static_handler_{}_{}", name, service_index);
                let factory_name = format_ident!("__portfu_make_static_{}_{}", name, service_index);
                inventory_defs.push(quote! {
                    #[allow(non_camel_case_types)]
                    struct #handler_name;

                    impl ::portfu::prelude::ServiceTrait for #handler_name {
                        fn name(&self) -> &str {
                            #resource_name
                        }
                        fn serve<'a>(
                            &'a self,
                            request: &'a mut ::portfu::prelude::Request
                        ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError>> + 'a + Send>> {
                            Box::pin(async move {
                                use ::portfu::prelude::http::Method;
                                if request.method() == Method::OPTIONS {
                                    return Ok(::portfu::prelude::Response::ok(""));
                                }
                                if request.method() == Method::HEAD {
                                    let mut response = ::portfu::prelude::Response::new();
                                    response.headers_mut().insert(
                                        ::portfu::prelude::http::header::CONTENT_LENGTH,
                                        ::portfu::prelude::http::HeaderValue::from(#static_name.len() as u64),
                                    );
                                    if let Ok(header) = ::portfu::prelude::http::HeaderValue::from_str(#mime) {
                                        response
                                            .headers_mut()
                                            .insert(::portfu::prelude::http::header::CONTENT_TYPE, header);
                                    }
                                    return Ok(response);
                                }
                                let mut response = ::portfu::prelude::Response::from(#static_name.to_vec());
                                if let Ok(header) = ::portfu::prelude::http::HeaderValue::from_str(#mime) {
                                    response
                                        .headers_mut()
                                        .insert(::portfu::prelude::http::header::CONTENT_TYPE, header);
                                }
                                Ok(response)
                            })
                        }
                    }

                    #[allow(non_snake_case)]
                    fn #factory_name(_registry: &mut ::portfu::prelude::ServiceRegistry) -> ::portfu::prelude::Service {
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
                            .handler(::std::sync::Arc::new(#handler_name))
                            .build()
                    }
                    ::portfu::prelude::inventory::submit! {
                        ::portfu::prelude::ServiceRegistration {
                            register: #factory_name
                        }
                    }
                });
                service_index += 1;
            }
        }

        let out = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            #(#static_defs)*
            #(#inventory_defs)*
        };
        output.extend(out);
    }
}

struct ParsedArgs {
    root_path: PathBuf,
    resource_name: Option<syn::LitStr>,
    scope: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
}

impl ParsedArgs {
    fn new(args: StaticFilesArgs) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut scope = None;
        let mut domains = Vec::new();
        for nv in args.options {
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
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: name, scope and domain",
                ));
            }
        }

        let root = args.root.value();
        let root_path = if root.starts_with('/') {
            PathBuf::from(root)
        } else {
            let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(|_| {
                syn::Error::new(Span::call_site(), "Expected CARGO_MANIFEST_DIR to be set")
            })?;
            PathBuf::from(manifest_dir).join(root)
        };
        Ok(Self {
            root_path,
            resource_name,
            scope,
            domains,
        })
    }
}

fn collect_static_files(root: &Path) -> syn::Result<Vec<(String, String)>> {
    let canonical_root = root.canonicalize().map_err(|e| {
        syn::Error::new(
            Span::call_site(),
            format!("Failed to read static files root `{}`: {e}", root.display()),
        )
    })?;
    let mut file_map: HashMap<String, String> = HashMap::new();
    read_directory(
        canonical_root.as_path(),
        canonical_root.as_path(),
        &mut file_map,
    )?;
    let mut files: Vec<(String, String)> = file_map.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn read_directory(
    root: &Path,
    path: &Path,
    file_map: &mut HashMap<String, String>,
) -> syn::Result<()> {
    for entry in fs::read_dir(path).map_err(|e| {
        syn::Error::new(
            Span::call_site(),
            format!("Failed to read directory `{}`: {e}", path.display()),
        )
    })? {
        let entry = entry.map_err(|e| {
            syn::Error::new(
                Span::call_site(),
                format!(
                    "Failed to read a directory entry in `{}`: {e}",
                    path.display()
                ),
            )
        })?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            read_directory(root, entry_path.as_path(), file_map)?;
        } else {
            read_file(root, entry_path.as_path(), file_map)?;
        }
    }
    Ok(())
}

fn read_file(
    root: &Path,
    file_path: &Path,
    file_map: &mut HashMap<String, String>,
) -> syn::Result<()> {
    let canonical_file = file_path.canonicalize().map_err(|e| {
        syn::Error::new(
            Span::call_site(),
            format!("Failed to canonicalize file `{}`: {e}", file_path.display()),
        )
    })?;
    let relative = canonical_file.strip_prefix(root).map_err(|e| {
        syn::Error::new(
            Span::call_site(),
            format!(
                "Failed to map file `{}` under root `{}`: {e}",
                canonical_file.display(),
                root.display()
            ),
        )
    })?;
    let route = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
    file_map.insert(route, canonical_file.to_string_lossy().replace('\\', "/"));
    Ok(())
}

fn sanitize(value: &str) -> String {
    value
        .replace(['/', '\\', '.', ')', '(', '-', ' ', '+'], "_")
        .replace('@', "_at_")
        .replace("__", "_")
}

fn guess_content_type(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else if lower.ends_with(".xml") {
        "application/xml"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else if lower.ends_with(".wasm") {
        "application/wasm"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else {
        "application/octet-stream"
    }
}
