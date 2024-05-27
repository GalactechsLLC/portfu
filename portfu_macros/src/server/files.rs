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
                    let mut files = ::std::collections::HashMap::new();
                    let root_path = ::std::path::Path::new(#root_path);
                    ::portfu::prelude::log::info!("Searching for files at: {root_path:?}");
                    if !root_path.exists() {
                        if let Err(e) = std::fs::create_dir(root_path) {
                            ::portfu::prelude::log::error!("Error Creating Root Directory: {e:?}");
                        }
                    }
                    if let Err(e) = ::portfu::pfcore::files::read_directory(root_path, root_path, &mut files) {
                        ::portfu::prelude::log::error!("Error Loading files: {e:?}");
                    }
                    ::portfu::prelude::ServiceGroup {
                        filters: vec![],
                        wrappers: vec![],
                        services: files.into_iter().map(| (name, path) | {
                            let mime = ::portfu::pfcore::files::get_mime_type(&name);
                            ::portfu::pfcore::service::ServiceBuilder::new(&name)
                                .name(&name)
                                .filter(::portfu::filters::method::GET.clone())
                                .handler(std::sync::Arc::new(::portfu::pfcore::files::FileLoader {
                                    name,
                                    mime,
                                    path,
                                    editable: true,
                                    cache_threshold: 65536,
                                    cache_status: std::sync::atomic::AtomicBool::default(),
                                    cached_value: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::with_capacity(0))),
                                })).build()
                        }).collect(),
                        shared_state: Default::default()
                    }
                }
            }
        };
        output.extend(out);
    }
}
