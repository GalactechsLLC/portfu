use proc_macro2::{Ident, TokenStream};
use quote::{quote, ToTokens};
use syn::{parse_quote, FnArg, GenericArgument, Pat, PathArguments, Type};

pub struct IntervalArgs {
    interval: u64,
}

impl syn::parse::Parse for IntervalArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let interval = input.parse::<syn::LitInt>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid interval definition, expected #[interval(<interval>)]"#,
            ));
            err
        })?;
        let interval: u64 = interval.base10_parse()?;
        Ok(Self { interval })
    }
}

pub struct Interval {
    /// Name of the handler function being annotated.
    name: Ident,
    /// AST of the handler function being annotated.
    ast: syn::ItemFn,
    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
    args: IntervalArgs,
}
impl Interval {
    pub fn new(args: IntervalArgs, ast: syn::ItemFn) -> syn::Result<Self> {
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
            args,
        })
    }
}
impl ToTokens for Interval {
    fn to_tokens(&self, output: &mut TokenStream) {
        let Self {
            name,
            ast,
            args,
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
                            let #ident_val: #ident_type = state.get::<Arc<#inner_type>>()
                                .cloned()
                                .map(|data| ::portfu::pfcore::State(data)).ok_or(
                                    ::std::io::Error::new(::std::io::ErrorKind::NotFound, format!("Failed to find State of type {}", stringify!(#inner_type)))
                                )?;
                            });
                            additional_function_vars.push(quote! {
                                #ident_val,
                            });
                            continue;
                        } else {
                            panic!("Only State Objects are Available to Intervals");
                        }
                    }
                } else {
                    panic!("Only State Objects are Available to Intervals");
                }
            } else {
                panic!("Only State Objects are Available to Intervals");
            }
        }
        let interval = args.interval;
        let out = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;
            impl From<#name> for ::portfu::pfcore::task::Task {
                fn from(interval: #name) -> ::portfu::pfcore::task::Task {
                    use ::portfu::pfcore::task::TaskFn;
                    ::portfu::pfcore::task::Task {
                        name: interval.name().to_string(),
                        task_fn: Arc::new(interval)
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
                    #ast
                    let mut __interval_duration = ::tokio::time::interval(std::time::Duration::from_millis(#interval));
                    loop {
                        #(#dyn_vars)*
                        tokio::select! {
                            _ = __interval_duration.tick() => {
                                let _ = #name(#(#additional_function_vars)*).await;
                            }
                            _ = ::portfu::pfcore::signal::await_termination() => {
                                break;
                            }
                        }
                    }
                    Ok::<(), ::std::io::Error>(())
                }
            }
        };
        output.extend(out);
    }
}
