use crate::endpoint::EndpointArgs;
use crate::method::Method;
use crate::utils::{extract_method_filters, validate_route};
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use std::collections::HashSet;
use syn::{Expr, ItemStruct};

const TEXT_HTML_UTF8: &str = "text/html; charset=utf-8";

pub struct MaudHttp {
    name: Ident,
    args: Args,
    ast: ItemStruct,
    doc_attributes: Vec<syn::Attribute>,
}

impl MaudHttp {
    pub fn new(args: EndpointArgs, ast: ItemStruct) -> syn::Result<Self> {
        let name = ast.ident.clone();
        validate_route(&args.path)?;
        if !ast.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &ast.generics,
                "maud_http macro does not support generic structs",
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
            args: Args::new(args)?,
            ast,
            doc_attributes,
        })
    }
}

impl ToTokens for MaudHttp {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            args,
            ast,
            doc_attributes,
        } = self;
        let Args {
            path,
            resource_name,
            scope,
            domains,
            filters,
            wrappers,
            methods,
        } = args;
        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), syn::LitStr::value);
        let scope = scope
            .as_ref()
            .map_or_else(|| "default".to_string(), syn::LitStr::value);
        let method_filters = extract_method_filters(methods);
        let factory_name = format_ident!("__portfu_make_maud_{}", name);

        output.extend(quote! {
            #(#doc_attributes)*
            #ast

            #[allow(non_snake_case)]
            fn #factory_name(_registry: &mut ::portfu::prelude::ServiceRegistry) -> ::portfu::prelude::Service {
                ::portfu::prelude::ServiceBuilder::new(#path)
                    .name(#resource_name)
                    .scope(#scope)
                    #(.domain(#domains))*
                    #method_filters
                    #(.filter(#filters))*
                    #(.wrap(#wrappers))*
                    .handler(::std::sync::Arc::new(#name::default()))
                    .build()
            }

            ::portfu::prelude::inventory::submit! {
                ::portfu::prelude::ServiceRegistration {
                    register: #factory_name
                }
            }

            impl ::portfu::prelude::ServiceTrait for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }

                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError>> + 'a + Send + Sync>> {
                    Box::pin(async move {
                        if request.method() == ::portfu::prelude::http::method::Method::OPTIONS {
                            return Ok(::portfu::prelude::Response::ok("").content_type(#TEXT_HTML_UTF8));
                        }

                        let body = ::portfu::prelude::maud::Render::render(self).into_string();
                        Ok(::portfu::prelude::Response::ok(body).content_type(#TEXT_HTML_UTF8))
                    })
                }
            }
        });
    }
}

struct Args {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    scope: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
    filters: Vec<Expr>,
    wrappers: Vec<syn::Expr>,
    methods: HashSet<Method>,
}

impl Args {
    fn new(args: EndpointArgs) -> syn::Result<Self> {
        let mut resource_name = None;
        let mut scope = None;
        let mut domains = Vec::new();
        let mut filters = Vec::new();
        let mut wrappers = Vec::new();
        let methods = HashSet::from([Method::Get, Method::Options]);

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
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: name, scope, domain, filter and wrap",
                ));
            }
        }

        Ok(Self {
            path: args.path,
            resource_name,
            scope,
            domains,
            filters,
            wrappers,
            methods,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, MaudHttp};
    use crate::endpoint::EndpointArgs;
    use quote::ToTokens;

    #[test]
    fn args_accept_route_options() {
        let args = syn::parse_str::<EndpointArgs>(
            r#""/index.html", name = "index", scope = "site", domain = "example.test", filter = ::portfu::prelude::filters::method::GET.clone(), wrap = my_wrapper()"#,
        )
        .expect("args should parse");
        let parsed = Args::new(args).expect("maud args should parse");
        assert_eq!(parsed.resource_name.unwrap().value(), "index");
        assert_eq!(parsed.scope.unwrap().value(), "site");
        assert_eq!(parsed.domains.len(), 1);
        assert_eq!(parsed.filters.len(), 1);
        assert_eq!(parsed.wrappers.len(), 1);
    }

    #[test]
    fn expansion_registers_get_options_html_service() {
        let args = syn::parse_str::<EndpointArgs>(r#""/index.html""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct IndexPage;
        };
        let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
        let rendered = endpoint.to_token_stream().to_string();
        assert!(rendered.contains("__portfu_make_maud_IndexPage"));
        assert!(rendered.contains("ServiceRegistration"));
        assert!(rendered.contains("text/html; charset=utf-8"));
        assert!(rendered.contains("Render :: render"));
    }

    #[test]
    fn generic_structs_are_rejected() {
        let args = syn::parse_str::<EndpointArgs>(r#""/index.html""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            pub struct IndexPage<T>(T);
        };
        let parsed = MaudHttp::new(args, ast);
        assert!(parsed.is_err());
        assert!(parsed
            .err()
            .unwrap()
            .to_string()
            .contains("does not support generic structs"));
    }
}
