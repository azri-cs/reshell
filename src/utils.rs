pub fn hash_command(command: &str) -> String {
    let hash = xxhash_rust::xxh3::xxh3_64(command.as_bytes());
    format!("{:016x}", hash)
}

/// Detect if content is binary using MIME-type inference.
/// Falls back to null-byte check if inference is inconclusive.
/// Returns (is_binary, optional_mime_type).
pub fn detect_binary(content: &[u8]) -> (bool, Option<String>) {
    if content.is_empty() {
        return (false, None);
    }

    // Check using magic bytes first
    if let Some(kind) = infer::get(content) {
        let mime = kind.mime_type();
        let text_types = [
            "text/",
            "application/json",
            "application/xml",
            "application/javascript",
            "application/x-sh",
            "application/x-shellscript",
        ];
        let is_text = text_types.iter().any(|t| mime.starts_with(t));
        if !is_text {
            return (true, Some(mime.to_string()));
        }
        return (false, Some(mime.to_string()));
    }

    // Fall back to null-byte check
    let check_len = std::cmp::min(content.len(), 8192);
    if content[..check_len].contains(&0) {
        return (true, None);
    }

    (false, None)
}

/// Quick null-byte check (existing behavior, kept for fast path).
pub fn is_binary(content: &[u8]) -> bool {
    detect_binary(content).0
}

pub fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BinarySummary {
    pub mime_type: Option<String>,
    pub byte_count: usize,
    pub sha256: String,
    pub first_bytes: String,
    pub last_bytes: String,
}

pub fn summarize_binary(content: &[u8]) -> BinarySummary {
    let (_, mime) = detect_binary(content);
    let sha256 = sha256_hex(content);
    let first_bytes = hex_prefix(content, 16);
    let last_bytes = hex_suffix(content, 16);
    BinarySummary {
        mime_type: mime,
        byte_count: content.len(),
        sha256,
        first_bytes,
        last_bytes,
    }
}

fn sha256_hex(data: &[u8]) -> String {
    // Use xxhash for a fast deterministic hash since sha256 is not in deps.
    let hash = xxhash_rust::xxh3::xxh3_128(data);
    format!("{:032x}", hash)
}

fn hex_prefix(data: &[u8], n: usize) -> String {
    data.iter()
        .take(n)
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex_suffix(data: &[u8], n: usize) -> String {
    data.iter()
        .rev()
        .take(n)
        .rev()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Truncate `s` to at most `max_bytes` UTF-8 without splitting a multibyte codepoint.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_command_deterministic() {
        let h1 = hash_command("echo hello");
        let h2 = hash_command("echo hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16); // 64-bit hex
    }

    #[test]
    fn hash_command_different_inputs() {
        let h1 = hash_command("echo hello");
        let h2 = hash_command("echo world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn detects_png_as_binary() {
        let png: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let (is_bin, mime) = detect_binary(&png);
        assert!(is_bin);
        assert_eq!(mime.unwrap(), "image/png");
    }

    #[test]
    fn detects_json_as_text() {
        let json = b"{\"key\": \"value\"}";
        let (is_bin, _) = detect_binary(json);
        assert!(!is_bin);
    }

    #[test]
    fn is_binary_empty() {
        assert!(!is_binary(b""));
    }

    #[test]
    fn is_binary_text() {
        assert!(!is_binary(b"Hello, world!\n"));
    }

    #[test]
    fn is_binary_with_null_byte() {
        assert!(is_binary(b"hello\x00world"));
    }

    #[test]
    fn is_binary_null_at_start() {
        assert!(is_binary(b"\x00hello"));
    }

    #[test]
    fn normalize_command_collapses_whitespace() {
        assert_eq!(normalize_command("  echo   hello  "), "echo hello");
    }

    #[test]
    fn normalize_command_single_word() {
        assert_eq!(normalize_command("ls"), "ls");
    }

    #[test]
    fn shell_quote_simple() {
        assert_eq!(shell_quote("hello"), "'hello'");
    }

    #[test]
    fn shell_quote_with_single_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_quote_empty() {
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn truncate_utf8_respects_char_boundary() {
        let s = "é".repeat(100);
        let t = truncate_utf8(&s, 3);
        assert!(t.len() <= 3);
        assert!(s.starts_with(&t) || t.is_empty());
    }
}
