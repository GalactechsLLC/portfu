use super::{PathSegment, Route};

fn segment_pattern(route: &Route) -> Vec<String> {
    match route {
        Route::Static(path, _) => vec![path.to_string()],
        Route::Segmented(segments, _) => segments
            .iter()
            .map(|segment| match segment {
                PathSegment::Static(path) => path.clone(),
                PathSegment::Variable(variable) => format!("{{{}}}", variable.name),
            })
            .collect(),
    }
}

#[test]
fn segmented_route_builds_expected_segments() {
    let route = Route::new("/users/{id}/role".to_string());
    let pattern = segment_pattern(&route).join("");
    assert_eq!(pattern, "/users/{id}/role");
}

#[test]
fn segmented_route_with_multiple_variables_builds_expected_segments() {
    let route = Route::new("/a/{x}/b/{y}/c".to_string());
    let pattern = segment_pattern(&route).join("");
    assert_eq!(pattern, "/a/{x}/b/{y}/c");
}
