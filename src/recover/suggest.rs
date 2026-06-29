use super::strategies::Suggestion;
use crate::classify::taxonomy::RecoveryCode;
use crate::env::Detector;

pub fn suggest(
    code: RecoveryCode,
    original_command: &str,
    _context: &str,
    detector: &Detector,
) -> Suggestion {
    match code {
        RecoveryCode::R10 => Suggestion {
            action: "none".to_string(),
            command: None,
            confidence: "high".to_string(),
            reason: "Command succeeded".to_string(),
        },
        RecoveryCode::R20 => Suggestion {
            action: "check_usage".to_string(),
            command: Some(format!("{} --help", first_word(original_command))),
            confidence: "high".to_string(),
            reason: "Syntax error detected. Review flags and usage.".to_string(),
        },
        RecoveryCode::R21 => {
            let safe_cmd = if contains_shell_operators(original_command) {
                first_word(original_command).to_string()
            } else {
                original_command.to_string()
            };
            Suggestion {
                action: "fix_permissions".to_string(),
                command: Some(format!("sudo {}", safe_cmd)),
                confidence: "medium".to_string(),
                reason: "Permission denied. Try with sudo or fix file permissions.".to_string(),
            }
        }
        RecoveryCode::R22 => {
            let cmd = first_word(original_command);
            // Try suggesting an alternative first, then fall back to install
            if let Some(alt_suggestion) = super::alternatives::suggest_alternative(cmd, detector) {
                let alt_cmd = alt_suggestion
                    .split("Try: ")
                    .nth(1)
                    .unwrap_or(&alt_suggestion)
                    .trim()
                    .to_string();
                return Suggestion {
                    action: "use_alternative".to_string(),
                    command: Some(alt_cmd),
                    confidence: "medium".to_string(),
                    reason: alt_suggestion,
                };
            }
            let install_cmd = detector.suggest_install_command(cmd);
            Suggestion {
                action: "install_missing_tool".to_string(),
                command: install_cmd,
                confidence: if detector.package_manager.is_some() {
                    "high"
                } else {
                    "medium"
                }
                .to_string(),
                reason: format!("`{}` not found in $PATH.", cmd),
            }
        }
        RecoveryCode::R23 => Suggestion {
            action: "chunked_execution".to_string(),
            command: None,
            confidence: "medium".to_string(),
            reason: "Command timed out. Consider increasing timeout or running in smaller chunks."
                .to_string(),
        },
        RecoveryCode::R24 => {
            // Try extracting missing dependency from stderr
            if let Some(dep_suggestion) =
                super::deps::suggest_missing_dep(original_command, _context, detector)
            {
                return Suggestion {
                    action: "install_missing_dep".to_string(),
                    command: dep_suggestion
                        .split(": `")
                        .nth(1)
                        .and_then(|s| s.split('`').next())
                        .map(|s| s.to_string()),
                    confidence: "medium".to_string(),
                    reason: dep_suggestion,
                };
            }
            Suggestion {
                action: "run_diagnostic".to_string(),
                command: diagnostic_for_tool(original_command),
                confidence: "medium".to_string(),
                reason: "Subcommand failed. Run a diagnostic to investigate root cause."
                    .to_string(),
            }
        }
        RecoveryCode::R25 => {
            // Try translating bashisms for the detected shell
            let shell = detector.shell.as_str();
            if let Some(translation) = super::bashisms::translate_bashisms(original_command, shell)
            {
                let fixed = translation
                    .lines()
                    .last()
                    .and_then(|l| l.strip_prefix("Suggested rewrite: "))
                    .unwrap_or(original_command);
                return Suggestion {
                    action: "rewrite_bashism".to_string(),
                    command: Some(fixed.to_string()),
                    confidence: "medium".to_string(),
                    reason: translation,
                };
            }
            Suggestion {
                action: "use_posix".to_string(),
                command: None,
                confidence: "medium".to_string(),
                reason:
                    "Environment mismatch detected. Use POSIX-compliant or shell-native syntax."
                        .to_string(),
            }
        }
        RecoveryCode::R26 => Suggestion {
            action: "scope_output".to_string(),
            command: Some(format!("{} | head -n 50", original_command)),
            confidence: "high".to_string(),
            reason: "Output overflow. Use grep/head/tail to scope results.".to_string(),
        },
        RecoveryCode::R27 => Suggestion {
            action: "rewrite_command".to_string(),
            command: None,
            confidence: "high".to_string(),
            reason: "Command blocked by safety validator. Use a safer alternative.".to_string(),
        },
        RecoveryCode::R28 => Suggestion {
            action: "await_approval".to_string(),
            command: None,
            confidence: "high".to_string(),
            reason: "Command flagged for human review. Re-issue with approve:true once approved."
                .to_string(),
        },
        RecoveryCode::R29 => Suggestion {
            action: "wait_or_raise_limit".to_string(),
            command: None,
            confidence: "high".to_string(),
            reason: "Budget cap reached. The command was not executed; wait for the window to reset or raise the cap."
                .to_string(),
        },
        RecoveryCode::R30 => Suggestion {
            action: "escalate".to_string(),
            command: None,
            confidence: "low".to_string(),
            reason: "Unknown or fatal failure. Requires human escalation.".to_string(),
        },
    }
}

fn first_word(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or(command)
}

/// Check if a command contains shell operators that could chain dangerous commands.
fn contains_shell_operators(command: &str) -> bool {
    const OPERATORS: &[&str] = &["|", ";", "&&", "||", ">", ">>", "<", "<<<", "<<", "&"];
    for op in OPERATORS {
        if command.contains(op) {
            return true;
        }
    }
    false
}

fn diagnostic_for_tool(command: &str) -> Option<String> {
    if command.starts_with("npm") {
        Some("npm ls".to_string())
    } else if command.starts_with("cargo") {
        Some("cargo check".to_string())
    } else if command.starts_with("make") {
        Some("make --debug=b".to_string())
    } else if command.starts_with("pytest") {
        Some("pytest --collect-only".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_detector() -> Detector {
        Detector::default()
    }

    #[test]
    fn r10_suggests_none() {
        let s = suggest(RecoveryCode::R10, "ls", "", &default_detector());
        assert_eq!(s.action, "none");
        assert!(s.command.is_none());
        assert_eq!(s.confidence, "high");
    }

    #[test]
    fn r20_suggests_help() {
        let s = suggest(
            RecoveryCode::R20,
            "grep --bad-flag foo",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "check_usage");
        assert_eq!(s.command.as_deref(), Some("grep --help"));
        assert_eq!(s.confidence, "high");
    }

    #[test]
    fn r21_suggests_sudo_simple_command() {
        let s = suggest(
            RecoveryCode::R21,
            "cat /etc/shadow",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "fix_permissions");
        assert_eq!(s.command.as_deref(), Some("sudo cat /etc/shadow"));
        assert_eq!(s.confidence, "medium");
    }

    #[test]
    fn r21_sanitizes_command_with_operators() {
        let s = suggest(
            RecoveryCode::R21,
            "cat /root/file; rm -rf /",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "fix_permissions");
        // Should only suggest sudo for the first word, not the full dangerous command
        assert_eq!(s.command.as_deref(), Some("sudo cat"));
    }

    #[test]
    fn r22_suggests_install() {
        let mut detector = default_detector();
        detector.package_manager = Some("apt".to_string());
        // "nonexistent_tool_xyz" has no registered alternative, so it falls through to install
        let s = suggest(
            RecoveryCode::R22,
            "nonexistent_tool_xyz arg1",
            "",
            &detector,
        );
        assert_eq!(s.action, "install_missing_tool");
        assert!(s
            .command
            .as_deref()
            .unwrap()
            .contains("nonexistent_tool_xyz"));
        assert_eq!(s.confidence, "high");
    }

    #[test]
    fn r22_suggests_alternative_when_available() {
        let mut detector = default_detector();
        detector.package_manager = Some("apt".to_string());
        detector
            .available_tools
            .push(crate::env::detector::ToolInfo {
                name: "hub".to_string(),
                version: None,
            });
        let s = suggest(RecoveryCode::R22, "gh pr view", "", &detector);
        assert_eq!(s.action, "use_alternative");
        assert!(s.reason.contains("hub"));
    }

    #[test]
    fn r22_no_package_manager() {
        // Use a command without registered alternatives so it falls to install
        let s = suggest(
            RecoveryCode::R22,
            "nonexistent_tool_xyz",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "install_missing_tool");
        assert!(s.command.is_none());
        assert_eq!(s.confidence, "medium");
    }

    #[test]
    fn r23_suggests_chunked() {
        let s = suggest(
            RecoveryCode::R23,
            "long-running-task",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "chunked_execution");
        assert!(s.command.is_none());
        assert_eq!(s.confidence, "medium");
    }

    #[test]
    fn r24_suggests_diagnostic_npm() {
        let s = suggest(RecoveryCode::R24, "npm install", "", &default_detector());
        assert_eq!(s.action, "run_diagnostic");
        assert_eq!(s.command.as_deref(), Some("npm ls"));
    }

    #[test]
    fn r24_suggests_diagnostic_cargo() {
        let s = suggest(RecoveryCode::R24, "cargo build", "", &default_detector());
        assert_eq!(s.action, "run_diagnostic");
        assert_eq!(s.command.as_deref(), Some("cargo check"));
    }

    #[test]
    fn r24_no_diagnostic_for_unknown_tool() {
        let s = suggest(
            RecoveryCode::R24,
            "unknown-tool build",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "run_diagnostic");
        assert!(s.command.is_none());
    }

    #[test]
    fn r25_suggests_posix() {
        // Use a command without bashisms so it falls to generic POSIX suggestion
        let s = suggest(RecoveryCode::R25, "echo $VAR", "", &default_detector());
        assert_eq!(s.action, "use_posix");
        assert!(s.command.is_none());
        assert_eq!(s.confidence, "medium");
    }

    #[test]
    fn r26_suggests_scoping() {
        let s = suggest(
            RecoveryCode::R26,
            "find / -name '*.log'",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "scope_output");
        assert_eq!(
            s.command.as_deref(),
            Some("find / -name '*.log' | head -n 50")
        );
        assert_eq!(s.confidence, "high");
    }

    #[test]
    fn r27_suggests_rewrite() {
        let s = suggest(RecoveryCode::R27, "rm -rf /", "", &default_detector());
        assert_eq!(s.action, "rewrite_command");
        assert!(s.command.is_none());
        assert_eq!(s.confidence, "high");
    }

    #[test]
    fn r30_suggests_escalate() {
        let s = suggest(
            RecoveryCode::R30,
            "mystery-command",
            "",
            &default_detector(),
        );
        assert_eq!(s.action, "escalate");
        assert!(s.command.is_none());
        assert_eq!(s.confidence, "low");
    }

    #[test]
    fn contains_shell_operators_detects_pipe() {
        assert!(contains_shell_operators("ls | grep foo"));
    }

    #[test]
    fn contains_shell_operators_detects_semicolon() {
        assert!(contains_shell_operators("ls; rm -rf /"));
    }

    #[test]
    fn contains_shell_operators_detects_and() {
        assert!(contains_shell_operators("make && make install"));
    }

    #[test]
    fn contains_shell_operators_clean_command() {
        assert!(!contains_shell_operators("cat /etc/passwd"));
    }
}
