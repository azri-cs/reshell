pub fn compute_hash(text: &str) -> String {
    let hash = xxhash_rust::xxh3::xxh3_64(text.as_bytes());
    format!("{:016x}", hash)
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
