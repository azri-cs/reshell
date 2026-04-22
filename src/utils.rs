use sha2::{Sha256, Digest};

pub fn hash_command(command: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.as_bytes());
    hex::encode(hasher.finalize())
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
    stderr.trim().to_string()
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
