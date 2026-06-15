use super::{
    validator, BinaryHandling, CompactionHint, ExecRequest, ExecResult, NextAction, OutputInfo,
};
use crate::classify::normalize::normalize_stderr;
use crate::classify::{classify, taxonomy::RecoveryCode};
use crate::compact;
use crate::config::ReshellConfig;
use crate::env::Detector;
use crate::memory::metrics::Metrics;
use crate::memory::pattern::Pattern;
use crate::memory::Store;
use crate::recover::resolve::{resolve_suggestion, STDERR_PATTERN_MAX_BYTES};
use crate::sandbox::overlay::OverlaySandbox;
use crate::sandbox::paths;
use crate::sandbox::scrubber;
use crate::utils::{
    detect_binary, hash_command, normalize_command, shell_quote, summarize_binary, truncate_utf8,
};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Maximum allowed timeout in seconds (10 minutes).
const MAX_TIMEOUT_SECS: u64 = 600;

/// Environment variables that must not be injected via the env parameter.
static BLOCKED_ENV_KEYS: Lazy<HashSet<&str>> = Lazy::new(|| {
    HashSet::from([
        // Dynamic linker injection
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "LD_ORIGIN_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        // Core environment
        "PATH",
        "SHELL",
        "HOME",
        "USER",
        "LOGNAME",
        // Shell auto-execute on startup
        "BASH_ENV",
        "ENV",
        "PROMPT_COMMAND",
        // Language runtime injection
        "NODE_OPTIONS",
        "NODE_PATH",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "RUBYOPT",
        "PERL5OPT",
        "PERL5LIB",
        "JAVA_TOOL_OPTIONS",
        // Rust debugging
        "RUST_LOG",
        "RUST_BACKTRACE",
        // Shell field separator
        "IFS",
        // Temp directory redirection
        "TMPDIR",
        "TEMP",
        "TMP",
        // SSH/Docker agent hijacking
        "SSH_AUTH_SOCK",
        "DOCKER_HOST",
        // Git operation hijacking
        "GIT_EXEC_PATH",
        "GIT_TEMPLATE_DIR",
        "GIT_DIR",
        "GIT_WORK_TREE",
        // Editor/visual could execute arbitrary programs
        "EDITOR",
        "VISUAL",
        "FCEDIT",
    ])
});

pub struct Runner {
    store: Store,
    config: ReshellConfig,
    sandbox: Option<OverlaySandbox>,
    metrics: Arc<Metrics>,
}

/// Returns true for commands that are trivially simple (no pipes, redirects,
/// subshells, variables, or shell operators). Simple commands skip DB persistence
/// for successful results since there's nothing to learn from "echo hello".
fn is_simple_command(command: &str) -> bool {
    !command.contains('|')
        && !command.contains('>')
        && !command.contains('<')
        && !command.contains('$')
        && !command.contains('`')
        && !command.contains("&&")
        && !command.contains("||")
        && !command.contains(';')
        && !command.contains('&')
        && !command.contains('~')
}

impl Runner {
    pub fn new() -> anyhow::Result<Self> {
        let store = Store::new()?;
        Ok(Self {
            store,
            config: ReshellConfig::default(),
            sandbox: None,
            metrics: Arc::new(Metrics::new()),
        })
    }

    pub fn new_with_sandbox() -> anyhow::Result<Self> {
        let store = Store::new()?;
        Ok(Self {
            store,
            config: ReshellConfig::default(),
            sandbox: Some(OverlaySandbox::new()?),
            metrics: Arc::new(Metrics::new()),
        })
    }

    pub fn new_with_config(config: ReshellConfig) -> anyhow::Result<Self> {
        let store = Store::new()?;
        Ok(Self {
            store,
            config,
            sandbox: None,
            metrics: Arc::new(Metrics::new()),
        })
    }

    pub fn with_store(store: Store) -> Self {
        Self {
            store,
            config: ReshellConfig::default(),
            sandbox: None,
            metrics: Arc::new(Metrics::new()),
        }
    }

    pub fn with_store_and_metrics(store: Store, metrics: Arc<Metrics>) -> Self {
        Self {
            store,
            config: ReshellConfig::default(),
            sandbox: None,
            metrics,
        }
    }

    pub async fn run(&self, req: &ExecRequest) -> anyhow::Result<ExecResult> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut persistence_warnings: Vec<String> = Vec::new();

        self.metrics.record_exec();

        // 1. Validate
        if let Err(e) = validator::validate(&req.command) {
            return Ok(ExecResult {
                status: "failed".to_string(),
                recovery_code: RecoveryCode::R27.to_string(),
                recovery_class: RecoveryCode::R27.class_name().to_string(),
                original_command: req.command.clone(),
                execution_id,
                output_id: None,
                suggestion: serde_json::to_value(
                    resolve_suggestion(
                        &self.store,
                        RecoveryCode::R27,
                        &req.command,
                        &e,
                        None,
                        &Detector::default(),
                    )
                    .await?
                    .suggestion,
                )?,
                output: OutputInfo {
                    stdout: String::new(),
                    stderr: e.clone(),
                    exit_code: -1,
                    truncated: false,
                    binary_summary: None,
                },
                next_action: Some(NextAction {
                    tool: "rsh_recover".to_string(),
                    params: serde_json::json!({
                        "recovery_code": "R27",
                        "original_command": req.command,
                        "context": e,
                        "stderr": "",
                    }),
                    reason: "Command blocked by safety validator. Use rsh_recover for an alternative approach.".to_string(),
                }),
                compaction_hint: None,
                platform: Some(crate::memory::pattern::current_platform_tag().to_string()),
                warnings: vec![],
                auto_retry: None,
            });
        }

        // 2. Execute
        let detector = Detector::cached().await;
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
            let attempt = if let Some(ref sandbox) = self.sandbox {
                let req_clone = current_req.clone();
                let shell_owned = current_shell.to_string();
                sandbox.run(move || {
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(self.execute_once(&req_clone, &shell_owned))
                })?
            } else {
                self.execute_once(current_req, current_shell).await?
            };
            // Quick check: only classify enough to decide if retry is needed.
            // Full classification happens once after the loop on scrubbed output.
            let should_retry = req.retry && attempt_idx == 0 && retry_shell.is_some() && {
                let norm = normalize_stderr(&attempt.stderr);
                classify(
                    attempt.exit_code,
                    &norm,
                    &attempt.stdout,
                    attempt.timed_out,
                    &detector.shell,
                    Some(&self.config),
                )
                .code
                    == RecoveryCode::R25
            };
            if should_retry {
                self.metrics.record_auto_retry_r25();
            }
            last_attempt = Some(attempt);
            if !should_retry {
                break;
            }
        }

        let attempt = last_attempt.expect("execution attempt should exist");
        let stdout = attempt.stdout;
        let stderr = attempt.stderr;
        let exit_code = attempt.exit_code;
        let timed_out = attempt.timed_out;
        let warnings = attempt.warnings;

        // 2b. Binary output handling
        let (stdout, binary_summary) = match req.binary_handling {
            BinaryHandling::Allow => (stdout, None),
            _ => {
                let (is_binary, _mime) = detect_binary(stdout.as_bytes());
                if is_binary {
                    let summary = summarize_binary(stdout.as_bytes());
                    match req.binary_handling {
                        BinaryHandling::Reject => {
                            return Ok(ExecResult {
                                status: "failed".to_string(),
                                recovery_code: RecoveryCode::R30.to_string(),
                                recovery_class: RecoveryCode::R30.class_name().to_string(),
                                original_command: req.command.clone(),
                                execution_id,
                                output_id: None,
                                suggestion: serde_json::json!({"action":"escalate","confidence":"high","reason":"Binary output was rejected by binary_handling=reject"}),
                                output: OutputInfo {
                                    stdout: String::new(),
                                    stderr: scrubber::scrub_secrets(&stderr),
                                    exit_code,
                                    truncated: false,
                                    binary_summary: Some(summary),
                                },
                                next_action: None,
                                compaction_hint: None,
                                platform: Some(
                                    crate::memory::pattern::current_platform_tag().to_string(),
                                ),
                                warnings,
                                auto_retry: None,
                            });
                        }
                        _ => {
                            let summary_json =
                                serde_json::to_string_pretty(&summary).unwrap_or_default();
                            (summary_json, Some(summary))
                        }
                    }
                } else {
                    (stdout, None)
                }
            }
        };

        // 3. Scrub secrets from both stderr and stdout.
        // Skip costly scrubbing for small success outputs (no secrets expected).
        let scrubbed_stderr = if exit_code == 0 && stderr.len() < 256 {
            stderr.clone()
        } else {
            scrubber::scrub_secrets(&stderr)
        };
        let scrubbed_stdout = if exit_code == 0 && stdout.len() < 1024 {
            stdout.clone()
        } else {
            scrubber::scrub_secrets(&stdout)
        };

        // 3b. Normalize stderr for cross-shell classification
        let normalized_stderr = normalize_stderr(&scrubbed_stderr);

        // 4. Classify once on scrubbed+normalized stderr
        let classification = classify(
            exit_code,
            &normalized_stderr,
            &scrubbed_stdout,
            timed_out,
            &detector.shell,
            Some(&self.config),
        );

        if classification.code != RecoveryCode::R10 {
            self.metrics.record_failure();
        }

        // Fast path: for simple successful commands, skip DB persistence,
        // compaction, and recovery suggestion (nothing to learn from
        // "echo hello" succeeding).
        let is_simple = is_simple_command(&req.command);
        if classification.code == RecoveryCode::R10 && is_simple {
            return Ok(ExecResult {
                status: "success".to_string(),
                recovery_code: "R10".to_string(),
                recovery_class: "Success".to_string(),
                original_command: req.command.clone(),
                execution_id,
                output_id: None,
                suggestion: serde_json::json!({"action":"none","confidence":"high","reason":"Command succeeded (fast path)"}),
                output: OutputInfo {
                    stdout: scrubbed_stdout,
                    stderr: scrubbed_stderr,
                    exit_code,
                    truncated: false,
                    binary_summary,
                },
                next_action: None,
                compaction_hint: None,
                platform: Some(crate::memory::pattern::current_platform_tag().to_string()),
                warnings: persistence_warnings,
                auto_retry: None,
            });
        }

        // 4b. Audit log
        if let Err(e) = self
            .store
            .log_audit_entry(
                &hash_command(&normalize_command(&req.command)),
                &normalize_command(&req.command),
                req.cwd.as_deref(),
                exit_code,
                &classification.code.to_string(),
                true, // validation passed
            )
            .await
        {
            persistence_warnings.push(format!("Failed to write audit log: {}", e));
        }

        // 5. Compact output
        let compacted = compact::compact(&scrubbed_stdout, None);
        let raw_len = scrubbed_stdout.len() as u64;
        let final_stdout = if compacted.compacted {
            compacted.content
        } else {
            scrubbed_stdout
        };
        let compacted_len = final_stdout.len() as u64;
        self.metrics.record_output_savings(raw_len, compacted_len);

        // 6. Recovery suggestion (learned pattern when confident, else heuristics)
        let normalized_command = normalize_command(&req.command);
        let stderr_for_recover = truncate_utf8(&normalized_stderr, STDERR_PATTERN_MAX_BYTES);
        let recovery_start = Instant::now();
        let resolved = resolve_suggestion(
            &self.store,
            classification.code,
            &req.command,
            &classification.reason,
            Some(&normalized_stderr),
            &detector,
        )
        .await?;
        let recovery_ms = recovery_start.elapsed().as_millis() as u64;
        let recovery_success = resolved.matched_pattern_row;
        self.metrics.record_recovery(recovery_ms, recovery_success);
        let suggestion = resolved.suggestion;

        // 7. Persist output
        let output_id = self.store.next_output_id();
        if let Err(e) = self
            .store
            .save_output(
                &output_id,
                &execution_id,
                &req.command,
                &final_stdout,
                &scrubbed_stderr,
                exit_code,
            )
            .await
        {
            persistence_warnings.push(format!(
                "Failed to persist output (output_id={}): {}",
                output_id, e
            ));
        }

        if classification.code != RecoveryCode::R10 && !resolved.matched_pattern_row {
            // If the recovery suggestion includes a concrete fix command, save it
            // so the pattern can be reused on subsequent occurrences.
            let fix_command = suggestion.command.clone();
            let fix_success_rate = if fix_command.is_some() { 0.1 } else { 0.0 };

            let learned_pattern = Pattern {
                id: None,
                command_hash: hash_command(&normalized_command),
                command_template: normalized_command.clone(),
                recovery_code: classification.code.to_string(),
                stderr_pattern: normalized_stderr.clone(),
                fix_command,
                fix_success_rate,
                last_used: Some(chrono::Utc::now()),
                usage_count: 1,
                platform_tag: Some(crate::memory::pattern::current_platform_tag().to_string()),
            };
            if let Err(e) = self.store.save_pattern(&learned_pattern).await {
                persistence_warnings.push(format!("Failed to save learned pattern: {}", e));
            }
        }

        // If the command succeeded and it matches a known fix_command pattern,
        // auto-increment that pattern's success rate (feedback loop without agent action).
        if classification.code == RecoveryCode::R10 {
            if let Err(e) = self
                .store
                .auto_bump_fix_success(&normalized_command, true)
                .await
            {
                persistence_warnings.push(format!("Failed to auto-bump fix success: {}", e));
            }
        }

        // Auto-retry: for R22 (Command Not Found) with a high-confidence fix,
        // execute the fix command inline (via execute_once, avoiding recursive run()).
        let auto_retry_result = if classification.code == RecoveryCode::R22
            && suggestion.confidence == "high"
            && req.retry
        {
            if let Some(ref fix_cmd) = suggestion.command {
                if fix_cmd != &req.command {
                    self.metrics.record_auto_retry_r22();
                    let fix_req = ExecRequest {
                        command: fix_cmd.clone(),
                        cwd: req.cwd.clone(),
                        timeout: req.timeout.min(60), // cap at 60s for auto-retry
                        env: req.env.clone(),
                        retry: false,
                        binary_handling: req.binary_handling,
                    };
                    // Use execute_once directly (avoids recursive run() call)
                    let fix_attempt = self
                        .execute_once(&fix_req, &detector.execution_shell())
                        .await;
                    match fix_attempt {
                        Ok(attempt) => {
                            let success = attempt.exit_code == 0;
                            // Auto-record feedback
                            let _ = self
                                .store
                                .update_fix_outcome(
                                    &normalized_command,
                                    &normalized_stderr,
                                    Some(fix_cmd),
                                    success,
                                )
                                .await;
                            Some(Box::new(ExecResult {
                                status: if success {
                                    "success".to_string()
                                } else {
                                    "failed".to_string()
                                },
                                recovery_code: if success {
                                    "R10".to_string()
                                } else {
                                    "R22".to_string()
                                },
                                recovery_class: if success {
                                    "Success".to_string()
                                } else {
                                    "Command Not Found".to_string()
                                },
                                original_command: fix_req.command.clone(),
                                execution_id: uuid::Uuid::new_v4().to_string(),
                                output_id: None,
                                suggestion: serde_json::json!({"action":"none","confidence":"high","reason":"Auto-retry of R22 fix"}),
                                output: OutputInfo {
                                    stdout: attempt.stdout,
                                    stderr: attempt.stderr,
                                    exit_code: attempt.exit_code,
                                    truncated: false,
                                    binary_summary: None,
                                },
                                next_action: None,
                                compaction_hint: None,
                                platform: Some(
                                    crate::memory::pattern::current_platform_tag().to_string(),
                                ),
                                warnings: attempt.warnings,
                                auto_retry: None,
                            }))
                        }
                        Err(_) => None,
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let status = if classification.code == RecoveryCode::R10 {
            "success"
        } else {
            "failed"
        }
        .to_string();

        // Build next_action hint so the agent knows exactly which tool to call next
        let next_action = if classification.code != RecoveryCode::R10 {
            Some(NextAction {
                tool: "rsh_recover".to_string(),
                params: serde_json::json!({
                    "recovery_code": classification.code.to_string(),
                    "original_command": req.command,
                    "context": classification.reason,
                    "stderr": stderr_for_recover,
                }),
                reason: format!(
                    "Command failed with {}. Use rsh_recover to get a deterministic fix suggestion.",
                    classification.code.class_name()
                ),
            })
        } else {
            None
        };

        // Build compaction_hint if output was truncated
        let compaction_hint = if compacted.compacted {
            Some(CompactionHint {
                tool: "rsh_compact".to_string(),
                params: serde_json::json!({
                    "output_id": output_id,
                    "view": "skeleton",
                }),
                reason: "Output was truncated to stay within context limits. Use rsh_compact with view='skeleton' for a structural summary, view='diff' to see changes, or view='errors_only' to see only error/warning lines.".to_string(),
            })
        } else {
            None
        };

        let platform_tag = crate::memory::pattern::current_platform_tag().to_string();

        Ok(ExecResult {
            status,
            recovery_code: classification.code.to_string(),
            recovery_class: classification.code.class_name().to_string(),
            original_command: req.command.clone(),
            execution_id,
            output_id: Some(output_id),
            suggestion: serde_json::to_value(suggestion)?,
            output: OutputInfo {
                stdout: final_stdout,
                stderr: scrubbed_stderr,
                exit_code,
                truncated: compacted.compacted,
                binary_summary,
            },
            next_action,
            compaction_hint,
            platform: Some(platform_tag),
            warnings: warnings.into_iter().chain(persistence_warnings).collect(),
            auto_retry: auto_retry_result,
        })
    }

    async fn execute_once(
        &self,
        req: &ExecRequest,
        shell: &str,
    ) -> anyhow::Result<ExecutionAttempt> {
        let mut cmd = Command::new(shell);
        cmd.arg("-c").arg(&req.command);
        if let Some(cwd) = &req.cwd {
            let validated = paths::validate_cwd(cwd)
                .map_err(|e| anyhow::anyhow!("CWD validation failed: {}", e))?;
            cmd.current_dir(validated);
        }
        let mut warnings = Vec::new();
        for (k, v) in &req.env {
            let blocked = BLOCKED_ENV_KEYS.contains(&k.as_str())
                || crate::config::get()
                    .sandbox
                    .additional_blocked_env
                    .contains(&k.to_string());
            let allowed = crate::config::get()
                .sandbox
                .allowed_env
                .contains(&k.to_string());

            if blocked && !allowed {
                warnings.push(format!("Blocked security-sensitive env var: {}", k));
                continue;
            }
            cmd.env(k, v);
        }

        // Apply seccomp sandbox if configured
        if crate::config::get().sandbox.seccomp {
            #[cfg(unix)]
            {
                use crate::sandbox::seccomp;
                if seccomp::is_seccomp_available() {
                    unsafe {
                        cmd.pre_exec(|| {
                            seccomp::apply_seccomp_filter().map_err(std::io::Error::other)
                        });
                    }
                }
            }
        }

        let effective_timeout = req.timeout.min(MAX_TIMEOUT_SECS);
        let output_res = timeout(Duration::from_secs(effective_timeout), cmd.output()).await;

        match output_res {
            Ok(Ok(output)) => Ok(ExecutionAttempt {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                timed_out: false,
                warnings,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Failed to spawn process: {}", e)),
            Err(_) => Ok(ExecutionAttempt {
                stdout: String::new(),
                stderr: "Process timed out".to_string(),
                exit_code: 124,
                timed_out: true,
                warnings,
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
    warnings: Vec<String>,
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
            binary_handling: super::BinaryHandling::Summary,
        }
    }

    #[tokio::test]
    async fn stores_output_id_and_pattern_for_failure() {
        let store = test_store();
        let runner = Runner::with_store(store);

        let result = runner
            .run(&test_request("nonexistent_command_xyz", false))
            .await
            .unwrap();

        assert!(result.output_id.is_some());
        let count = runner.store.pattern_count().await.unwrap();
        assert_eq!(count, 1);
        // Use the actual normalized stderr from the command output
        // (cross-platform: shell error formats differ between bash, dash, etc.)
        let normalized_cmd = normalize_command("nonexistent_command_xyz");
        let normalized_err = normalize_stderr(&result.output.stderr);
        assert!(!normalized_err.is_empty(), "stderr should have content");
        let pattern = runner
            .store
            .find_pattern_exact(&normalized_cmd, &normalized_err)
            .await
            .unwrap()
            .unwrap();
        // fix_command may be set if the recovery suggestion produced a concrete command
        if pattern.fix_command.is_some() {
            assert_eq!(pattern.fix_success_rate, 0.1);
        } else {
            assert_eq!(pattern.fix_success_rate, 0.0);
        }
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
                platform_tag: Some("linux".to_string()),
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

    #[tokio::test]
    async fn binary_stdout_returns_summary() {
        let store = test_store();
        let runner = Runner::with_store(store);
        // PNG magic bytes via printf
        let req = ExecRequest {
            command: "printf '\\000\\001\\002\\003'".to_string(),
            cwd: None,
            timeout: 5,
            env: HashMap::new(),
            retry: false,
            binary_handling: super::BinaryHandling::Summary,
        };

        let result = runner.run(&req).await.unwrap();
        assert_eq!(result.status, "success");
        assert!(
            result.output.binary_summary.is_some(),
            "binary_summary should be present"
        );
        assert!(result.output.stdout.contains("byte_count"));
    }
}
