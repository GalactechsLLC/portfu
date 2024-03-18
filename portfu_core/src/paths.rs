use std::borrow::Cow;
use regex::{escape, Regex};

/// Regex flags to allow '.' in regex to match '\n'
/// See the docs under: https://docs.rs/regex/1/regex/#grouping-and-flags
const REGEX_FLAGS: &str = "(?s-m)";

#[derive(Debug)]
pub struct PathVariable {
    pub name: String,
    pub data: String
}

#[derive(Debug)]
pub enum PathSegment {
    Static(String),
    Variable(PathVariable),
}

#[derive(Debug)]
pub enum Path {
    Static(Cow<'static, str>, Regex),
    Segmented(Vec<PathSegment>, Regex),
}
impl Path {
    pub fn matches(&self, path: &str) -> bool {
        match self {
            Path::Static(_, r) => r.is_match(path),
            Path::Segmented(_, r) => r.is_match(path),
        }
    }
    pub fn extract(&self, path: &str, name: &str) -> String {
        match self {
            Path::Static(_, _) => String::new(),
            Path::Segmented(_, r) => {
                if let Some(captures) = r.captures(path) {
                    captures.name(name).map(|m| m.as_str().to_string()).unwrap_or_default()
                } else {
                    String::new()
                }
            },
        }
    }
    pub fn parse(input: String) -> Self {
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

        } else if !has_tail && !to_parse.is_empty() {
            segments.push(PathSegment::Static(to_parse.to_string()));
            re.push_str(&escape(to_parse));
        }
        if segments.is_empty() {
            Self::Static(Cow::Owned(input.to_string()), Regex::new(re.as_str()).unwrap())
        } else {
            Self::Segmented(segments, Regex::new(re.as_str()).unwrap())
        }
    }
    fn parse_param(input: &str) -> (PathSegment, String, &str, bool){
        const DEFAULT_PATTERN: &str = "[^/]+";
        const DEFAULT_PATTERN_TAIL: &str = ".*";
        let close_idx = input
            .find('}')
            .unwrap_or_else(|| {
                panic!(r#"pattern "{}" contains malformed dynamic segment"#, input)
            });
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
            }
        );

        let segment = PathSegment::Variable(PathVariable{ name: name.to_string(), data: "".to_string()});
        let regex = format!(r"(?P<{}>{})", &name, &pattern);
        (segment, regex, unprocessed, tail)
    }
}
