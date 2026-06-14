use crate::compact::jsonpath;

/// Deprecated compatibility shim: delegates to the richer JSONPath engine.
pub fn extract_json_path(json: &str, path: &str) -> Result<String, String> {
    jsonpath::extract(json, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backwards_compatible_dot_path() {
        let json = r#"{"name": "reshell", "version": "0.1.0"}"#;
        assert_eq!(extract_json_path(json, ".name").unwrap(), "\"reshell\"");
    }

    #[test]
    fn backwards_compatible_array_index() {
        let json = r#"{"items": [10, 20, 30]}"#;
        assert_eq!(extract_json_path(json, ".items[1]").unwrap(), "20");
    }
}
