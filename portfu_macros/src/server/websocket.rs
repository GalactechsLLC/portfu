use crate::parse_path_variables;
use crate::server::endpoints::EndpointArgs;
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{parse_quote, FnArg, GenericArgument, LitStr, Pat, Path, PathArguments, Type};

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
                    let headers: Ident = Ident::new("HeaderMap", segment.ident.span());
                    if body_ident == segment.ident {
                        panic!("Body Not Supported for Websocket");
                    } else if state_ident == segment.ident {
                        if let Some(_inner_type) = match &segment.arguments {
                            PathArguments::None => panic!("State Inner Object Cannot be None"),
                            PathArguments::AngleBracketed(args) => {
                                if let Some(GenericArgument::Type(ty)) = args.args.first() {
                                    Some(ty)
                                } else {
                                    continue;
                                }
                            }
                            PathArguments::Parenthesized(args) => args.inputs.first(),
                        } {
                            dyn_vars.push(quote! {
                                let #ident_val: #ident_type = match handle_data.request.get()
                                    .cloned()
                                    .map(|data| ::portfu::pfcore::State(data)).ok_or(
                                        ::std::io::Error::new(::std::io::ErrorKind::NotFound, format!("Failed to find State of type {}", stringify!(#ident_type)))
                                    ) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        return Err((handle_data, e));
                                    }
                                };
                            });
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                        }
                        continue;
                    } else if ws_ident == segment.ident {
                        additional_function_vars.push(quote! {
                            websocket,
                        });
                        continue;
                    } else if headers == segment.ident {
                        additional_function_vars.push(quote! {
                            headers,
                        });
                        continue;
                    }
                }
            }
            dyn_vars.push(quote! {
                let #ident_val: #ident_type = match ::portfu::pfcore::FromRequest::from_request(&mut handle_data.request, stringify!(#ident_val)).await {
                    Ok(v) => v,
                    Err(e) => {
                        *handle_data.response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                        handle_data.response.set_body(
                            ::portfu::pfcore::service::BodyType::Stream(
                                ::portfu::prelude::hyper::body::Bytes::from(
                                    format!("Failed to extract {} as {}, {e:?}",
                                        stringify!(#ident_val), stringify!(#ident_type).replace(' ',"")
                                    )
                                ).stream_body()
                            )
                        );
                        return Ok(handle_data);
                    }
                };
            });
            additional_function_vars.push(quote! {
                #ident_val,
            });
        }
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name {
                pub peers: ::portfu::prelude::Peers
            }
            impl Default for #name {
                fn default() -> Self {
                    Self {
                        peers: Default::default()
                    }
                }
            }
            impl ::portfu::pfcore::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry, shared_state: portfu::prelude::http::Extensions) {
                    let __resource = ::portfu::pfcore::service::ServiceBuilder::new(#path)
                        .name(#resource_name)
                        .extend_state(shared_state.clone())
                        .filter(::portfu::filters::method::GET.clone())
                        .extend_state(shared_state.clone())
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
                fn service_type(&self) -> ::portfu::prelude::ServiceType {
                    ::portfu::prelude::ServiceType::API
                }
                async fn handle(
                    &self,
                    mut handle_data: ::portfu::prelude::ServiceData
                ) -> Result<::portfu::prelude::ServiceData, (::portfu::prelude::ServiceData, ::std::io::Error)> {
                    use ::portfu::pfcore::IntoStreamBody;
                    use ::portfu::prelude::futures_util::StreamExt;
                    ::portfu::prelude::log::info!("Checking for Upgrade");
                    if handle_data.request.request.is_upgrade_request() {
                        #ast
                        #(#dyn_vars)*
                        ::portfu::prelude::log::info!("Upgrading Websocket");
                        let key = match handle_data.request.request.headers() {
                            Some(header_map) => match header_map.get("Sec-WebSocket-Key") {
                                Some(key) => key.clone(),
                                None => {
                                    return Err((handle_data, ::std::io::Error::new(::std::io::ErrorKind::Other, "Missing Sec-WebSocket-Key Header")));
                                }
                            }
                            None => {
                                return Err((handle_data, ::std::io::Error::new(::std::io::ErrorKind::Other, "No Headers in Request")));
                            }
                        };
                        let response = match ::portfu::prelude::http::Response::builder()
                            .status(::portfu::prelude::http::StatusCode::SWITCHING_PROTOCOLS)
                            .header(::portfu::prelude::hyper::header::CONNECTION, "upgrade")
                            .header(::portfu::prelude::hyper::header::UPGRADE, "websocket")
                            .header("Sec-WebSocket-Accept", &::portfu::prelude::tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes()))
                            .body(::portfu::prelude::http_body_util::Full::default()) {
                            Ok(response) => response,
                            Err(e) => {
                                ::portfu::prelude::log::error!("Failed to build WebSocket Response: {}", e);
                                return Err((handle_data, ::std::io::Error::new(::std::io::ErrorKind::Other, format!("{e:?}"))));
                            }
                        };
                        ::portfu::prelude::log::info!("Got Past Request Upgrade");
                        let peers = self.peers.clone();
                        let headers = handle_data.request.request.headers().cloned().unwrap_or_default();
                        let websocket = match &mut handle_data.request.request {
                            ::portfu::prelude::IncomingRequest::Stream(request) => Ok(::portfu::prelude::hyper::upgrade::on(request)),
                            ::portfu::prelude::IncomingRequest::Sized(request) => Ok(::portfu::prelude::hyper::upgrade::on(request)),
                            ::portfu::prelude::IncomingRequest::Consumed(parts) => Ok(
                                ::portfu::prelude::hyper::upgrade::on(::portfu::prelude::http::Request::<::portfu::prelude::http_body_util::Empty<()>>::from_parts(
                                    parts.clone(),
                                    ::portfu::prelude::http_body_util::Empty::default(),
                                )),
                            ),
                            ::portfu::prelude::IncomingRequest::Empty => Err(
                                ::std::io::Error::new(::std::io::ErrorKind::Other, format!("Empty Socket Request"))
                            ),
                        };
                        ::tokio::spawn( async move {
                            ::tokio::select! {
                                _ = async {
                                    let stream = websocket?.await.map_err(|e| {
                                        ::portfu::prelude::log::error!("Failed to Upgrade Connection: {}", e);
                                        ::std::io::Error::new(::std::io::ErrorKind::Other, format!("{e:?}"))
                                    })?;
                                    let stream = ::portfu::prelude::WebsocketMsgStream::TokioIo( Box::new(::portfu::prelude::tokio_tungstenite::WebSocketStream::from_raw_socket(
                                        portfu::prelude::hyper_util::rt::TokioIo::new(stream),
                                        ::portfu::prelude::tokio_tungstenite::tungstenite::protocol::Role::Server, None)).await
                                    );
                                    let (write, read) = stream.split();
                                    let connection = ::std::sync::Arc::new(::portfu::prelude::WebsocketConnection {
                                        write: ::tokio::sync::RwLock::new(write),
                                        read: ::tokio::sync::RwLock::new(read),
                                    });
                                    let uuid = ::std::sync::Arc::new( ::portfu::prelude::uuid::Uuid::new_v4());
                                    peers.write().await.insert(*uuid.as_ref(), connection.clone());
                                    let mut websocket = ::portfu::prelude::WebSocket {
                                        connection,
                                        uuid: uuid.clone(),
                                        peers: peers.clone(),
                                    };
                                    if let Err(e) = #name(#(#additional_function_vars)*).await {
                                        ::portfu::prelude::log::error!("Websocket Exited: {e:?}");
                                    }
                                    peers.write().await.remove(uuid.as_ref());
                                    Ok::<(), ::std::io::Error>(())
                                } => {
                                     Ok::<(), ::std::io::Error>(())
                                }
                                _ = ::portfu::pfcore::signal::await_termination() => {
                                    Ok::<(), ::std::io::Error>(())
                                }
                            }
                        });
                        ::portfu::prelude::log::debug!("Sending Upgrade Response");
                        let (parts, body) = response.into_parts();
                        handle_data.response.set_response(
                            ::portfu::pfcore::service::OutgoingResponse::Stream(
                                ::portfu::prelude::http::response::Response::from_parts(parts, body.stream_body())
                            )
                        );
                        Ok::<::portfu::prelude::ServiceData, (::portfu::prelude::ServiceData, ::std::io::Error)>(handle_data)
                    } else {
                        let bytes = ::portfu::prelude::hyper::body::Bytes::from("HTTP NOT SUPPORTED ON THIS ENDPOINT");
                        handle_data.response.set_body(::portfu::pfcore::service::BodyType::Stream(bytes.stream_body()));
                        Ok::<::portfu::prelude::ServiceData, (::portfu::prelude::ServiceData, ::std::io::Error)>(handle_data)
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
