use once_cell::sync::Lazy;
use regex::Regex;

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
        // Docker config auth in JSON
        Regex::new(r#""auth"\s*:\s*"[A-Za-z0-9+/=]{20,}""#).unwrap(),
        // GitLab tokens
        Regex::new(r"(glpat-[A-Za-z0-9_\-]{20,})").unwrap(),
        // Azure AccountKey
        Regex::new(r"AccountKey=[A-Za-z0-9+/=]{40,}").unwrap(),
        // Google Cloud API keys
        Regex::new(r"(AIza[0-9A-Za-z\-_]{35})").unwrap(),
        // npm access tokens
        Regex::new(r"(npm_[A-Za-z0-9]{36,})").unwrap(),
        // PyPI tokens
        Regex::new(r"(pypi-[A-Za-z0-9_\-]{36,})").unwrap(),
        // Sentry DSN
        Regex::new(r"https://[a-f0-9]{32}@sentry\.io/\d+").unwrap(),
    ]
});

pub fn scrub_secrets(text: &str) -> String {
    // Short strings can't contain valid secrets (shortest pattern: ~10 chars minimum)
    if text.len() < 16 {
        return text.to_string();
    }

    let mut result = text.to_string();

    // First pass: patterns with capture group prefix (keep prefix, redact value)
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
    result = PEM_RE
        .replace_all(&result, "[REDACTED PEM KEY]")
        .to_string();

    // Third pass: entropy-based detection for unknown credential formats
    let cfg = crate::config::get();
    if !cfg.scrubber.disable_entropy {
        result = scrub_high_entropy_strings(&result, cfg.scrubber.entropy_threshold);
    }

    // Fourth pass: user-configured custom patterns
    for pattern_str in &cfg.scrubber.additional_patterns {
        if let Ok(re) = Regex::new(pattern_str) {
            result = re.replace_all(&result, "[REDACTED]").to_string();
        }
    }

    result
}

/// Scan for high-entropy strings that look like base64-encoded secrets
/// but don't match any known token prefix pattern.
fn scrub_high_entropy_strings(text: &str, threshold: f64) -> String {
    let mut result = String::with_capacity(text.len());
    let mut current_candidate = String::new();

    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_' {
            current_candidate.push(c);
        } else {
            // End of candidate — check it
            if current_candidate.len() >= 32 {
                let entropy = shannon_entropy(&current_candidate);
                if entropy > threshold {
                    // High-entropy base64-like string, likely a token
                    result.push_str("[REDACTED]");
                    current_candidate.clear();
                    result.push(c);
                    continue;
                }
            }
            result.push_str(&current_candidate);
            current_candidate.clear();
            result.push(c);
        }
    }

    // Check trailing candidate
    if current_candidate.len() >= 32 && shannon_entropy(&current_candidate) > 3.5 {
        result.push_str("[REDACTED]");
    } else {
        result.push_str(&current_candidate);
    }

    result
}

/// Calculate Shannon entropy of a string (0.0-8.0 for byte values).
fn shannon_entropy(s: &str) -> f64 {
    let mut freq = [0u32; 256];
    let bytes = s.as_bytes();
    for &b in bytes {
        freq[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    let mut entropy = 0.0;
    for &count in &freq {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
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
        let text =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
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

    #[test]
    fn test_scrub_high_entropy_random_string() {
        // A 40-char random-looking hex string should be scrubbed
        let text = "key=a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0";
        let scrubbed = scrub_secrets(text);
        // Either regex-based hex pattern or entropy-based should catch this
        assert!(
            scrubbed.contains("[REDACTED]") || !scrubbed.contains("a1b2c3"),
            "high-entropy hex string should be redacted"
        );
    }

    #[test]
    fn test_does_not_scrub_low_entropy_normal_text() {
        let text = "Hello world, this is a normal sentence with no secrets.";
        let scrubbed = scrub_secrets(text);
        assert_eq!(scrubbed, text);
    }

    #[test]
    fn test_scrub_gitlab_token() {
        let text = "GITLAB_TOKEN=glpat-abcdefghijklmnopqrstuvwx";
        let scrubbed = scrub_secrets(text);
        assert!(scrubbed.contains("[REDACTED]"));
        assert!(!scrubbed.contains("glpat-"));
    }

    #[test]
    fn test_scrub_docker_auth() {
        let text = r#"{"auth": "dXNlcm5hbWU6cGFzc3dvcmQ="}"#;
        let scrubbed = scrub_secrets(text);
        // The auth value should be redacted
        assert!(!scrubbed.contains("dXNlcm5hbWU6cGFzc3dvcmQ="));
    }
}
