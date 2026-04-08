use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::{FnArg, Pat, PathArguments, Token, Type};

pub struct TaskArgs {
    pub(crate) options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for TaskArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self {
                options: Punctuated::new(),
            });
        }
        let options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;
        Ok(Self { options })
    }
}

pub struct Task {
    pub(crate) name: Ident,
    pub(crate) scope: String,
    pub(crate) ast: syn::ItemFn,
    pub(crate) state_args: Vec<(Ident, Type, Type)>,
    pub(crate) doc_attributes: Vec<syn::Attribute>,
}

impl Task {
    pub fn new(args: TaskArgs, ast: syn::ItemFn) -> syn::Result<Self> {
        if !ast.sig.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &ast.sig.generics,
                "task macro does not support generic functions",
            ));
        }

        let name = ast.sig.ident.clone();
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();
        let parsed = ParsedArgs::new(args)?;
        let state_args = parse_state_args(&ast)?;
        Ok(Self {
            name,
            scope: parsed.scope,
            ast,
            state_args,
            doc_attributes,
        })
    }
}

impl ToTokens for Task {
    fn to_tokens(&self, output: &mut TokenStream2) {
        self.render(output, None);
    }
}

impl Task {
    pub(crate) fn render(&self, output: &mut TokenStream2, interval_ms: Option<u64>) {
        let name = &self.name;
        let ast = &self.ast;
        let scope = &self.scope;
        let doc_attributes = &self.doc_attributes;
        let register_name = if let Some(interval_ms) = interval_ms {
            format_ident!("__portfu_interval_register_{}_{}", name, interval_ms)
        } else {
            format_ident!("__portfu_task_register_{}", name)
        };
        let mut dyn_vars = vec![];
        let mut args = vec![];
        for (ident_val, ident_type, inner_type) in &self.state_args {
            dyn_vars.push(quote! {
                let #ident_val: #ident_type = merged_state
                    .get::<::std::sync::Arc<#inner_type>>()
                    .cloned()
                    .map(::portfu_updated::prelude::State)
                    .ok_or_else(|| {
                        ::portfu_updated::prelude::PortfuError::Parsing(
                            format!("Failed to find State of type {}", stringify!(#inner_type))
                        )
                    })?;
            });
            args.push(quote! { #ident_val, });
        }
        let run_body = if let Some(interval_ms) = interval_ms {
            quote! {
                let mut ticker = ::tokio::time::interval(::std::time::Duration::from_millis(#interval_ms));
                loop {
                    if !server.run.load(::std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    ticker.tick().await;
                    #(#dyn_vars)*
                    #name(#(#args)*).await?;
                }
                Ok(())
            }
        } else {
            quote! {
                #(#dyn_vars)*
                #name(#(#args)*).await
            }
        };

        let stream = quote! {
            #(#doc_attributes)*
            #ast

            fn #register_name(
                server: ::std::sync::Arc<::portfu_updated::prelude::Server>
            ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<(), ::portfu_updated::prelude::PortfuError>> + Send + 'static>> {
                Box::pin(async move {
                    let merged_state = {
                        let scoped_state = server.scoped_state.read().await;
                        let mut extensions = scoped_state
                            .get("default")
                            .cloned()
                            .unwrap_or_default();
                        if #scope != "default" {
                            if let Some(scope_extensions) = scoped_state.get(#scope) {
                                extensions.extend(scope_extensions.clone());
                            }
                        }
                        extensions
                    };
                    #run_body
                })
            }

            ::portfu_updated::prelude::inventory::submit! {
                ::portfu_updated::prelude::TaskRegistration {
                    run: #register_name
                }
            }
        };
        output.extend(stream);
    }
}

struct ParsedArgs {
    scope: String,
}

impl ParsedArgs {
    fn new(args: TaskArgs) -> syn::Result<Self> {
        let mut scope = "default".to_string();
        for nv in args.options {
            if nv.path.is_ident("scope") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = nv.value
                {
                    scope = lit.value();
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute scope expects literal string",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    nv.path,
                    "Unknown attribute key is specified; allowed: scope",
                ));
            }
        }
        Ok(Self { scope })
    }
}

fn parse_state_args(ast: &syn::ItemFn) -> syn::Result<Vec<(Ident, Type, Type)>> {
    let mut state_args = Vec::new();
    for arg in ast.sig.inputs.iter() {
        let (ident_type, ident_val): (Type, Ident) = match arg {
            FnArg::Receiver(_) => {
                return Err(syn::Error::new_spanned(
                    arg,
                    "task functions cannot have a self receiver",
                ));
            }
            FnArg::Typed(typed) => {
                if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                    let ty = &typed.ty;
                    let ident = &pat_ident.ident;
                    (syn::parse_quote! { #ty }, syn::parse_quote! { #ident })
                } else {
                    return Err(syn::Error::new_spanned(
                        &typed.pat,
                        "Unsupported argument pattern in task signature; use a simple identifier binding",
                    ));
                }
            }
        };

        let Type::Path(path) = &ident_type else {
            return Err(syn::Error::new_spanned(
                &ident_type,
                "Only State<T> arguments are supported in task functions",
            ));
        };
        let Some(segment) = path.path.segments.first() else {
            return Err(syn::Error::new(
                Span::call_site(),
                "Invalid task argument type",
            ));
        };
        if segment.ident != "State" {
            return Err(syn::Error::new_spanned(
                &ident_type,
                "Only State<T> arguments are supported in task functions",
            ));
        }
        let inner_type = match &segment.arguments {
            PathArguments::AngleBracketed(args) => args
                .args
                .first()
                .and_then(|v| match v {
                    syn::GenericArgument::Type(ty) => Some(ty.clone()),
                    _ => None,
                })
                .ok_or_else(|| syn::Error::new_spanned(&ident_type, "State<T> requires a type"))?,
            _ => {
                return Err(syn::Error::new_spanned(
                    &ident_type,
                    "State<T> requires a generic inner type",
                ));
            }
        };
        state_args.push((ident_val, ident_type, inner_type));
    }
    Ok(state_args)
}
