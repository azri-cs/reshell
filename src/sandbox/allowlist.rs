//! Command allowlist/blocklist mode configuration.
//!
//! Reads from `~/.reshell/allowlist.toml`:
//!
//! ```toml
//! [mode]
//! # "blocklist" = block dangerous, allow everything else (default)
//! # "allowlist" = only allow explicitly listed commands
//! type = "allowlist"
//!
//! [[allow]]
//! commands = ["git", "cargo", "npm", "ls", "cat", "grep"]
//! allow_args = true
//!
//! [[allow]]
//! commands = ["echo", "printf", "date"]
//! allow_args = true
//! ```

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// Block dangerous commands (regex patterns), allow everything else.
    Blocklist,
    /// Only allow explicitly listed commands.
    Allowlist,
}

#[derive(Debug, Clone)]
pub struct AllowlistConfig {
    pub mode: SandboxMode,
    /// Set of allowed command names (first word of the command).
    pub allowed_commands: HashSet<String>,
    /// If true, any arguments are permitted for allowed commands.
    /// If false, only exact command matches are allowed (no args).
    pub allow_args: bool,
}

static ALLOWLIST_CONFIG: Lazy<AllowlistConfig> = Lazy::new(|| match load_allowlist_config() {
    Ok(config) => config,
    Err(e) => {
        crate::config::warn(&format!(
            "failed to load allowlist config: {}; using blocklist mode",
            e
        ));
        AllowlistConfig {
            mode: SandboxMode::Blocklist,
            allowed_commands: HashSet::new(),
            allow_args: true,
        }
    }
});

pub fn current_mode() -> SandboxMode {
    ALLOWLIST_CONFIG.mode
}

pub fn is_command_allowed(command: &str) -> Result<(), String> {
    if ALLOWLIST_CONFIG.mode == SandboxMode::Blocklist {
        return Ok(()); // Blocklist mode — validator handles blocking
    }

    let first_word = command.split_whitespace().next().unwrap_or("").trim();

    if first_word.is_empty() {
        return Err("Empty command is not allowed in allowlist mode".to_string());
    }

    if !ALLOWLIST_CONFIG.allowed_commands.contains(first_word) {
        return Err(format!(
            "Command '{}' is not in the allowlist. Allowed: {}",
            first_word,
            ALLOWLIST_CONFIG
                .allowed_commands
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // If allow_args is false, check that the command has no arguments
    if !ALLOWLIST_CONFIG.allow_args {
        let has_args = command.split_whitespace().count() > 1;
        if has_args {
            return Err(format!(
                "Arguments are not allowed for command '{}' in current allowlist mode",
                first_word
            ));
        }
    }

    // In allowlist mode, also block shell operators
    let operators = ["|", ";", "&&", "||", ">", ">>", "<", "<<<", "<<", "&"];
    for op in &operators {
        if command.contains(op) {
            return Err(format!(
                "Shell operator '{}' is not allowed in allowlist mode",
                op
            ));
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct AllowlistFile {
    mode: Option<ModeSection>,
    allow: Option<Vec<AllowSection>>,
}

#[derive(Debug, Deserialize)]
struct ModeSection {
    #[serde(rename = "type")]
    mode_type: String,
}

#[derive(Debug, Deserialize)]
struct AllowSection {
    commands: Vec<String>,
    #[serde(default = "default_true")]
    allow_args: bool,
}

fn default_true() -> bool {
    true
}

fn load_allowlist_config() -> anyhow::Result<AllowlistConfig> {
    let path = config_path();
    if !path.exists() {
        return Ok(AllowlistConfig {
            mode: SandboxMode::Blocklist,
            allowed_commands: HashSet::new(),
            allow_args: true,
        });
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;

    let config: AllowlistFile = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid TOML in {}: {}", path.display(), e))?;

    let mode = match config.mode.as_ref().map(|m| m.mode_type.as_str()) {
        Some("allowlist") => SandboxMode::Allowlist,
        Some("blocklist") | None => SandboxMode::Blocklist,
        Some(other) => anyhow::bail!("unknown mode type '{}'; valid: allowlist, blocklist", other),
    };

    let mut allowed_commands = HashSet::new();
    let allow_args_default = match &config.allow {
        None => true,
        Some(allows) => allows.iter().all(|a| a.allow_args),
    };

    if let Some(allows) = config.allow {
        for section in allows {
            for cmd in section.commands {
                allowed_commands.insert(cmd.trim().to_string());
            }
        }
    }

    Ok(AllowlistConfig {
        mode,
        allowed_commands,
        allow_args: allow_args_default,
    })
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".reshell")
        .join("allowlist.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_blocklist() {
        let mode = current_mode();
        assert_eq!(mode, SandboxMode::Blocklist);
    }

    #[test]
    fn blocklist_mode_allows_all() {
        assert!(is_command_allowed("ls -la").is_ok());
        assert!(is_command_allowed("rm -rf /tmp/build").is_ok());
    }
}
