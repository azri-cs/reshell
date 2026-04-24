use regex::Regex;
use once_cell::sync::Lazy;

static SECRET_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r#"(?i)(api[_-]?key\s*[:=]\s*)['"]?[\w-]{16,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(token\s*[:=]\s*)['"]?[\w-]{8,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(password\s*[:=]\s*)['"]?[^\s'"]+['"]?"#).unwrap(),
        Regex::new(r#"(?i)(secret\s*[:=]\s*)['"]?[\w-]{8,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(bearer\s+)['"]?[\w-]{8,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(aws_access_key_id\s*[:=]\s*)['"]?[A-Z0-9]{20}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(aws_secret_access_key\s*[:=]\s*)['"]?[\w/+=]{40}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(private[_-]?key\s*[:=]\s*)['"]?[^\s'"]+['"]?"#).unwrap(),
    ]
});

pub fn scrub_secrets(text: &str) -> String {
    SECRET_PATTERNS.iter().fold(text.to_string(), |acc, re| {
        if re.is_match(&acc) {
            re.replace_all(&acc, "${1}[REDACTED]").to_string()
        } else {
            acc
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrub_api_key() {
        let text = "api_key=1234567890123456";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("1234567890123456"));
    }

    #[test]
    fn test_scrub_password() {
        let text = "password=SuperSecret123!";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
    }

    #[test]
    fn test_no_false_positive() {
        let text = "path=/home/user/file.txt";
        let scrubbed = scrub_secrets(text);
        assert_eq!(scrubbed, text);
    }
}
