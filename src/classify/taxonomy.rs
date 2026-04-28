use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecoveryCode {
    R10,
    R20,
    R21,
    R22,
    R23,
    R24,
    R25,
    R26,
    R27,
    R30,
}

impl RecoveryCode {
    pub fn class_name(&self) -> &'static str {
        match self {
            RecoveryCode::R10 => "Success",
            RecoveryCode::R20 => "Syntax Error",
            RecoveryCode::R21 => "Permission Denied",
            RecoveryCode::R22 => "Command Not Found",
            RecoveryCode::R23 => "Timeout",
            RecoveryCode::R24 => "Subcommand Failure",
            RecoveryCode::R25 => "Environment Mismatch",
            RecoveryCode::R26 => "Output Overflow",
            RecoveryCode::R27 => "Blocked / Safety Violation",
            RecoveryCode::R30 => "Fatal / Unknown",
        }
    }
}

impl fmt::Display for RecoveryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
