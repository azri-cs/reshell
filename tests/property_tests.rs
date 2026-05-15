//! Property-based tests for reshell core components.
//! These ensure that key functions never panic on arbitrary input.

use proptest::prelude::*;
use reshell::classify::{classify, taxonomy::RecoveryCode};
use reshell::exec::validator;
use reshell::sandbox::scrubber;

proptest! {
    #[test]
    fn validator_never_panics(cmd in "\\PC{0,200}") {
        let _ = validator::validate(&cmd);
    }

    #[test]
    fn classifier_returns_valid_code(exit_code in -128i32..255i32, stderr in "\\PC{0,500}", stdout in "\\PC{0,500}") {
        let result = classify(exit_code, &stderr, &stdout, false, "bash", None);
        let code_str = result.code.to_string();
        assert!(code_str.starts_with('R'));
        let num: i32 = code_str[1..].parse().unwrap_or(0);
        assert!((10..=30).contains(&num));
    }

    #[test]
    fn scrubber_does_not_expand_output(input in "\\PC{0,500}") {
        let scrubbed = scrubber::scrub_secrets(&input);
        let max_growth = 14 * 10;
        assert!(scrubbed.len() <= input.len() + max_growth);
    }

    #[test]
    fn classifier_handles_timed_out(exit_code in -128i32..255i32, stderr in "\\PC{0,100}") {
        let result = classify(exit_code, &stderr, "", true, "bash", None);
        assert_eq!(result.code, RecoveryCode::R23);
    }

    #[test]
    fn classifier_success_on_exit_zero(stderr in "\\PC{0,100}", stdout in "\\PC{0,100}") {
        let result = classify(0, &stderr, &stdout, false, "bash", None);
        assert_eq!(result.code, RecoveryCode::R10);
    }
}
