use once_cell::sync::Lazy;
use regex::Regex;
use super::taxonomy::RecoveryCode;

pub struct Pattern {
    pub code: RecoveryCode,
    pub exit_codes: Vec<i32>,
    pub stderr_regexes: Vec<Regex>,
}

/// Use this sentinel exit code to indicate "match regardless of exit code."
/// Only use for patterns whose stderr regex is specific enough to avoid
/// false positives (e.g., R25 environment mismatch heuristics).
pub const EXIT_CODE_ANY: i32 = -1;

pub static PATTERNS: Lazy<Vec<Pattern>> = Lazy::new(|| {
    vec![
        // ── R20: Syntax Error ──────────────────────────────────────────────
        Pattern {
            code: RecoveryCode::R20,
            exit_codes: vec![2, 1, 127],
            stderr_regexes: vec![
                // Generic / cross-shell
                Regex::new(r"invalid option").unwrap(),
                Regex::new(r"usage:").unwrap(),
                Regex::new(r"unrecognized argument").unwrap(),
                Regex::new(r"error: unexpected argument").unwrap(),
                Regex::new(r"bad option").unwrap(),
                Regex::new(r"unknown flag").unwrap(),
                Regex::new(r"syntax error").unwrap(),
                Regex::new(r"parse error").unwrap(),
                // fish
                Regex::new(r"Expected a command name").unwrap(),
                Regex::new(r"fish:.*parse error").unwrap(),
                // PowerShell
                Regex::new(r"is not recognized as the name of a cmdlet").unwrap(),
                Regex::new(r"is not recognized as an internal or external command").unwrap(),
            ],
        },
        // ── R21: Permission Denied ─────────────────────────────────────────
        Pattern {
            code: RecoveryCode::R21,
            exit_codes: vec![126, 128, 1],
            stderr_regexes: vec![
                Regex::new(r"Permission denied").unwrap(),
                Regex::new(r"Operation not permitted").unwrap(),
                Regex::new(r"EACCES").unwrap(),
                Regex::new(r"cannot open.*Permission denied").unwrap(),
                Regex::new(r"cannot execute").unwrap(),
                Regex::new(r"not executable").unwrap(),
            ],
        },
        // ── R24: Subcommand Failure ───────────────────────────────────────
        // IMPORTANT: must be checked BEFORE R22 for exit_code=1.
        Pattern {
            code: RecoveryCode::R24,
            exit_codes: vec![1],
            stderr_regexes: vec![
                // Node.js / npm
                Regex::new(r"npm ERR!").unwrap(),
                Regex::new(r"node:.*error").unwrap(),
                // Rust / Cargo
                Regex::new(r"error\[.*\]").unwrap(),
                Regex::new(r"error:.*compilation").unwrap(),
                Regex::new(r"could not compile").unwrap(),
                // Make
                Regex::new(r"make: \*\*\*").unwrap(),
                // Python
                Regex::new(r"pytest failed").unwrap(),
                Regex::new(r"TypeError:").unwrap(),
                Regex::new(r"ImportError:").unwrap(),
                Regex::new(r"ModuleNotFoundError:").unwrap(),
                Regex::new(r"SyntaxError:").unwrap(),
                Regex::new(r"pip failed with error").unwrap(),
                // Docker
                Regex::new(r"docker: Error response from daemon").unwrap(),
                // K8s
                Regex::new(r"kubectl: error:").unwrap(),
                // Go
                Regex::new(r"go: .*: .*: .*:").unwrap(), // go build multi-path errors
                Regex::new(r"panic:").unwrap(),
                // Generic
                Regex::new(r"FAILED").unwrap(),
                Regex::new(r"FAIL").unwrap(),
            ],
        },
        // ── R22: Command Not Found ────────────────────────────────────────
        // Checked AFTER R24 for exit_code=1 to avoid false positives
        // on subcommand failures that contain "ENOENT"-like text.
        Pattern {
            code: RecoveryCode::R22,
            exit_codes: vec![127, 1],
            stderr_regexes: vec![
                // All shells normalize to "command not found" via normalize_stderr
                Regex::new(r"command not found").unwrap(),
                // Raw fallbacks (before normalization strips prefixes)
                Regex::new(r": not found$").unwrap(),
                Regex::new(r"No such file or directory").unwrap(),
                // PowerShell (after normalization becomes "command not found: X")
                Regex::new(r"is not recognized as (?:the name of )?a(?:n?)\s*(?:cmdlet|command)").unwrap(),
                Regex::new(r"is not recognized as an internal or external command").unwrap(),
                // fish (after normalization)
                Regex::new(r"Unknown command").unwrap(),
            ],
        },
        // ── R25: Environment Mismatch ──────────────────────────────────────
        // These use EXIT_CODE_ANY because they are specific enough to
        // only match genuine shell/environment issues.
        Pattern {
            code: RecoveryCode::R25,
            exit_codes: vec![EXIT_CODE_ANY],
            stderr_regexes: vec![
                // Bashisms in non-bash shells
                Regex::new(r"bad substitution").unwrap(),
                Regex::new(r"syntax error near unexpected token").unwrap(),
                // Zsh-specific errors
                Regex::new(r"no matches found").unwrap(),
                Regex::new(r"no such word in event").unwrap(),
                Regex::new(r"command not found.*did you mean").unwrap(),
                Regex::new(r"zsh:.*not found").unwrap(),
                // Fish-specific errors
                Regex::new(r"fish:.*Unknown command").unwrap(),
                // Bash array/associative array errors in POSIX shells
                Regex::new(r"unexpected token.*\(\)").unwrap(),
            ],
        },
    ]
});
