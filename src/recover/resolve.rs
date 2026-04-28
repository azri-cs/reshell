//! Merge heuristic recovery with learned patterns from the store.

use crate::classify::normalize::normalize_stderr;
use crate::classify::taxonomy::RecoveryCode;
use crate::env::Detector;
use crate::memory::Store;
use crate::recover::memory::pattern_to_suggestion;
use crate::recover::strategies::Suggestion;
use crate::recover::suggest;
use crate::utils::{normalize_command, truncate_utf8};

/// Max stderr bytes passed through MCP / JSON for pattern lookup (defensive bound).
pub const STDERR_PATTERN_MAX_BYTES: usize = 8192;

/// Derive text used for pattern DB lookup: explicit stderr from exec, else normalized `context`.
pub fn stderr_for_pattern_lookup(stderr_field: Option<&str>, context: &str) -> String {
    let raw = stderr_field
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(context);
    let normalized = normalize_stderr(raw);
    truncate_utf8(&normalized, STDERR_PATTERN_MAX_BYTES)
}

pub struct ResolvedSuggestion {
    pub suggestion: Suggestion,
    /// True if `find_pattern` returned a row for this command and stderr probe (any fix / rate).
    pub matched_pattern_row: bool,
}

/// Prefer a high-confidence learned fix when one matches; otherwise delegate to `suggest`.
pub async fn resolve_suggestion(
    store: &Store,
    code: RecoveryCode,
    original_command: &str,
    context: &str,
    stderr_field: Option<&str>,
    detector: &Detector,
) -> anyhow::Result<ResolvedSuggestion> {
    if code == RecoveryCode::R10 {
        return Ok(ResolvedSuggestion {
            suggestion: suggest::suggest(code, original_command, context, detector),
            matched_pattern_row: false,
        });
    }

    let normalized_command = normalize_command(original_command);
    let probe = stderr_for_pattern_lookup(stderr_field, context);

    let matched_row = if !probe.is_empty() {
        store.find_pattern(&normalized_command, &probe).await?
    } else {
        None
    };

    let suggestion = if let Some(pattern) = matched_row
        .as_ref()
        .filter(|pattern| pattern.fix_command.is_some() && pattern.fix_success_rate >= 0.5)
    {
        pattern_to_suggestion(pattern, original_command)
    } else {
        suggest::suggest(code, original_command, context, detector)
    };

    Ok(ResolvedSuggestion {
        suggestion,
        matched_pattern_row: matched_row.is_some(),
    })
}
