use std::collections::HashMap;
use std::{env, fs};
use std::path::{Path, PathBuf};
use proc_macro2::{Ident, TokenStream};
use quote::{quote, ToTokens};
use syn::__private::TokenStream2;

pub struct StaticFileArgs {
    files: HashMap<String, String>
}

impl syn::parse::Parse for StaticFileArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let root_path = input.parse::<syn::LitStr>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid static_file definition, expected #[files("<root_path>")]"#,
            ));
            err
        })?;
        let as_str = root_path.value();
        let path = if as_str.starts_with('/') {
            PathBuf::from(as_str)
        } else {
            let path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("Expected to find env var CARGO_MANIFEST_DIR"));
            path.join(as_str)
        };
        let mut files = HashMap::new();
        read_directory(path.as_path(), path.as_path(), &mut files);
        Ok(Self {
            files
        })

    }
}

pub struct StaticFiles {
    args: StaticFileArgs,
    name: Ident
}
impl StaticFiles {
    pub fn new(args: StaticFileArgs, name: Ident) -> syn::Result<Self> {
        Ok(Self {
            args,
            name
        })
    }
}
impl ToTokens for StaticFiles {
    fn to_tokens(&self, output: &mut TokenStream) {
        let name = &self.name;
        let registrations: Vec<TokenStream2> = self.args.files.iter()
            .map(| (key, value)| quote! {
                    let __resource = ::portfu::core::service::ServiceBuilder::new(#key)
                        .name(stringify!(#name))
                        .filter(
                            Arc::new(::portfu::core::filters::any(
                                "static_filters".to_string(),
                                &[
                                    ::portfu::core::filters::GET.clone(),
                                    ::portfu::core::filters::HEAD.clone(),
                                    ::portfu::core::filters::OPTIONS.clone(),
                                    ::portfu::core::filters::TRACE.clone(),
                                ]
                            ))
                        )
                        .handler(Arc::new((stringify!(#key), stringify!(#value)))).build();
                    service_registry.register(__resource);
                }
            ).collect();
        let out = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            impl ::portfu::core::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry) {
                    let _self = std::sync::Arc::new(self);
                    #(#registrations)*
                }
            }
        };
        output.extend(out);
    }
}

fn read_directory(root: &Path, path: &Path, file_map: &mut HashMap<String, String>) {
    let mut dir_reader = path.read_dir().unwrap();
    while let Some(Ok(entry)) = dir_reader.next() {
        let entry_path = entry.path();
        if entry.path().is_dir() {
            read_directory(root, entry_path.as_path(), file_map);
        } else {
            read_file(root, entry_path.as_path(), file_map);
        }
    }
}

fn read_file(root: &Path, path: &Path, file_map: &mut HashMap<String, String>) {
    let as_string = fs::read_to_string(path).unwrap();
    let mut new_root = PathBuf::from("/");
    let path = path.canonicalize().unwrap();
    let path = path.strip_prefix(root).unwrap();
    new_root.extend(path);
    file_map.insert(new_root.to_string_lossy().to_string(), as_string);
}