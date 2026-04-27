//! Shell command structure analysis for security.
//!
//! Detects patterns that could bypass simple regex-based allow/block lists:
//!   - Subshell expansion: $(...), backticks
//!   - Variable indirection: $x, ${cmd}, $((...))
//!   - Eval/exec/source (which re-interpret strings as commands)
//!   - Heredocs piped to shells
//!   - Process substitution: <(...), >(...)
//!
//! Commands are classified by "origin" which determines how the validator
//! handles them.

use std::fmt;

/// Classification of a command's complexity/origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOrigin {
    /// Simple command with no expansion, eval, or shell metaprogramming.
    Simple,
    /// Contains variable/command expansion ($(cmd), backticks, $var).
    WithExpansions(u8), // count of expansion sites
    /// Contains eval, exec, source, or `.` builtins.
    WithEval,
    /// Multiple layers of obfuscation (expansions + eval, or deeply nested).
    Obfuscated,
}

impl fmt::Display for CommandOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandOrigin::Simple => write!(f, "simple"),
            CommandOrigin::WithExpansions(n) => write!(f, "with_expansions({})", n),
            CommandOrigin::WithEval => write!(f, "with_eval"),
            CommandOrigin::Obfuscated => write!(f, "obfuscated"),
        }
    }
}

/// Result of command structure analysis.
#[derive(Debug)]
pub struct AnalysisResult {
    pub origin: CommandOrigin,
    /// Specific warnings for the caller.
    pub warnings: Vec<String>,
    /// Whether execution should be blocked outright.
    pub blocked: bool,
}

/// Analyze a command's structure for security-relevant patterns.
pub fn analyze(command: &str) -> AnalysisResult {
    let mut warnings = Vec::new();
    let mut origin = CommandOrigin::Simple;

    // ── Subshell expansion: $(...) and backticks ─────────────────
    let subshell_count = count_subshells(command);
    if subshell_count > 0 {
        warnings.push(format!(
            "Command contains {} subshell expansion(s) ($(...) or backticks). \
             These can execute arbitrary commands and bypass static validation.",
            subshell_count
        ));
        origin = CommandOrigin::WithExpansions(subshell_count as u8);
    }

    // ── Variable expansion that could mask commands ─────────────
    let var_indirect = detect_variable_indirection(command);
    if var_indirect {
        warnings.push(
            "Command contains variable indirection (${...}, $x) which could mask \
             dangerous commands.".to_string(),
        );
        match origin {
            CommandOrigin::Simple => origin = CommandOrigin::WithExpansions(1),
            CommandOrigin::WithExpansions(n) => origin = CommandOrigin::WithExpansions(n + 1),
            _ => {}
        }
    }

    // ── Process substitution: <(...) and >(...) ─────────────────
    let proc_sub_count = count_process_substitutions(command);
    if proc_sub_count > 0 {
        warnings.push(format!(
            "Command contains {} process substitution(s) (<(...) or >(...)). \
             These create anonymous pipes/files.",
            proc_sub_count
        ));
        match origin {
            CommandOrigin::Simple => origin = CommandOrigin::WithExpansions(proc_sub_count as u8),
            CommandOrigin::WithExpansions(n) => {
                origin = CommandOrigin::WithExpansions(n + proc_sub_count as u8)
            }
            _ => {}
        }
    }

    // ── Eval / exec / source / . detection ──────────────────────
    let has_eval = detect_eval_exec(command);
    if has_eval {
        warnings.push(
            "Command contains eval, exec, source, or '.' builtin which \
             re-interprets strings as shell code. This can bypass static analysis."
                .to_string(),
        );
        if matches!(origin, CommandOrigin::WithExpansions(_) | CommandOrigin::WithEval) {
            origin = CommandOrigin::Obfuscated;
        } else {
            origin = CommandOrigin::WithEval;
        }
    }

    // ── Heredoc to shell ────────────────────────────────────────
    let heredoc_shell = detect_heredoc_to_shell(command);
    if heredoc_shell {
        warnings.push(
            "Command pipes a heredoc or here-string to a shell interpreter. \
             This is a common bypass technique."
                .to_string(),
        );
        if matches!(origin, CommandOrigin::WithExpansions(_) | CommandOrigin::WithEval) {
            origin = CommandOrigin::Obfuscated;
        } else {
            origin = CommandOrigin::WithEval;
        }
    }

    // ── Deeply nested expansions ────────────────────────────────
    let total_expansions = match origin {
        CommandOrigin::WithExpansions(n) => n,
        _ => 0,
    };
    if total_expansions > 2 {
        origin = CommandOrigin::Obfuscated;
        warnings.push(
            "Command contains deeply nested expansions (>2 levels). \
             This is highly suspicious for shell-based attacks."
                .to_string(),
        );
    }

    // ── Blocking decision ───────────────────────────────────────
    let blocked = matches!(origin, CommandOrigin::Obfuscated);

    AnalysisResult {
        origin,
        warnings,
        blocked,
    }
}

/// Count $(...) subshell expansions (not nested — counts top-level only).
fn count_subshells(command: &str) -> usize {
    let chars: Vec<char> = command.chars().collect();
    let mut count = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
            count += 1;
            i += 2;
            continue;
        }
        // Backtick (crude: just count opening backticks)
        if chars[i] == '`' {
            count += 1;
        }
        i += 1;
    }
    count
}

/// Detect variable indirection patterns like ${!var}, ${cmd}, $var used
/// in ways that could mask commands.
fn detect_variable_indirection(command: &str) -> bool {
    // $x where x could be a command (positional: $1 or named: $eval)
    // This is inherently contextual, so we flag any standalone $var
    // that isn't a simple positional or commonly safe variable.
    let bytes = command.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'(' | b'{' => {
                    // ${...} that isn't a known-safe pattern
                    return true;
                }
                b'0'..=b'9' | b'?' | b'$' | b'!' | b'#' | b'@' | b'*' | b'-' => {
                    // Positional params and special vars: generally safe
                    i += 2;
                    continue;
                }
                _ if bytes[i + 1].is_ascii_alphabetic() => {
                    // $VAR — could be anything. Check if it's followed by
                    // shell-operators suggesting it's being used as a command.
                    // We need more context: look ahead for command-like usage.
                    // For now, flag if the command starts with $ or if $var
                    // appears at the beginning.
                    if i == 0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    false
}

/// Count <(...) and >(...) process substitutions.
fn count_process_substitutions(command: &str) -> usize {
    let chars: Vec<char> = command.chars().collect();
    let mut count = 0;
    let mut i = 0;
    while i + 2 < chars.len() {
        if (chars[i] == '<' || chars[i] == '>') && chars[i + 1] == '(' {
            count += 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    count
}

/// Detect eval, exec, source, or `.` at command position.
fn detect_eval_exec(command: &str) -> bool {
    // Check if the command starts with or pipes to eval/exec/source
    let trimmed = command.trim();

    // Direct invocation: "eval ...", "exec ..."
    if trimmed.starts_with("eval ") || trimmed.starts_with("exec ") {
        return true;
    }

    // "source file" or ". file" (note: "." alone is tricky — it's also a file path)
    // Only match "source " or ". " at command start (after pipe/semicolon)
    let parts: Vec<&str> = command.split(|c| c == '|' || c == ';' || c == '&').collect();
    for part in parts {
        let p = part.trim();
        if p.starts_with("source ") {
            return true;
        }
        // ". " as command: careful not to match "./file" or "../file"
        if p.starts_with(". ") && !p.starts_with("./") && !p.starts_with("../") {
            return true;
        }
    }

    false
}

/// Detect heredoc/here-string piped to a shell interpreter.
fn detect_heredoc_to_shell(command: &str) -> bool {
    // Pattern: "sh <<EOF", "bash <<< '...'", "zsh << 'HEREDOC'"
    let shell_patterns = ["sh", "bash", "zsh", "dash", "fish", "ksh"];

    for shell in &shell_patterns {
        for heredoc_marker in &["<<", "<<<", "<<-"] {
            let search = format!("{} {}", shell, heredoc_marker);
            if command.contains(&search) {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_simple_command() {
        let result = analyze("ls -la");
        assert_eq!(result.origin, CommandOrigin::Simple);
        assert!(!result.blocked);
    }

    #[test]
    fn detects_subshell_expansion() {
        let result = analyze("echo $(whoami)");
        assert!(matches!(result.origin, CommandOrigin::WithExpansions(_)));
        assert!(!result.blocked);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn detects_backtick_expansion() {
        let result = analyze("echo `whoami`");
        assert!(matches!(result.origin, CommandOrigin::WithExpansions(_)));
    }

    #[test]
    fn detects_eval() {
        let result = analyze("eval rm -rf /");
        assert!(matches!(result.origin, CommandOrigin::WithEval));
        assert!(!result.blocked); // eval alone isn't blocked, but it's warned
    }

    #[test]
    fn detects_source() {
        let result = analyze("source /etc/malicious.sh");
        assert!(matches!(result.origin, CommandOrigin::WithEval));
    }

    #[test]
    fn detects_dot_command() {
        let result = analyze(". /tmp/script.sh");
        assert!(matches!(result.origin, CommandOrigin::WithEval));
    }

    #[test]
    fn dot_file_path_not_detected_as_source() {
        // "./script.sh" should NOT be detected as source
        let result = analyze("./script.sh");
        assert_eq!(result.origin, CommandOrigin::Simple);
    }

    #[test]
    fn detects_heredoc_to_shell() {
        let result = analyze("sh <<EOF\nrm -rf /\nEOF");
        assert!(matches!(result.origin, CommandOrigin::WithEval));
    }

    #[test]
    fn detects_herestring_to_shell() {
        let result = analyze("bash <<< 'rm -rf /'");
        assert!(matches!(result.origin, CommandOrigin::WithEval));
    }

    #[test]
    fn detects_process_substitution() {
        let result = analyze("diff <(ls) <(ls /tmp)");
        assert!(matches!(result.origin, CommandOrigin::WithExpansions(_)));
    }

    #[test]
    fn blocks_obfuscated_commands() {
        // Subshell + eval = obfuscated
        let result = analyze("eval $(echo rm -rf /)");
        assert_eq!(result.origin, CommandOrigin::Obfuscated);
        assert!(result.blocked);
    }

    #[test]
    fn blocks_deeply_nested_expansions() {
        let result = analyze("$(echo $(echo $(whoami)))");
        assert_eq!(result.origin, CommandOrigin::Obfuscated);
        assert!(result.blocked);
    }

    #[test]
    fn simple_pipe_not_blocked() {
        let result = analyze("ls | grep foo");
        assert_eq!(result.origin, CommandOrigin::Simple);
        assert!(!result.blocked);
    }

    #[test]
    fn var_usage_in_arguments_not_blocked() {
        // $VAR in arguments is normal shell behavior, not dangerous
        let result = analyze("ls $HOME");
        assert_eq!(result.origin, CommandOrigin::Simple);
        assert!(!result.blocked);
    }
}
