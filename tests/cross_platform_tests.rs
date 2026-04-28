//! Cross-platform stderr normalization and classification tests.
//!
//! Verifies that error messages from bash, zsh, fish, dash, and
//! PowerShell are all normalized to canonical forms that the
//! classifier correctly identifies.

use reshell::classify::{classify, normalize::normalize_stderr, taxonomy::RecoveryCode};

/// Each test case: (raw_stderr, exit_code, expected_recovery_code, expected_normalized_contains)
struct CrossShellCase {
    raw: &'static str,
    exit_code: i32,
    expected_code: RecoveryCode,
    normalized_contains: &'static str,
}

fn cross_shell_cases() -> Vec<CrossShellCase> {
    vec![
        // ── R22: Command Not Found ────────────────────────────
        CrossShellCase {
            raw: "bash: gh: command not found",
            exit_code: 127,
            expected_code: RecoveryCode::R22,
            normalized_contains: "command not found",
        },
        CrossShellCase {
            raw: "zsh: command not found: gh",
            exit_code: 127,
            expected_code: RecoveryCode::R22,
            normalized_contains: "command not found",
        },
        CrossShellCase {
            raw: "fish: Unknown command: gh",
            exit_code: 127,
            expected_code: RecoveryCode::R22,
            normalized_contains: "command not found",
        },
        CrossShellCase {
            raw: "dash: 1: gh: not found",
            exit_code: 127,
            expected_code: RecoveryCode::R22,
            normalized_contains: "command not found",
        },
        CrossShellCase {
            raw: "sh: 1: nonexistent_cmd: not found",
            exit_code: 127,
            expected_code: RecoveryCode::R22,
            normalized_contains: "not found",
        },
        // pwsh: unrecognized command
        CrossShellCase {
            raw: "The term 'mytool' is not recognized as the name of a cmdlet, function, script file, or operable program.",
            exit_code: 1,
            expected_code: RecoveryCode::R22,
            normalized_contains: "command not found",
        },
        // ── R21: Permission Denied ────────────────────────────
        CrossShellCase {
            raw: "bash: /usr/local/bin/tool: Permission denied",
            exit_code: 126,
            expected_code: RecoveryCode::R21,
            normalized_contains: "Permission denied",
        },
        CrossShellCase {
            raw: "zsh: permission denied: ./script.sh",
            exit_code: 126,
            expected_code: RecoveryCode::R21,
            normalized_contains: "Permission denied",
        },
        CrossShellCase {
            raw: "fish: Permission denied: ./script.sh",
            exit_code: 126,
            expected_code: RecoveryCode::R21,
            normalized_contains: "Permission denied",
        },
        CrossShellCase {
            raw: "cannot execute binary file",
            exit_code: 126,
            expected_code: RecoveryCode::R21,
            normalized_contains: "cannot execute",
        },
        // ── R20: Syntax Error ─────────────────────────────────
        CrossShellCase {
            raw: "invalid option -- 'z'",
            exit_code: 2,
            expected_code: RecoveryCode::R20,
            normalized_contains: "invalid option",
        },
        CrossShellCase {
            raw: "zsh: parse error near `}'",
            exit_code: 1,
            expected_code: RecoveryCode::R20,
            normalized_contains: "syntax error",
        },
        CrossShellCase {
            raw: "dash: 1: Syntax error: \"(\" unexpected",
            exit_code: 2,
            expected_code: RecoveryCode::R20,
            normalized_contains: "syntax error",
        },
        CrossShellCase {
            raw: "fish: Expected a command name, got token '}'",
            exit_code: 127,
            expected_code: RecoveryCode::R20,
            normalized_contains: "syntax error",
        },
        // pwsh: "not recognized" → R22 (command not found equivalent)
        CrossShellCase {
            raw: "The term 'bad-cmd' is not recognized as the name of a cmdlet",
            exit_code: 1,
            expected_code: RecoveryCode::R22,
            normalized_contains: "command not found",
        },
        // ── R24: Subcommand Failure ───────────────────────────
        CrossShellCase {
            raw: "npm ERR! code ENOENT",
            exit_code: 1,
            expected_code: RecoveryCode::R24,
            normalized_contains: "npm ERR!",
        },
        CrossShellCase {
            raw: "error[E0432]: unresolved import `foo`",
            exit_code: 1,
            expected_code: RecoveryCode::R24,
            normalized_contains: "error[E0432]",
        },
        CrossShellCase {
            raw: "make: *** [build] Error 2",
            exit_code: 1,
            expected_code: RecoveryCode::R24,
            normalized_contains: "make: ***",
        },
        // ── R25: Environment Mismatch ─────────────────────────
        CrossShellCase {
            raw: "bash: ${!indirect}: bad substitution",
            exit_code: 1,
            expected_code: RecoveryCode::R25,
            normalized_contains: "bad substitution",
        },
        CrossShellCase {
            raw: "zsh: no matches found: *.txt",
            exit_code: 1,
            expected_code: RecoveryCode::R25,
            normalized_contains: "no matches found",
        },
        // ── R10: Success ──────────────────────────────────────
        CrossShellCase {
            raw: "",
            exit_code: 0,
            expected_code: RecoveryCode::R10,
            normalized_contains: "",
        },
        // ── R23: Timeout ──────────────────────────────────────
        CrossShellCase {
            raw: "Process timed out",
            exit_code: 124,
            expected_code: RecoveryCode::R23,
            normalized_contains: "Process timed out",
        },
    ]
}

#[test]
fn test_cross_shell_normalization_produces_expected_substrings() {
    let cases = cross_shell_cases();
    for (i, case) in cases.iter().enumerate() {
        let normalized = normalize_stderr(case.raw);
        if !case.normalized_contains.is_empty() {
            assert!(
                normalized.contains(case.normalized_contains),
                "Case {} ({}):\n  raw: `{}`\n  normalized: `{}`\n  expected to contain: `{}`",
                i + 1,
                case.raw,
                case.raw,
                normalized,
                case.normalized_contains,
            );
        }
    }
}

#[test]
fn test_cross_shell_classification_matches_expected_codes() {
    let cases = cross_shell_cases();
    for (i, case) in cases.iter().enumerate() {
        let normalized = normalize_stderr(case.raw);
        let timed_out = case.exit_code == 124;
        let result = classify(case.exit_code, &normalized, "", timed_out, "");
        assert_eq!(
            result.code,
            case.expected_code,
            "Case {}: raw=`{}` exit={} expected={:?} got={:?} (normalized=`{}` reason=`{}`)",
            i + 1,
            case.raw,
            case.exit_code,
            case.expected_code,
            result.code,
            normalized,
            result.reason,
        );
    }
}

#[test]
fn test_ansi_stripped_before_normalization() {
    let input = "\x1b[1;31merror:\x1b[0m something failed";
    let normalized = normalize_stderr(input);
    assert!(
        !normalized.contains("\x1b"),
        "ANSI escape codes should be stripped"
    );
    assert!(normalized.contains("error: something failed"));
}

#[test]
fn test_wsl_path_duplicate_slashes_normalized() {
    let input = "cannot access '//var//log//file': Permission denied";
    let normalized = normalize_stderr(input);
    assert!(
        !normalized.contains("//"),
        "duplicate slashes should be collapsed"
    );
    assert!(normalized.contains("/var/log/file"));
}
