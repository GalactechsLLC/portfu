use crate::server::state::SharedState;
use std::sync::Arc;

#[test]
fn shared_state_wraps_owned_values_and_existing_arcs() {
    let owned: SharedState<String> = "owned".to_string().into();
    let owned_arc: Arc<String> = owned.into();
    assert_eq!(owned_arc.as_str(), "owned");

    let original = Arc::new(vec![1_u8, 2, 3]);
    let shared: SharedState<Vec<u8>> = original.clone().into();
    let shared_arc: Arc<Vec<u8>> = shared.into();
    assert!(Arc::ptr_eq(&original, &shared_arc));
}
