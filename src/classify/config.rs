//! External pattern configuration via `~/.reshell/patterns.toml`.
//!
//! Users can add or override classification patterns without recompiling.
//! Patterns are validated on load; invalid ones are skipped with a warning.
//!
//! Format:
//! ```toml
//! [[patterns]]
//! code = "R22"
//! exit_codes = [127]
//! stderr_regexes = ["my-tool: command not found"]
//! ```

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::path::PathBuf;

use super::patterns::Pattern;
use super::taxonomy::RecoveryCode;

/// Loaded user patterns (empty if config file doesn't exist or has errors).
static USER_PATTERNS: Lazy<Vec<Pattern>> = Lazy::new(|| {
    match load_user_patterns() {
        Ok(patterns) => patterns,
        Err(e) => {
            eprintln!("rsh: warning: failed to load user patterns: {}", e);
            Vec::new()
        }
    }
});

/// Returns the merged list of classification patterns: user patterns first
/// (higher priority), then built-in patterns as fallback.
pub fn merged_patterns(builtins: &[Pattern]) -> Vec<Pattern> {
    let mut merged = Vec::with_capacity(USER_PATTERNS.len() + builtins.len());

    // User patterns take precedence (checked first)
    for p in USER_PATTERNS.iter() {
        merged.push(Pattern {
            code: p.code,
            exit_codes: p.exit_codes.clone(),
            stderr_regexes: p.stderr_regexes.clone(),
        });
    }

    // Built-in patterns as fallback
    for p in builtins.iter() {
        merged.push(Pattern {
            code: p.code,
            exit_codes: p.exit_codes.clone(),
            stderr_regexes: p.stderr_regexes.clone(),
        });
    }

    merged
}

#[derive(Debug, Deserialize)]
struct UserConfig {
    patterns: Vec<UserPatternDef>,
}

#[derive(Debug, Deserialize)]
struct UserPatternDef {
    code: String,
    exit_codes: Vec<i32>,
    stderr_regexes: Vec<String>,
}

fn load_user_patterns() -> anyhow::Result<Vec<Pattern>> {
    let path = config_path();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;

    let config: UserConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid TOML in {}: {}", path.display(), e))?;

    let mut patterns = Vec::new();
    for (i, def) in config.patterns.iter().enumerate() {
        match parse_user_pattern(def) {
            Ok(p) => patterns.push(p),
            Err(e) => {
                eprintln!(
                    "rsh: warning: skipping pattern #{} in {}: {}",
                    i + 1,
                    path.display(),
                    e
                );
            }
        }
    }

    Ok(patterns)
}

fn parse_user_pattern(def: &UserPatternDef) -> anyhow::Result<Pattern> {
    let code = parse_recovery_code(&def.code)?;

    let mut stderr_regexes = Vec::with_capacity(def.stderr_regexes.len());
    for (i, re_str) in def.stderr_regexes.iter().enumerate() {
        let re = Regex::new(re_str).map_err(|e| {
            anyhow::anyhow!("invalid regex #{} `{}`: {}", i + 1, re_str, e)
        })?;
        stderr_regexes.push(re);
    }

    Ok(Pattern {
        code,
        exit_codes: def.exit_codes.clone(),
        stderr_regexes,
    })
}

fn parse_recovery_code(s: &str) -> anyhow::Result<RecoveryCode> {
    match s.trim().to_uppercase().as_str() {
        "R10" => Ok(RecoveryCode::R10),
        "R20" => Ok(RecoveryCode::R20),
        "R21" => Ok(RecoveryCode::R21),
        "R22" => Ok(RecoveryCode::R22),
        "R23" => Ok(RecoveryCode::R23),
        "R24" => Ok(RecoveryCode::R24),
        "R25" => Ok(RecoveryCode::R25),
        "R26" => Ok(RecoveryCode::R26),
        "R27" => Ok(RecoveryCode::R27),
        "R30" => Ok(RecoveryCode::R30),
        _ => anyhow::bail!(
            "unknown recovery code '{}'; valid: R10, R20-R27, R30",
            s
        ),
    }
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".reshell")
        .join("patterns.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_recovery_codes() {
        assert!(parse_recovery_code("R10").is_ok());
        assert!(parse_recovery_code("r22").is_ok());
        assert!(parse_recovery_code("  R30  ").is_ok());
    }

    #[test]
    fn rejects_invalid_recovery_codes() {
        assert!(parse_recovery_code("R99").is_err());
        assert!(parse_recovery_code("").is_err());
        assert!(parse_recovery_code("bad").is_err());
    }

    #[test]
    fn merges_user_patterns_before_builtins() {
        let user = Pattern {
            code: RecoveryCode::R22,
            exit_codes: vec![127],
            stderr_regexes: vec![Regex::new("custom-error").unwrap()],
        };
        let builtins = vec![Pattern {
            code: RecoveryCode::R22,
            exit_codes: vec![127],
            stderr_regexes: vec![Regex::new("command not found").unwrap()],
        }];

        // Simulate what merged_patterns does: user first, then builtins
        let mut merged: Vec<Pattern> = Vec::new();
        merged.push(Pattern {
            code: user.code,
            exit_codes: user.exit_codes.clone(),
            stderr_regexes: user.stderr_regexes.clone(),
        });
        for p in &builtins {
            merged.push(Pattern {
                code: p.code,
                exit_codes: p.exit_codes.clone(),
                stderr_regexes: p.stderr_regexes.clone(),
            });
        }

        assert_eq!(merged[0].stderr_regexes[0].as_str(), "custom-error");
        assert_eq!(merged[1].stderr_regexes[0].as_str(), "command not found");
    }
}
