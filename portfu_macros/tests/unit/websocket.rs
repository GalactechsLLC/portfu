use super::{WebSocketRoute, WsArgs};
use crate::endpoint::EndpointArgs;
use quote::ToTokens;

#[test]
fn websocket_args_accept_filter_and_wrap_expressions() {
    let args = syn::parse_str::<EndpointArgs>(
        r#""/ws", filter = ::portfu::prelude::filters::method::GET.clone(), wrap = my_wrapper()"#,
    )
    .expect("args should parse");
    let parsed = WsArgs::new(args).expect("websocket args should parse");
    assert_eq!(parsed.filters.len(), 1);
    assert_eq!(parsed.wrappers.len(), 1);
}

#[test]
fn websocket_args_keep_filter_and_wrap_string_compat() {
    let args = syn::parse_str::<EndpointArgs>(
        r#""/ws", filter = "::portfu::prelude::filters::method::GET.clone()", wrap = "my_wrapper()""#,
    )
    .expect("args should parse");
    let parsed = WsArgs::new(args).expect("websocket args should parse");
    assert_eq!(parsed.filters.len(), 1);
    assert_eq!(parsed.wrappers.len(), 1);
}

#[test]
fn websocket_expansion_includes_route_limits_timeout_and_trust() {
    let args = syn::parse_str::<EndpointArgs>(
        r#""/ws", client_trust = "public-clients", max_message_size = 67108864, max_frame_size = 8388608, upgrade_timeout_ms = 5000"#,
    )
    .expect("args should parse");
    let ast: syn::ItemFn = syn::parse_quote! {
        async fn peer_socket(websocket: WebSocket) -> Result<(), PortfuError> {
            let _ = websocket;
            Ok(())
        }
    };
    let route = WebSocketRoute::new(args, ast).expect("route should build");
    let rendered = route.to_token_stream().to_string();
    assert!(rendered.contains("ClientTrust :: new"));
    assert!(rendered.contains("public-clients"));
    assert!(rendered.contains("max_message_size = Some (67108864)"));
    assert!(rendered.contains("max_frame_size = Some (8388608)"));
    assert!(rendered.contains("Duration :: from_millis (5000)"));
    assert!(rendered.contains("websocket_upgrade"));
    assert!(!rendered.contains("tokio :: spawn"));
}
