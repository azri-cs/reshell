use crate::memory::pattern::Pattern;
use crate::recover::strategies::Suggestion;

pub fn pattern_to_suggestion(pattern: &Pattern, original_command: &str) -> Suggestion {
    Suggestion {
        action: "reuse_learned_fix".to_string(),
        command: pattern.fix_command.clone(),
        confidence: learned_confidence(pattern.fix_success_rate).to_string(),
        reason: format!(
            "Matched a previously seen failure for `{}` with historical success rate {:.0}%.",
            original_command,
            pattern.fix_success_rate * 100.0
        ),
    }
}

fn learned_confidence(success_rate: f64) -> &'static str {
    if success_rate >= 0.8 {
        "high"
    } else if success_rate >= 0.5 {
        "medium"
    } else {
        "low"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn converts_pattern_to_suggestion() {
        let pattern = Pattern {
            id: Some(1),
            command_hash: "abc".to_string(),
            command_template: "cargo test".to_string(),
            recovery_code: "R24".to_string(),
            stderr_pattern: "FAILED".to_string(),
            fix_command: Some("cargo test -- --nocapture".to_string()),
            fix_success_rate: 0.9,
            last_used: Some(Utc::now()),
            usage_count: 3,
            platform_tag: Some("linux".to_string()),
        };

        let suggestion = pattern_to_suggestion(&pattern, "cargo test");
        assert_eq!(suggestion.action, "reuse_learned_fix");
        assert_eq!(suggestion.confidence, "high");
    }
}
