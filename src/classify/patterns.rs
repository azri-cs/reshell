use once_cell::sync::Lazy;
use regex::Regex;
use super::taxonomy::RecoveryCode;

pub struct Pattern {
    pub code: RecoveryCode,
    pub exit_codes: Vec<i32>,
    pub stderr_regexes: Vec<Regex>,
}

pub static PATTERNS: Lazy<Vec<Pattern>> = Lazy::new(|| {
    vec![
        Pattern {
            code: RecoveryCode::R20,
            exit_codes: vec![2],
            stderr_regexes: vec![
                Regex::new(r"invalid option").unwrap(),
                Regex::new(r"usage:").unwrap(),
                Regex::new(r"unrecognized argument").unwrap(),
                Regex::new(r"error: unexpected argument").unwrap(),
                Regex::new(r"bad option").unwrap(),
                Regex::new(r"unknown flag").unwrap(),
            ],
        },
        Pattern {
            code: RecoveryCode::R21,
            exit_codes: vec![126, 128],
            stderr_regexes: vec![
                Regex::new(r"Permission denied").unwrap(),
                Regex::new(r"Operation not permitted").unwrap(),
                Regex::new(r"EACCES").unwrap(),
                Regex::new(r"cannot open.*Permission denied").unwrap(),
            ],
        },
        Pattern {
            code: RecoveryCode::R22,
            exit_codes: vec![127],
            stderr_regexes: vec![
                Regex::new(r"command not found").unwrap(),
                Regex::new(r": not found").unwrap(),
                Regex::new(r"No such file or directory").unwrap(),
                Regex::new(r"ENOENT").unwrap(),
            ],
        },
        Pattern {
            code: RecoveryCode::R24,
            exit_codes: vec![1],
            stderr_regexes: vec![
                Regex::new(r"npm ERR!").unwrap(),
                Regex::new(r"pytest failed").unwrap(),
                Regex::new(r"make: \*\*\*").unwrap(),
                Regex::new(r"error\[.*\]").unwrap(),
                Regex::new(r"FAILED").unwrap(),
                Regex::new(r"FAIL").unwrap(),
                Regex::new(r"error:.*compilation").unwrap(),
            ],
        },
    ]
});
