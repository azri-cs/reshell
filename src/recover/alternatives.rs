//! Command alternatives for R22 (Command Not Found).
//!
//! When a command is not found, check if an alternative is already installed
//! and suggest using it instead.

use crate::env::Detector;

struct Alternative {
    /// The missing command name(s).
    missing: &'static [&'static str],
    /// Alternative command(s) — first is preferred.
    alternative: &'static str,
}

static ALTERNATIVES: &[Alternative] = &[
    Alternative {
        missing: &["gh"],
        alternative: "hub",
    },
    Alternative {
        missing: &["hub"],
        alternative: "gh",
    },
    Alternative {
        missing: &["rg", "ripgrep"],
        alternative: "grep -rn",
    },
    Alternative {
        missing: &["fd", "fd-find"],
        alternative: "find . -name",
    },
    Alternative {
        missing: &["bat"],
        alternative: "cat",
    },
    Alternative {
        missing: &["exa", "lsd", "eza"],
        alternative: "ls -la",
    },
    Alternative {
        missing: &["btm", "bottom"],
        alternative: "htop",
    },
    Alternative {
        missing: &["htop"],
        alternative: "top",
    },
    Alternative {
        missing: &["jq"],
        alternative: "python3 -c 'import json,sys; ...'",
    },
    Alternative {
        missing: &["fzf"],
        alternative: "grep",
    },
    Alternative {
        missing: &["ncdu"],
        alternative: "du -sh * | sort -rh | head",
    },
    Alternative {
        missing: &["tldr"],
        alternative: "man",
    },
    Alternative {
        missing: &["delta"],
        alternative: "diff -u",
    },
    Alternative {
        missing: &["sd"],
        alternative: "sed",
    },
    Alternative {
        missing: &["dust"],
        alternative: "du -sh",
    },
    Alternative {
        missing: &["dog", "doggo"],
        alternative: "dig",
    },
    Alternative {
        missing: &["zoxide", "z"],
        alternative: "cd",
    },
    Alternative {
        missing: &["hyperfine"],
        alternative: "time",
    },
];

/// Check if a known alternative is installed for the missing tool.
/// Returns a suggestion string if one is found.
pub fn suggest_alternative(missing_command: &str, detector: &Detector) -> Option<String> {
    for alt in ALTERNATIVES {
        if alt.missing.contains(&missing_command) {
            // Check if the alternative is available
            let installed = detector.available_tools.iter().any(|t| {
                let alt_cmd = alt
                    .alternative
                    .split_whitespace()
                    .next()
                    .unwrap_or(alt.alternative);
                t.name == alt_cmd
            });

            if installed {
                return Some(format!(
                    "`{}` is not installed, but `{}` is available and can be used as an alternative. Try: {}",
                    missing_command, alt.alternative, alt.alternative
                ));
            }

            // Even if not installed, suggest the alternative as a known option
            return Some(format!(
                "`{}` is not installed. Consider using `{}` as an alternative.",
                missing_command, alt.alternative
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_detector(with_tools: &[&str]) -> Detector {
        Detector {
            available_tools: with_tools
                .iter()
                .map(|&name| crate::env::detector::ToolInfo {
                    name: name.to_string(),
                    version: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn suggests_alternative_when_installed() {
        let detector = test_detector(&["hub", "git"]);
        let result = suggest_alternative("gh", &detector);
        assert!(result.is_some());
        assert!(result.unwrap().contains("hub"));
    }

    #[test]
    fn no_suggestion_for_unknown_tool() {
        let detector = test_detector(&["git"]);
        let result = suggest_alternative("nonexistent_tool", &detector);
        assert!(result.is_none());
    }

    #[test]
    fn suggests_rg_alternative() {
        let detector = test_detector(&["grep"]);
        let result = suggest_alternative("rg", &detector);
        assert!(result.is_some());
        assert!(result.unwrap().contains("grep"));
    }
}
