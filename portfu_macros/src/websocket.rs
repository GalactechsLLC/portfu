use crate::endpoints::EndpointArgs;
use crate::parse_path_variables;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{parse_quote, FnArg, LitStr, Pat, Path, Type};

pub struct WebSocketRoute {
    /// Name of the handler function being annotated.
    name: Ident,
    /// Args passed to macro.
    args: WsArgs,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}
impl WebSocketRoute {
    pub fn new(args: EndpointArgs, ast: syn::ItemFn) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();
        // Try and pull out the doc comments so that we can reapply them to the generated struct.
        // Note that multi line doc comments are converted to multiple doc attributes.
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();

        let args = WsArgs::new(args)?;

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
            filters,
            wrappers,
        } = args;

        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), LitStr::value);
        let mut additional_function_vars = vec![];
        let (mut dyn_vars, path_vars) = parse_path_variables(path);
        for arg in ast.sig.inputs.iter() {
            let (ident_type, ident_val): (Type, Ident) = match arg {
                FnArg::Receiver(_) => {
                    continue;
                }
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        if path_vars.contains(&format!("{}", &pat_ident.ident)) {
                            let ident = &pat_ident.ident;
                            additional_function_vars.push(quote! {
                                #ident,
                            });
                            continue;
                        } else {
                            let ty = &typed.ty;
                            let ident = &pat_ident.ident;
                            (parse_quote! { #ty }, parse_quote! { #ident })
                        }
                    } else {
                        continue;
                    }
                }
            };
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    let body_ident: Ident = Ident::new("Body", segment.ident.span());
                    let state_ident: Ident = Ident::new("State", segment.ident.span());
                    let ws_ident: Ident = Ident::new("WebSocket", segment.ident.span());
                    if body_ident == segment.ident {
                        panic!("Body Not Supported for Websocket");
                    } else if state_ident == segment.ident {
                        dyn_vars.push(quote! {
                            let #ident_val: #ident_type = match ::portfu::prelude::State::extract(&mut request).await {
                                Some(v) => v,
                                None => {
                                    *response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                                    *response.body_mut() = ::portfu::prelude::http_body_util::Full::new(::portfu::prelude::hyper::body::Bytes::from(format!("Failed to find {}", stringify!(#ident_type).replace(' ',""))));
                                    return Err(ServiceResponse {
                                        request,
                                        response
                                    });
                                }
                            };
                        });
                        additional_function_vars.push(quote! {
                            #ident_val,
                        });
                        continue;
                    } else if ws_ident == segment.ident {
                        additional_function_vars.push(quote! {
                            websocket,
                        });
                        continue;
                    }
                }
            }
            let function_name = &ast.sig.ident;
            additional_function_vars.push(quote! {
                match request.get() {
                    Some(v) => v,
                    None => {
                        *response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                        *response.body_mut() = ::portfu::prelude::http_body_util::Full::new(::portfu::prelude::hyper::body::Bytes::from(format!("Failed to find {} for {}", stringify!(#ident_type).replace(' ',""), stringify!(#function_name))));
                        return Err(ServiceResponse {
                            request,
                            response
                        });
                    }
                },
            });
        }
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name {
                peers: ::portfu::prelude::Peers
            }
            impl ::portfu::pfcore::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry) {
                    let __resource = ::portfu::pfcore::service::ServiceBuilder::new(#path)
                        .name(#resource_name)
                        .filter(::portfu::filters::method::GET.clone())
                        #(.filter(::portfu::pfcore::filters::fn_guard(#filters)))*
                        #(.wrap(#wrappers))*
                        .handler(std::sync::Arc::new(self)).build();
                    service_registry.register(__resource);
                }
            }
            impl From<#name> for ::portfu::prelude::Service {
                fn from(service: #name) -> ::portfu::prelude::Service {
                    ::portfu::pfcore::service::ServiceBuilder::new(#path)
                        .name(#resource_name)
                        .filter(::portfu::filters::method::GET.clone())
                        #(.filter(::portfu::pfcore::filters::fn_guard(#filters)))*
                        #(.wrap(#wrappers))*
                        .handler(std::sync::Arc::new(service)).build()
                }
            }
            #[::portfu::prelude::async_trait::async_trait]
            impl ::portfu::pfcore::ServiceHandler for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                async fn handle(
                    &self,
                    data: &mut ::portfu::prelude::ServiceData
                ) -> Result<(), ::std::io::Error> {
                    if data.request.request.is_upgrade_request() {
                        #ast
                        #(#dyn_vars)*
                        log::info!("Upgrading Websocket");
                        let (response, websocket) = match data.request.request.upgrade() {
                            Ok((response, websocket)) => (response, websocket),
                            Err(e) => {
                                let bytes = ::portfu::prelude::hyper::body::Bytes::from("Failed to Upgrade Request");
                                *data.response.body_mut() = ::portfu::prelude::http_body_util::Full::new(bytes);
                                return Ok::<(), ::std::io::Error>(());
                            }
                        };
                        let peers = self.peers.clone();
                        ::tokio::spawn( async move {
                            select! {
                                _ = async {
                                    let websocket = match websocket.await {
                                        Ok(ws) => ::portfu::prelude::tokio_tungstenite::WebSocketStream::from_raw_socket(
                                            ::portfu::prelude::hyper_util::rt::tokio::TokioIo::new(ws),
                                            ::portfu::prelude::tokio_tungstenite::tungstenite::protocol::Role::Server,
                                            None
                                        ).await,
                                        Err(e) => {
                                            log::error!("{e:?}");
                                            return Ok::<(), ::std::io::Error>(());
                                        }
                                    };
                                    let uuid = ::std::sync::Arc::new(::portfu::prelude::uuid::Uuid::new_v4());
                                    let connection = ::std::sync::Arc::new(::portfu::prelude::WebsocketConnection::new(websocket));
                                    peers.write().await.insert(*uuid.as_ref(), connection.clone());
                                    let websocket = ::portfu::prelude::WebSocket {
                                        connection,
                                        uuid: uuid.clone(),
                                        peers: peers.clone()
                                    };
                                    let _ = #name(#(#additional_function_vars)*).await;
                                    peers.write().await.remove(uuid.as_ref());
                                    Ok::<(), ::std::io::Error>(())
                                } => {
                                     Ok::<(), ::std::io::Error>(())
                                }
                                _ = portfu::signal::await_termination() => {
                                    Ok::<(), ::std::io::Error>(())
                                }
                            }
                        });
                        log::info!("Sending Upgrade Response");
                        data.response = response;
                        Ok::<(), ::std::io::Error>(())
                    } else {
                        let bytes = ::portfu::prelude::hyper::body::Bytes::from("HTTP NOT SUPPORTED ON THIS ENDPOINT");
                        *data.response.body_mut() = ::portfu::prelude::http_body_util::Full::new(bytes);
                        Ok::<(), ::std::io::Error>(())
                    }
                }
            }
        };
        output.extend(stream);
    }
}

struct WsArgs {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    filters: Vec<Path>,
    wrappers: Vec<syn::Expr>,
}

impl WsArgs {
    fn new(args: EndpointArgs) -> syn::Result<Self> {
        let mut resource_name = None;
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
            } else if nv.path.is_ident("filter") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    filters.push(lit.parse::<Path>()?);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute filter expects literal string",
                    ));
                }
            } else if nv.path.is_ident("wrap") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    wrappers.push(lit.parse()?);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute wrap expects type",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: filter, method and wrap",
                ));
            }
        }

        Ok(WsArgs {
            path: args.path,
            resource_name,
            filters,
            wrappers,
        })
    }
}
