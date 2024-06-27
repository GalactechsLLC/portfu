use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{quote, ToTokens};

pub struct FilesArgs {
    path: String,
}

impl syn::parse::Parse for FilesArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid file definition, expected #[files("<root_path>")]"#,
            ));
            err
        })?;
        let path = path.value();
        Ok(Self { path })
    }
}

pub struct Files {
    args: FilesArgs,
    name: Ident,
}
impl Files {
    pub fn new(args: FilesArgs, name: Ident) -> syn::Result<Self> {
        Ok(Self { args, name })
    }
}
impl ToTokens for Files {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let name = &self.name;
        let mut path = self.args.path.clone();
        let root_path = if path.ends_with('/') {
            path
        } else {
            path.push('/');
            path
        };
        let out = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            impl From<#name> for ::portfu::prelude::ServiceGroup {
                fn from(slf: #name) -> ::portfu::prelude::ServiceGroup {
                    ::portfu::prelude::ServiceGroup::from(::portfu::pfcore::files::DynamicFiles {
                        root_directory: std::path::PathBuf::from(#root_path),
                        editable: true
                    })
                }
            }
        };
        output.extend(out);
    }
}
