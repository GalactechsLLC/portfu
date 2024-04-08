use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env};

pub struct StaticFileArgs {
    files: HashMap<String, String>,
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
            let path = PathBuf::from(
                env::var("CARGO_MANIFEST_DIR")
                    .expect("Expected to find env var CARGO_MANIFEST_DIR"),
            );
            path.join(as_str)
        };
        let mut files = HashMap::new();
        read_directory(path.as_path(), path.as_path(), &mut files);
        Ok(Self { files })
    }
}

pub struct StaticFiles {
    args: StaticFileArgs,
    name: Ident,
}
impl StaticFiles {
    pub fn new(args: StaticFileArgs, name: Ident) -> syn::Result<Self> {
        Ok(Self { args, name })
    }
}
impl ToTokens for StaticFiles {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let name = &self.name;
        let mut static_file_defs = vec![];
        let service_defs: Vec<TokenStream2> = self
            .args
            .files
            .iter()
            .map(|(key, value)| {
                let file_len = Path::new(value).metadata().unwrap().len() as usize;
                let key_name = key.replace(['/', '.',')','(','-',' ','+'], "_").replace("__", "_");
                let static_bytes_name = format_ident!("STATIC_FILE{}", key_name);
                let static_ref_name = format_ident!("STATIC_FILE_REF{}", key_name);
                static_file_defs.push( quote! {
                    static #static_bytes_name: &'static [u8; #file_len] = include_bytes!(#value);
                    static #static_ref_name: ::portfu::prelude::once_cell::sync::Lazy<&'static [u8]> = ::portfu::prelude::once_cell::sync::Lazy::new(|| {
                        #static_ref_name.as_ref()
                    });
                });
                quote! {
                    ::portfu::pfcore::service::ServiceBuilder::new(#key)
                    .name(stringify!(#name))
                    .handler(::std::sync::Arc::new((stringify!(#key), #static_ref_name.as_ref()))).build(),
                }
            })
            .collect();
        let static_file_group = quote! {
            ServiceGroup {
                services: vec![
                    #(#service_defs)*
                ],
                filters: vec![
                    ::std::sync::Arc::new(::portfu::filters::any(
                        "static_filters".to_string(),
                        &[
                            ::portfu::filters::method::GET.clone(),
                            ::portfu::filters::method::HEAD.clone(),
                            ::portfu::filters::method::OPTIONS.clone(),
                            ::portfu::filters::method::TRACE.clone(),
                        ]
                    ))
                ],
                wrappers: vec![]
            }
        };
        let out = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            impl ::portfu::pfcore::ServiceRegister for #name {
                fn register(self, service_registry: &mut portfu::prelude::ServiceRegistry) {
                    let group: ::portfu::prelude::ServiceGroup = self.into();
                    for service in group.services {
                        service_registry.register(service);
                    }
                }
            }
            #(#static_file_defs)*
            impl From<#name> for ::portfu::prelude::ServiceGroup {
                fn from(_: #name) -> ::portfu::prelude::ServiceGroup {
                    #static_file_group
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

fn read_file(root: &'_ Path, starting_path: &'_ Path, file_map: &'_ mut HashMap<String, String>) {
    let mut new_root = PathBuf::from("/");
    let path = starting_path.canonicalize().unwrap();
    let path = path.strip_prefix(root).unwrap();
    new_root.extend(path);
    file_map.insert(new_root.to_string_lossy().to_string(), starting_path.to_string_lossy().to_string());
}
