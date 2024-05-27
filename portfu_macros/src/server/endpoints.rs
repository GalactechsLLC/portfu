use crate::method::Method;
use crate::{extract_method_filters, parse_path_variables};
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use std::collections::HashSet;
use syn::{
    parse_quote, punctuated::Punctuated, FnArg, GenericParam, Generics, LitStr, Pat, Path, Token,
    Type,
};

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
    generics: Generics,
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
        let generics = ast.sig.generics.clone();

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
            generics,
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
            generics,
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
        let mut additional_function_vars = vec![];
        let (mut dyn_vars, path_vars) = parse_path_variables(path);
        let mut has_generics = false;
        let generic_vals: Vec<Ident> = generics
            .params
            .iter()
            .map(|p| match p {
                GenericParam::Lifetime(l) => {
                    has_generics = true;
                    l.lifetime.ident.clone()
                }
                GenericParam::Type(t) => {
                    has_generics = true;
                    t.ident.clone()
                }
                GenericParam::Const(_) => {
                    panic!("CONST Generics not Supported Yet");
                }
            })
            .collect();
        let generic_lables = if has_generics {
            quote! {
                <#(#generic_vals),*>
            }
        } else {
            quote! {}
        };
        let struct_def = if has_generics {
            quote! {
                pub struct #name #generics {
                    _phantom_data: std::marker::PhantomData #generic_lables
                }
            }
        } else {
            quote! {
                pub struct #name;
            }
        };
        let default_struct = if has_generics {
            quote! {
                impl #generics Default for #name #generic_lables {
                    fn default() -> Self {
                        Self {
                            _phantom_data: Default::default()
                        }
                    }
                }
            }
        } else {
            quote! {
                impl Default for #name {
                    fn default() -> Self {
                        Self {}
                    }
                }
            }
        };
        let function_def = if has_generics {
            let mut new_ast = ast.clone();
            new_ast.sig.generics = parse_quote! {
                #generics
            };
            quote! { #new_ast }
        } else {
            quote! { #ast }
        };
        let function_ext = if has_generics {
            quote! { :: #generic_lables }
        } else {
            quote! {}
        };
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
                        panic!("Invalid Type Passed to Endpoint: {typed:?}");
                    }
                }
            };
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    let response: Ident = Ident::new("Response", segment.ident.span());
                    let service_data: Ident = Ident::new("ServiceData", segment.ident.span());
                    if response == segment.ident {
                        dyn_vars.push(quote! {
                            let #ident_val: &mut Response<Full<Bytes>> = &mut handle_data.response;
                        });
                        additional_function_vars.push(quote! {
                            #ident_val,
                        });
                        continue;
                    } else if service_data == segment.ident {
                        dyn_vars.push(quote! {
                            let #ident_val = &mut handle_data;
                        });
                        additional_function_vars.push(quote! {
                            #ident_val,
                        });
                        continue;
                    }
                }
            }
            if let Type::Reference(reference) = &ident_type {
                if let Type::Path(path) = &reference.elem.as_ref() {
                    if let Some(segment) = path.path.segments.first() {
                        let service_data: Ident = Ident::new("ServiceData", segment.ident.span());
                        if service_data == segment.ident {
                            dyn_vars.push(quote! {
                                let #ident_val = &mut handle_data;
                            });
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        }
                    }
                }
            }
            dyn_vars.push(quote! {
                let #ident_val: #ident_type = match ::portfu::pfcore::FromRequest::from_request(&mut handle_data.request, stringify!(#ident_val)).await {
                    Ok(v) => v,
                    Err(e) => {
                        *handle_data.response.status_mut() = ::portfu::prelude::http::StatusCode::INTERNAL_SERVER_ERROR;
                        *handle_data.response.body_mut() = ::portfu::prelude::hyper::body::Bytes::from(format!("Failed to extract {} as {}, {e:?}", stringify!(#ident_val), stringify!(#ident_type).replace(' ',""))).stream_body();
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
            #struct_def
            #default_struct
            impl #generics ::portfu::pfcore::ServiceRegister for #name #generic_lables {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry, _shared_state: portfu::prelude::http::Extensions) {
                    #registrations
                }
            }
            impl #generics From<#name #generic_lables > for ::portfu::prelude::Service {
                fn from(service: #name #generic_lables) -> ::portfu::prelude::Service {
                    #service_def
                }
            }
            #[::portfu::prelude::async_trait::async_trait]
            impl #generics ::portfu::pfcore::ServiceHandler for #name #generic_lables {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                async fn handle(
                    &self,
                    mut handle_data: ::portfu::prelude::ServiceData
                ) -> Result<::portfu::prelude::ServiceData, (::portfu::prelude::ServiceData, ::std::io::Error)> {
                    use ::portfu::pfcore::IntoStreamBody;
                    #function_def
                    #(#dyn_vars)*
                    match #name #function_ext(#(#additional_function_vars)*).await {
                        Ok(t) => {
                            let bytes: ::portfu::prelude::hyper::body::Bytes = t.into();
                            *handle_data.response.body_mut() = bytes.stream_body();
                            Ok(handle_data)
                        }
                        Err(e) => {
                            let err = format!("{e:?}");
                            let bytes: ::portfu::prelude::hyper::body::Bytes = err.into();
                            *handle_data.response.body_mut() = bytes.stream_body();
                            Ok(handle_data)
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
