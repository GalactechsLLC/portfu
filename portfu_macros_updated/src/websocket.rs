use crate::endpoint::EndpointArgs;
use crate::utils::{parse_path_variables, validate_route};
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use syn::{parse_quote, Expr, FnArg, Pat, Type};

pub struct WebSocketRoute {
    name: Ident,
    args: WsArgs,
    ast: syn::ItemFn,
    doc_attributes: Vec<syn::Attribute>,
}

impl WebSocketRoute {
    pub fn new(args: EndpointArgs, ast: syn::ItemFn) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();
        validate_route(&args.path)?;
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();
        let args = WsArgs::new(args)?;
        if !ast.sig.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &ast.sig.generics,
                "websocket macro does not support generic functions",
            ));
        }
        if matches!(ast.sig.output, syn::ReturnType::Default) {
            return Err(syn::Error::new_spanned(
                ast,
                "Function has no return type. Cannot be used as handler",
            ));
        }
        Ok(Self {
            name,
            args,
            ast,
            doc_attributes,
        })
    }
}

impl ToTokens for WebSocketRoute {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            ast,
            args,
            doc_attributes,
        } = self;
        let WsArgs {
            path,
            resource_name,
            scope,
            domains,
            filters,
            wrappers,
        } = args;

        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), syn::LitStr::value);
        let scope = scope
            .as_ref()
            .map_or_else(|| "default".to_string(), syn::LitStr::value);
        let mut additional_function_vars = vec![];
        let (mut dyn_vars, path_vars) = match parse_path_variables(path) {
            Ok(v) => v,
            Err(err) => {
                output.extend(err.into_compile_error());
                return;
            }
        };

        for arg in ast.sig.inputs.iter() {
            let (ident_type, ident_val): (Type, Ident) = match arg {
                FnArg::Receiver(_) => {
                    continue;
                }
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        if path_vars.contains(&format!("{}", pat_ident.ident)) {
                            let ident = &pat_ident.ident;
                            additional_function_vars.push(quote! { #ident, });
                            continue;
                        } else {
                            let ty = &typed.ty;
                            let ident = &pat_ident.ident;
                            (parse_quote! { #ty }, parse_quote! { #ident })
                        }
                    } else {
                        output.extend(
                            syn::Error::new_spanned(
                                &typed.pat,
                                "Unsupported argument pattern in websocket signature; use a simple identifier binding",
                            )
                            .into_compile_error(),
                        );
                        return;
                    }
                }
            };

            if let Type::Reference(reference) = &ident_type {
                if let Type::Path(path) = &reference.elem.as_ref() {
                    if let Some(segment) = path.path.segments.first() {
                        let request: Ident = Ident::new("Request", segment.ident.span());
                        let request_headers: Ident =
                            Ident::new("RequestHeaders", segment.ident.span());
                        let response_headers: Ident =
                            Ident::new("ResponseHeaders", segment.ident.span());
                        if request == segment.ident {
                            dyn_vars.push(quote! {
                                let #ident_val = request;
                            });
                            additional_function_vars.push(quote! { #ident_val, });
                            continue;
                        } else if request_headers == segment.ident {
                            if reference.mutability.is_some() {
                                dyn_vars.push(quote! {
                                    let #ident_val: &mut ::portfu_updated::prelude::RequestHeaders = request.headers_mut();
                                });
                            } else {
                                dyn_vars.push(quote! {
                                    let #ident_val: &::portfu_updated::prelude::RequestHeaders = request.headers();
                                });
                            }
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        } else if response_headers == segment.ident {
                            if reference.mutability.is_some() {
                                dyn_vars.push(quote! {
                                    let #ident_val: &mut ::portfu_updated::prelude::ResponseHeaders = response.headers_mut();
                                });
                            } else {
                                dyn_vars.push(quote! {
                                    let #ident_val: &::portfu_updated::prelude::ResponseHeaders = response.headers();
                                });
                            }
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        }
                    }
                }
            }
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    if segment.ident == "WebSocket" {
                        additional_function_vars.push(quote! { websocket_wrapper.clone(), });
                        continue;
                    } else if segment.ident == "Peers" {
                        additional_function_vars.push(quote! { peers.clone(), });
                        continue;
                    }
                }
            }

            dyn_vars.push(quote! {
                let #ident_val: #ident_type = match ::portfu_updated::prelude::FromRequest::try_from(request).await {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(::portfu_updated::prelude::Response::internal_error(
                            format!("Failed to extract {} as {}, {e:?}",
                                stringify!(#ident_val), stringify!(#ident_type).replace(' ',"")
                            )
                        ));
                    }
                };
            });
            additional_function_vars.push(quote! { #ident_val, });
        }
        let factory_name = format_ident!("__portfu_make_ws_{}", name);
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name {
                peers: ::portfu_updated::prelude::Peers,
            }

            impl Default for #name {
                fn default() -> Self {
                    Self {
                        peers: ::std::sync::Arc::new(
                            ::tokio::sync::RwLock::new(::std::collections::HashMap::new())
                        ),
                    }
                }
            }

            fn #factory_name(_registry: &mut ::portfu_updated::prelude::ServiceRegistry) -> ::portfu_updated::prelude::Service {
                ::portfu_updated::prelude::ServiceBuilder::new(#path)
                    .name(#resource_name)
                    .scope(#scope)
                    #(.domain(#domains))*
                    .filter(::std::sync::Arc::new(::portfu_updated::prelude::filters::any(
                        String::new(),
                        &[
                            ::portfu_updated::prelude::filters::method::GET.clone(),
                            ::portfu_updated::prelude::filters::method::OPTIONS.clone(),
                        ]
                    )))
                    #(.filter(#filters))*
                    #(.wrap(#wrappers))*
                    .handler(::std::sync::Arc::new(#name::default()))
                    .build()
            }

            ::portfu_updated::prelude::inventory::submit! {
                ::portfu_updated::prelude::ServiceRegistration {
                    register: #factory_name
                }
            }

            impl #name {
                #ast
            }

            impl ::portfu_updated::prelude::ServiceTrait for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu_updated::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu_updated::prelude::Response, ::portfu_updated::prelude::PortfuError>> + 'a + Send + Sync>> {
                    Box::pin(async move {
                        use ::portfu_updated::prelude::http::StatusCode;
                        use ::portfu_updated::prelude::tokio_tungstenite::tungstenite::handshake::derive_accept_key;
                        use ::portfu_updated::prelude::tokio_tungstenite::tungstenite::protocol::Role;

                        if request.method() == ::portfu_updated::prelude::http::method::Method::OPTIONS {
                            return Ok(::portfu_updated::prelude::Response::ok(""));
                        }

                        #(#dyn_vars)*

                        let is_upgrade = match request.request_type() {
                            ::portfu_updated::prelude::RequestType::Stream(req) => {
                                req.headers()
                                    .get(::portfu_updated::prelude::http::header::UPGRADE)
                                    .and_then(|v| v.to_str().ok())
                                    .map(|v| v.eq_ignore_ascii_case("websocket"))
                                    .unwrap_or(false)
                            }
                            ::portfu_updated::prelude::RequestType::Sized(req) => {
                                req.headers()
                                    .get(::portfu_updated::prelude::http::header::UPGRADE)
                                    .and_then(|v| v.to_str().ok())
                                    .map(|v| v.eq_ignore_ascii_case("websocket"))
                                    .unwrap_or(false)
                            }
                            _ => false,
                        };
                        if !is_upgrade {
                            return Ok(::portfu_updated::prelude::Response::from_status_and_message(
                                StatusCode::BAD_REQUEST,
                                "Expected websocket upgrade request",
                            ));
                        }

                        let (key, version_ok, ) = match request.request_type() {
                            ::portfu_updated::prelude::RequestType::Stream(req) => {
                                let key = req.headers().get("Sec-WebSocket-Key").cloned();
                                (
                                    req.headers().get("Sec-WebSocket-Key").cloned(),
                                    req.headers().get("Sec-WebSocket-Version")
                                        .and_then(|v| v.to_str().ok())
                                        .map(|v| v == "13")
                                        .unwrap_or(false),
                                )
                            }
                            ::portfu_updated::prelude::RequestType::Sized(req) => {
                                let key = req.headers().get("Sec-WebSocket-Key").cloned();
                                (
                                    req.headers().get("Sec-WebSocket-Key").cloned(),
                                    req.headers().get("Sec-WebSocket-Version")
                                        .and_then(|v| v.to_str().ok())
                                        .map(|v| v == "13")
                                        .unwrap_or(false),
                                )
                            }
                            _ => {
                                return Ok(::portfu_updated::prelude::Response::from_status_and_message(
                                    StatusCode::BAD_REQUEST,
                                    "WebSocket upgrade requires a live HTTP request",
                                ));
                            }
                        };
                        let Some(key) = key else {
                            return Ok(::portfu_updated::prelude::Response::from_status_and_message(
                                StatusCode::BAD_REQUEST,
                                "Missing Sec-WebSocket-Key header",
                            ));
                        };
                        if !version_ok {
                            return Ok(::portfu_updated::prelude::Response::from_status_and_message(
                                StatusCode::BAD_REQUEST,
                                "Unsupported websocket version",
                            ));
                        }
                        let accept = derive_accept_key(key.as_bytes());
                        let response = match ::portfu_updated::prelude::http::Response::builder()
                            .status(StatusCode::SWITCHING_PROTOCOLS)
                            .header(::portfu_updated::prelude::http::header::CONNECTION, "upgrade")
                            .header(::portfu_updated::prelude::http::header::UPGRADE, "websocket")
                            .header("Sec-WebSocket-Accept", accept)
                            .body(())
                        {
                            Ok(response) => response,
                            Err(e) => {
                                return Ok(::portfu_updated::prelude::Response::internal_error(
                                    format!("Failed to build websocket response: {e:?}")
                                ));
                            }
                        };

                        let peers = self.peers.clone();
                        let upgrade = match request.request_type() {
                            ::portfu_updated::prelude::RequestType::Stream(req) => {
                                ::portfu_updated::prelude::hyper::upgrade::on(req)
                            }
                            ::portfu_updated::prelude::RequestType::Sized(req) => {
                                ::portfu_updated::prelude::hyper::upgrade::on(req)
                            }
                            _ => {
                                ::portfu_updated::prelude::log::error!("WebSocket upgrade requires a live HTTP request");
                                return Ok(::portfu_updated::prelude::Response::from_status_and_message(
                                    StatusCode::BAD_REQUEST,
                                    "WebSocket upgrade requires a live HTTP request",
                                ));
                            }
                        };
                        ::tokio::spawn(async move {
                            match upgrade.await {
                                Ok(upgraded) => {
                                    let websocket = ::portfu_updated::prelude::tokio_tungstenite::WebSocketStream::from_raw_socket(
                                        ::portfu_updated::prelude::hyper_util::rt::TokioIo::new(upgraded),
                                        Role::Server,
                                        None
                                    ).await;
                                    let websocket_wrapper = ::portfu_updated::prelude::WebSocket::with_peers(websocket, peers.clone()).await;
                                    if let Err(e) = Self::#name(#(#additional_function_vars)*).await {
                                        eprintln!("websocket handler exited with error: {e}");
                                    }
                                    let _ = websocket_wrapper.leave().await;
                                }
                                Err(e) => {
                                    eprintln!("websocket upgrade failed: {e:?}");
                                }
                            }
                        });

                        Ok(response.into())
                    })
                }
            }
        };
        output.extend(stream);
    }
}

struct WsArgs {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    scope: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
    filters: Vec<Expr>,
    wrappers: Vec<syn::Expr>,
}

impl WsArgs {
    fn new(args: EndpointArgs) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut scope = None;
        let mut domains = Vec::new();
        let mut filters = Vec::new();
        let mut wrappers = Vec::new();
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
            } else if nv.path.is_ident("filter") {
                let value = nv.value;
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = &value
                {
                    filters.push(lit.parse::<Expr>()?);
                } else {
                    filters.push(value);
                }
            } else if nv.path.is_ident("wrap") {
                let value = nv.value;
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = &value
                {
                    wrappers.push(lit.parse()?);
                } else {
                    wrappers.push(value);
                }
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: name, scope, domain, filter and wrap",
                ));
            }
        }
        Ok(Self {
            path: args.path,
            resource_name,
            scope,
            domains,
            filters,
            wrappers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WsArgs;
    use crate::endpoint::EndpointArgs;

    #[test]
    fn websocket_args_accept_filter_and_wrap_expressions() {
        let args = syn::parse_str::<EndpointArgs>(
            r#""/ws", filter = ::portfu_updated::prelude::filters::method::GET.clone(), wrap = my_wrapper()"#,
        )
        .expect("args should parse");
        let parsed = WsArgs::new(args).expect("websocket args should parse");
        assert_eq!(parsed.filters.len(), 1);
        assert_eq!(parsed.wrappers.len(), 1);
    }

    #[test]
    fn websocket_args_keep_filter_and_wrap_string_compat() {
        let args = syn::parse_str::<EndpointArgs>(
            r#""/ws", filter = "::portfu_updated::prelude::filters::method::GET.clone()", wrap = "my_wrapper()""#,
        )
        .expect("args should parse");
        let parsed = WsArgs::new(args).expect("websocket args should parse");
        assert_eq!(parsed.filters.len(), 1);
        assert_eq!(parsed.wrappers.len(), 1);
    }
}
