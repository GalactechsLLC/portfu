use super::{Args, Endpoint, EndpointArgs};
use crate::method::Method;
use quote::ToTokens;

#[test]
fn endpoint_args_rejects_malformed_route_without_panicking() {
    let parsed = syn::parse_str::<EndpointArgs>(r#""/users/{id""#);
    assert!(parsed.is_err());
    let message = parsed.err().unwrap().to_string();
    assert!(message.contains("Invalid route pattern"));
}

#[test]
fn args_reject_duplicate_method_entries() {
    let args = syn::parse_str::<EndpointArgs>(r#""/users", method = "GET", method = "GET""#)
        .expect("args should parse");
    let parsed = Args::new(args, vec![]);
    assert!(parsed.is_err());
    let message = parsed.err().unwrap().to_string();
    assert!(message.contains("HTTP method defined more than once"));
}

#[test]
fn args_accept_scope_option() {
    let args = syn::parse_str::<EndpointArgs>(r#""/users", scope = "admin", name = "users-list""#)
        .expect("args should parse");
    let parsed = Args::new(args, vec![Method::Get]).expect("options should parse");
    let scope = parsed.scope.expect("scope should be set");
    assert_eq!(scope.value(), "admin");
}

#[test]
fn args_accept_filter_and_wrap_expressions() {
    let args = syn::parse_str::<EndpointArgs>(
        r#""/users", filter = ::portfu::prelude::filters::method::GET.clone(), wrap = my_wrapper()"#,
    )
    .expect("args should parse");
    let parsed = Args::new(args, vec![Method::Get]).expect("options should parse");
    assert_eq!(parsed.filters.len(), 1);
    assert_eq!(parsed.wrappers.len(), 1);
}

#[test]
fn args_accept_client_trust_middleware() {
    let args = syn::parse_str::<EndpointArgs>(r#""/reports", client_trust = "internal-clients""#)
        .expect("args should parse");
    let parsed = Args::new(args, vec![Method::Post]).expect("options should parse");
    assert_eq!(
        parsed.client_trust.as_ref().map(syn::LitStr::value),
        Some("internal-clients".to_string())
    );
}

#[test]
fn args_keep_filter_and_wrap_string_compat() {
    let args = syn::parse_str::<EndpointArgs>(
        r#""/users", filter = "::portfu::prelude::filters::method::GET.clone()", wrap = "my_wrapper()""#,
    )
    .expect("args should parse");
    let parsed = Args::new(args, vec![Method::Get]).expect("options should parse");
    assert_eq!(parsed.filters.len(), 1);
    assert_eq!(parsed.wrappers.len(), 1);
}

#[test]
fn unsupported_argument_pattern_emits_compile_error() {
    let args = syn::parse_str::<EndpointArgs>(r#""/users/{id}""#).expect("args should parse");
    let ast: syn::ItemFn = syn::parse_quote! {
        async fn list_users((id, _): (String, String)) -> Result<String, ::portfu::prelude::PortfuError> {
            Ok(id)
        }
    };
    let endpoint = Endpoint::new(args, ast, vec![Method::Get]).expect("endpoint should build");
    let rendered = endpoint.to_token_stream().to_string();
    assert!(rendered.contains("compile_error"));
    assert!(rendered.contains("Unsupported argument pattern"));
}

#[test]
fn const_generic_endpoints_no_longer_panic_during_expansion() {
    let args = syn::parse_str::<EndpointArgs>(r#""/n/{id}""#).expect("args should parse");
    let ast: syn::ItemFn = syn::parse_quote! {
        async fn generic<const N: usize>(id: ::portfu::prelude::Path) -> Result<String, ::portfu::prelude::PortfuError> {
            let _ = N;
            Ok(id.inner().to_string())
        }
    };
    let endpoint = Endpoint::new(args, ast, vec![Method::Get]).expect("endpoint should build");
    let rendered = endpoint.to_token_stream().to_string();
    assert!(rendered.contains("compile_error"));
    assert!(rendered.contains("Generic endpoints cannot be auto-registered"));
}

#[test]
fn serialize_return_types_use_json_response_fallback() {
    let args = syn::parse_str::<EndpointArgs>(r#""/users""#).expect("args should parse");
    let ast: syn::ItemFn = syn::parse_quote! {
        async fn users() -> Result<Vec<User>, ::portfu::prelude::PortfuError> {
            Ok(vec![])
        }
    };
    let endpoint = Endpoint::new(args, ast, vec![Method::Get]).expect("endpoint should build");
    let rendered = endpoint.to_token_stream().to_string();
    assert!(rendered.contains("Response :: json"));
}

#[test]
fn known_response_return_types_keep_into_conversion() {
    for signature in [
        quote::quote! {
            async fn text() -> Result<String, ::portfu::prelude::PortfuError> {
                Ok(String::new())
            }
        },
        quote::quote! {
            async fn bytes() -> Result<Vec<u8>, ::portfu::prelude::PortfuError> {
                Ok(vec![])
            }
        },
        quote::quote! {
            async fn response() -> Result<::portfu::prelude::Response, ::portfu::prelude::PortfuError> {
                Ok(::portfu::prelude::Response::new())
            }
        },
        quote::quote! {
            async fn json() -> Result<::portfu::prelude::JsonResponse<Vec<User>>, ::portfu::prelude::PortfuError> {
                Ok(::portfu::prelude::JsonResponse::from(vec![]))
            }
        },
    ] {
        let args = syn::parse_str::<EndpointArgs>(r#""/value""#).expect("args should parse");
        let ast: syn::ItemFn = syn::parse2(signature).expect("signature should parse");
        let endpoint = Endpoint::new(args, ast, vec![Method::Get]).expect("endpoint should build");
        let rendered = endpoint.to_token_stream().to_string();
        assert!(rendered.contains("Ok (resp . into ())"));
        assert!(!rendered.contains("Response :: json"));
    }
}
