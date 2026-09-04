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
            client_trust,
            max_message_size,
            max_frame_size,
            upgrade_timeout_ms,
        } = args;

        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), syn::LitStr::value);
        let scope = scope
            .as_ref()
            .map_or_else(|| "default".to_string(), syn::LitStr::value);
        let max_message_size_config = max_message_size.as_ref().map(|value| {
            quote! { websocket_config.max_message_size = Some(#value); }
        });
        let max_frame_size_config = max_frame_size.as_ref().map(|value| {
            quote! { websocket_config.max_frame_size = Some(#value); }
        });
        let upgrade_timeout_config = upgrade_timeout_ms.as_ref().map(|value| {
            quote! {
                websocket_config.upgrade_timeout = Some(::std::time::Duration::from_millis(#value));
            }
        });
        let client_trust_wrapper = client_trust.as_ref().map(|trust_store| {
            quote! {
                .wrap(::std::sync::Arc::new(::portfu::prelude::ClientTrust::new(#trust_store)))
            }
        });
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
                    if let Some(segment) = path.path.segments.last() {
                        let request_headers: Ident =
                            Ident::new("RequestHeaders", segment.ident.span());
                        let response_headers: Ident =
                            Ident::new("ResponseHeaders", segment.ident.span());
                        if request_headers == segment.ident {
                            output.extend(
                                syn::Error::new_spanned(
                                    reference,
                                    "websocket handlers receive owned RequestHeaders; remove the reference",
                                )
                                .into_compile_error(),
                            );
                            return;
                        } else if response_headers == segment.ident {
                            output.extend(
                                syn::Error::new_spanned(
                                    reference,
                                    "websocket handlers cannot receive response headers",
                                )
                                .into_compile_error(),
                            );
                            return;
                        }
                    }
                }
                output.extend(
                    syn::Error::new_spanned(
                        reference,
                        "websocket handler parameters must be owned because upgrade tasks are spawned",
                    )
                    .into_compile_error(),
                );
                return;
            }
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.last() {
                    if segment.ident == "RequestHeaders" {
                        dyn_vars.push(quote! {
                            let #ident_val: #ident_type = request.headers().clone();
                        });
                        additional_function_vars.push(quote! { #ident_val, });
                        continue;
                    } else if segment.ident == "WebSocket" {
                        additional_function_vars.push(quote! { websocket_wrapper.clone(), });
                        continue;
                    } else if segment.ident == "Peers" {
                        additional_function_vars.push(quote! { peers.clone(), });
                        continue;
                    }
                }
            }

            dyn_vars.push(quote! {
                let #ident_val: #ident_type = match ::portfu::prelude::FromRequest::try_from(request).await {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(::portfu::prelude::IntoResponse::into_response(e));
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
                peers: ::portfu::prelude::Peers,
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

            fn #factory_name(_registry: &mut ::portfu::prelude::ServiceRegistry) -> ::portfu::prelude::Service {
                ::portfu::prelude::ServiceBuilder::new(#path)
                    .name(#resource_name)
                    .scope(#scope)
                    #(.domain(#domains))*
                    .filter(::std::sync::Arc::new(::portfu::prelude::filters::any(
                        String::new(),
                        &[
                            ::portfu::prelude::filters::method::GET.clone(),
                            ::portfu::prelude::filters::method::OPTIONS.clone(),
                        ]
                    )))
                    #(.filter(#filters))*
                    #client_trust_wrapper
                    #(.wrap(#wrappers))*
                    .handler(::std::sync::Arc::new(#name::default()))
                    .build()
            }

            ::portfu::prelude::inventory::submit! {
                ::portfu::prelude::ServiceRegistration {
                    register: #factory_name
                }
            }

            impl #name {
                #ast
            }

            impl ::portfu::prelude::ServiceTrait for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError>> + 'a + Send>> {
                    Box::pin(async move {
                        if request.method() == ::portfu::prelude::http::method::Method::OPTIONS {
                            return Ok(::portfu::prelude::Response::ok(""));
                        }

                        #(#dyn_vars)*
                        let mut websocket_config = ::portfu::prelude::WebSocketRouteConfig::default();
                        #max_message_size_config
                        #max_frame_size_config
                        #upgrade_timeout_config
                        let peers = self.peers.clone();
                        ::portfu::prelude::websocket_upgrade(
                            request,
                            websocket_config,
                            peers.clone(),
                            move |websocket_wrapper| async move {
                                Self::#name(#(#additional_function_vars)*).await
                            },
                        ).await
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
    client_trust: Option<syn::LitStr>,
    max_message_size: Option<Expr>,
    max_frame_size: Option<Expr>,
    upgrade_timeout_ms: Option<Expr>,
}

impl WsArgs {
    fn new(args: EndpointArgs) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut scope = None;
        let mut domains = Vec::new();
        let mut filters = Vec::new();
        let mut wrappers = Vec::new();
        let mut client_trust = None;
        let mut max_message_size = None;
        let mut max_frame_size = None;
        let mut upgrade_timeout_ms = None;
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
            } else if nv.path.is_ident("client_trust") {
                if client_trust.is_some() {
                    return Err(syn::Error::new_spanned(
                        nv.path,
                        "Attribute client_trust may only be specified once",
                    ));
                }
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    client_trust = Some(lit);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute client_trust expects a literal string",
                    ));
                }
            } else if nv.path.is_ident("max_message_size") {
                set_once(&mut max_message_size, nv.value, "max_message_size")?;
            } else if nv.path.is_ident("max_frame_size") {
                set_once(&mut max_frame_size, nv.value, "max_frame_size")?;
            } else if nv.path.is_ident("upgrade_timeout_ms") {
                set_once(&mut upgrade_timeout_ms, nv.value, "upgrade_timeout_ms")?;
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: name, scope, domain, filter, wrap, client_trust, max_message_size, max_frame_size and upgrade_timeout_ms",
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
            client_trust,
            max_message_size,
            max_frame_size,
            upgrade_timeout_ms,
        })
    }
}

fn set_once(target: &mut Option<Expr>, value: Expr, name: &str) -> syn::Result<()> {
    if target.is_some() {
        return Err(syn::Error::new_spanned(
            value,
            format!("Attribute {name} may only be specified once"),
        ));
    }
    *target = Some(value);
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/websocket.rs"]
mod tests;
