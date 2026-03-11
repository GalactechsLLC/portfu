use crate::method::Method;
use crate::utils::{extract_method_filters, parse_path_variables};
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use std::collections::HashSet;
use syn::{
    parse_quote, punctuated::Punctuated, FnArg, GenericArgument, GenericParam, Generics, LitStr,
    Pat, Path, PathArguments, Token, Type,
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
        let _ = portfu_common::router::route::Route::new(path.value());

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
                fn #factory_name(_registry: &mut ::portfu_updated::prelude::ServiceRegistry) -> ::portfu_updated::prelude::Service {
                    ::portfu_updated::prelude::ServiceBuilder::new(#path)
                        .name(#resource_name)
                        #method_filters
                        #(.filter(::portfu_updated::prelude::filters::all(#filters_name.to_string(), #filters)))*
                        #(.wrap(#wrappers))*
                        .handler(::std::sync::Arc::new(#name::default()))
                        .build()
                }
                ::portfu_updated::prelude::inventory::submit! {
                    ::portfu_updated::prelude::ServiceRegistration {
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
                    let state_ident: Ident = Ident::new("State", segment.ident.span());
                    if state_ident == segment.ident {
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
                                let #ident_val: #ident_type = match request.get()
                                    .cloned()
                                    .map(|data| ::portfu_updated::prelude::State(data)).ok_or(
                                        ::std::io::Error::new(::std::io::ErrorKind::NotFound, format!("Failed to find State of type {}", stringify!(#ident_type)))
                                    ) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        return Ok(::portfu_updated::prelude::Response::internal_error(
                                            format!("{e:?}")
                                        ));
                                    }
                                };
                            });
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                        }
                        continue;
                    }
                }
            } else if let Type::Reference(reference) = &ident_type {
                if let Type::Path(path) = &reference.elem.as_ref() {
                    if let Some(segment) = path.path.segments.first() {
                        let request: Ident = Ident::new("Request", segment.ident.span());
                        if request == segment.ident {
                            dyn_vars.push(quote! {
                                let #ident_val = request;
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
                let #ident_val: #ident_type = match ::portfu_updated::prelude::FromRequest::try_from(request).await {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(::portfu_updated::prelude::Response::internal_error(
                            format!("Failed to extract {} as {}, {e:?}",
                                stringify!(#ident_val), stringify!(#ident_type).replace(' ',"")
                            )
                        ));
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
            #inventory_def
            impl #generics ::portfu_updated::prelude::ServiceTrait for #name #generic_lables {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu_updated::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu_updated::prelude::Response, ::portfu_updated::prelude::PortfuError>> + 'a + Send + Sync>> {
                    Box::pin(async {
                        if request.method() == ::portfu_updated::prelude::http::method::Method::OPTIONS {
                            return Ok(::portfu_updated::prelude::Response::ok(""))
                        }
                        #(#dyn_vars)*
                        match Self::#name (#(#additional_function_vars)*).await {
                            Ok(resp) => {
                                Ok(resp.into())
                            }
                            Err(e) => {
                                Ok(::portfu_updated::prelude::Response::internal_error(&format!("{e:?}")))
                            }
                        }
                    })
                }
            }
        };
        token_out.extend(stream);
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
    fn new(args: EndpointArgs, method: Vec<Method>) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut filters = Vec::new();
        let mut wrappers = Vec::new();
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
