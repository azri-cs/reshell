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
            "You are analyzing a command failure.\n\n\
             Steps:\n\
             1. Check the `auto_retry` field first — if present and status=success,\n\
                the fix was already auto-applied. Just call rsh_feedback.\n\
             2. If auto_retry was absent or failed, call `rsh_recover` with the\n\
                recovery_code and original_command from the failure.\n\
             3. `rsh_recover` returns a fix command — pass it to `rsh_exec`.\n\
             4. After retrying, **call `rsh_feedback` immediately** with the outcome.\n\
                Without feedback, pattern memory cannot learn and this failure\n\
                will not self-heal on future occurrences."
                .to_string(),
        ),
        "environment_audit" => Some(
            "Audit the shell environment before running commands.\n\n\
             Actions:\n\
             1. Call `rsh_env` to detect the current environment (OS, shell, tools).\n\
             2. Check that essential tools are available:\n\
                - Is `git` available? → version detected\n\
                - Is the project's language toolchain available? (cargo, node, npm, python3, go, etc.)\n\
                - Is a package manager detected? (brew on macOS, apt on Debian, etc.)\n\
             3. If a critical tool is missing, install it and call `rsh_env refresh=true`.\n\
             4. Note the shell type — rsh auto-retries with the detected recovery\n\
                shell on environment mismatch (R25).\n\
             5. Use the package manager to suggest install commands for missing tools."
                .to_string(),
        ),
        _ => None,
    }
}
