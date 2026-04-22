use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: Option<i64>,
    pub command_hash: String,
    pub command_template: String,
    pub recovery_code: String,
    pub stderr_pattern: String,
    pub fix_command: Option<String>,
    pub fix_success_rate: f64,
    pub last_used: Option<DateTime<Utc>>,
    pub usage_count: i64,
}
