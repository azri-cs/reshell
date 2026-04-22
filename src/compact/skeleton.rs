use regex::Regex;

pub fn extract_skeleton(text: &str) -> String {
    let mut skeleton = Vec::new();
    let re = Regex::new(r"^(\s*)(fn |function |class |struct |mod |pub fn |impl |ERROR|WARN|INFO|DEBUG|TRACE)").unwrap();
    for line in text.lines() {
        if re.is_match(line) {
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
