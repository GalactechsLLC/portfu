use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::{Expr, Token};

pub struct FilesArgs {
    pub path: Expr,
    pub options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for FilesArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path: Expr = input.parse()?;

        // if there's no comma, assume that no options are provided
        if !input.peek(Token![,]) {
            return Ok(Self {
                path,
                options: Punctuated::new(),
            });
        }

        // advance past comma separator
        input.parse::<Token![,]>()?;

        let options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;

        Ok(Self { path, options })
    }
}

pub struct Files {
    parsed_args: ParsedArgs,
    name: Ident,
}
impl Files {
    pub fn new(args: FilesArgs, name: Ident) -> syn::Result<Self> {
        let parsed_args = ParsedArgs::new(args)?;
        Ok(Self { parsed_args, name })
    }
}
impl ToTokens for Files {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let name = &self.name;
        let path_expr = &self.parsed_args.path;
        let cache_size_limit = self.parsed_args.cache_size_limit;
        let out = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;

            impl TryFrom<#name> for ::portfu::prelude::ServiceGroup {
                type Error = std::io::Error;

                fn try_from(slf: #name) -> Result<::portfu::prelude::ServiceGroup, std::io::Error> {
                    let mut root_path = (#path_expr).to_string();
                    if !(root_path.ends_with('/') || root_path.ends_with('\\')) {
                        root_path.push(std::path::MAIN_SEPARATOR);
                    }
                    ::portfu::prelude::ServiceGroup::try_from(::portfu::pfcore::files::dynamic::DynamicFiles {
                        root_directory: std::path::PathBuf::from(root_path),
                        editable: true,
                        cache_size_limit: #cache_size_limit
                    })
                }
            }
        };
        output.extend(out);
    }
}

struct ParsedArgs {
    path: Expr,
    cache_size_limit: u64,
}

impl ParsedArgs {
    fn new(args: FilesArgs) -> syn::Result<Self> {
        let mut cache_size_limit = None;
        for nv in args.options {
            if nv.path.is_ident("cache_limit") {
                if let Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(lit),
                    ..
                }) = nv.value
                {
                    cache_size_limit = Some(lit.base10_parse::<u64>()?);
                } else {
                    return Err(syn::Error::new_spanned(
                        nv.value,
                        "Attribute cache_limit expects literal u64",
                    ));
                }
            }
        }
        Ok(Self {
            path: args.path,
            cache_size_limit: cache_size_limit.unwrap_or(65536),
        })
    }
}
