use regex::{escape, Regex};
use std::borrow::Cow;

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
impl Route {
    pub fn new(input: String) -> Self {
        let mut re = format!("{}^", REGEX_FLAGS);
        let mut to_parse = input.as_str();
        let mut segments = Vec::new();
        let mut has_tail = false;
        while let Some(idx) = to_parse.find('{') {
            let (prefix, rem) = to_parse.split_at(idx);
            segments.push(PathSegment::Static(to_parse.to_string()));
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
        }
        if segments.is_empty() {
            Self::Static(Cow::Owned(input), Regex::new(re.as_str()).unwrap())
        } else {
            Self::Segmented(segments, Regex::new(re.as_str()).unwrap())
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
                    captures.name(name).map(|m| m.as_str().to_string())
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
            .unwrap_or_else(|| panic!(r#"pattern "{}" contains malformed dynamic segment"#, input));
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
