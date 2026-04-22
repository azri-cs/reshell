use crate::classify::taxonomy::RecoveryCode;
use crate::env::Detector;
use super::strategies::Suggestion;

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
        RecoveryCode::R21 => Suggestion {
            action: "fix_permissions".to_string(),
            command: Some(format!("sudo {}", original_command)),
            confidence: "medium".to_string(),
            reason: "Permission denied. Try with sudo or fix file permissions.".to_string(),
        },
        RecoveryCode::R22 => {
            let cmd = first_word(original_command);
            let install_cmd = detector.suggest_install_command(cmd);
            Suggestion {
                action: "install_missing_tool".to_string(),
                command: install_cmd,
                confidence: if detector.package_manager.is_some() { "high" } else { "medium" }.to_string(),
                reason: format!("`{}` not found in $PATH.", cmd),
            }
        }
        RecoveryCode::R23 => Suggestion {
            action: "chunked_execution".to_string(),
            command: None,
            confidence: "medium".to_string(),
            reason: "Command timed out. Consider increasing timeout or running in smaller chunks.".to_string(),
        },
        RecoveryCode::R24 => Suggestion {
            action: "run_diagnostic".to_string(),
            command: diagnostic_for_tool(original_command),
            confidence: "medium".to_string(),
            reason: "Subcommand failed. Run a diagnostic to investigate root cause.".to_string(),
        },
        RecoveryCode::R25 => Suggestion {
            action: "use_posix".to_string(),
            command: None,
            confidence: "medium".to_string(),
            reason: "Environment mismatch detected. Use POSIX-compliant or shell-native syntax.".to_string(),
        },
        RecoveryCode::R26 => Suggestion {
            action: "scope_output".to_string(),
            command: Some(format!("{} | head -n 50", original_command)),
            confidence: "high".to_string(),
            reason: "Output overflow. Use grep/head/tail to scope results.".to_string(),
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
