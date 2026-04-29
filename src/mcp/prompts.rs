//! MCP prompt templates for agent workflows.

use serde_json::json;

pub fn list_prompts() -> Vec<serde_json::Value> {
    vec![
        json!({
            "name": "recovery_analysis",
            "description": "Analyze a command failure and plan recovery steps. Use when rsh_exec returns status='failed'.",
            "arguments": [
                { "name": "recovery_code", "description": "Recovery code from the failed rsh_exec response", "required": true },
                { "name": "original_command", "description": "The command that failed", "required": true },
                { "name": "stderr", "description": "Stderr output from the failure", "required": false }
            ]
        }),
        json!({
            "name": "environment_audit",
            "description": "Audit the current shell environment and identify potential issues before running commands.",
            "arguments": []
        }),
    ]
}

pub fn get_prompt(name: &str, _arguments: Option<&serde_json::Value>) -> Option<String> {
    match name {
        "recovery_analysis" => Some(
            "You are analyzing a command failure. Follow these steps:\n\
             1. Identify the failure class from the recovery_code.\n\
             2. Check if rsh has a learned fix for this failure pattern (from rsh_recover).\n\
             3. Apply the suggested fix or reason about the root cause.\n\
             4. Re-execute with the fixed command via rsh_exec.\n\
             5. If successful, call rsh_feedback to improve pattern memory."
                .to_string(),
        ),
        "environment_audit" => Some(
            "Review the detected shell environment. Consider:\n\
             1. Is the right shell being used? (rsh auto-detects)\n\
             2. Are essential dev tools available? (git, cargo, npm, etc.)\n\
             3. Is a package manager detected? (apt, brew, etc.)\n\
             4. Any known environment issues? (WSL paths, missing dependencies)\n\
             5. Use rsh_env for full environment details."
                .to_string(),
        ),
        _ => None,
    }
}
