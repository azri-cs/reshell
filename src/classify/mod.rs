pub mod patterns;
pub mod taxonomy;

use patterns::{PATTERNS, PATTERN_INDEX};
use taxonomy::RecoveryCode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    pub code: RecoveryCode,
    pub reason: String,
}

pub fn classify(exit_code: i32, stderr: &str, _stdout: &str, timed_out: bool, detected_shell: &str) -> ClassificationResult {
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

    if let Some(indices) = PATTERN_INDEX.get(&exit_code) {
        for &idx in indices {
            let pattern = &PATTERNS[idx];
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

    // Fallback heuristics
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
        let r = classify(0, "", "", false, "");
        assert_eq!(r.code, RecoveryCode::R10);
    }

    #[test]
    fn test_command_not_found() {
        let r = classify(127, "gh: command not found", "", false, "");
        assert_eq!(r.code, RecoveryCode::R22);
    }

    #[test]
    fn test_syntax_error() {
        let r = classify(2, "invalid option -- 'z'", "", false, "");
        assert_eq!(r.code, RecoveryCode::R20);
    }

    #[test]
    fn test_permission_denied() {
        let r = classify(126, "Permission denied", "", false, "");
        assert_eq!(r.code, RecoveryCode::R21);
    }

    #[test]
    fn test_subcommand_failure() {
        let r = classify(1, "npm ERR! code ENOENT", "", false, "");
        assert_eq!(r.code, RecoveryCode::R24);
    }

    #[test]
    fn test_timeout() {
        let r = classify(124, "", "", true, "");
        assert_eq!(r.code, RecoveryCode::R23);
    }
}
