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

pub fn normalize_stderr(stderr: &str) -> String {
    // Delegates to the cross-platform normalizer for consistent behavior.
    crate::classify::normalize::normalize_stderr(stderr)
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
