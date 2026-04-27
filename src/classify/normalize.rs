//! Cross-platform stderr normalization for consistent classification.
//!
//! Shells produce different error formats for the same underlying failure:
//!   bash: "bash: gh: command not found"
//!   zsh:  "zsh: command not found: gh"
//!   fish: "fish: Unknown command: gh"
//!   dash: "dash: 1: gh: not found"
//!
//! This module canonicalizes all variants so the classifier only needs
//! to match against one common form.

use once_cell::sync::Lazy;
use regex::Regex;

// ── ANSI escape stripping ──────────────────────────────────────────

/// Strips ANSI CSI (Control Sequence Introducer) and OSC sequences.
/// CSI: `\x1b[` + params + letter  (colors, cursor movements)
/// OSC: `\x1b]` + content + `\x07`  (terminal title, etc.)
static ANSI_ESCAPE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?(\x07|\x1b\\)").unwrap()
});

fn strip_ansi_escapes(text: &str) -> String {
    ANSI_ESCAPE_RE.replace_all(text, "").to_string()
}

// ── Shell prefix stripping ─────────────────────────────────────────

/// Shells prefix error lines with their name. Remove these so the
/// underlying error message can be matched generically.
static SHELL_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:bash|zsh|sh|dash|fish|pwsh|powershell):\s*(?:\d+:\s*)?").unwrap()
});

fn strip_shell_prefix(text: &str) -> String {
    SHELL_PREFIX_RE.replace_all(text, "").to_string()
}

// ── Canonical phrasing ─────────────────────────────────────────────

/// Map shell-specific phrases to canonical forms that the classifier
/// regexes can match uniformly.
static CANONICALIZE_REPLACEMENTS: Lazy<Vec<(Regex, &str)>> = Lazy::new(|| {
    vec![
        // zsh: "command not found:" → canonical "command not found"
        (Regex::new(r"(?i)zsh:\s*command not found:\s*").unwrap(), "command not found: "),
        // fish: "Unknown command" / "Unknown command:" → "command not found"
        (Regex::new(r"(?i)fish:\s*Unknown command:?\s*").unwrap(), "command not found: "),
        // dash: "X: not found" → "X: command not found"
        (Regex::new(r"(?i)dash:\s*\d+:\s*(\S+):\s*not found").unwrap(), "${1}: command not found"),
        // pwsh: "The term 'X' is not recognized as ... a cmdlet/command"
        (Regex::new(r"(?i)The term '([^']+)' is not recognized as (?:the name of )?a(?:n?)\s*(?:cmdlet|command)").unwrap(), "command not found: ${1}"),
        // pwsh: "is not recognized as an internal or external command"
        (Regex::new(r#"(?i)('[^']+')\s*is not recognized as an internal or external command"#).unwrap(), "command not found: ${1}"),
        // fish: "Permission denied" → keep as-is but strip fish prefix
        (Regex::new(r"(?i)fish:\s*Permission denied:?\s*").unwrap(), "Permission denied: "),
        // zsh: "permission denied:" → "Permission denied"
        (Regex::new(r"(?i)zsh:\s*permission denied:?\s*").unwrap(), "Permission denied: "),
        // fish: "Expected a command name" / "parse error" → syntax error
        (Regex::new(r"(?i)fish:\s*(?:Expected a command name|parse error)").unwrap(), "syntax error"),
        // dash: "Syntax error:" → "syntax error"
        (Regex::new(r"(?i)dash:\s*\d+:\s*Syntax error:").unwrap(), "syntax error:"),
        // zsh: "parse error near" → "syntax error near"
        (Regex::new(r"(?i)zsh:\s*parse error near").unwrap(), "syntax error near"),
    ]
});

fn canonicalize_phrasing(text: &str) -> String {
    let mut result = text.to_string();
    for (re, replacement) in CANONICALIZE_REPLACEMENTS.iter() {
        result = re.replace_all(&result, *replacement).to_string();
    }
    result
}

// ── Path normalization ─────────────────────────────────────────────

/// Normalize WSL paths (e.g., /mnt/c/Users/...) to standard form.
/// Also remove duplicate slashes (but not in protocol:// prefixes).
fn normalize_paths(text: &str) -> String {
    // Use a simple iterative approach to avoid look-behind (not supported in regex crate).
    // Replace all "://" with a placeholder, squash slashes, restore the placeholder.
    let placeholder = "\x00PROTOCOL\x00";
    let text = text.replace("://", placeholder);
    let text = squash_slashes(&text);
    text.replace(placeholder, "://")
}

/// Replace runs of 2+ slashes with a single slash.
fn squash_slashes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_slash = false;
    for c in text.chars() {
        if c == '/' {
            if !prev_slash {
                result.push('/');
            }
            prev_slash = true;
        } else {
            prev_slash = false;
            result.push(c);
        }
    }
    result
}

// ── Public API ─────────────────────────────────────────────────────

/// Compile-once regex for whitespace collapsing.
static WHITESPACE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

/// Normalize stderr output from any shell to a canonical form suitable
/// for classification pattern matching.
///
/// Pipeline:
///   1. Strip ANSI escape codes (colors, cursor moves)
///   2. Canonicalize phrasing (different shells → same error)
///   3. Strip shell name prefixes (bash:, zsh:, etc.)
///   4. Normalize paths (WSL, duplicate slashes)
///   5. Collapse whitespace and trim
pub fn normalize_stderr(stderr: &str) -> String {
    let mut text = strip_ansi_escapes(stderr);
    text = canonicalize_phrasing(&text);
    text = strip_shell_prefix(&text);
    text = normalize_paths(&text);

    // Collapse whitespace and trim
    let text = WHITESPACE_RE.replace_all(&text, " ").to_string();
    text.trim().to_string()
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ANSI stripping ─────────────────────────────────────────

    #[test]
    fn strips_ansi_color_codes() {
        let input = "\x1b[31merror\x1b[0m: something failed";
        let result = strip_ansi_escapes(input);
        assert_eq!(result, "error: something failed");
    }

    #[test]
    fn strips_ansi_bold_and_reset() {
        let input = "\x1b[1mWarning\x1b[0m: path not found";
        let result = strip_ansi_escapes(input);
        assert_eq!(result, "Warning: path not found");
    }

    #[test]
    fn preserves_text_without_ansi() {
        let input = "command not found";
        let result = strip_ansi_escapes(input);
        assert_eq!(result, "command not found");
    }

    // ── Shell prefix stripping ────────────────────────────────

    #[test]
    fn strips_bash_prefix() {
        let input = "bash: gh: command not found";
        let result = strip_shell_prefix(input);
        assert_eq!(result, "gh: command not found");
    }

    #[test]
    fn strips_zsh_prefix() {
        let input = "zsh: command not found: gh";
        let result = strip_shell_prefix(input);
        assert_eq!(result, "command not found: gh");
    }

    #[test]
    fn strips_dash_prefix_with_line_number() {
        let input = "dash: 1: gh: not found";
        let result = strip_shell_prefix(input);
        assert_eq!(result, "gh: not found");
    }

    #[test]
    fn strips_fish_prefix() {
        let input = "fish: Unknown command: gh";
        let result = strip_shell_prefix(input);
        assert_eq!(result, "Unknown command: gh");
    }

    // ── Canonical phrasing ────────────────────────────────────

    #[test]
    fn canonicalizes_zsh_command_not_found() {
        let input = "zsh: command not found: mytool";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("command not found"));
    }

    #[test]
    fn canonicalizes_fish_unknown_command() {
        let input = "fish: Unknown command: mytool";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("command not found"));
    }

    #[test]
    fn canonicalizes_dash_not_found() {
        let input = "dash: 1: mytool: not found";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("command not found"));
    }

    #[test]
    fn canonicalizes_pwsh_not_recognized() {
        let input = "The term 'mytool' is not recognized as the name of a cmdlet, function, script file, or operable program.";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("command not found"));
    }

    #[test]
    fn canonicalizes_zsh_permission_denied() {
        let input = "zsh: permission denied: ./script.sh";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("Permission denied"));
    }

    #[test]
    fn canonicalizes_fish_parse_error() {
        let input = "fish: Expected a command name, got token '}'";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("syntax error"));
    }

    #[test]
    fn canonicalizes_zsh_parse_error() {
        let input = "zsh: parse error near `}'";
        let result = canonicalize_phrasing(input);
        assert!(result.contains("syntax error"));
    }

    // ── Path normalization ────────────────────────────────────

    #[test]
    fn normalizes_double_slashes() {
        let input = "cannot open //var//log//file: Permission denied";
        let result = normalize_paths(input);
        assert_eq!(result, "cannot open /var/log/file: Permission denied");
    }

    #[test]
    fn preserves_protocol_slashes() {
        let input = "Download from https://example.com failed";
        let result = normalize_paths(input);
        assert_eq!(result, "Download from https://example.com failed");
    }

    // ── Full normalization pipeline ───────────────────────────

    #[test]
    fn normalizes_bash_command_not_found() {
        let input = "bash: gh: command not found";
        let result = normalize_stderr(input);
        assert_eq!(result, "gh: command not found");
    }

    #[test]
    fn normalizes_zsh_command_not_found() {
        let input = "zsh: command not found: gh";
        let result = normalize_stderr(input);
        assert_eq!(result, "command not found: gh");
    }

    #[test]
    fn normalizes_fish_unknown_command() {
        let input = "fish: Unknown command: gh";
        let result = normalize_stderr(input);
        assert_eq!(result, "command not found: gh");
    }

    #[test]
    fn normalizes_dash_not_found() {
        let input = "dash: 1: gh: not found";
        let result = normalize_stderr(input);
        let expected = "gh: command not found";
        assert_eq!(result, expected);
    }

    #[test]
    fn normalizes_pwsh_not_recognized() {
        let input = "The term 'gh' is not recognized as the name of a cmdlet, function, script file, or operable program.";
        let result = normalize_stderr(input);
        assert!(result.contains("command not found"), "got: {result}");
    }

    #[test]
    fn normalizes_ansi_colored_error() {
        let input = "\x1b[1;31mbash:\x1b[0m \x1b[31mgh:\x1b[0m command not found";
        let result = normalize_stderr(input);
        assert_eq!(result, "gh: command not found");
    }

    #[test]
    fn normalizes_zsh_permission_denied() {
        let input = "zsh: permission denied: /usr/local/bin/tool";
        let result = normalize_stderr(input);
        assert_eq!(result, "Permission denied: /usr/local/bin/tool");
    }

    #[test]
    fn preserves_plain_error_without_shell_prefix() {
        let input = "error: could not compile `my_crate`";
        let result = normalize_stderr(input);
        assert_eq!(result, "error: could not compile `my_crate`");
    }
}
