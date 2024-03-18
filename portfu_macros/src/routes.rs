use std::collections::HashSet;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{FnArg, LitStr, parse_quote, Pat, Path, punctuated::Punctuated, Token, Type};
use portfu_core::paths::PathSegment;
use crate::http::Method;

pub struct RouteArgs {
    path: syn::LitStr,
    options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for RouteArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid service definition, expected #[route("<path>", options...)]"#,
            ));
            err
        })?;

        // verify that path pattern is valid
        let _ = portfu_core::paths::Path::parse(path.value());

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



pub struct Route {
    /// Name of the handler function being annotated.
    name: syn::Ident,
    /// Args passed to routing macro.
    args: Args,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}
impl Route {
    pub fn new(args: RouteArgs, ast: syn::ItemFn, method: Option<Method>) -> syn::Result<Self> {
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


impl ToTokens for Route {
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
                            .or(::portfu_core::filters::#method.clone())
                        }
                    ).collect();
                quote! {
                    .filter(
                        ::portfu_core::filters::any(::portfu_core::filters::#first.clone())
                            #(#other_method_guards)*
                    )
                }
            } else {
                quote! {
                    .filter(::portfu_core::filters::#first.clone())
                }
            }
        };

        let registrations = quote! {
            let __resource = ::portfu_core::service::ServiceBuilder::new(#path)
                .name(#resource_name)
                #method_guards
                #(.filter(::portfu_core::filters::fn_guard(#filters)))*
                #(.wrap(#wrappers))*
                .handler(std::sync::Arc::new(self)).build();
            service_registry.register(__resource);
        };
        let mut existing_vars: Vec<String> = vec![];
        let (path_vars, mut additional_function_vars) = match portfu_core::paths::Path::parse(path.value()) {
            portfu_core::paths::Path::Static(_, _) => (vec![quote! {}], vec![quote! {}]),
            portfu_core::paths::Path::Segmented(segments, _) => {
                let mut variables = vec![];
                let fn_args = vec![];
                for segment in segments.iter().filter_map(|v| {
                    match v {
                        PathSegment::Static(_) => None,
                        PathSegment::Variable(v) => Some(Ident::new(v.name.as_str(), Span::call_site()))
                    }
                }) {
                    variables.push(
                    quote! {
                            let #segment = request.path.extract(request.request.uri().path(), stringify!(#segment));
                        }
                    );
                    existing_vars.push(format!("{segment}"));
                }
                (variables, fn_args)
            }
        };

        let mut dynamic_args = vec![];
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
            let ty_val : Type = parse_quote! { #param_type };
            let ident_val : Ident = parse_quote! { #param_name };
            if existing_vars.contains(&format!("{ident_val}")) {
                continue;
            }
            additional_function_vars.push(
                quote! {
                    request_args.remove().unwrap()
                }
            );
            let stmt = quote! {
                let #ident_val: #ty_val = request_args.remove().unwrap();
            };
            dynamic_args.push(stmt.to_token_stream());
        }

        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            impl ::portfu_core::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu_core::ServiceRegistry) {
                    #registrations
                }
            }
            #[async_trait::async_trait]
            impl ::portfu_core::ServiceHandler for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                async fn handle(
                    &self,
                    address: &::std::net::SocketAddr,
                    request: &::portfu_core::service::ServiceRequest,
                    response: ::http::Response<::http_body_util::Full<::hyper::body::Bytes>>
                ) -> Result<::http::Response<::http_body_util::Full<::hyper::body::Bytes>>, ::http::Response<::http_body_util::Full<::hyper::body::Bytes>>> {
                    #ast
                    use std::any::Any;
                    let mut request_args = ::portfu_core::data_map::DynMap::new();
                    request_args.insert(address);
                    request_args.insert(request);
                    request_args.insert(response);
                    #(#path_vars)*
                    #(#dynamic_args)*
                    #name(#(#additional_function_vars)*
                    ).await
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
    fn new(args: RouteArgs, method: Option<Method>) -> syn::Result<Self> {
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
