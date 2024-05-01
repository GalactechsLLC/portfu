use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{parse_quote, FnArg, Pat, Type};
use url::Url;

pub struct UrlArgs {
    pub url: syn::LitStr,
}

impl syn::parse::Parse for UrlArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let url = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid endpoint definition, expected #[<method>("<path>", options...)]"#,
            ));
            err
        })?;

        // verify that path pattern is valid
        let _ = Url::parse(&url.value()).unwrap();

        Ok(Self { url })
    }
}

pub struct WebSocketClient {
    /// Name of the handler function being annotated.
    name: Ident,
    /// Args passed to macro.
    args: UrlArgs,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}
impl WebSocketClient {
    pub fn new(args: UrlArgs, ast: syn::ItemFn) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();
        // Try and pull out the doc comments so that we can reapply them to the generated struct.
        // Note that multi line doc comments are converted to multiple doc attributes.
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
        let mut additional_function_vars = vec![];
        for arg in ast.sig.inputs.iter() {
            let (ident_type, ident_val): (Type, Ident) = match arg {
                FnArg::Receiver(_) => {
                    continue;
                }
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        let ty = &typed.ty;
                        let ident = &pat_ident.ident;
                        (parse_quote! { #ty }, parse_quote! { #ident })
                    } else {
                        continue;
                    }
                }
            };
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    let ws_ident: Ident = Ident::new("WebSocket", segment.ident.span());
                    let reponse_ident: Ident = Ident::new("Response", segment.ident.span());
                    if ws_ident == segment.ident {
                        additional_function_vars.push(quote! {
                            _websocket,
                        });
                        continue;
                    } else if reponse_ident == segment.ident {
                        additional_function_vars.push(quote! {
                            _response,
                        });
                        continue;
                    } else {
                        panic!("Invalid Input Type for Websocket Client {}", segment.ident);
                    }
                } else {
                    panic!(
                        "Invalid Type({}) Found in Function Definition {}",
                        ident_val, name
                    );
                }
            } else {
                panic!(
                    "Invalid Type({}) Found in Function Definition {}",
                    ident_val, name
                );
            }
        }
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub async fn #name -> Result<(), std::io::Error> {
                #ast
                let request = #url.into_client_request()
                    .map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Failed to Parse Request: {}", e),
                        )
                    })?;
                let (_websocket, _response) = ::portfu::prelude::tokio_tungstenite::connect::connect_async_tls_with_config(
                    request,
                    None,
                    false,
                    Some(::portfu::prelude::tokio_tungstenite::tls::Connector::Rustls(::std::sync::Arc::new(
                        ::portfu::prelude::rustls::client::client_conn::ClientConfig::builder()
                            .with_safe_default_cipher_suites()
                            .with_safe_default_kx_groups()
                            .with_safe_default_protocol_versions()
                            .map_err(|e| {
                                std::io::Error::new(std::io::ErrorKind::Other, format!("Error Building Client: {:?}", e))
                            })?,
                    ))),
                )
                .await
                .map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Error Connecting Client: {:?}", e),
                    )
                })?;
                let _ = #name(#(#additional_function_vars)*).await;
            }
        };
        output.extend(stream);
    }
}
