//! JSON/XML content compaction.
//!
//! When output is detected as JSON or XML, produce a structural summary
//! instead of the raw content.

/// Compact JSON output to a structural summary.
/// Returns the summary string, or the original content if parsing fails.
pub fn compact_json(content: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(val) => summarize_json_value(&val, 0),
        Err(_) => content.to_string(),
    }
}

fn summarize_json_value(val: &serde_json::Value, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    match val {
        serde_json::Value::Object(map) => {
            let key_count = map.len();
            if key_count == 0 {
                return format!("{}{{}} (empty object)", indent);
            }
            let mut lines = vec![format!("{}{{}} ({} keys)", indent, key_count)];
            for (k, v) in map.iter().take(20) {
                let summary = summarize_json_value(v, depth + 1);
                lines.push(format!("{}  \"{}\": {}", indent, k, summary));
            }
            if key_count > 20 {
                lines.push(format!("{}  ... ({} more keys)", indent, key_count - 20));
            }
            lines.join("\n")
        }
        serde_json::Value::Array(arr) => {
            let len = arr.len();
            if len == 0 {
                return format!("{}[] (empty array)", indent);
            }
            let mut lines = vec![format!("{}[] ({} items)", indent, len)];
            for (i, item) in arr.iter().take(5).enumerate() {
                let summary = summarize_json_value(item, depth + 1);
                lines.push(format!("{}  [{}]: {}", indent, i, summary));
            }
            if len > 5 {
                lines.push(format!("{}  ... ({} more items)", indent, len - 5));
            }
            lines.join("\n")
        }
        serde_json::Value::String(s) => {
            if s.len() > 80 {
                format!("\"...{}...\" ({} chars)", &s[..40], s.len())
            } else {
                format!("\"{}\"", s)
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
    }
}

/// Compact XML output to a structural summary.
pub fn compact_xml(content: &str) -> String {
    // Simple tag extraction — count occurrences of each tag
    let mut tags: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut in_tag = false;
    let mut tag_start = 0;

    for (i, c) in content.char_indices() {
        match c {
            '<' if !in_tag => {
                in_tag = true;
                tag_start = i + 1;
            }
            '>' | ' ' if in_tag => {
                let tag_content = &content[tag_start..i];
                // Strip trailing / from self-closing tags: <meta/> -> meta
                let tag_content = tag_content.trim_end_matches('/');
                if !tag_content.starts_with('/')
                    && !tag_content.starts_with('?')
                    && !tag_content.starts_with('!')
                {
                    let tag_name = tag_content.split([' ', '>']).next().unwrap_or(tag_content);
                    if !tag_name.is_empty() {
                        *tags.entry(tag_name).or_insert(0) += 1;
                    }
                }
                in_tag = false;
            }
            _ => {}
        }
    }

    if tags.is_empty() {
        return "[No XML tags detected]".to_string();
    }

    let mut sorted: Vec<_> = tags.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));

    let mut lines = vec!["XML structural summary:".to_string()];
    for (tag, count) in sorted.iter().take(20) {
        lines.push(format!("  <{}> x{}", tag, count));
    }
    lines.join("\n")
}

/// Detect whether content looks like JSON (starts with { or [).
pub fn looks_like_json(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

/// Detect whether content looks like XML (starts with <).
pub fn looks_like_xml(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.starts_with('<') && !trimmed.starts_with("<<") // Not a heredoc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_json_object() {
        let content = r#"{"name": "test", "value": 42, "active": true}"#;
        let result = compact_json(content);
        assert!(result.contains("3 keys"));
        assert!(result.contains("name"));
        assert!(result.contains("42"));
    }

    #[test]
    fn compact_json_bad_content_passes_through() {
        let content = "not json";
        let result = compact_json(content);
        assert_eq!(result, "not json");
    }

    #[test]
    fn compact_xml_extracts_tags() {
        let content = r#"<root><item>a</item><item>b</item><meta/></root>"#;
        let result = compact_xml(content);
        assert!(result.contains("root"));
        assert!(result.contains("item> x2"), "result: {}", result);
    }

    #[test]
    fn looks_like_json_true() {
        assert!(looks_like_json(r#"{"key": "value"}"#));
        assert!(looks_like_json(r#"[1,2,3]"#));
    }

    #[test]
    fn looks_like_json_false() {
        assert!(!looks_like_json("plain text"));
    }

    #[test]
    fn looks_like_xml_true() {
        assert!(looks_like_xml("<root></root>"));
    }

    #[test]
    fn looks_like_xml_false() {
        assert!(!looks_like_xml("plain text"));
    }
}
