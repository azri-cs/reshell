use serde_json::Value;

/// Extract a value from JSON using a small JSONPath-like expression.
///
/// Supported syntax:
/// - `.key` or `["key"]` — object key access
/// - `.array[0]` or `[0]` — array index (negative indices count from end)
/// - `.*` or `[*]` — wildcard (returns all object values or array items as an array)
/// - `[0:5]` — array slice (start inclusive, end exclusive)
/// - `[?(@.price < 10)]` — filter objects in an array by a simple comparison
///
/// The result is returned as a pretty-printed JSON string.
pub fn extract(json: &str, path: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("Invalid JSON: {}", e))?;
    let segments = parse(path)?;
    let result = navigate(&value, &segments)?;
    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Key(String),
    Index(i64),
    Wildcard,
    Slice(Option<i64>, Option<i64>),
    Filter(Comparison),
}

#[derive(Debug, Clone, PartialEq)]
struct Comparison {
    key: String,
    op: Op,
    value: serde_json::Number,
}

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

fn parse(path: &str) -> Result<Vec<Segment>, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Empty path".to_string());
    }

    // A leading dot is optional if the path starts with a bracket.
    let mut segments = Vec::new();
    let mut chars = path.chars().peekable();

    #[allow(clippy::while_let_on_iterator)]
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    segments.push(Segment::Wildcard);
                } else {
                    let key = parse_key(&mut chars)?;
                    if !key.is_empty() {
                        segments.push(Segment::Key(key));
                    }
                }
            }
            '[' => {
                segments.push(parse_bracket(&mut chars)?);
            }
            _ => {
                // Bare key without leading dot (e.g. "name")
                let mut key = String::new();
                key.push(ch);
                key.push_str(&parse_key(&mut chars)?);
                if !key.is_empty() {
                    segments.push(Segment::Key(key));
                }
            }
        }
    }

    if segments.is_empty() {
        return Err(format!("Could not parse path: {}", path));
    }

    Ok(segments)
}

fn parse_key(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<String, String> {
    let mut key = String::new();
    while let Some(&ch) = chars.peek() {
        if ch == '.' || ch == '[' {
            break;
        }
        key.push(ch);
        chars.next();
    }
    Ok(key)
}

fn parse_bracket(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<Segment, String> {
    let mut content = String::new();
    let mut depth = 1;
    let mut in_string = false;
    let mut escaped = false;

    #[allow(clippy::while_let_on_iterator)]
    while let Some(ch) = chars.next() {
        if escaped {
            content.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            content.push(ch);
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
        }
        if !in_string {
            if ch == '[' {
                depth += 1;
            } else if ch == ']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
        content.push(ch);
    }

    let content = content.trim();

    if content == "*" {
        return Ok(Segment::Wildcard);
    }

    // Quoted key: ["key"] or ['key']
    if (content.starts_with('"') && content.ends_with('"'))
        || (content.starts_with('\'') && content.ends_with('\''))
    {
        let key = &content[1..content.len() - 1];
        return Ok(Segment::Key(key.to_string()));
    }

    // Filter: [?(@.price < 10)]
    if content.starts_with("?(") && content.ends_with(')') {
        let inner = &content[2..content.len() - 1];
        return parse_filter(inner);
    }

    // Slice: [0:5]
    if content.contains(':') {
        return parse_slice(content);
    }

    // Index
    let idx: i64 = content
        .parse()
        .map_err(|_| format!("Invalid array index: {}", content))?;
    Ok(Segment::Index(idx))
}

fn parse_slice(content: &str) -> Result<Segment, String> {
    let parts: Vec<&str> = content.splitn(2, ':').collect();
    let start = if parts[0].is_empty() {
        None
    } else {
        Some(
            parts[0]
                .parse()
                .map_err(|_| format!("Invalid slice start: {}", parts[0]))?,
        )
    };
    let end = if parts[1].is_empty() {
        None
    } else {
        Some(
            parts[1]
                .parse()
                .map_err(|_| format!("Invalid slice end: {}", parts[1]))?,
        )
    };
    Ok(Segment::Slice(start, end))
}

fn parse_filter(content: &str) -> Result<Segment, String> {
    // Expected forms: @.key OP value, @.key == value, @.key != value
    let content = content.trim();
    if !content.starts_with("@.") {
        return Err(format!("Filter must start with @.: {}", content));
    }
    let rest = &content[2..];

    // Find operator by scanning for comparison tokens, respecting that the key
    // comes first after @.
    let ops = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("==", Op::Eq),
        ("!=", Op::Ne),
        ("<", Op::Lt),
        (">", Op::Gt),
    ];
    let mut found: Option<(usize, Op, usize)> = None;
    for (op_str, op) in &ops {
        if let Some(pos) = rest.find(op_str) {
            // Make sure this is not a prefix of a longer operator we already found.
            if found.as_ref().is_none_or(|(p, _, _)| pos < *p) {
                found = Some((pos, op.clone(), op_str.len()));
            }
        }
    }

    let (pos, op, op_len) =
        found.ok_or_else(|| format!("No comparison operator in filter: {}", content))?;
    let key = rest[..pos].trim().to_string();
    let value_str = rest[pos + op_len..].trim();

    let value: serde_json::Number = if value_str.contains('.') {
        serde_json::Number::from_f64(
            value_str
                .parse::<f64>()
                .map_err(|_| format!("Invalid number in filter: {}", value_str))?,
        )
        .ok_or_else(|| format!("Invalid float in filter: {}", value_str))?
    } else {
        value_str
            .parse::<i64>()
            .map_err(|_| format!("Invalid number in filter: {}", value_str))?
            .into()
    };

    Ok(Segment::Filter(Comparison { key, op, value }))
}

fn navigate(value: &Value, segments: &[Segment]) -> Result<Value, String> {
    let mut current = value.clone();
    for (i, seg) in segments.iter().enumerate() {
        current = match seg {
            Segment::Key(key) => current
                .get(key)
                .cloned()
                .ok_or_else(|| format!("Key '{}' not found at segment {}", key, i))?,
            Segment::Index(idx) => {
                let arr = current
                    .as_array()
                    .ok_or_else(|| format!("Expected array at segment {}", i))?;
                let len = arr.len() as i64;
                let real_idx = if *idx < 0 { len + *idx } else { *idx };
                if real_idx < 0 || real_idx >= len {
                    return Err(format!("Index {} out of bounds at segment {}", idx, i));
                }
                arr[real_idx as usize].clone()
            }
            Segment::Wildcard => match current {
                Value::Object(map) => Value::Array(map.values().cloned().collect()),
                Value::Array(arr) => Value::Array(arr),
                _ => {
                    return Err(format!(
                        "Cannot wildcard over non-container at segment {}",
                        i
                    ))
                }
            },
            Segment::Slice(start, end) => {
                let arr = current
                    .as_array()
                    .ok_or_else(|| format!("Expected array for slice at segment {}", i))?;
                let len = arr.len() as i64;
                let start = start.unwrap_or(0);
                let end = end.unwrap_or(len);
                let start = start.clamp(0, len) as usize;
                let end = end.clamp(0, len) as usize;
                Value::Array(arr[start..end].to_vec())
            }
            Segment::Filter(comp) => {
                let arr = current
                    .as_array()
                    .ok_or_else(|| format!("Expected array for filter at segment {}", i))?;
                let filtered: Vec<Value> = arr
                    .iter()
                    .filter(|item| matches_filter(item, comp))
                    .cloned()
                    .collect();
                Value::Array(filtered)
            }
        };
    }
    Ok(current)
}

fn matches_filter(item: &Value, comp: &Comparison) -> bool {
    let candidate = item.get(&comp.key).and_then(|v| v.as_f64());
    let target = comp.value.as_f64();
    match (candidate, target) {
        (Some(c), Some(t)) => match comp.op {
            Op::Eq => (c - t).abs() < f64::EPSILON,
            Op::Ne => (c - t).abs() >= f64::EPSILON,
            Op::Lt => c < t,
            Op::Le => c <= t,
            Op::Gt => c > t,
            Op::Ge => c >= t,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"
    {
        "name": "reshell",
        "versions": ["0.1.0", "0.2.0"],
        "items": [
            {"name": "a", "price": 10},
            {"name": "b", "price": 5},
            {"name": "c", "price": 20}
        ]
    }
    "#;

    #[test]
    fn simple_key() {
        assert_eq!(extract(JSON, ".name").unwrap(), "\"reshell\"");
    }

    #[test]
    fn quoted_key() {
        assert_eq!(extract(JSON, "[\"name\"]").unwrap(), "\"reshell\"");
    }

    #[test]
    fn array_index() {
        assert_eq!(extract(JSON, ".versions[0]").unwrap(), "\"0.1.0\"");
    }

    #[test]
    fn negative_index() {
        assert_eq!(extract(JSON, ".versions[-1]").unwrap(), "\"0.2.0\"");
    }

    #[test]
    fn wildcard() {
        let result = extract(JSON, ".items[*]").unwrap();
        assert!(result.contains("\"a\""));
        assert!(result.contains("\"b\""));
        assert!(result.contains("\"c\""));
    }

    #[test]
    fn slice() {
        let result = extract(JSON, ".versions[0:1]").unwrap();
        assert!(result.contains("0.1.0"));
        assert!(!result.contains("0.2.0"));
    }

    #[test]
    fn filter() {
        let result = extract(JSON, ".items[?(@.price < 10)]").unwrap();
        assert!(result.contains("\"b\""));
        assert!(!result.contains("\"a\""));
        assert!(!result.contains("\"c\""));
    }

    #[test]
    fn bare_key() {
        assert_eq!(extract(JSON, "name").unwrap(), "\"reshell\"");
    }
}
