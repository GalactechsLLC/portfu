use proc_macro2::TokenStream as TokenStream2;
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::Token;

pub struct IntervalArgs {
    interval_ms: u64,
    options: Punctuated<syn::MetaNameValue, Token![,]>,
}

impl syn::parse::Parse for IntervalArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let interval = input.parse::<syn::LitInt>().map_err(|mut err| {
            err.combine(syn::Error::new(
                err.span(),
                r#"invalid interval definition, expected #[interval(<milliseconds>, options...)]"#,
            ));
            err
        })?;
        let interval_ms: u64 = interval.base10_parse()?;
        if !input.peek(Token![,]) {
            if input.is_empty() {
                return Ok(Self {
                    interval_ms,
                    options: Punctuated::new(),
                });
            }
            return Err(syn::Error::new(
                input.span(),
                "Expected comma after interval value",
            ));
        }
        input.parse::<Token![,]>()?;
        let options = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;
        Ok(Self {
            interval_ms,
            options,
        })
    }
}

pub struct Interval {
    task: crate::task::Task,
    interval_ms: u64,
}

impl Interval {
    pub fn new(args: IntervalArgs, ast: syn::ItemFn) -> syn::Result<Self> {
        let task_args = crate::task::TaskArgs {
            options: args.options,
        };
        let task = crate::task::Task::new(task_args, ast)?;
        Ok(Self {
            task,
            interval_ms: args.interval_ms,
        })
    }
}

impl ToTokens for Interval {
    fn to_tokens(&self, output: &mut TokenStream2) {
        self.task.render(output, Some(self.interval_ms));
    }
}
