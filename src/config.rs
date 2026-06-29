//! Reshell configuration via `~/.reshell/config.toml`.
//!
//! All values have sensible defaults; the config file is optional.

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag to suppress stderr warnings (set in MCP mode so warnings
/// don't interleave with JSON-RPC frames on stderr).
static SUPPRESS_STDERR: AtomicBool = AtomicBool::new(false);

/// Suppress eprintln! warnings — call once during MCP server startup.
pub fn suppress_stderr_warnings() {
    SUPPRESS_STDERR.store(true, Ordering::Relaxed);
}

/// Emit a warning to stderr unless suppressed (MCP mode).
pub(crate) fn warn(msg: &str) {
    if !SUPPRESS_STDERR.load(Ordering::Relaxed) {
        eprintln!("rsh: warning: {}", msg);
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReshellConfig {
    #[serde(default)]
    pub execution: ExecutionConfig,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub scrubber: ScrubberConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// Per-session / hourly / daily shell-invocation budgets. All default to 0
    /// (unlimited) so existing deployments see no behavior change until a cap
    /// is set in `[budget]`.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Human-in-the-loop review triggers for high-risk commands. Commands that
    /// match return R28 (Approval Required) unless the caller passes
    /// `approve: true`. Off by default (auto_approve = false).
    #[serde(default)]
    pub safety: SafetyConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionConfig {
    #[serde(default = "default_max_timeout")]
    pub max_timeout_secs: u64,
    #[serde(default = "default_timeout")]
    pub default_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_max_output_lines")]
    pub max_output_lines: usize,
    #[serde(default = "default_tail_lines")]
    pub tail_lines: usize,
    #[serde(default = "default_large_stdout")]
    pub large_stdout_bytes: usize,
    #[serde(default = "default_max_stored_rows")]
    pub max_stored_output_rows: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScrubberConfig {
    #[serde(default = "default_entropy_threshold")]
    pub entropy_threshold: f64,
    #[serde(default)]
    pub disable_entropy: bool,
    #[serde(default)]
    pub additional_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SandboxConfig {
    #[serde(default)]
    pub additional_blocked_env: Vec<String>,
    #[serde(default)]
    pub allowed_env: Vec<String>,
    #[serde(default)]
    pub seccomp: bool,
}

/// Shell-invocation budget caps. Any value of `0` means "unlimited" for that
/// dimension. Session caps live in-process (atomic counters in `ServerState`);
/// hourly/daily caps persist across server restarts via the `budget_ledger`
/// SQLite table. Output bytes are charged *after* execution (shell output size
/// cannot be known in advance); invocation and wall-time caps are pre-checked.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BudgetConfig {
    /// Max `rsh_exec` calls within a single server process. 0 = unlimited.
    #[serde(default)]
    pub max_invocations_per_session: u64,
    /// Max total stdout bytes charged to the session. 0 = unlimited.
    #[serde(default)]
    pub max_output_bytes_per_session: u64,
    /// Max total wall-clock seconds charged to the session. 0 = unlimited.
    #[serde(default)]
    pub max_wall_secs_per_session: u64,
    /// Max `rsh_exec` calls per wall-clock hour (persisted). 0 = unlimited.
    #[serde(default)]
    pub max_invocations_per_hour: u64,
    /// Max `rsh_exec` calls per calendar day (persisted). 0 = unlimited.
    #[serde(default)]
    pub max_invocations_per_day: u64,
}

impl BudgetConfig {
    /// True when no cap is configured anywhere — lets the hot path skip the
    /// check entirely (and skip DB reads for the ledger).
    pub fn is_unlimited(&self) -> bool {
        self.max_invocations_per_session == 0
            && self.max_output_bytes_per_session == 0
            && self.max_wall_secs_per_session == 0
            && self.max_invocations_per_hour == 0
            && self.max_invocations_per_day == 0
    }
}

/// Human-in-the-loop review configuration. Commands flagged as high-risk
/// (recursive deletes outside `/tmp`, `git push --force`, `docker system prune`,
/// `sudo`-bearing, or matching `review_patterns`) return `R28` unless the
/// caller passes `approve: true`. `auto_approve` skips review entirely.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SafetyConfig {
    /// If true, all high-risk commands run without prompting for approval.
    /// Defaults to false (review enabled).
    #[serde(default)]
    pub auto_approve: bool,
    /// Extra regex patterns (matched against the raw command) that should
    /// trigger review, on top of the built-in triggers.
    #[serde(default)]
    pub review_patterns: Vec<String>,
}

fn default_max_timeout() -> u64 {
    600
}
fn default_timeout() -> u64 {
    120
}
fn default_max_output_lines() -> usize {
    100
}
fn default_tail_lines() -> usize {
    20
}
fn default_large_stdout() -> usize {
    512 * 1024
}
fn default_max_stored_rows() -> usize {
    5000
}
fn default_entropy_threshold() -> f64 {
    3.5
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_timeout_secs: default_max_timeout(),
            default_timeout_secs: default_timeout(),
        }
    }
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_output_lines: default_max_output_lines(),
            tail_lines: default_tail_lines(),
            large_stdout_bytes: default_large_stdout(),
            max_stored_output_rows: default_max_stored_rows(),
        }
    }
}

impl Default for ScrubberConfig {
    fn default() -> Self {
        Self {
            entropy_threshold: default_entropy_threshold(),
            disable_entropy: false,
            additional_patterns: vec![],
        }
    }
}

impl ReshellConfig {
    /// Returns a config with defaults suitable for testing (smaller thresholds).
    pub fn test_defaults() -> Self {
        Self {
            compaction: CompactionConfig {
                max_output_lines: 10,
                tail_lines: 5,
                large_stdout_bytes: 1024, // small for fast tests
                max_stored_output_rows: 100,
            },
            ..Default::default()
        }
    }
}

/// Singleton config, loaded on first use.
static CONFIG: Lazy<ReshellConfig> = Lazy::new(|| match load_config() {
    Ok(cfg) => cfg,
    Err(e) => {
        warn(&format!("failed to load config: {}; using defaults", e));
        ReshellConfig::default()
    }
});

pub fn get() -> &'static ReshellConfig {
    &CONFIG
}

fn load_config() -> anyhow::Result<ReshellConfig> {
    let path = config_path();
    if !path.exists() {
        return Ok(ReshellConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
    let config: ReshellConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid TOML in {}: {}", path.display(), e))?;
    Ok(config)
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".reshell")
        .join("config.toml")
}
