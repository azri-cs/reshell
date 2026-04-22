use sha2::{Sha256, Digest};

pub fn compute_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Simple line diff: returns lines present in new but not in old
pub fn line_diff(old: &str, new: &str) -> String {
    let old_lines: std::collections::HashSet<&str> = old.lines().collect();
    let mut result = Vec::new();
    for line in new.lines() {
        if !old_lines.contains(line) {
            result.push(line);
        }
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_diff() {
        let old = "line1\nline2";
        let new = "line1\nline3";
        assert_eq!(line_diff(old, new), "line3");
    }
}
