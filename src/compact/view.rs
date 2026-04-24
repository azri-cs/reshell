use serde::{Deserialize, Serialize};

use super::{CompactResult, diff};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompactView {
    Full,
    Skeleton,
    Diff,
    ErrorsOnly,
}

impl CompactView {
    pub fn parse(value: &str) -> Self {
        match value {
            "full" => Self::Full,
            "diff" => Self::Diff,
            "errors_only" => Self::ErrorsOnly,
            _ => Self::Skeleton,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewedCompactResult {
    pub view: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_output_id: Option<String>,
}

pub fn render_view(
    output: &str,
    view: CompactView,
    previous: Option<&str>,
    source_output_id: Option<String>,
) -> ViewedCompactResult {
    let content = match view {
        CompactView::Full => output.to_string(),
        CompactView::Skeleton => {
            let compacted = super::compact(output, None);
            skeleton_or_content(&compacted)
        }
        CompactView::Diff => {
            if let Some(previous_output) = previous {
                let diff_output = diff::line_diff(previous_output, output);
                if diff_output.trim().is_empty() {
                    "[No structural changes since previous output]".to_string()
                } else {
                    diff_output
                }
            } else {
                let compacted = super::compact(output, None);
                if compacted.compacted {
                    compacted.content
                } else {
                    output.to_string()
                }
            }
        }
        CompactView::ErrorsOnly => {
            let errors: Vec<&str> = output
                .lines()
                .filter(|line| {
                    let bytes = line.as_bytes();
                    bytes.windows(5).any(|w| w.eq_ignore_ascii_case(b"ERROR"))
                        || bytes.windows(4).any(|w| w.eq_ignore_ascii_case(b"WARN"))
                        || bytes.windows(5).any(|w| w.eq_ignore_ascii_case(b"FATAL"))
                })
                .collect();

            if errors.is_empty() {
                "[No error or warning lines found]".to_string()
            } else {
                errors.join("\n")
            }
        }
    };

    ViewedCompactResult {
        view: view_name(view).to_string(),
        content,
        base_hash: Some(diff::compute_hash(output)),
        source_output_id,
    }
}

fn skeleton_or_content(compacted: &CompactResult) -> String {
    if !compacted.skeleton.is_empty() {
        compacted.skeleton.clone()
    } else {
        compacted.content.clone()
    }
}

fn view_name(view: CompactView) -> &'static str {
    match view {
        CompactView::Full => "full",
        CompactView::Skeleton => "skeleton",
        CompactView::Diff => "diff",
        CompactView::ErrorsOnly => "errors_only",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_errors_only_view() {
        let output = "INFO start\nWARN slow\nERROR failed\nDEBUG tail";
        let rendered = render_view(output, CompactView::ErrorsOnly, None, None);
        assert!(rendered.content.contains("WARN slow"));
        assert!(rendered.content.contains("ERROR failed"));
        assert!(!rendered.content.contains("DEBUG tail"));
    }

    #[test]
    fn renders_diff_view_with_previous_output() {
        let rendered = render_view("line1\nline3", CompactView::Diff, Some("line1\nline2"), None);
        assert_eq!(rendered.content, "line3");
    }
}
