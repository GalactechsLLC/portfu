use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{parse_quote, FnArg, Pat, Type};

pub struct UrlArgs {
    pub url: syn::LitStr,
}

impl syn::parse::Parse for UrlArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let url = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid websocket client definition, expected #[client_websocket("<url>")]"#,
            ));
            err
        })?;
        if !url.value().starts_with("ws://") && !url.value().starts_with("wss://") {
            return Err(syn::Error::new_spanned(
                &url,
                "client_websocket URL must start with ws:// or wss://",
            ));
        }
        Ok(Self { url })
    }
}

pub struct WebSocketClient {
    name: Ident,
    args: UrlArgs,
    ast: syn::ItemFn,
    doc_attributes: Vec<syn::Attribute>,
}

impl WebSocketClient {
    pub fn new(args: UrlArgs, ast: syn::ItemFn) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();
        if !ast.sig.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &ast.sig.generics,
                "client_websocket macro does not support generic functions",
            ));
        }
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();
        Ok(Self {
            name,
            args,
            ast,
            doc_attributes,
        })
    }
}

impl ToTokens for WebSocketClient {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            ast,
            args,
            doc_attributes,
        } = self;
        let url = &args.url;
        let inner_name = quote::format_ident!("__portfu_ws_client_inner_{}", name);
        let mut inner_ast = ast.clone();
        inner_ast.sig.ident = inner_name.clone();
        let mut additional_function_vars = vec![];
        for arg in inner_ast.sig.inputs.iter() {
            let (ident_type, ident_val): (Type, Ident) = match arg {
                FnArg::Receiver(_) => continue,
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        let ty = &typed.ty;
                        let ident = &pat_ident.ident;
                        (parse_quote! { #ty }, parse_quote! { #ident })
                    } else {
                        output.extend(
                            syn::Error::new_spanned(
                                &typed.pat,
                                "Unsupported argument pattern in client websocket signature; use a simple identifier binding",
                            )
                            .into_compile_error(),
                        );
                        return;
                    }
                }
            };
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    if segment.ident == "ClientWebSocket" {
                        additional_function_vars.push(quote! { websocket, });
                        continue;
                    }
                }
            }
            output.extend(
                syn::Error::new(
                    Span::call_site(),
                    format!(
                        "Invalid argument `{ident_val}` in client websocket function `{name}`. Supported type: WebSocketClient"
                    ),
                )
                .into_compile_error(),
            );
            return;
        }
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub async fn #name() -> Result<(), ::portfu::prelude::PortfuError> {
                #inner_ast
                let (websocket, _response) = ::portfu::prelude::tokio_tungstenite::connect_async(#url)
                    .await
                    .map_err(|e| ::portfu::prelude::PortfuError::Internal(format!("WebSocket client connection failed: {e:?}")))?;
                let websocket = ::portfu::prelude::ClientWebSocket::new(websocket);
                #inner_name(#(#additional_function_vars)*).await
            }
        };
        output.extend(stream);
    }
}
