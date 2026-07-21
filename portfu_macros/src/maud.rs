use crate::method::Method;
use crate::utils::{extract_method_filters, validate_route};
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use std::collections::{BTreeSet, HashSet};
use syn::{punctuated::Punctuated, Expr, Fields, ItemStruct, LitStr, Token, Type};

const TEXT_HTML_UTF8: &str = "text/html; charset=utf-8";

pub struct MaudHttp {
    name: Ident,
    args: Args,
    ast: ItemStruct,
    doc_attributes: Vec<syn::Attribute>,
}

impl MaudHttp {
    pub fn new(args: MaudHttpArgs, ast: ItemStruct) -> syn::Result<Self> {
        let name = ast.ident.clone();
        if !ast.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &ast.generics,
                "maud_http macro does not support generic structs",
            ));
        }
        let args = Args::new(args)?;
        validate_struct_fields(&ast, &args.path_variables)?;
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

impl ToTokens for MaudHttp {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            args,
            ast,
            doc_attributes,
        } = self;
        let Args {
            paths,
            path_variables,
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
        let page_factory = page_factory(name, path_variables, ast);
        let mut inventory_defs = Vec::new();

        for (index, path) in paths.iter().enumerate() {
            let factory_name = format_ident!("__portfu_make_maud_{}_{}", name, index);
            inventory_defs.push(quote! {
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
            });
        }

        output.extend(quote! {
            #(#doc_attributes)*
            #ast

            #(#inventory_defs)*

            impl ::portfu::prelude::ServiceTrait for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }

                fn serve<'a>(
                    &'a self,
                    request: &'a mut ::portfu::prelude::Request
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError>> + 'a + Send>> {
                    Box::pin(async move {
                        if request.method() == ::portfu::prelude::http::method::Method::OPTIONS {
                            return Ok(::portfu::prelude::Response::ok("").content_type(#TEXT_HTML_UTF8));
                        }

                        let page = #page_factory;
                        let body = ::portfu::prelude::maud::Render::render(&page).into_string();
                        Ok(::portfu::prelude::Response::ok(body).content_type(#TEXT_HTML_UTF8))
                    })
                }
            }
        });
    }
}

pub struct MaudHttpArgs {
    pub paths: Vec<syn::LitStr>,
    pub options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for MaudHttpArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut paths = vec![input.parse::<LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid maud_http definition, expected #[maud_http("<path>", options...)]"#,
            ));
            err
        })?];

        validate_route(paths.first().expect("path exists"))?;

        let mut options = Punctuated::new();
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }

            if input.peek(LitStr) {
                let path = input.parse::<LitStr>()?;
                validate_route(&path)?;
                paths.push(path);
            } else {
                options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;
                break;
            }
        }

        if !input.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "Expected comma after maud_http path",
            ));
        }

        Ok(Self { paths, options })
    }
}

struct Args {
    paths: Vec<syn::LitStr>,
    path_variables: Vec<Ident>,
    resource_name: Option<syn::LitStr>,
    scope: Option<syn::LitStr>,
    domains: Vec<syn::LitStr>,
    filters: Vec<Expr>,
    wrappers: Vec<syn::Expr>,
    methods: HashSet<Method>,
}

impl Args {
    fn new(args: MaudHttpArgs) -> syn::Result<Self> {
        let path_variables = shared_path_variables(&args.paths)?;
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
            paths: args.paths,
            path_variables,
            resource_name,
            scope,
            domains,
            filters,
            wrappers,
            methods,
        })
    }
}

fn shared_path_variables(paths: &[syn::LitStr]) -> syn::Result<Vec<Ident>> {
    let Some(first) = paths.first() else {
        return Ok(Vec::new());
    };
    let first_variables = path_variable_names(first)?;
    for path in paths.iter().skip(1) {
        let variables = path_variable_names(path)?;
        if variables != first_variables {
            return Err(syn::Error::new_spanned(
                path,
                format!(
                    "All maud_http paths must use the same path variables. Expected {:?}, found {:?}",
                    first_variables, variables
                ),
            ));
        }
    }

    first_variables
        .into_iter()
        .map(|name| {
            syn::parse_str::<Ident>(&name).map_err(|_| {
                syn::Error::new_spanned(
                    first,
                    format!("Path variable `{name}` must be a valid Rust field identifier"),
                )
            })
        })
        .collect()
}

fn path_variable_names(path: &syn::LitStr) -> syn::Result<BTreeSet<String>> {
    validate_route(path)?;
    let mut names = BTreeSet::new();
    if let portfu_common::router::route::Route::Segmented(segments, _) =
        portfu_common::router::route::Route::new(path.value())
    {
        for segment in segments {
            if let portfu_common::router::route::PathSegment::Variable(variable) = segment {
                names.insert(variable.name);
            }
        }
    }
    Ok(names)
}

fn validate_struct_fields(ast: &ItemStruct, path_variables: &[Ident]) -> syn::Result<()> {
    if path_variables.is_empty() {
        return Ok(());
    }
    let Fields::Named(fields) = &ast.fields else {
        return Err(syn::Error::new_spanned(
            &ast.fields,
            "maud_http path variables require a struct with named fields",
        ));
    };

    for variable in path_variables {
        let Some(field) = fields
            .named
            .iter()
            .find(|field| field.ident.as_ref() == Some(variable))
        else {
            return Err(syn::Error::new_spanned(
                &ast.ident,
                format!("maud_http path variable `{variable}` requires a matching struct field"),
            ));
        };

        if !is_string_type(&field.ty) {
            return Err(syn::Error::new_spanned(
                &field.ty,
                format!("maud_http path field `{variable}` must be a String"),
            ));
        }
    }

    Ok(())
}

fn is_string_type(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "String")
}

fn page_factory(name: &Ident, path_variables: &[Ident], ast: &ItemStruct) -> TokenStream2 {
    if path_variables.is_empty() {
        return quote! { #name::default() };
    }

    let field_values = path_variables.iter().map(|variable| {
        let variable_name = syn::LitStr::new(&variable.to_string(), variable.span());
        quote! {
            #variable: request
                .route()
                .extract(request.uri().path(), #variable_name)
                .ok_or_else(|| {
                    ::portfu::prelude::PortfuError::Parsing(format!(
                        "Failed to parse path variable {} in path {}",
                        #variable_name,
                        request.uri().path()
                    ))
                })?,
        }
    });
    let struct_update = match &ast.fields {
        Fields::Named(fields) if fields.named.len() == path_variables.len() => quote! {},
        _ => quote! { ..::core::default::Default::default() },
    };

    quote! {
        #name {
            #(#field_values)*
            #struct_update
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, MaudHttp, MaudHttpArgs};
    use quote::ToTokens;

    #[test]
    fn args_accept_route_options() {
        let args = syn::parse_str::<MaudHttpArgs>(
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
    fn args_accept_multiple_paths() {
        let args = syn::parse_str::<MaudHttpArgs>(
            r#""/", "/index.html", name = "index", domain = "example.test""#,
        )
        .expect("args should parse");
        let parsed = Args::new(args).expect("maud args should parse");
        assert_eq!(
            parsed
                .paths
                .iter()
                .map(syn::LitStr::value)
                .collect::<Vec<_>>(),
            vec!["/", "/index.html"]
        );
        assert_eq!(parsed.resource_name.unwrap().value(), "index");
        assert_eq!(parsed.domains.len(), 1);
    }

    #[test]
    fn args_reject_paths_with_different_variables() {
        let args = syn::parse_str::<MaudHttpArgs>(
            r#""/users/{id}", "/users/{id}/posts/{post_id}", name = "user""#,
        )
        .expect("args should parse");
        let parsed = Args::new(args);
        assert!(parsed.is_err());
        assert!(parsed
            .err()
            .unwrap()
            .to_string()
            .contains("must use the same path variables"));
    }

    #[test]
    fn expansion_registers_get_options_html_service() {
        let args = syn::parse_str::<MaudHttpArgs>(r#""/index.html""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct IndexPage;
        };
        let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
        let rendered = endpoint.to_token_stream().to_string();
        assert!(rendered.contains("__portfu_make_maud_IndexPage_0"));
        assert!(rendered.contains("ServiceRegistration"));
        assert!(rendered.contains("text/html; charset=utf-8"));
        assert!(rendered.contains("Render :: render"));
    }

    #[test]
    fn expansion_registers_every_path() {
        let args =
            syn::parse_str::<MaudHttpArgs>(r#""/", "/index.html""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct IndexPage;
        };
        let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
        let rendered = endpoint.to_token_stream().to_string();
        assert!(rendered.contains("__portfu_make_maud_IndexPage_0"));
        assert!(rendered.contains("__portfu_make_maud_IndexPage_1"));
        assert_eq!(rendered.matches("ServiceRegistration").count(), 2);
    }

    #[test]
    fn expansion_populates_matching_string_fields_from_path_variables() {
        let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}", "/people/{id}""#)
            .expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct UserPage {
                id: String,
            }
        };
        let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
        let rendered = endpoint.to_token_stream().to_string();
        assert!(rendered.contains("extract"));
        assert!(rendered.contains("\"id\""));
        assert!(rendered.contains("UserPage"));
    }

    #[test]
    fn path_variables_require_matching_string_fields() {
        let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct UserPage;
        };
        let parsed = MaudHttp::new(args, ast);
        assert!(parsed.is_err());
        assert!(parsed
            .err()
            .unwrap()
            .to_string()
            .contains("require a struct with named fields"));

        let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct UserPage {
                other: String,
            }
        };
        let parsed = MaudHttp::new(args, ast);
        assert!(parsed.is_err());
        assert!(parsed
            .err()
            .unwrap()
            .to_string()
            .contains("requires a matching struct field"));

        let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}""#).expect("args should parse");
        let ast: syn::ItemStruct = syn::parse_quote! {
            #[derive(Clone, Debug, Default)]
            pub struct UserPage {
                id: u64,
            }
        };
        let parsed = MaudHttp::new(args, ast);
        assert!(parsed.is_err());
        assert!(parsed
            .err()
            .unwrap()
            .to_string()
            .contains("must be a String"));
    }

    #[test]
    fn generic_structs_are_rejected() {
        let args = syn::parse_str::<MaudHttpArgs>(r#""/index.html""#).expect("args should parse");
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
