pub mod config;
pub mod normalize;
pub mod patterns;
pub mod taxonomy;

use once_cell::sync::Lazy;
use patterns::Pattern;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use taxonomy::RecoveryCode;

type MergedPatternIndex = (Vec<Pattern>, HashMap<i32, Vec<usize>>);

/// Merged patterns (user overrides + built-in) with pre-built index.
static MERGED_PATTERNS: Lazy<MergedPatternIndex> = Lazy::new(|| {
    use patterns::PATTERNS;
    let merged = config::merged_patterns(&PATTERNS);
    let mut index: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, p) in merged.iter().enumerate() {
        for &code in &p.exit_codes {
            index.entry(code).or_default().push(i);
        }
    }
    (merged, index)
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    pub code: RecoveryCode,
    pub reason: String,
}

/// Classify a command failure using its exit code, stderr, and shell context.
///
/// The `stderr` passed here should already be normalized via
/// `normalize::normalize_stderr()` for best cross-shell matching.
pub fn classify(
    exit_code: i32,
    stderr: &str,
    stdout: &str,
    timed_out: bool,
    detected_shell: &str,
    config: Option<&crate::config::ReshellConfig>,
) -> ClassificationResult {
    if timed_out {
        return ClassificationResult {
            code: RecoveryCode::R23,
            reason: "Process exceeded time limit".to_string(),
        };
    }

    if exit_code == 0 {
        return ClassificationResult {
            code: RecoveryCode::R10,
            reason: "Success".to_string(),
        };
    }

    // Primary: exit-code-indexed pattern matching on (now normalized) stderr
    if let Some(indices) = MERGED_PATTERNS.1.get(&exit_code) {
        for &idx in indices {
            let pattern = &MERGED_PATTERNS.0[idx];
            for re in &pattern.stderr_regexes {
                if re.is_match(stderr) {
                    return ClassificationResult {
                        code: pattern.code,
                        reason: format!("Matched stderr pattern for {:?}", pattern.code),
                    };
                }
            }
        }
    }

    // Secondary: shell-agnostic pattern matching (any exit code)
    if let Some(indices) = MERGED_PATTERNS.1.get(&-1) {
        for &idx in indices {
            let pattern = &MERGED_PATTERNS.0[idx];
            for re in &pattern.stderr_regexes {
                if re.is_match(stderr) {
                    return ClassificationResult {
                        code: pattern.code,
                        reason: format!("Matched shell-agnostic pattern for {:?}", pattern.code),
                    };
                }
            }
        }
    }

    // Fallback heuristics for environment mismatch (R25)
    // Check raw stderr for shell-specific markers that survived normalization
    if stderr.contains("bash:") && detected_shell.contains("zsh") {
        return ClassificationResult {
            code: RecoveryCode::R25,
            reason: "Possible bashism running in Zsh".to_string(),
        };
    }

    if stderr.contains("zsh:") && detected_shell.contains("bash") {
        return ClassificationResult {
            code: RecoveryCode::R25,
            reason: "Possible zsh-ism running in Bash".to_string(),
        };
    }

    // Additional R25 heuristics for cross-shell mismatch
    if stderr.contains("bad substitution") {
        return ClassificationResult {
            code: RecoveryCode::R25,
            reason: "Shell substitution error (possible environment mismatch)".to_string(),
        };
    }

    if stderr.contains("no matches found") || stderr.contains("no such word in event") {
        return ClassificationResult {
            code: RecoveryCode::R25,
            reason: "Shell-specific globbing or history expansion error".to_string(),
        };
    }

    // Large stdout with no clearer stderr signal: suggest scoping output (R26).
    let large_stdout = if let Some(cfg) = config {
        cfg.compaction.large_stdout_bytes
    } else {
        crate::config::get().compaction.large_stdout_bytes
    };
    if exit_code != 0 && stdout.len() > large_stdout {
        return ClassificationResult {
            code: RecoveryCode::R26,
            reason: "Very large stdout on failure; try head/tail/grep to scope output".to_string(),
        };
    }

    ClassificationResult {
        code: RecoveryCode::R30,
        reason: "Non-matching failure, requires escalation".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success() {
        let r = classify(0, "", "", false, "", None);
        assert_eq!(r.code, RecoveryCode::R10);
    }

    #[test]
    fn test_command_not_found() {
        let r = classify(127, "gh: command not found", "", false, "", None);
        assert_eq!(r.code, RecoveryCode::R22);
    }

    #[test]
    fn test_syntax_error() {
        let r = classify(2, "invalid option -- 'z'", "", false, "", None);
        assert_eq!(r.code, RecoveryCode::R20);
    }

    #[test]
    fn test_permission_denied() {
        let r = classify(126, "Permission denied", "", false, "", None);
        assert_eq!(r.code, RecoveryCode::R21);
    }

    #[test]
    fn test_subcommand_failure() {
        let r = classify(1, "npm ERR! code ENOENT", "", false, "", None);
        assert_eq!(r.code, RecoveryCode::R24);
    }

    #[test]
    fn test_timeout() {
        let r = classify(124, "", "", true, "", None);
        assert_eq!(r.code, RecoveryCode::R23);
    }

    #[test]
    fn test_large_stdout_hints_r26() {
        // Use test defaults (small large_stdout_bytes = 1024 so test is fast)
        let cfg = crate::config::ReshellConfig::test_defaults();
        let big = "x".repeat(2000);
        let r = classify(1, "", &big, false, "", Some(&cfg));
        assert_eq!(r.code, RecoveryCode::R26);
    }
}
