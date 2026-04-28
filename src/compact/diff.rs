/// Compute a hash of text content for change detection.
pub fn compute_hash(text: &str) -> String {
    crate::utils::hash_command(text)
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
