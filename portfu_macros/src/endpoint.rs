use crate::method::Method;
use crate::utils::{extract_method_filters, parse_path_variables, validate_route};
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use std::collections::HashSet;
use syn::{
    parse_quote, punctuated::Punctuated, FnArg, GenericArgument, GenericParam, Generics, LitStr,
    Pat, PathArguments, ReturnType, Token, Type,
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
        validate_route(&path)?;

        // if there's no comma, assume that no options are provided
        if !input.peek(Token![,]) {
            if input.is_empty() {
                return Ok(Self {
                    path,
                    options: Punctuated::new(),
                });
            } else {
                return Err(syn::Error::new(
                    Span::call_site(),
                    format!(
                        "Expected comma after path, but found {}",
                        input
                            .cursor()
                            .ident()
                            .map(|s| s.0.to_string())
                            .unwrap_or("unknown".to_string())
                    ),
                ));
            }
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
    pub fn new(args: EndpointArgs, ast: syn::ItemFn, methods: Vec<Method>) -> syn::Result<Self> {
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

        let args = Args::new(args, methods)?;

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
    fn to_tokens(&self, token_out: &mut TokenStream2) {
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
            scope,
            domains,
            filters,
            wrappers,
            client_trust,
            methods,
        } = args;
        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), LitStr::value);
        let scope = scope
            .as_ref()
            .map_or_else(|| "default".to_string(), LitStr::value);
        let method_filters = extract_method_filters(methods);
        let client_trust_wrapper = client_trust.as_ref().map(|trust_store| {
            quote! {
                .wrap(::std::sync::Arc::new(::portfu::prelude::ClientTrust::new(#trust_store)))
            }
        });
        let mut additional_function_vars = vec![];
        let (mut dyn_vars, path_vars) = match parse_path_variables(path) {
            Ok(v) => v,
            Err(err) => {
                token_out.extend(err.into_compile_error());
                return;
            }
        };
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
                GenericParam::Const(c) => {
                    has_generics = true;
                    c.ident.clone()
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
            new_ast.sig.generics.params.clear();
            quote! { #new_ast }
        } else {
            quote! { #ast }
        };
        let struct_def = if has_generics {
            quote! {
                pub struct #name #generics {
                    _phantom_data: std::marker::PhantomData #generic_lables
                }
                impl #generics #name #generic_lables {
                    #function_def
                }
            }
        } else {
            quote! {
                pub struct #name;
                impl #name {
                    #function_def
                }
            }
        };
        let factory_name = format_ident!("__portfu_make_{}", name);
        let inventory_def = if has_generics {
            quote! {
                ::core::compile_error!("Generic endpoints cannot be auto-registered with inventory. Register a concrete type manually.");
            }
        } else {
            quote! {
                fn #factory_name(_registry: &mut ::portfu::prelude::ServiceRegistry) -> ::portfu::prelude::Service {
                    ::portfu::prelude::ServiceBuilder::new(#path)
                        .name(#resource_name)
                        .scope(#scope)
                        #(.domain(#domains))*
                        #method_filters
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
                        token_out.extend(
                            syn::Error::new_spanned(
                                &typed.pat,
                                "Unsupported argument pattern in endpoint signature; use a simple identifier binding",
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
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        } else if request_headers == segment.ident {
                            if reference.mutability.is_some() {
                                dyn_vars.push(quote! {
                                    let #ident_val: &mut ::portfu::prelude::RequestHeaders = request.headers_mut();
                                });
                            } else {
                                dyn_vars.push(quote! {
                                    let #ident_val: &::portfu::prelude::RequestHeaders = request.headers();
                                });
                            }
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        } else if response_headers == segment.ident {
                            if reference.mutability.is_some() {
                                dyn_vars.push(quote! {
                                    let #ident_val: &mut ::portfu::prelude::ResponseHeaders = response.headers_mut();
                                });
                            } else {
                                dyn_vars.push(quote! {
                                    let #ident_val: &::portfu::prelude::ResponseHeaders = response.headers();
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
            dyn_vars.push(quote! {
                let #ident_val: #ident_type = match ::portfu::prelude::FromRequest::try_from(request).await {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(::portfu::prelude::IntoResponse::into_response(e));
                    }
                };
            });
            additional_function_vars.push(quote! {
                #ident_val,
            });
        }
        let ok_response = ok_response_conversion(&ast.sig.output);
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            #struct_def
            #default_struct
            #inventory_def
            impl #generics ::portfu::prelude::ServiceTrait for #name #generic_lables {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError>> + 'a + Send>> {
                    Box::pin(async {
                        if request.method() == ::portfu::prelude::http::method::Method::OPTIONS {
                            return Ok(::portfu::prelude::Response::ok(""))
                        }
                        #(#dyn_vars)*
                        match Self::#name (#(#additional_function_vars)*).await {
                            Ok(resp) => #ok_response,
                            Err(e) => Ok(::portfu::prelude::IntoResponse::into_response(e))
                        }
                    })
                }
            }
        };
        token_out.extend(stream);
    }
}

fn ok_response_conversion(output: &ReturnType) -> TokenStream2 {
    let Some(ok_type) = result_ok_type(output) else {
        return quote! { Ok(resp.into()) };
    };

    if uses_json_response_fallback(ok_type) {
        quote! { Ok(::portfu::prelude::Response::json(resp)) }
    } else {
        quote! { Ok(resp.into()) }
    }
}

fn result_ok_type(output: &ReturnType) -> Option<&Type> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };
    let Type::Path(path) = ty.as_ref() else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| {
        if let GenericArgument::Type(ty) = arg {
            Some(ty)
        } else {
            None
        }
    })
}

fn uses_json_response_fallback(ty: &Type) -> bool {
    match ty {
        Type::Tuple(tuple) if tuple.elems.is_empty() => false,
        Type::Reference(reference) => !is_text_or_bytes_reference(reference.elem.as_ref()),
        Type::Path(path) => !is_known_response_type(path),
        _ => true,
    }
}

fn is_text_or_bytes_reference(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "str"),
        Type::Slice(slice) => is_u8_type(slice.elem.as_ref()),
        _ => false,
    }
}

fn is_known_response_type(path: &syn::TypePath) -> bool {
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    if matches!(
        segment.ident.to_string().as_str(),
        "Response" | "Json" | "JsonResponse" | "String" | "Bytes"
    ) {
        return true;
    }
    if segment.ident == "Vec" {
        return vec_inner_type(segment).is_some_and(is_u8_type);
    }
    false
}

fn vec_inner_type(segment: &syn::PathSegment) -> Option<&Type> {
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| {
        if let GenericArgument::Type(ty) = arg {
            Some(ty)
        } else {
            None
        }
    })
}

fn is_u8_type(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "u8")
}

pub(crate) struct Args {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    pub(crate) scope: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
    pub(crate) filters: Vec<syn::Expr>,
    pub(crate) wrappers: Vec<syn::Expr>,
    pub(crate) client_trust: Option<syn::LitStr>,
    methods: HashSet<Method>,
}

impl Args {
    pub(crate) fn new(args: EndpointArgs, method: Vec<Method>) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut scope = None;
        let mut domains = Vec::new();
        let mut filters = Vec::new();
        let mut wrappers = Vec::new();
        let mut client_trust = None;
        let mut methods = HashSet::from_iter(method);
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
                    filters.push(lit.parse::<syn::Expr>()?);
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
            } else if nv.path.is_ident("method") {
                if let syn::Expr::Lit(syn::ExprLit {
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
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: name, scope, domain, filter, method, wrap and client_trust",
                ));
            }
        }

        Ok(Args {
            path: args.path,
            resource_name,
            scope,
            domains,
            filters,
            wrappers,
            client_trust,
            methods,
        })
    }
}
