use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub action: String,
    pub command: Option<String>,
    pub confidence: String,
    pub reason: String,
}
