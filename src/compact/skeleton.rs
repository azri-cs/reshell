use once_cell::sync::Lazy;
use regex::Regex;

/// Generic fallback regex for extracting structural "skeleton" lines.
pub static SKELETON_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(\s*)(fn |function |class |struct |mod |pub fn |impl |ERROR|WARN|INFO|DEBUG|TRACE|FATAL)",
    )
    .unwrap()
});

/// Extract skeleton using generic patterns (backward-compatible).
pub fn extract_skeleton(text: &str) -> String {
    let mut skeleton = Vec::new();
    for line in text.lines() {
        if SKELETON_RE.is_match(line) {
            skeleton.push(line.to_string());
        }
    }
    skeleton.join("\n")
}

/// Extract skeleton using language-aware detection if filename is provided.
pub fn extract_skeleton_with_lang(text: &str, filename: Option<&str>) -> String {
    let lang = super::languages::detect_language(text, filename);
    let result = super::languages::extract_skeleton_for_language(text, lang);
    if result.is_empty() {
        // Fallback to generic
        extract_skeleton(text)
    } else {
        result
    }
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
