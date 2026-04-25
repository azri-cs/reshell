use regex::Regex;
use once_cell::sync::Lazy;

static SECRET_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Generic key/value patterns
        Regex::new(r#"(?i)(api[_-]?key\s*[:=]\s*)['"]?[\w-]{16,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(token\s*[:=]\s*)['"]?[\w-]{8,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(password\s*[:=]\s*)['"]?[^\s'"]+['"]?"#).unwrap(),
        Regex::new(r#"(?i)(secret\s*[:=]\s*)['"]?[\w-]{8,}['"]?"#).unwrap(),
        // Bearer / Authorization headers
        Regex::new(r#"(?i)(bearer\s+)['"]?[\w-]{8,}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(authorization\s*:\s*basic\s+)[\w+/=]{8,}"#).unwrap(),
        // AWS credentials
        Regex::new(r#"(?i)(aws_access_key_id\s*[:=]\s*)['"]?[A-Z0-9]{20}['"]?"#).unwrap(),
        Regex::new(r#"(?i)(aws_secret_access_key\s*[:=]\s*)['"]?[\w/+=]{40}['"]?"#).unwrap(),
        // Private keys
        Regex::new(r#"(?i)(private[_-]?key\s*[:=]\s*)['"]?[^\s'"]+['"]?"#).unwrap(),
        // GitHub tokens (ghp_, gho_, ghu_, ghs_, github_pat_)
        Regex::new(r"(ghp_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(gho_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(ghu_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(ghs_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(github_pat_[A-Za-z0-9_]{22,})").unwrap(),
        // Slack tokens
        Regex::new(r"(xox[baprs]-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{24,})").unwrap(),
        // Stripe keys
        Regex::new(r"(sk_live_[a-zA-Z0-9]{24,})").unwrap(),
        Regex::new(r"(pk_live_[a-zA-Z0-9]{24,})").unwrap(),
        // JWT tokens (eyJ... header)
        Regex::new(r"(eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})").unwrap(),
        // PEM private key blocks
        Regex::new(r"(-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----[\s\S]*?-----END\s+(RSA\s+)?PRIVATE\s+KEY-----)").unwrap(),
        // Database connection strings
        Regex::new(r#"(?i)(mongodb(\+srv)?://[^:\s]+:)[^\s@]+"#).unwrap(),
        Regex::new(r#"(?i)(postgres(ql)?://[^:\s]+:)[^\s@]+"#).unwrap(),
        Regex::new(r#"(?i)(mysql://[^:\s]+:)[^\s@]+"#).unwrap(),
        Regex::new(r#"(?i)(redis://[^:\s]*:)[^\s@]+"#).unwrap(),
        // Generic connection string with password
        Regex::new(r#"(?i)(://[^:\s]+:)([^\s@]{4,})(@)"#).unwrap(),
    ]
});

/// Patterns where the entire match should be replaced (no capture group prefix).
static FULL_REPLACE_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(ghp_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(gho_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(ghu_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(ghs_[A-Za-z0-9_]{36,})").unwrap(),
        Regex::new(r"(github_pat_[A-Za-z0-9_]{22,})").unwrap(),
        Regex::new(r"(xox[baprs]-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{24,})").unwrap(),
        Regex::new(r"(sk_live_[a-zA-Z0-9]{24,})").unwrap(),
        Regex::new(r"(pk_live_[a-zA-Z0-9]{24,})").unwrap(),
        Regex::new(r"(eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})").unwrap(),
    ]
});

pub fn scrub_secrets(text: &str) -> String {
    // Short strings can't contain valid secrets (shortest pattern: ~10 chars minimum)
    if text.len() < 16 {
        return text.to_string();
    }

    let mut result = text.to_string();

    // First pass: patterns with capture group prefix (keep prefix, redact value)
    // replace_all returns Cow::Borrowed when nothing matches — no allocation overhead
    for re in SECRET_PATTERNS.iter() {
        result = re.replace_all(&result, "${1}[REDACTED]").to_string();
    }

    // Second pass: full-replace patterns (replace entire token)
    for re in FULL_REPLACE_PATTERNS.iter() {
        result = re.replace_all(&result, "[REDACTED]").to_string();
    }

    // PEM blocks — replace entire block
    static PEM_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----[\s\S]*?-----END\s+(?:RSA\s+)?PRIVATE\s+KEY-----)").unwrap()
    });
    result = PEM_RE.replace_all(&result, "[REDACTED PEM KEY]").to_string();

    result
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

    #[test]
    fn test_scrub_github_token() {
        let text = r#"token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij"#;
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij"));
    }

    #[test]
    fn test_scrub_stripe_key() {
        let text = "STRIPE_KEY=sk_live_abcdefghijklmnopqrstuvwxyz12";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("sk_live_"));
    }

    #[test]
    fn test_scrub_jwt() {
        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let scrubbed = scrub_secrets(text);
        assert!(!scrubbed.contains("eyJhbGci"));
    }

    #[test]
    fn test_scrub_connection_string() {
        let text = "DATABASE_URL=postgres://admin:secretpass123@db.example.com:5432/mydb";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("secretpass123"));
    }

    #[test]
    fn test_scrub_pem_key() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED PEM KEY]"));
        assert!(!scrubbed.contains("MIIEpAIBAAKCAQEA"));
    }

    #[test]
    fn test_scrub_slack_token() {
        let text = "SLACK_TOKEN=xoxb-1234567890123-1234567890123-abcdefghijklmnopqrstuvwx";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("xoxb-"));
    }

    #[test]
    fn test_scrub_aws_credentials() {
        let text = "aws_access_key_id=AKIAIOSFODNN7EXAMPLE";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("AKIAIOSFODNN7EXAMPLE"));
    }
}
