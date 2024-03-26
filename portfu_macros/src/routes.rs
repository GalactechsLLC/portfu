use std::collections::HashSet;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{FnArg, LitStr, parse_quote, Pat, Path, punctuated::Punctuated, Token, Type};
use portfu_core::route::PathSegment;
use crate::http::Method;

pub struct DynRouteArgs {
    path: syn::LitStr,
    options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for DynRouteArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid service definition, expected #[route("<path>", options...)]"#,
            ));
            err
        })?;

        // verify that path pattern is valid
        let _ = portfu_core::route::Route::parse(path.value());

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



pub struct DynRoute {
    /// Name of the handler function being annotated.
    name: Ident,
    /// Args passed to routing macro.
    args: Args,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}
impl DynRoute {
    pub fn new(args: DynRouteArgs, ast: syn::ItemFn, method: Option<Method>) -> syn::Result<Self> {
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
                "The #[route(..)] macro requires at least one `method` attribute",
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


impl ToTokens for DynRoute {
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

        let method_guards = {
            debug_assert!(!methods.is_empty(), "Args::methods should not be empty");

            let mut others = methods.iter();
            let first = others.next().unwrap();
            if methods.len() > 1 {
                let other_method_guards: Vec<TokenStream2> = others
                    .map(| method| quote! {
                            .or(::portfu::core::filters::#method.clone())
                        }
                    ).collect();
                quote! {
                    .filter(
                        ::portfu::core::filters::any(::portfu::core::filters::#first.clone())
                            #(#other_method_guards)*
                    )
                }
            } else {
                quote! {
                    .filter(::portfu::core::filters::#first.clone())
                }
            }
        };

        let registrations = quote! {
            let __resource = ::portfu::core::service::ServiceBuilder::new(#path)
                .name(#resource_name)
                #method_guards
                #(.filter(::portfu::core::filters::fn_guard(#filters)))*
                #(.wrap(#wrappers))*
                .handler(std::sync::Arc::new(self)).build();
            service_registry.register(__resource);
        };
        let mut path_vars = vec![];
        let mut additional_function_vars = vec![];
        let mut dyn_vars = match portfu_core::route::Route::parse(path.value()) {
            portfu_core::route::Route::Static(_, _) => vec![quote! {}],
            portfu_core::route::Route::Segmented(segments, _) => {
                let mut variables = vec![];
                for segment in segments.iter().filter_map(|v| {
                    match v {
                        PathSegment::Static(_) => None,
                        PathSegment::Variable(v) => Some(Ident::new(v.name.as_str(), Span::call_site()))
                    }
                }) {
                    variables.push(
                    quote! {
                            let #segment = ::portfu::prelude::Path::extract(&mut request, stringify!(#segment)).await.unwrap();
                        }
                    );
                    path_vars.push(format!("{segment}"));
                }
                variables
            }
        };
        for arg in ast.sig.inputs.iter() {
            let (param_type, param_name) = match arg {
                FnArg::Receiver(_) => {
                    continue;
                }
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        (&typed.ty, &pat_ident.ident)
                    } else {
                        continue
                    }
                }
            };
            let ident_type : Type = parse_quote! { #param_type };
            let ident_val : Ident = parse_quote! { #param_name };
            if path_vars.contains(&format!("{ident_val}")) {
                additional_function_vars.push(quote! {
                    #ident_val,
                });
                continue;
            }
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    let body_ident: Ident = Ident::new("Body", segment.ident.span());
                    let state_ident: Ident = Ident::new("State", segment.ident.span());
                    if body_ident == segment.ident {
                        dyn_vars.push(quote! {
                            let #ident_val: #ident_type = match ::portfu::prelude::Body::extract(&mut request).await {
                                Ok(v) => v,
                                Err(e) => {
                                    *response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                                    *response.body_mut() = Full::new(Bytes::from(format!("Failed to extract Body as {}, {e:?}", stringify!(#ident_type).replace(' ',""))));
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
                    } else if state_ident == segment.ident {
                        dyn_vars.push(quote! {
                            let #ident_val: #ident_type = match ::portfu::prelude::State::extract(&mut request).await {
                                Some(v) => v,
                                None => {
                                    *response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                                    *response.body_mut() = Full::new(Bytes::from(format!("Failed to find {}", stringify!(#ident_type).replace(' ',""))));
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
                    }
                }
            }
            let function_name = &ast.sig.ident;
            additional_function_vars.push(quote! {
                match request.get() {
                    Some(v) => v,
                    None => {
                        *response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                        *response.body_mut() = Full::new(Bytes::from(format!("Failed to find {} for {}", stringify!(#ident_type).replace(' ',""), stringify!(#function_name))));
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
            pub struct #name;
            impl ::portfu::core::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry) {
                    #registrations
                }
            }
            #[::portfu::prelude::async_trait::async_trait]
            impl ::portfu::core::ServiceHandler for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                async fn handle(
                    &self,
                    mut request: ::portfu::prelude::ServiceRequest,
                    mut response: ::portfu::prelude::http::Response<::portfu::prelude::http_body_util::Full<::portfu::prelude::hyper::body::Bytes>>
                ) -> Result<::portfu::prelude::ServiceResponse, ::portfu::prelude::ServiceResponse> {
                    #ast
                    #(#dyn_vars)*
                    match #name(#(#additional_function_vars)*).await {
                        Ok(t) => {
                            let bytes: ::portfu::prelude::hyper::body::Bytes = t.into();
                            *response.body_mut() = ::portfu::prelude::http_body_util::Full::new(bytes);
                            Ok(::portfu::prelude::ServiceResponse {
                                request,
                                response
                            })
                        }
                        Err(e) => {
                            let err = format!("{e:?}");
                            let bytes: ::portfu::prelude::hyper::body::Bytes = err.into();
                            *response.body_mut() = ::portfu::prelude::http_body_util::Full::new(bytes);
                            Ok(::portfu::prelude::ServiceResponse {
                                request,
                                response
                            })
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
    fn new(args: DynRouteArgs, method: Option<Method>) -> syn::Result<Self> {
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
