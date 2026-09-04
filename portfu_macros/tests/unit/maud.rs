use super::{Args, MaudHttp, MaudHttpArgs};
use quote::ToTokens;

#[test]
fn args_accept_route_options() {
    let args = syn::parse_str::<MaudHttpArgs>(
        r#""/index.html", name = "index", scope = "site", domain = "example.test", filter = ::portfu::prelude::filters::method::GET.clone(), method = "POST", wrap = my_wrapper()"#,
    )
    .expect("args should parse");
    let parsed = Args::new(args).expect("maud args should parse");
    assert_eq!(parsed.resource_name.unwrap().value(), "index");
    assert_eq!(parsed.scope.unwrap().value(), "site");
    assert_eq!(parsed.domains.len(), 1);
    assert_eq!(parsed.filters.len(), 1);
    assert_eq!(parsed.wrappers.len(), 1);
    assert!(parsed.methods.contains(&crate::method::Method::Post));
}

#[test]
fn args_reject_duplicate_method_entries() {
    let args = syn::parse_str::<MaudHttpArgs>(r#""/index.html", method = "GET""#)
        .expect("args should parse");
    let parsed = Args::new(args);
    assert!(parsed.is_err());
    assert!(parsed
        .err()
        .unwrap()
        .to_string()
        .contains("HTTP method defined more than once"));
}

#[test]
fn args_accept_multiple_paths() {
    let args = syn::parse_str::<MaudHttpArgs>(
        r#""/", "/index.html", name = "index", domain = "example.test""#,
    )
    .expect("args should parse");
    let parsed = Args::new(args).expect("maud args should parse");
    assert_eq!(
        parsed
            .paths
            .iter()
            .map(syn::LitStr::value)
            .collect::<Vec<_>>(),
        vec!["/", "/index.html"]
    );
    assert_eq!(parsed.resource_name.unwrap().value(), "index");
    assert_eq!(parsed.domains.len(), 1);
}

#[test]
fn args_reject_paths_with_different_variables() {
    let args = syn::parse_str::<MaudHttpArgs>(
        r#""/users/{id}", "/users/{id}/posts/{post_id}", name = "user""#,
    )
    .expect("args should parse");
    let parsed = Args::new(args);
    assert!(parsed.is_err());
    assert!(parsed
        .err()
        .unwrap()
        .to_string()
        .contains("must use the same path variables"));
}

#[test]
fn expansion_registers_get_options_html_service() {
    let args = syn::parse_str::<MaudHttpArgs>(r#""/index.html""#).expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        #[derive(Clone, Debug, Default)]
        pub struct IndexPage;
    };
    let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
    let rendered = endpoint.to_token_stream().to_string();
    assert!(rendered.contains("__portfu_make_maud_IndexPage_0"));
    assert!(rendered.contains("ServiceRegistration"));
    assert!(rendered.contains("text/html; charset=utf-8"));
    assert!(rendered.contains("Render :: render"));
}

#[test]
fn expansion_registers_every_path() {
    let args = syn::parse_str::<MaudHttpArgs>(r#""/", "/index.html""#).expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        #[derive(Clone, Debug, Default)]
        pub struct IndexPage;
    };
    let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
    let rendered = endpoint.to_token_stream().to_string();
    assert!(rendered.contains("__portfu_make_maud_IndexPage_0"));
    assert!(rendered.contains("__portfu_make_maud_IndexPage_1"));
    assert_eq!(rendered.matches("ServiceRegistration").count(), 2);
}

#[test]
fn expansion_populates_matching_string_fields_from_path_variables() {
    let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}", "/people/{id}""#)
        .expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        #[derive(Clone, Debug, Default)]
        pub struct UserPage {
            id: String,
        }
    };
    let endpoint = MaudHttp::new(args, ast).expect("maud endpoint should build");
    let rendered = endpoint.to_token_stream().to_string();
    assert!(rendered.contains("extract"));
    assert!(rendered.contains("\"id\""));
    assert!(rendered.contains("UserPage"));
}

#[test]
fn path_variables_require_matching_string_fields() {
    let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}""#).expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        #[derive(Clone, Debug, Default)]
        pub struct UserPage;
    };
    let parsed = MaudHttp::new(args, ast);
    assert!(parsed.is_err());
    assert!(parsed
        .err()
        .unwrap()
        .to_string()
        .contains("require a struct with named fields"));

    let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}""#).expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        #[derive(Clone, Debug, Default)]
        pub struct UserPage {
            other: String,
        }
    };
    let parsed = MaudHttp::new(args, ast);
    assert!(parsed.is_err());
    assert!(parsed
        .err()
        .unwrap()
        .to_string()
        .contains("requires a matching struct field"));

    let args = syn::parse_str::<MaudHttpArgs>(r#""/users/{id}""#).expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        #[derive(Clone, Debug, Default)]
        pub struct UserPage {
            id: u64,
        }
    };
    let parsed = MaudHttp::new(args, ast);
    assert!(parsed.is_err());
    assert!(parsed
        .err()
        .unwrap()
        .to_string()
        .contains("must be a String"));
}

#[test]
fn generic_structs_are_rejected() {
    let args = syn::parse_str::<MaudHttpArgs>(r#""/index.html""#).expect("args should parse");
    let ast: syn::ItemStruct = syn::parse_quote! {
        pub struct IndexPage<T>(T);
    };
    let parsed = MaudHttp::new(args, ast);
    assert!(parsed.is_err());
    assert!(parsed
        .err()
        .unwrap()
        .to_string()
        .contains("does not support generic structs"));
}
