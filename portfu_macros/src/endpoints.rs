use crate::method::Method;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use std::collections::HashSet;
use syn::{parse_quote, punctuated::Punctuated, FnArg, LitStr, Pat, Path, Token, Type};
use crate::{extract_method_filters, parse_path_variables};

pub struct EndpointArgs {
    pub path: syn::LitStr,
    pub options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for EndpointArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid endpoint definition, expected #[<method>("<path>", options...)]"#,
            ));
            err
        })?;

        // verify that path pattern is valid
        let _ = portfu_core::routes::Route::new(path.value());

        // if there's no comma, assume that no options are provided
        if !input.peek(Token![,]) {
            return Ok(Self {
                path,
                options: Punctuated::new(),
            });
        }

        // advance past comma separator
        input.parse::<Token![,]>()?;

        // if next char is a literal, assume that it is a string and show multi-path error
        if input.cursor().literal().is_some() {
            return Err(syn::Error::new(
                Span::call_site(),
                r#"Multiple paths specified! There should be only one."#,
            ));
        }

        // zero or more options: name = "foo"
        let options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;

        Ok(Self { path, options })
    }
}

pub struct Endpoint {
    /// Name of the handler function being annotated.
    name: Ident,
    /// Args passed to routing macro.
    args: Args,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}
impl Endpoint {
    pub fn new(args: EndpointArgs, ast: syn::ItemFn, method: Option<Method>) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();

        // Try and pull out the doc comments so that we can reapply them to the generated struct.
        // Note that multi line doc comments are converted to multiple doc attributes.
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();

        let args = Args::new(args, method)?;

        if args.methods.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "The #[<route>(..)] macro requires at least one `method` attribute",
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

impl ToTokens for Endpoint {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            ast,
            args,
            doc_attributes,
        } = self;
        let Args {
            path,
            resource_name,
            filters,
            wrappers,
            methods,
        } = args;
        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), LitStr::value);
        let filters_name = format!("{resource_name}_filters");
        let method_filters = extract_method_filters(methods);
        let registrations = quote! {
            let __resource = ::portfu::pfcore::service::ServiceBuilder::new(#path)
                .name(#resource_name)
                #method_filters
                #(.filter(::portfu::pfcore::filters::all(#filters_name, #filters)))*
                #(.wrap(#wrappers))*
                .handler(std::sync::Arc::new(self)).build();
            service_registry.register(__resource);
        };
        let service_def = quote! {
            ::portfu::pfcore::service::ServiceBuilder::new(#path)
                .name(#resource_name)
                #method_filters
                #(.filter(::portfu::pfcore::filters::all(#filters_name, #filters)))*
                #(.wrap(#wrappers))*
                .handler(std::sync::Arc::new(service)).build()
        };
        let mut additional_function_vars = vec![];
        let (mut dyn_vars, path_vars) = parse_path_variables(path);
        for arg in ast.sig.inputs.iter() {
            let (param_type, param_name) = match arg {
                FnArg::Receiver(_) => {
                    continue;
                }
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        (&typed.ty, &pat_ident.ident)
                    } else {
                        continue;
                    }
                }
            };
            let ident_type: Type = parse_quote! { #param_type };
            let ident_val: Ident = parse_quote! { #param_name };
            if path_vars.contains(&format!("{ident_val}")) {
                additional_function_vars.push(quote! {
                    #ident_val,
                });
                continue;
            }
            dyn_vars.push(quote! {
                let #ident_val: #ident_type = match ::portfu::pfcore::FromRequest::from_request(&mut data.request, stringify!(#ident_val)).await {
                    Ok(v) => v,
                    Err(e) => {
                        *data.response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                        *data.response.body_mut() = ::portfu::prelude::http_body_util::Full::new(::portfu::prelude::hyper::body::Bytes::from(format!("Failed to extract {} as {}, {e:?}", stringify!(#ident_val), stringify!(#ident_type).replace(' ',""))));
                        return Ok(());
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
            pub struct #name;
            impl ::portfu::pfcore::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry) {
                    #registrations
                }
            }
            impl From<#name> for ::portfu::prelude::Service {
                fn from(service: #name) -> Service {
                    #service_def
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
                    #ast
                    #(#dyn_vars)*
                    match #name(#(#additional_function_vars)*).await {
                        Ok(t) => {
                            let bytes: ::portfu::prelude::hyper::body::Bytes = t.into();
                            *data.response.body_mut() = ::portfu::prelude::http_body_util::Full::new(bytes);
                            Ok(())
                        }
                        Err(e) => {
                            let err = format!("{e:?}");
                            let bytes: ::portfu::prelude::hyper::body::Bytes = err.into();
                            *data.response.body_mut() = ::portfu::prelude::http_body_util::Full::new(bytes);
                            Ok(())
                        }
                    }
                }
            }
        };
        output.extend(stream);
    }
}

struct Args {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    filters: Vec<Path>,
    wrappers: Vec<syn::Expr>,
    methods: HashSet<Method>,
}

impl Args {
    fn new(args: EndpointArgs, method: Option<Method>) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut filters = Vec::new();
        let mut wrappers = Vec::new();
        let mut methods = HashSet::new();

        let is_route_macro = method.is_none();
        if let Some(method) = method {
            methods.insert(method);
        }

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
            } else if nv.path.is_ident("method") {
                if !is_route_macro {
                    return Err(syn::Error::new_spanned(
                        &nv,
                        "HTTP method forbidden here; to handle multiple methods, use `route` instead",
                    ));
                } else if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value.clone()
                {
                    if !methods.insert(Method::try_from(&lit)?) {
                        return Err(syn::Error::new_spanned(
                            nv.value,
                            format!("HTTP method defined more than once: `{}`", lit.value()),
                        ));
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute method expects literal string",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: filter, method and wrap",
                ));
            }
        }

        Ok(Args {
            path: args.path,
            resource_name,
            filters,
            wrappers,
            methods,
        })
    }
}
