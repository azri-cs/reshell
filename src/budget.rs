//! Tool-call budget guardrail for `rsh_exec`.
//!
//! Hybrid model (per the improvement plan):
//! - **In-process** counters in [`BudgetCounters`] track the live MCP session
//!   via atomics — zero DB overhead on the hot path.
//! - **Persistent** hourly/daily windows live in the `budget_ledger` SQLite
//!   table so caps survive server restarts.
//!
//! Enforcement happens at the top of the `rsh_exec` tool handler:
//! invocation and wall-time caps are checked *before* execution; output-byte
//! caps are charged *after* execution (shell output size cannot be known in
//! advance) and may cause the *next* call to be refused.
//!
//! All caps default to `0` (unlimited), so existing deployments see no behavior
//! change until a `[budget]` section is added to `config.toml`.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::BudgetConfig;

/// In-process session counters. Lives in `ServerState` and is bumped on every
/// `rsh_exec`. Cheap to read (relaxed atomics); no locking.
#[derive(Debug, Default)]
pub struct BudgetCounters {
    invocations: AtomicU64,
    output_bytes: AtomicU64,
    wall_nanos: AtomicU64,
}

impl BudgetCounters {
    /// Increment the invocation counter and return the new count.
    pub fn inc_invocation(&self) -> u64 {
        self.invocations.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn add_output_bytes(&self, n: u64) {
        self.output_bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_wall_secs(&self, secs: u64) {
        self.wall_nanos
            .fetch_add(secs * 1_000_000_000, Ordering::Relaxed);
    }

    pub fn invocations(&self) -> u64 {
        self.invocations.load(Ordering::Relaxed)
    }
    pub fn output_bytes(&self) -> u64 {
        self.output_bytes.load(Ordering::Relaxed)
    }
    pub fn wall_secs(&self) -> u64 {
        self.wall_nanos.load(Ordering::Relaxed) / 1_000_000_000
    }
}

/// A pre-execution budget decision. On `Exhausted`, the call is refused with
/// recovery code R29 and a human-readable reason naming the breached cap.
#[derive(Debug)]
pub enum BudgetDecision {
    /// The call may proceed (no cap breached).
    Allowed,
    /// A configured cap has been reached; refuse the call.
    Exhausted {
        cap: &'static str,
        used: u64,
        limit: u64,
    },
}

/// Evaluate the session-scoped caps (in-process). Called before execution.
/// Does NOT touch the DB — hourly/daily windows are checked separately by the
/// handler because they require a SQLite read.
pub fn check_session(counters: &BudgetCounters, cfg: &BudgetConfig) -> BudgetDecision {
    if cfg.max_invocations_per_session > 0 {
        let used = counters.invocations();
        if used >= cfg.max_invocations_per_session {
            return BudgetDecision::Exhausted {
                cap: "max_invocations_per_session",
                used,
                limit: cfg.max_invocations_per_session,
            };
        }
    }
    if cfg.max_wall_secs_per_session > 0 {
        let used = counters.wall_secs();
        if used >= cfg.max_wall_secs_per_session {
            return BudgetDecision::Exhausted {
                cap: "max_wall_secs_per_session",
                used,
                limit: cfg.max_wall_secs_per_session,
            };
        }
    }
    if cfg.max_output_bytes_per_session > 0 {
        let used = counters.output_bytes();
        if used >= cfg.max_output_bytes_per_session {
            return BudgetDecision::Exhausted {
                cap: "max_output_bytes_per_session",
                used,
                limit: cfg.max_output_bytes_per_session,
            };
        }
    }
    BudgetDecision::Allowed
}

/// A persistent window (hourly or daily) and its key, ready to read/charge.
#[derive(Debug, Clone, Copy)]
pub struct BudgetWindow {
    pub bucket: &'static str,
    pub window_key: &'static str,
}

/// Compute the current hourly and daily window keys from a UTC timestamp.
/// Kept pure (takes secs-since-epoch) so it is trivially unit-testable.
pub fn current_windows(secs_since_epoch: i64) -> [BudgetWindow; 2] {
    // Day-of-week-agnostic calendar math: days since the Unix epoch.
    let day_index = secs_since_epoch.div_euclid(86_400);
    let daily = format!("day-{}", day_index);

    // Hour within the epoch (monotonic; we only ever compare equal keys).
    let hour_index = secs_since_epoch.div_euclid(3600);
    let hourly = format!("hour-{}", hour_index);

    // Leak the strings to get 'static keys. This runs at most once per window
    // transition in practice (the handler caches the keys per call), and the
    // total number of windows over a server's lifetime is bounded (hours/days).
    let daily_key: &'static str = daily.leak();
    let hourly_key: &'static str = hourly.leak();
    [
        BudgetWindow {
            bucket: "hourly",
            window_key: hourly_key,
        },
        BudgetWindow {
            bucket: "daily",
            window_key: daily_key,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_config_is_detected() {
        assert!(BudgetConfig::default().is_unlimited());
        let cfg = BudgetConfig {
            max_invocations_per_session: 10,
            ..Default::default()
        };
        assert!(!cfg.is_unlimited());
    }

    #[test]
    fn session_cap_blocks_after_limit() {
        let counters = BudgetCounters::default();
        let cfg = BudgetConfig {
            max_invocations_per_session: 2,
            ..Default::default()
        };
        assert!(matches!(
            check_session(&counters, &cfg),
            BudgetDecision::Allowed
        ));
        counters.inc_invocation();
        counters.inc_invocation();
        // Now used (2) >= limit (2) → exhausted.
        match check_session(&counters, &cfg) {
            BudgetDecision::Exhausted { cap, used, limit } => {
                assert_eq!(cap, "max_invocations_per_session");
                assert_eq!(used, 2);
                assert_eq!(limit, 2);
            }
            _ => panic!("expected Exhausted"),
        }
    }

    #[test]
    fn unlimited_session_never_blocks() {
        let counters = BudgetCounters::default();
        for _ in 0..1000 {
            counters.inc_invocation();
        }
        // Default config (all zeros) never blocks.
        assert!(matches!(
            check_session(&counters, &BudgetConfig::default()),
            BudgetDecision::Allowed
        ));
    }

    #[test]
    fn wall_secs_cap_blocks() {
        let counters = BudgetCounters::default();
        counters.add_wall_secs(600);
        let cfg = BudgetConfig {
            max_wall_secs_per_session: 600,
            ..Default::default()
        };
        assert!(matches!(
            check_session(&counters, &cfg),
            BudgetDecision::Exhausted { .. }
        ));
    }

    #[test]
    fn output_bytes_cap_blocks() {
        let counters = BudgetCounters::default();
        counters.add_output_bytes(10 * 1024 * 1024);
        let cfg = BudgetConfig {
            max_output_bytes_per_session: 10 * 1024 * 1024,
            ..Default::default()
        };
        assert!(matches!(
            check_session(&counters, &cfg),
            BudgetDecision::Exhausted { .. }
        ));
    }

    #[test]
    fn window_keys_are_stable_within_period() {
        // Two calls in the same hour produce identical keys.
        let a = current_windows(1_800_000_000); // a fixed epoch second
        let b = current_windows(1_800_000_000 + 60);
        assert_eq!(a[0].window_key, b[0].window_key);
        assert_eq!(a[1].window_key, b[1].window_key);
    }

    #[test]
    fn hourly_key_advances_across_hours() {
        let a = current_windows(1_800_000_000);
        let b = current_windows(1_800_000_000 + 3700); // >1h later
        assert_ne!(a[0].window_key, b[0].window_key);
        // ~1h is well under a day, so the daily key must NOT change yet.
        assert_eq!(a[1].window_key, b[1].window_key);
    }

    #[test]
    fn daily_key_advances_across_days() {
        let a = current_windows(1_800_000_000);
        let b = current_windows(1_800_000_000 + 90_000); // >1 day later
        assert_ne!(a[1].window_key, b[1].window_key);
    }
}
