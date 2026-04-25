use super::{ExecRequest, ExecResult, OutputInfo, validator};
use crate::classify::{classify, taxonomy::RecoveryCode};
use crate::compact;
use crate::env::Detector;
use crate::memory::pattern::Pattern;
use crate::memory::Store;
use crate::recover::memory::pattern_to_suggestion;
use crate::recover::suggest;
use crate::sandbox::scrubber;
use crate::sandbox::paths;
use crate::utils::{hash_command, normalize_command, normalize_stderr, shell_quote};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use std::collections::HashSet;
use once_cell::sync::Lazy;

/// Maximum allowed timeout in seconds (10 minutes).
const MAX_TIMEOUT_SECS: u64 = 600;

/// Environment variables that must not be injected via the env parameter.
static BLOCKED_ENV_KEYS: Lazy<HashSet<&str>> = Lazy::new(|| {
    HashSet::from([
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "PATH",
        "SHELL",
        "HOME",
        "USER",
        "LOGNAME",
        "NODE_OPTIONS",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "RUST_LOG",
        "RUST_BACKTRACE",
        "IFS",
    ])
});

pub struct Runner {
    store: Store,
}

impl Runner {
    pub fn new() -> anyhow::Result<Self> {
        let store = Store::new()?;
        Ok(Self { store })
    }

    pub fn with_store(store: Store) -> Self {
        Self { store }
    }

    pub async fn run(&self, req: &ExecRequest) -> anyhow::Result<ExecResult> {
        // 1. Validate
        if let Err(e) = validator::validate(&req.command) {
            return Ok(ExecResult {
                status: "failed".to_string(),
                recovery_code: RecoveryCode::R30.to_string(),
                recovery_class: RecoveryCode::R30.class_name().to_string(),
                original_command: req.command.clone(),
                output_id: None,
                suggestion: serde_json::to_value(suggest::suggest(
                    RecoveryCode::R30,
                    &req.command,
                    &e,
                    &Detector::default(),
                ))?,
                output: OutputInfo {
                    stdout: String::new(),
                    stderr: e,
                    exit_code: -1,
                    truncated: false,
                },
            });
        }

        // 2. Execute
        let detector = Detector::cached().await.clone();
        let shell = detector.execution_shell();
        let retry_shell = detector.recovery_shell();
        let retry_request = retry_shell
            .as_ref()
            .map(|fallback_shell| Self::posix_retry_request(req, fallback_shell));

        let mut last_attempt = None;
        let attempts = if req.retry { 2 } else { 1 };

        for attempt_idx in 0..attempts {
            let current_shell = if attempt_idx == 0 {
                shell.as_str()
            } else {
                retry_shell.as_deref().unwrap_or(shell.as_str())
            };
            let current_req = if attempt_idx == 0 {
                req
            } else {
                retry_request.as_ref().unwrap_or(req)
            };
            let attempt = self.execute_once(current_req, current_shell).await?;
            let classification = classify(attempt.exit_code, &attempt.stderr, &attempt.stdout, attempt.timed_out, &detector.shell);
            let should_retry = req.retry
                && attempt_idx == 0
                && classification.code == RecoveryCode::R25
                && retry_shell.is_some();
            last_attempt = Some((attempt, classification));
            if !should_retry {
                break;
            }
        }

        let (attempt, _initial_classification) = last_attempt.expect("execution attempt should exist");
        let stdout = attempt.stdout;
        let stderr = attempt.stderr;
        let exit_code = attempt.exit_code;
        let timed_out = attempt.timed_out;

        // 3. Scrub secrets from both stderr and stdout
        let scrubbed_stderr = scrubber::scrub_secrets(&stderr);
        let scrubbed_stdout = scrubber::scrub_secrets(&stdout);
        let normalized_stderr = normalize_stderr(&scrubbed_stderr);

        // 4. Classify
        let classification = classify(exit_code, &scrubbed_stderr, &scrubbed_stdout, timed_out, &detector.shell);

        // 5. Compact output
        let compacted = compact::compact(&scrubbed_stdout, None);
        let final_stdout = if compacted.compacted {
            compacted.content
        } else {
            scrubbed_stdout
        };

        // 6. Recovery suggestion
        let normalized_command = normalize_command(&req.command);
        let memory_pattern = if classification.code != RecoveryCode::R10 {
            self.store.find_pattern(&normalized_command, &normalized_stderr).await?
        } else {
            None
        };
        let suggestion = if let Some(pattern) = memory_pattern.as_ref().filter(|pattern| {
            pattern.fix_command.is_some() && pattern.fix_success_rate >= 0.5
        }) {
            pattern_to_suggestion(pattern, &req.command)
        } else {
            suggest::suggest(
                classification.code,
                &req.command,
                &classification.reason,
                &detector,
            )
        };

        // 7. Persist output
        let output_id = self.store.next_output_id();
        let _ = self.store.save_output(
            &output_id,
            &req.command,
            &final_stdout,
            &scrubbed_stderr,
            exit_code,
        ).await;

        if classification.code != RecoveryCode::R10 && memory_pattern.is_none() {
            let learned_pattern = Pattern {
                id: None,
                command_hash: hash_command(&normalized_command),
                command_template: normalized_command.clone(),
                recovery_code: classification.code.to_string(),
                stderr_pattern: normalized_stderr.clone(),
                fix_command: None,
                fix_success_rate: 0.0,
                last_used: Some(chrono::Utc::now()),
                usage_count: 1,
            };
            let _ = self.store.save_pattern(&learned_pattern).await;
        }

        let status = if classification.code == RecoveryCode::R10 {
            "success"
        } else {
            "failed"
        }
        .to_string();

        Ok(ExecResult {
            status,
            recovery_code: classification.code.to_string(),
            recovery_class: classification.code.class_name().to_string(),
            original_command: req.command.clone(),
            output_id: Some(output_id),
            suggestion: serde_json::to_value(suggestion)?,
            output: OutputInfo {
                stdout: final_stdout,
                stderr: scrubbed_stderr,
                exit_code,
                truncated: compacted.compacted,
            },
        })
    }

    async fn execute_once(&self, req: &ExecRequest, shell: &str) -> anyhow::Result<ExecutionAttempt> {
        let mut cmd = Command::new(shell);
        cmd.arg("-c").arg(&req.command);
        if let Some(cwd) = &req.cwd {
            let validated = paths::validate_cwd(cwd).map_err(|e| {
                anyhow::anyhow!("CWD validation failed: {}", e)
            })?;
            cmd.current_dir(validated);
        }
        for (k, v) in &req.env {
            if BLOCKED_ENV_KEYS.contains(&k.as_str()) {
                continue; // Silently skip security-sensitive env vars
            }
            cmd.env(k, v);
        }

        let effective_timeout = req.timeout.min(MAX_TIMEOUT_SECS);
        let output_res = timeout(Duration::from_secs(effective_timeout), cmd.output()).await;

        match output_res {
            Ok(Ok(output)) => Ok(ExecutionAttempt {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Failed to spawn process: {}", e)),
            Err(_) => Ok(ExecutionAttempt {
                stdout: String::new(),
                stderr: "Process timed out".to_string(),
                exit_code: 124,
                timed_out: true,
            }),
        }
    }

    fn posix_retry_request(req: &ExecRequest, fallback_shell: &str) -> ExecRequest {
        let mut retry_req = req.clone();
        retry_req.command = format!("{} -c {}", fallback_shell, shell_quote(&req.command));
        retry_req
    }
}

struct ExecutionAttempt {
    stdout: String,
    stderr: String,
    exit_code: i32,
    timed_out: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_store() -> Store {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("patterns.db");
        let _ = dir.keep();
        Store::new_at_path(db_path).unwrap()
    }

    fn test_request(command: &str, retry: bool) -> ExecRequest {
        ExecRequest {
            command: command.to_string(),
            cwd: None,
            timeout: 5,
            env: HashMap::new(),
            retry,
        }
    }

    #[tokio::test]
    async fn stores_output_id_and_pattern_for_failure() {
        let store = test_store();
        let runner = Runner::with_store(store);

        let result = runner.run(&test_request("nonexistent_command_xyz", false)).await.unwrap();

        assert!(result.output_id.is_some());
        let count = runner.store.pattern_count().await.unwrap();
        assert_eq!(count, 1);
        let pattern = runner
            .store
            .find_pattern_exact(
                &normalize_command("nonexistent_command_xyz"),
                &normalize_stderr("sh: 1: nonexistent_command_xyz: not found"),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(pattern.fix_command.is_none());
        assert_eq!(pattern.fix_success_rate, 0.0);
    }

    #[tokio::test]
    async fn reuses_learned_pattern_suggestion() {
        let store = test_store();
        store
            .save_pattern(&Pattern {
                id: None,
                command_hash: hash_command("sh -c 'echo FAILED 1>&2; exit 1'"),
                command_template: "sh -c 'echo FAILED 1>&2; exit 1'".to_string(),
                recovery_code: "R24".to_string(),
                stderr_pattern: "FAILED".to_string(),
                fix_command: Some("cargo test -- --nocapture".to_string()),
                fix_success_rate: 0.9,
                last_used: Some(chrono::Utc::now()),
                usage_count: 1,
            })
            .await
            .unwrap();
        let runner = Runner::with_store(store);

        let result = runner
            .run(&test_request("sh -c 'echo FAILED 1>&2; exit 1'", false))
            .await
            .unwrap();

        assert_eq!(result.suggestion["action"], "reuse_learned_fix");
    }

    #[test]
    fn builds_posix_retry_request_for_fallback_shell() {
        let req = test_request("[[ -n \"$BASH_VERSION\" ]] && echo ok", true);
        let retry = Runner::posix_retry_request(&req, "zsh");

        assert!(retry.command.starts_with("zsh -c '"));
        assert!(retry.command.contains("[[ -n"));
    }
}
