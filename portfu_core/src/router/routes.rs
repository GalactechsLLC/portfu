use regex::{escape, Regex};
use std::borrow::Cow;
use std::fmt::Display;

const REGEX_FLAGS: &str = "(?s-m)";

#[derive(Debug)]
pub struct PathVariable {
    pub name: String,
}

#[derive(Debug)]
pub struct PathData {
    pub name: String,
}

#[derive(Debug)]
pub enum PathSegment {
    Static(String),
    Variable(PathVariable),
}

#[derive(Debug)]
pub enum Route {
    Static(Cow<'static, str>, Regex),
    Segmented(Vec<PathSegment>, Regex),
}
impl Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Static(s, _) => write!(f, "{}", s),
            Route::Segmented(s, _) => {
                for segment in s {
                    match segment {
                        PathSegment::Static(s) => write!(f, "{}", s)?,
                        PathSegment::Variable(v) => write!(f, "{{{}}}", v.name)?,
                    }
                }
                Ok(())
            }
        }
    }
}
impl Route {
    pub fn new(input: String) -> Self {
        let mut re = format!("{REGEX_FLAGS}^");
        let mut to_parse = input.as_str();
        let mut segments = Vec::new();
        let mut has_tail = false;
        while let Some(idx) = to_parse.find('{') {
            let (prefix, rem) = to_parse.split_at(idx);
            segments.push(PathSegment::Static(prefix.to_string()));
            re.push_str(&escape(prefix));
            let (param_pattern, re_part, rem, tail) = Self::parse_param(rem);
            if tail {
                has_tail = true;
            }
            segments.push(param_pattern);
            re.push_str(&re_part);
            to_parse = rem;
        }
        if to_parse.ends_with('*') {
            re.push_str(&escape(to_parse.strip_suffix('*').unwrap()));
            re.push_str(".*");
        } else if !has_tail && !to_parse.is_empty() {
            segments.push(PathSegment::Static(to_parse.to_string()));
            re.push_str(&escape(to_parse));
            re.push('$');
        } else {
            re.push('$');
        }
        let regex_string = Regex::new(re.as_str()).unwrap();
        if segments.is_empty() {
            Self::Static(Cow::Owned(input), regex_string)
        } else {
            Self::Segmented(segments, regex_string)
        }
    }
    pub fn matches(&self, path: &str) -> bool {
        match self {
            Route::Static(_, r) => r.is_match(path),
            Route::Segmented(_, r) => r.is_match(path),
        }
    }
    pub fn extract(&self, path: &str, name: &str) -> Option<String> {
        match self {
            Route::Static(_, _) => None,
            Route::Segmented(_, r) => {
                if let Some(captures) = r.captures(path) {
                    captures.name(name).map(|m| {
                        //Decode the Value
                        urlencoding::decode(m.as_str())
                            .map(|v| v.to_string())
                            .unwrap_or(m.as_str().to_string())
                    })
                } else {
                    None
                }
            }
        }
    }
    fn parse_param(input: &str) -> (PathSegment, String, &str, bool) {
        const DEFAULT_PATTERN: &str = "[^/]+";
        const DEFAULT_PATTERN_TAIL: &str = ".*";
        let close_idx = input
            .find('}')
            .unwrap_or_else(|| panic!(r#"pattern "{input}" contains malformed dynamic segment"#));
        let (mut param, mut unprocessed) = input.split_at(close_idx + 1);
        let tail = unprocessed == "*";
        // remove outer curly brackets
        param = &param[1..param.len() - 1];
        let (name, pattern) = (
            param,
            if tail {
                unprocessed = &unprocessed[1..];
                DEFAULT_PATTERN_TAIL
            } else {
                DEFAULT_PATTERN
            },
        );

        let segment = PathSegment::Variable(PathVariable {
            name: name.to_string(),
        });
        let regex = format!(r"(?P<{}>{})", &name, &pattern);
        (segment, regex, unprocessed, tail)
    }
}

#[cfg(test)]
mod tests {
    use super::Route;

    #[test]
    fn route_to_string_static_path_is_stable() {
        let route = Route::new("/admin/users".to_string());
        assert_eq!(route.to_string(), "/admin/users");
    }

    #[test]
    fn route_to_string_dynamic_path_is_stable() {
        let route = Route::new("/users/{id}/role".to_string());
        assert_eq!(route.to_string(), "/users/{id}/role");
    }

    #[test]
    fn route_to_string_multiple_dynamic_segments_are_stable() {
        let route = Route::new("/a/{x}/b/{y}/c".to_string());
        assert_eq!(route.to_string(), "/a/{x}/b/{y}/c");
    }
}
