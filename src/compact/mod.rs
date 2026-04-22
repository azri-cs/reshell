pub mod diff;
pub mod skeleton;
pub mod view;

use serde::{Deserialize, Serialize};
use crate::utils::is_binary;

const MAX_OUTPUT_LINES: usize = 100;
const TAIL_LINES: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactResult {
    pub compacted: bool,
    pub content: String,
    pub skeleton: String,
    pub is_binary: bool,
}

pub fn compact(output: &str, previous_hash: Option<&str>) -> CompactResult {
    // Binary detection
    if is_binary(output.as_bytes()) {
        return CompactResult {
            compacted: true,
            content: "[Binary output detected and omitted]".to_string(),
            skeleton: String::new(),
            is_binary: true,
        };
    }

    // Diff mode
    if let Some(prev) = previous_hash {
        let current_hash = diff::compute_hash(output);
        if current_hash == prev {
            return CompactResult {
                compacted: true,
                content: "[Output unchanged since last read]".to_string(),
                skeleton: String::new(),
                is_binary: false,
            };
        }
    }

    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= MAX_OUTPUT_LINES {
        return CompactResult {
            compacted: false,
            content: output.to_string(),
            skeleton: String::new(),
            is_binary: false,
        };
    }

    let head: Vec<&str> = lines.iter().take(50).copied().collect();
    let tail: Vec<&str> = lines.iter().rev().take(TAIL_LINES).rev().copied().collect();
    let sk = skeleton::extract_skeleton(output);

    let mut content = String::new();
    content.push_str(&format!("[Output truncated: {} lines total]\n", lines.len()));
    content.push_str("--- HEAD (first 50 lines) ---\n");
    content.push_str(&head.join("\n"));
    content.push_str("\n--- SKELETON (structural lines) ---\n");
    content.push_str(&sk);
    content.push_str("\n--- TAIL (last 20 lines) ---\n");
    content.push_str(&tail.join("\n"));

    CompactResult {
        compacted: true,
        content,
        skeleton: sk,
        is_binary: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_short() {
        let text = "line1\nline2\nline3";
        let res = compact(text, None);
        assert!(!res.compacted);
        assert_eq!(res.content, text);
    }

    #[test]
    fn test_compact_long() {
        let text: String = (0..200).map(|i| format!("line{}\n", i)).collect();
        let res = compact(&text, None);
        assert!(res.compacted);
        assert!(res.content.contains("HEAD"));
        assert!(res.content.contains("TAIL"));
    }

    #[test]
    fn test_compact_binary() {
        let mut bytes = vec![0u8; 100];
        bytes[0..5].copy_from_slice(b"hello");
        let text = String::from_utf8_lossy(&bytes).to_string();
        let res = compact(&text, None);
        assert!(res.is_binary);
    }
}
