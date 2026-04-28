use once_cell::sync::Lazy;
use regex::Regex;

/// Regex for extracting structural "skeleton" lines (function defs, classes, log levels, etc.)
pub static SKELETON_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(\s*)(fn |function |class |struct |mod |pub fn |impl |ERROR|WARN|INFO|DEBUG|TRACE)").unwrap()
});

pub fn extract_skeleton(text: &str) -> String {
    let mut skeleton = Vec::new();
    for line in text.lines() {
        if SKELETON_RE.is_match(line) {
            skeleton.push(line.to_string());
        }
    }
    skeleton.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_skeleton() {
        let text = r#"
fn hello() {}
class Foo {}
struct Bar;
ERROR something
random line
pub fn baz() {}
"#;
        let sk = extract_skeleton(text);
        assert!(sk.contains("fn hello"));
        assert!(sk.contains("class Foo"));
        assert!(sk.contains("ERROR something"));
        assert!(!sk.contains("random line"));
    }
}
