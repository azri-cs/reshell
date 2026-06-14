use serde_json::Value;

pub fn extract_json_path(json: &str, path: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("Invalid JSON: {}", e))?;
    let segments = parse_path(path)?;
    let result = navigate(&value, &segments)?;
    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
}

fn parse_path(path: &str) -> Result<Vec<PathSegment>, String> {
    let path = path.trim_start_matches('.');
    if path.is_empty() {
        return Err("Empty path".to_string());
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();
                }
                let mut idx_str = String::new();
                while let Some(&next) = chars.peek() {
                    if next == ']' {
                        chars.next();
                        break;
                    }
                    idx_str.push(next);
                    chars.next();
                }
                let idx: usize = idx_str
                    .parse()
                    .map_err(|_| format!("Invalid array index: {}", idx_str))?;
                segments.push(PathSegment::Index(idx));
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        segments.push(PathSegment::Key(current));
    }

    if segments.is_empty() {
        return Err(format!("Could not parse path: {}", path));
    }

    Ok(segments)
}

#[derive(Debug, Clone)]
enum PathSegment {
    Key(String),
    Index(usize),
}

fn navigate<'a>(value: &'a Value, segments: &[PathSegment]) -> Result<&'a Value, String> {
    let mut current = value;
    for (i, seg) in segments.iter().enumerate() {
        match seg {
            PathSegment::Key(key) => {
                current = current
                    .get(key)
                    .ok_or_else(|| format!("Key '{}' not found at segment {}", key, i))?;
            }
            PathSegment::Index(idx) => {
                current = current
                    .get(*idx)
                    .ok_or_else(|| format!("Index {} out of bounds at segment {}", idx, i))?;
            }
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_key() {
        let json = r#"{"name": "reshell", "version": "0.1.0"}"#;
        assert_eq!(extract_json_path(json, ".name").unwrap(), "\"reshell\"");
    }

    #[test]
    fn extract_nested_key() {
        let json = r#"{"meta": {"author": {"name": "azri"}}}"#;
        assert_eq!(
            extract_json_path(json, ".meta.author.name").unwrap(),
            "\"azri\""
        );
    }

    #[test]
    fn extract_array_index() {
        let json = r#"{"items": [10, 20, 30]}"#;
        assert_eq!(extract_json_path(json, ".items[1]").unwrap(), "20");
    }

    #[test]
    fn error_on_missing_key() {
        let json = r#"{"a": 1}"#;
        assert!(extract_json_path(json, ".missing").is_err());
    }

    #[test]
    fn error_on_bad_index() {
        let json = r#"{"a": [1, 2]}"#;
        assert!(extract_json_path(json, ".a[99]").is_err());
    }

    #[test]
    fn empty_path_is_error() {
        let json = r#"{"a": 1}"#;
        assert!(extract_json_path(json, "").is_err());
    }
}
