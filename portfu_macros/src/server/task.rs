use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{quote, ToTokens};
use syn::{parse_quote, FnArg, GenericArgument, Pat, PathArguments, Type};

pub struct Task {
    /// Name of the handler function being annotated.
    name: Ident,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}
impl Task {
    pub fn new(ast: syn::ItemFn) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();
        // Try and pull out the doc comments so that we can reapply them to the generated struct.
        // Note that multi line doc comments are converted to multiple doc attributes.
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();

        if matches!(ast.sig.output, syn::ReturnType::Default) {
            return Err(syn::Error::new_spanned(
                ast,
                "Function has no return type. Cannot be used as handler",
            ));
        }

        Ok(Self {
            name,
            ast,
            doc_attributes,
        })
    }
}

impl ToTokens for Task {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            ast,
            doc_attributes,
        } = self;
        let mut additional_function_vars = vec![];
        let mut dyn_vars = vec![];
        for arg in ast.sig.inputs.iter() {
            let (ident_type, ident_val): (Type, Ident) = match arg {
                FnArg::Receiver(_) => {
                    continue;
                }
                FnArg::Typed(typed) => {
                    if let Pat::Ident(pat_ident) = typed.pat.as_ref() {
                        let ty = &typed.ty;
                        let ident = &pat_ident.ident;
                        (parse_quote! { #ty }, parse_quote! { #ident })
                    } else {
                        continue;
                    }
                }
            };
            if let Type::Path(path) = &ident_type {
                if let Some(segment) = path.path.segments.first() {
                    if let Some(inner_type) = match &segment.arguments {
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
                        let state_ident: Ident = Ident::new("State", segment.ident.span());
                        if state_ident == segment.ident {
                            dyn_vars.push(quote! {
                            let #ident_val: #ident_type = state.get::<::std::sync::Arc<#inner_type>>()
                                .cloned()
                                .map(::portfu::pfcore::State).ok_or(
                                    ::std::io::Error::new(::std::io::ErrorKind::NotFound, "Failed to find State")
                                )?;
                            });
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        } else {
                            panic!("Only State Objects are Available to Tasks");
                        }
                    }
                }
            }
        }
        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            impl From<#name> for ::portfu::pfcore::task::Task {
                fn from(task: #name) -> ::portfu::pfcore::task::Task {
                    use ::portfu::pfcore::task::TaskFn;
                    ::portfu::pfcore::task::Task {
                        name: task.name().to_string(),
                        task_fn: Arc::new(task)
                    }
                }
            }
            #[::portfu::prelude::async_trait::async_trait]
            impl ::portfu::pfcore::task::TaskFn for #name {
                fn name(&self) -> &str {
                    stringify!(#name)
                }
                async fn run(
                    &self,
                    state: std::sync::Arc< ::portfu::prelude::http::Extensions >
                ) -> Result<(), ::std::io::Error> {
                    ::tokio::spawn( async move {
                        ::tokio::select! {
                            _ = async {
                                #ast
                                #(#dyn_vars)*
                                let _ = #name(#(#additional_function_vars)*).await;
                                Ok::<(), ::std::io::Error>(())
                            } => {
                                 Ok::<(), ::std::io::Error>(())
                            }
                            _ = ::portfu::pfcore::signal::await_termination() => {
                                Ok::<(), ::std::io::Error>(())
                            }
                        }
                    });
                    Ok::<(), ::std::io::Error>(())
                }
            }
        };
        output.extend(stream);
    }
}
