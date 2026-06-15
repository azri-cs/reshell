//! Command audit and verbose validation mode.
//!
//! Provides detailed explanations of why a command was blocked or allowed,
//! for use with `rsh check --verbose`.

use super::analyze;
use super::validator;

/// An audit result explaining what checks were performed and their outcomes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditResult {
    /// Whether the command was allowed.
    pub allowed: bool,
    /// The reason if blocked.
    pub reason: Option<String>,
    /// Details of each check that was performed.
    pub checks: Vec<AuditCheck>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditCheck {
    /// Name of the check.
    pub name: &'static str,
    /// Whether the command passed this check.
    pub passed: bool,
    /// Explanation of the result.
    pub detail: String,
}

/// Run a verbose validation of a command, returning details of each check.
pub fn audit_command(command: &str) -> AuditResult {
    let mut checks = Vec::new();

    // Check 1: Allowlist mode check
    let is_allowlist = crate::sandbox::allowlist::current_mode()
        == crate::sandbox::allowlist::SandboxMode::Allowlist;
    checks.push(AuditCheck {
        name: "allowlist mode",
        passed: true,
        detail: if is_allowlist {
            "Allowlist mode is active; checking allowlist...".to_string()
        } else {
            "Blocklist mode (default): dangerous commands blocked, everything else allowed."
                .to_string()
        },
    });

    // Check 2: Interactive commands
    let first_word = command.split_whitespace().next().unwrap_or("");
    let interactive = matches!(
        first_word,
        "vim" | "vi" | "nano" | "emacs" | "less" | "more" | "man" | "top" | "htop"
    );
    checks.push(AuditCheck {
        name: "interactive command check",
        passed: !interactive,
        detail: if interactive {
            format!(
                "Command starts with '{}', which is an interactive editor/pager and is blocked.",
                first_word
            )
        } else {
            format!(
                "Command starts with '{}', which is not in the interactive command list.",
                first_word
            )
        },
    });

    // Check 3: Quote balance
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_escape = false;
    for c in command.chars() {
        match c {
            '\\' if !prev_escape => prev_escape = true,
            '"' if !in_single && !prev_escape => in_double = !in_double,
            '\'' if !in_double && !prev_escape => in_single = !in_single,
            _ => prev_escape = false,
        }
    }
    let quotes_balanced = !in_single && !in_double;
    checks.push(AuditCheck {
        name: "quote balance check",
        passed: quotes_balanced,
        detail: if quotes_balanced {
            "All quotes are properly balanced.".to_string()
        } else {
            let mut issues = Vec::new();
            if in_single {
                issues.push("single-quote string unclosed");
            }
            if in_double {
                issues.push("double-quote string unclosed");
            }
            format!("Unmatched quotes: {}", issues.join(", "))
        },
    });

    // Check 4: Interpreter -c flag
    let interpreter_cmds = [
        "python", "python3", "python2", "perl", "ruby", "node", "lua", "php",
    ];
    let is_interpreter = interpreter_cmds.contains(&first_word);
    let has_c_flag = if is_interpreter {
        let remainder = &command[first_word.len()..];
        remainder.trim_start().starts_with("-c")
    } else {
        false
    };
    checks.push(AuditCheck {
        name: "interpreter -c flag check",
        passed: !has_c_flag,
        detail: if has_c_flag {
            format!(
                "'{} -c' can execute arbitrary code passed as a string argument. This is blocked for security reasons.",
                first_word
            )
        } else if is_interpreter {
            format!(
                "'{}' is an interpreter but no -c flag was detected.",
                first_word
            )
        } else {
            format!("'{}' is not in the interpreter list.", first_word)
        },
    });

    // Check 5: Dangerous pattern matches
    let validation_error = validator::validate(command).err();
    let validation_passed = validation_error.is_none();
    checks.push(AuditCheck {
        name: "dangerous pattern check",
        passed: validation_passed,
        detail: if validation_passed {
            "Command passed all validation checks.".to_string()
        } else {
            format!("Command blocked: {}", validation_error.as_ref().unwrap())
        },
    });

    // Check 6: Obfuscation check
    let analysis = analyze::analyze(command);
    checks.push(AuditCheck {
        name: "obfuscation analysis",
        passed: !analysis.blocked,
        detail: if analysis.blocked {
            format!(
                "Command was classified as '{:?}' and blocked. Warnings: {}",
                analysis.origin,
                analysis.warnings.join("; ")
            )
        } else {
            format!(
                "Command classified as '{:?}' — no obfuscation detected.",
                analysis.origin
            )
        },
    });

    let allowed =
        validation_passed && !interactive && quotes_balanced && !has_c_flag && !analysis.blocked;
    let reason = if allowed {
        None
    } else {
        if let Some(e) = validation_error {
            Some(e)
        } else {
            Some(
                "Blocked by interactive command list, quote imbalance, or obfuscation check."
                    .to_string(),
            )
        }
    };

    AuditResult {
        allowed,
        reason,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_allows_simple_command() {
        let result = audit_command("ls -la");
        assert!(result.allowed);
        assert!(result.reason.is_none());
    }

    #[test]
    fn audit_blocks_dangerous_command() {
        let result = audit_command("rm -rf /");
        assert!(!result.allowed);
        assert!(result.reason.is_some());
    }

    #[test]
    fn audit_detects_unmatched_quotes() {
        let result = audit_command("echo 'hello");
        assert!(!result.allowed);
    }

    #[test]
    fn audit_checks_all_checks_present() {
        let result = audit_command("ls");
        assert_eq!(result.checks.len(), 6);
    }
}
