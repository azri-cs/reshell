pub fn hash_command(command: &str) -> String {
    let hash = xxhash_rust::xxh3::xxh3_64(command.as_bytes());
    format!("{:016x}", hash)
}

pub fn is_binary(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    // Check for null bytes in first 8KB
    let check_len = std::cmp::min(data.len(), 8192);
    data[..check_len].contains(&0)
}

pub fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
}
