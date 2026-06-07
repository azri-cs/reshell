pub mod diff;
pub mod jq;
pub mod json;
pub mod languages;
pub mod skeleton;
pub mod view;

use crate::utils::detect_binary;
use serde::{Deserialize, Serialize};

fn max_output_lines() -> usize {
    crate::config::get().compaction.max_output_lines
}
fn tail_lines() -> usize {
    crate::config::get().compaction.tail_lines
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactResult {
    pub compacted: bool,
    pub content: String,
    pub skeleton: String,
    pub is_binary: bool,
}

pub fn compact(output: &str, previous_hash: Option<&str>) -> CompactResult {
    // Binary detection
    let (binary, mime) = detect_binary(output.as_bytes());
    if binary {
        return CompactResult {
            compacted: true,
            content: format!(
                "[Binary output detected{} and omitted]",
                mime.map(|m| format!(": {}", m)).unwrap_or_default()
            ),
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

    // Single-pass compaction: count lines, collect head/tail, and extract skeleton
    // simultaneously instead of 3 separate output.lines() iterations.
    let mut line_count: usize = 0;
    let mut head: Vec<&str> = Vec::with_capacity(50);
    let mut tail_ring: std::collections::VecDeque<&str> =
        std::collections::VecDeque::with_capacity(tail_lines());
    let mut skeleton_lines: Vec<String> = Vec::new();

    for line in output.lines() {
        line_count += 1;
        if head.len() < 50 {
            head.push(line);
        }
        if tail_ring.len() == tail_lines() {
            tail_ring.pop_front();
        }
        tail_ring.push_back(line);
        if skeleton::SKELETON_RE.is_match(line) {
            skeleton_lines.push(line.to_string());
        }
    }

    if line_count <= max_output_lines() {
        return CompactResult {
            compacted: false,
            content: output.to_string(),
            skeleton: String::new(),
            is_binary: false,
        };
    }

    let sk = skeleton_lines.join("\n");

    let mut content = String::new();
    content.push_str(&format!("[Output truncated: {} lines total]\n", line_count));
    content.push_str("--- HEAD (first 50 lines) ---\n");
    content.push_str(&head.join("\n"));
    content.push_str("\n--- SKELETON (structural lines) ---\n");
    content.push_str(&sk);
    content.push_str("\n--- TAIL (last 20 lines) ---\n");
    content.push_str(&tail_ring.iter().copied().collect::<Vec<_>>().join("\n"));

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
