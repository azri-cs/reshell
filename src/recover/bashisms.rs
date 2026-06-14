//! Bashism-to-shell-native translation for R25 (Environment Mismatch).
//!
//! When a command fails with R25 and the active shell is known,
//! this module suggests specific rewrites of bash-isms.

/// A bashism pattern with its shell-specific replacement.
struct Bashism {
    /// Brief description of what's wrong.
    name: &'static str,
    /// What to look for in the original command.
    pattern: &'static str,
    /// Replacement when the active shell is zsh.
    zsh_fix: Option<&'static str>,
    /// Replacement when the active shell is POSIX sh/dash.
    posix_fix: Option<&'static str>,
}

static BASHISMS: &[Bashism] = &[
    Bashism {
        name: "[[ double-bracket test",
        pattern: "[[ ",
        zsh_fix: Some("[[ "), // zsh also supports [[ ]], so no change needed
        posix_fix: Some("[ "),
    },
    Bashism {
        name: "== inside [ ] (string equality)",
        pattern: " == ",
        zsh_fix: Some(" = "),
        posix_fix: Some(" = "),
    },
    Bashism {
        name: "function keyword",
        pattern: "function ",
        zsh_fix: Some(""),
        posix_fix: Some(""),
    },
    Bashism {
        name: "declare -A (associative array)",
        pattern: "declare -A",
        zsh_fix: Some("typeset -A"),
        posix_fix: None, // No POSIX equivalent
    },
    Bashism {
        name: "source builtin",
        pattern: "source ",
        zsh_fix: Some(". "),
        posix_fix: Some(". "),
    },
    Bashism {
        name: "${! indirect expansion",
        pattern: "${!",
        zsh_fix: Some("${(P)"),
        posix_fix: None,
    },
    Bashism {
        name: "&> combined redirect",
        pattern: "&> ",
        zsh_fix: Some(">& "),
        posix_fix: Some(">file 2>&1"),
    },
    Bashism {
        name: "shopt builtin",
        pattern: "shopt ",
        zsh_fix: None, // No zsh equivalent
        posix_fix: None,
    },
    Bashism {
        name: "read -p prompt",
        pattern: "read -p",
        zsh_fix: Some("read '?prompt'"),
        posix_fix: None,
    },
    Bashism {
        name: "echo -e (interpret backslash escapes)",
        pattern: "echo -e",
        zsh_fix: Some("print"),
        posix_fix: Some("printf"),
    },
    Bashism {
        name: "declare -n (nameref)",
        pattern: "declare -n",
        zsh_fix: None,
        posix_fix: None,
    },
    Bashism {
        name: "$'...' ANSI-C quoting",
        pattern: "$'",
        zsh_fix: Some("$'"),
        posix_fix: None,
    },
    Bashism {
        name: "(( )) arithmetic evaluation",
        pattern: "(( ",
        zsh_fix: Some("(( "),
        posix_fix: Some("expr "),
    },
    Bashism {
        name: "$(( )) arithmetic expansion",
        pattern: "$((",
        zsh_fix: Some("$(("),
        posix_fix: Some("$(expr "),
    },
    Bashism {
        name: "here-string (<<<)",
        pattern: "<<<",
        zsh_fix: Some("<<<"),
        posix_fix: None,
    },
    Bashism {
        name: "select loop",
        pattern: "select ",
        zsh_fix: Some("select "),
        posix_fix: None,
    },
    Bashism {
        name: "coproc keyword",
        pattern: "coproc ",
        zsh_fix: None,
        posix_fix: None,
    },
    Bashism {
        name: "PROMPT_COMMAND variable",
        pattern: "PROMPT_COMMAND",
        zsh_fix: Some("precmd()"),
        posix_fix: None,
    },
];

/// Detect bashisms in a command and suggest rewrites for the target shell.
pub fn translate_bashisms(command: &str, target_shell: &str) -> Option<String> {
    let shell_lower = target_shell.to_lowercase();
    let is_zsh = shell_lower.contains("zsh");
    let is_posix = shell_lower.contains("sh") || shell_lower.contains("dash");

    let mut suggestions = Vec::new();
    let mut fixed_command = command.to_string();

    for bashism in BASHISMS {
        if !command.contains(bashism.pattern) {
            continue;
        }

        let replacement = if is_zsh {
            bashism.zsh_fix
        } else if is_posix {
            bashism.posix_fix
        } else {
            // Unknown shell — try POSIX
            bashism.posix_fix
        };

        if let Some(repl) = replacement {
            // Skip no-op replacements (where the fix is the same as the pattern)
            if repl == bashism.pattern {
                continue;
            }
            suggestions.push(format!(
                "{} → {}",
                bashism.name,
                if repl.is_empty() { "(remove)" } else { repl }
            ));
            fixed_command = fixed_command.replace(bashism.pattern, repl);
        } else {
            suggestions.push(format!(
                "{} — no direct {} equivalent, try rewriting",
                bashism.name, target_shell
            ));
        }
    }

    if suggestions.is_empty() {
        None
    } else {
        let reason = format!(
            "Bashisms detected for {} shell:\n- {}\n\nSuggested rewrite: {}",
            target_shell,
            suggestions.join("\n- "),
            fixed_command
        );
        Some(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_double_bracket_bashism_for_sh() {
        let result = translate_bashisms("[[ -n $VAR ]] && echo ok", "sh");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("double-bracket"));
        assert!(text.contains("[ "));
    }

    #[test]
    fn zsh_retains_double_bracket() {
        let result = translate_bashisms("[[ -n $VAR ]] && echo ok", "zsh");
        // zsh_fix is the same as the pattern, so it won't suggest a change
        assert!(result.is_none() || !result.unwrap().contains("double-bracket"));
    }

    #[test]
    fn detects_source_bashism() {
        let result = translate_bashisms("source /etc/profile", "sh");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("source"));
        assert!(text.contains(". "));
    }

    #[test]
    fn detects_indirect_expansion_for_zsh() {
        let result = translate_bashisms("echo ${!var_name}", "zsh");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("indirect"));
        assert!(text.contains("${(P)"));
    }

    #[test]
    fn detects_arithmetic_expansion_for_posix() {
        let result = translate_bashisms("echo $((1 + 1))", "sh");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("arithmetic expansion"));
    }

    #[test]
    fn detects_ansi_c_quoting_for_posix() {
        let result = translate_bashisms("echo $'hello\\nworld'", "sh");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("ANSI-C quoting"));
    }
}
