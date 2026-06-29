use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::classify::taxonomy::RecoveryCode;
use crate::compact::view::{render_view, CompactView};
use crate::env::Detector;
use crate::exec::{runner::Runner, ExecRequest, ExecResult, NextAction, OutputInfo};
use crate::memory::Store;
use crate::recover::resolve::resolve_suggestion;
use crate::sandbox::paths;

use super::router::{CallContext, ServerState};

pub fn list_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "rsh_exec",
            "description": "Execute ANY shell command. USE THIS INSTEAD OF RAW BASH for every command. Secrets are auto-redacted. On failure, high-confidence fixes (missing tools) are auto-applied (check auto_retry field). Failures return a concrete fix command in suggestion.command. Successes are compacted. Call rsh_feedback after trying a fix to build pattern memory. If a command returns recovery_code R28 (Approval Required), re-issue the SAME command with approve:true after a human approves.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute (required)" },
                    "cwd": { "type": "string", "description": "Working directory (default: current directory)" },
                    "timeout": { "type": "integer", "default": 120, "description": "Timeout in seconds (max 600)" },
                    "approve": { "type": "boolean", "default": false, "description": "Set to true to execute a command previously returned as R28 (Approval Required), after human review." }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "rsh_env",
            "description": "Detect and describe the current shell environment: OS, shell type/version, available dev tools, package manager. Call at session start. Pass refresh=true after installing new tools.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "refresh": { "type": "boolean", "description": "Force re-detection of environment (useful after installing new tools)" }
                }
            }
        }),
        json!({
            "name": "rsh_recover",
            "description": "Get a deterministic recovery strategy for a known failure. CALL THIS when rsh_exec returns status='failed' with a recovery_code other than R10. You can either pass the execution_id from the failed rsh_exec response (simplest), or pass recovery_code and original_command explicitly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "execution_id": { "type": "string", "description": "The execution_id from the failed rsh_exec response. If provided, all other context is resolved automatically." },
                    "recovery_code": { "type": "string", "description": "Recovery code from rsh_exec response (R20-R30). Required if execution_id is not provided." },
                    "original_command": { "type": "string", "description": "The command that failed (from rsh_exec original_command). Required if execution_id is not provided." },
                    "context": { "type": "string", "description": "Optional additional context about the failure" },
                    "stderr": { "type": "string", "description": "Normalized stderr from rsh_exec (pass next_action.stderr when present) for learned-pattern lookup" }
                }
            }
        }),
        json!({
            "name": "rsh_compact",
            "description": "Retrieve a compacted view of a previously stored large output. Use when rsh_exec returns truncated=true or when inspecting previous command outputs. Views: 'skeleton' (structural summary — function defs, error/warn lines, class/struct defs), 'diff' (only new lines since previous read), 'errors_only' (only ERROR/WARN/FATAL lines), 'full' (complete output). Use 'path' for jq-like JSON extraction (e.g., '.items[0].name').",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "output_id": { "type": "string", "description": "The output_id from a previous rsh_exec response" },
                    "file": { "type": "string", "description": "Path to a file to compact (must be within working directory)" },
                    "view": { "type": "string", "enum": ["full", "skeleton", "diff", "errors_only"], "default": "skeleton", "description": "Compaction view to apply" },
                    "path": { "type": "string", "description": "Optional jq-like JSON path to extract (e.g., '.items[0].name')" }
                }
            }
        }),
        json!({
            "name": "rsh_read_file",
            "description": "Read a file through the reshell safety sandbox. Path traversal (..), symlinks outside the working directory, and sensitive system files (/etc/shadow, /proc, /sys, /dev) are blocked. USE THIS instead of raw cat/read for any file path that might be user-supplied or untrusted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to read (relative or absolute within allowed locations)" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "rsh_write_file",
            "description": "Write content to a file through the reshell safety sandbox. Same security rules as rsh_read_file. Creates parent directories if needed. USE THIS instead of shell redirection (> or tee) for writing files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to write to" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "rsh_check",
            "description": "Quick health check and onboarding guide. Call this at the start of a session to verify reshell is functioning and to get usage guidance for the recovery pipeline (rsh_exec → rsh_recover → rsh_compact).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "rsh_feedback",
            "description": "Report whether a recovery fix succeeded or failed. CALL THIS after you retry a failed command using a fix suggested by rsh_recover. This updates the pattern memory so future occurrences get higher-confidence suggestions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "original_command": { "type": "string", "description": "The command that originally failed (from the rsh_exec response's original_command field)" },
                    "stderr": { "type": "string", "description": "The stderr from the original failure (from rsh_exec output.stderr or next_action.params.stderr)" },
                    "fix_command": { "type": "string", "description": "The command you actually ran that fixed the issue" },
                    "success": { "type": "boolean", "description": "Whether the fix resolved the failure" }
                },
                "required": ["original_command", "fix_command", "success"]
            }
        }),
        json!({
            "name": "rsh_stats",
            "description": "Get statistics about rsh pattern memory: recovery attempt counts, top fixing patterns, and command failure rates. Use for diagnostics and understanding learning effectiveness.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    ]
}

/// Handle a tool call. Returns `(data_value, is_error)` where `data_value` is the
/// tool-specific result payload (NOT wrapped in a ToolResponse envelope).
///
/// `call_ctx` carries the session id (per server process) and the JSON-RPC
/// request id (per call) so every audit row is traceable end-to-end.
pub(crate) async fn handle_tool_call(
    name: &str,
    arguments: Value,
    state: &Arc<Mutex<ServerState>>,
    call_ctx: CallContext,
) -> (Value, bool) {
    match name {
        "rsh_exec" => {
            let wrapper = match serde_json::from_value::<ExecRequestWrapper>(arguments.clone()) {
                Ok(w) => w,
                Err(e) => {
                    return (
                        json!({ "error": format!("Invalid arguments: {}", e) }),
                        true,
                    );
                }
            };
            // Scrub the raw arguments before persisting to the audit log so no
            // secret in env/args is ever written to disk. The scrubber is a no-op
            // for short, low-entropy strings, so this is cheap on the happy path.
            let scrubbed_args = crate::sandbox::scrubber::scrub_secrets(&arguments.to_string());
            let req = ExecRequest {
                command: wrapper.command,
                cwd: wrapper.cwd,
                timeout: wrapper.timeout.unwrap_or(120).min(600),
                env: wrapper.env.unwrap_or_default(),
                retry: wrapper.retry.unwrap_or(true),
                binary_handling: wrapper.binary_handling,
                call_ctx: Some(crate::exec::ExecCallContext {
                    session_id: call_ctx.session_id.clone(),
                    request_id: call_ctx.request_id.clone(),
                }),
                raw_args_json: Some(scrubbed_args),
                approve: wrapper.approve.unwrap_or(false),
            };
            let metrics = {
                let s = state.lock().await;
                s.metrics.clone()
            };
            let cwd_path = req.cwd.as_deref().map(std::path::Path::new);
            let store = match Store::new_for_cwd(cwd_path) {
                Ok(s) => s,
                Err(e) => {
                    return (
                        json!({ "error": format!("Failed to open pattern store: {}", e) }),
                        true,
                    );
                }
            };

            // ── Budget guardrail (pre-exec check) ──────────────────────────
            // Invocations and wall-time are pre-checkable; output bytes are
            // charged after execution (size is unknown in advance).
            let budget_cfg = crate::config::get().budget.clone();
            if !budget_cfg.is_unlimited() {
                // Session-scoped caps (in-process atomics in shared Router state).
                let session_decision = {
                    let s = state.lock().await;
                    crate::budget::check_session(&s.budget, &budget_cfg)
                };
                if let Some(r29) = budget_refusal(&session_decision, &req, &call_ctx, &store).await
                {
                    return (json!(r29), true);
                }

                // Persistent hourly/daily caps (SQLite ledger).
                if budget_cfg.max_invocations_per_hour > 0 || budget_cfg.max_invocations_per_day > 0
                {
                    let windows = crate::budget::current_windows(now_secs());
                    let mut blocked = None;
                    for w in windows {
                        let usage = store
                            .budget_window_usage(w.bucket, w.window_key)
                            .await
                            .unwrap_or_default();
                        let limit = if w.bucket == "hourly" {
                            budget_cfg.max_invocations_per_hour
                        } else {
                            budget_cfg.max_invocations_per_day
                        } as i64;
                        if limit > 0 && usage.invocations >= limit {
                            blocked = Some((w.bucket, usage.invocations as u64, limit as u64));
                            break;
                        }
                    }
                    if let Some((bucket, used, limit)) = blocked {
                        let code = "R29";
                        let _ = audit_tool_call(
                            &store,
                            &call_ctx,
                            "rsh_exec",
                            &req.command,
                            &format!("{{\"budget\":\"{}\"}}", bucket),
                            -1,
                            code,
                        )
                        .await;
                        return (
                            json!(budget_exhausted_result(&req.command, bucket, used, limit,)),
                            true,
                        );
                    }
                }
            }

            // Bump the session invocation counter now (the call is going ahead).
            let budget_started = std::time::Instant::now();
            if !budget_cfg.is_unlimited() {
                let s = state.lock().await;
                s.budget.inc_invocation();
            }

            let runner = Runner::with_store_and_metrics(store.clone(), metrics);
            let run_result = runner.run(&req).await;

            // ── Budget charge (post-exec): actual stdout bytes + wall time ──
            if !budget_cfg.is_unlimited() {
                let wall_secs = budget_started.elapsed().as_secs();
                let output_bytes = match &run_result {
                    Ok(r) => r.output.stdout.len() as u64,
                    Err(_) => 0,
                };
                {
                    let s = state.lock().await;
                    s.budget.add_output_bytes(output_bytes);
                    s.budget.add_wall_secs(wall_secs);
                }
                // Charge the persistent daily window (hourly invocation caps are
                // pre-checked above; we still record bytes/secs on the daily window
                // for completeness and future daily byte caps).
                let windows = crate::budget::current_windows(now_secs());
                if let Some(daily) = windows.iter().find(|w| w.bucket == "daily") {
                    let _ = store
                        .record_budget_charge(
                            daily.bucket,
                            daily.window_key,
                            1,
                            output_bytes as i64,
                            wall_secs as i64,
                        )
                        .await;
                }
            }

            match run_result {
                Ok(result) => {
                    let is_error = result.status == "failed";
                    (json!(result), is_error)
                }
                Err(e) => (json!({ "error": e.to_string() }), true),
            }
        }
        "rsh_env" => {
            let refresh = arguments
                .get("refresh")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if refresh {
                Detector::invalidate_cache().await;
            }
            let detector = Detector::cached().await;
            (json!(detector), false)
        }
        "rsh_check" => {
            let detector = Detector::cached().await;
            let store = {
                let s = state.lock().await;
                s.store.clone()
            };
            let (pattern_count, store_status) = match store.pattern_count().await {
                Ok(n) => (n, "ok".to_string()),
                Err(e) => (0, format!("degraded: {}", e)),
            };
            let healthy = store_status == "ok";

            // Gather pattern stats
            let (fixes_count, avg_success_rate) = if healthy {
                let fixes = store.patterns_with_fixes_count().await.unwrap_or(0);
                let avg = store.average_fix_success_rate().await.unwrap_or(0.0);
                (fixes, avg)
            } else {
                (0, 0.0)
            };

            let guidance = json!({
                "status": if healthy { "healthy" } else { "degraded" },
                "store": store_status,
                "environment": {
                    "os": detector.os,
                    "shell": detector.shell,
                    "package_manager": detector.package_manager,
                },
                "usage": {
                    "workflow": "rsh_exec → on failure check next_action → call rsh_recover → rsh_exec (retry with fix) → rsh_feedback to record outcome",
                    "tools": {
                        "rsh_exec": "Execute commands with automatic failure classification and recovery hints. Always use this instead of raw bash.",
                        "rsh_recover": "Call when rsh_exec returns status='failed'. Pass recovery_code and original_command from the response.",
                        "rsh_compact": "Call when rsh_exec returns truncated=true. Use the output_id from the response with view='skeleton' for structural summary.",
                        "rsh_env": "Detect OS, shell, available dev tools, and package manager. Call at session start.",
                        "rsh_check": "This tool — verify reshell is working and get usage guidance.",
                        "rsh_feedback": "CRITICAL: Call EVERY TIME you try a fix from rsh_recover.\n  Example: after rsh_exec 'brew install gh' succeeds, call rsh_feedback\n  { original_command: 'gh pr view', fix_command: 'brew install gh', success: true }.\n  This builds pattern memory so future failures resolve instantly.\n  If fix FAILS, report success: false to downgrade the pattern."
                    },
                    "recovery_codes": {
                        "R10": "Success — no action needed.",
                        "R20": "Syntax Error — check --help for correct flags.",
                        "R21": "Permission Denied — try with sudo or fix file permissions.",
                        "R22": "Command Not Found — install the missing tool via package manager.",
                        "R23": "Timeout — increase timeout or run in smaller chunks.",
                        "R24": "Subcommand Failure — run diagnostic command to investigate.",
                        "R25": "Environment Mismatch — use POSIX-compatible syntax. rsh will auto-retry with fallback shell.",
                        "R26": "Output Overflow — use rsh_compact to get a compacted view.",
                        "R27": "Blocked / Safety Violation — command was blocked by the safety validator.",
                        "R28": "Approval Required — high-risk command; re-issue with approve:true after a human approves.",
                        "R29": "Budget Exhausted — a configured call/byte/time cap was hit; wait for the window to reset.",
                        "R30": "Fatal/Unknown — requires human escalation."
                    }
                },
                "learned_patterns": {
                    "total": pattern_count,
                    "with_fixes": fixes_count,
                    "average_fix_success_rate": avg_success_rate
                },
            });
            (guidance, !healthy)
        }
        "rsh_recover" => {
            let req = match serde_json::from_value::<RecoverRequest>(arguments) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        json!({ "error": format!("Invalid arguments: {}", e) }),
                        true,
                    );
                }
            };

            let store = {
                let s = state.lock().await;
                s.store.clone()
            };

            // Resolve from execution_id if provided
            let (recovery_code, original_command, context, stderr) =
                if let Some(ref exec_id) = req.execution_id {
                    match store.get_output_by_execution_id(exec_id).await {
                        Ok(Some(output)) => {
                            let stderr = output.stderr;
                            // For recovery_code, we don't store it in outputs table.
                            // Fall back to explicit params or re-classify from exit_code and stderr.
                            let code = req
                                .recovery_code
                                .clone()
                                .unwrap_or_else(|| "R30".to_string());
                            let command = req
                                .original_command
                                .clone()
                                .unwrap_or(output.original_command);
                            let ctx = req
                                .context
                                .clone()
                                .unwrap_or_else(|| format!("execution_id={}", exec_id));
                            (code, command, ctx, Some(stderr))
                        }
                        Ok(None) => {
                            return (
                                json!({ "error": format!("Unknown execution_id: {}", exec_id) }),
                                true,
                            );
                        }
                        Err(e) => {
                            return (
                                json!({ "error": format!("Failed to lookup execution_id: {}", e) }),
                                true,
                            );
                        }
                    }
                } else {
                    // Traditional explicit params
                    let code = req
                        .recovery_code
                        .clone()
                        .unwrap_or_else(|| "R30".to_string());
                    let command = req
                        .original_command
                        .clone()
                        .unwrap_or_else(|| "".to_string());
                    let ctx = req.context.clone().unwrap_or_default();
                    (code, command, ctx, req.stderr.clone())
                };

            let code = parse_recovery_code(&recovery_code);
            let detector = Detector::cached().await;

            let resolved = match resolve_suggestion(
                &store,
                code,
                &original_command,
                &context,
                stderr.as_deref(),
                &detector,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    return (json!({ "error": format!("Recovery failed: {}", e) }), true);
                }
            };
            let suggestion = resolved.suggestion;

            // Log telemetry: a recovery_attempts row (historical) plus a full
            // audit row so the call appears in rsh_stats' recent slice.
            let store_clone = {
                let s = state.lock().await;
                s.store.clone()
            };
            let _ = store_clone
                .log_recovery_attempt(&recovery_code, &original_command, &suggestion.action)
                .await;
            let _ = audit_tool_call(
                &store_clone,
                &call_ctx,
                "rsh_recover",
                &original_command,
                json!({ "recovery_code": recovery_code })
                    .to_string()
                    .as_str(),
                0,
                &recovery_code,
            )
            .await;

            (json!(suggestion), false)
        }
        "rsh_compact" => {
            let req = match serde_json::from_value::<CompactRequest>(arguments) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        json!({ "error": format!("Invalid arguments: {}", e) }),
                        true,
                    );
                }
            };
            let store = {
                let s = state.lock().await;
                s.store.clone()
            };
            let view = CompactView::parse(req.view.as_deref().unwrap_or("skeleton"));

            if let Some(ref jq_path) = req.path {
                let content = if let Some(ref file_path) = req.file {
                    match paths::validate_and_read_file(file_path) {
                        Ok((_path, c)) => c,
                        Err(e) => {
                            return (
                                json!({ "error": format!("File access failed: {}", e) }),
                                true,
                            );
                        }
                    }
                } else if let Some(ref output_id) = req.output_id {
                    match store.get_output(output_id).await {
                        Ok(Some(output)) => output.stdout,
                        Ok(None) => {
                            return (
                                json!({ "error": format!("Unknown output_id: {}", output_id) }),
                                true,
                            );
                        }
                        Err(e) => {
                            return (
                                json!({ "error": format!("Failed to fetch output: {}", e) }),
                                true,
                            );
                        }
                    }
                } else {
                    return (
                        json!({ "error": "path requires file or output_id to read from" }),
                        true,
                    );
                };
                match crate::compact::jq::extract_json_path(&content, jq_path) {
                    Ok(extracted) => {
                        return (
                            json!({ "status": "success", "extracted": extracted }),
                            false,
                        );
                    }
                    Err(e) => {
                        return (
                            json!({ "error": format!("jq extraction failed: {}", e) }),
                            true,
                        );
                    }
                }
            }

            if let Some(file_path) = req.file {
                match paths::validate_and_read_file(&file_path) {
                    Ok((_path, content)) => {
                        let compacted = render_view(&content, view, None, None);
                        (json!(compacted), false)
                    }
                    Err(e) => (
                        json!({ "error": format!("File access failed: {}", e) }),
                        true,
                    ),
                }
            } else if let Some(output_id) = req.output_id {
                match store.get_output(&output_id).await {
                    Ok(Some(output)) => {
                        let previous = if matches!(view, CompactView::Diff) {
                            store
                                .previous_output(&output.output_id)
                                .await
                                .ok()
                                .flatten()
                                .map(|previous| previous.stdout)
                        } else {
                            None
                        };
                        let compacted = render_view(
                            &output.stdout,
                            view,
                            previous.as_deref(),
                            Some(output.output_id),
                        );
                        (json!(compacted), false)
                    }
                    Ok(None) => (
                        json!({ "error": format!("Unknown output_id: {}", output_id) }),
                        true,
                    ),
                    Err(e) => (
                        json!({ "error": format!("Failed to fetch output: {}", e) }),
                        true,
                    ),
                }
            } else {
                (
                    json!({ "error": "No file or output_id provided for compact" }),
                    true,
                )
            }
        }
        "rsh_feedback" => {
            let req = match serde_json::from_value::<FeedbackRequest>(arguments) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        json!({ "error": format!("Invalid arguments: {}", e) }),
                        true,
                    );
                }
            };
            let store = {
                let s = state.lock().await;
                s.store.clone()
            };

            let command_template = crate::utils::normalize_command(&req.original_command);
            let stderr_for_lookup = req
                .stderr
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    crate::utils::truncate_utf8(
                        s,
                        super::super::recover::resolve::STDERR_PATTERN_MAX_BYTES,
                    )
                })
                .unwrap_or_default();

            match store
                .update_fix_outcome(
                    &command_template,
                    &stderr_for_lookup,
                    Some(&req.fix_command),
                    req.success,
                )
                .await
            {
                Ok(()) => {
                    let _ = audit_tool_call(
                        &store,
                        &call_ctx,
                        "rsh_feedback",
                        &req.original_command,
                        json!({ "fix_command": req.fix_command, "success": req.success })
                            .to_string()
                            .as_str(),
                        0,
                        "R10",
                    )
                    .await;
                    (
                        json!({ "status": "recorded", "success": req.success, "fix_command": req.fix_command }),
                        false,
                    )
                }
                Err(e) => (
                    json!({ "error": format!("Failed to record feedback: {}", e) }),
                    true,
                ),
            }
        }
        "rsh_stats" => {
            let (store, metrics) = {
                let s = state.lock().await;
                (s.store.clone(), s.metrics.clone())
            };
            let snapshot = metrics.snapshot();
            let recovery_counts = store.recovery_attempt_counts().await.unwrap_or_default();
            let pattern_fixes = store.patterns_with_fixes_count().await.unwrap_or(0);
            let avg_success = store.average_fix_success_rate().await.unwrap_or(0.0);
            let by_code = store.pattern_counts_by_code().await.unwrap_or_default();
            let total_patterns = store.pattern_count().await.unwrap_or(0);
            // Recent audit slice: the last 10 invocations across the tool surface,
            // for tracing what the agent has been doing (MCP "trace and improve").
            let recent = store.recent_audit_entries(10).await.unwrap_or_default();
            let recent_json: Vec<Value> = recent
                .into_iter()
                .map(|e| {
                    json!({
                        "at": e.executed_at,
                        "tool": e.tool,
                        "session_id": e.session_id,
                        "request_id": e.request_id,
                        "recovery_code": e.recovery_code,
                        "exit_code": e.exit_code,
                        "command": e.raw_command.unwrap_or(e.command_template),
                    })
                })
                .collect();

            let stats = json!({
                "metrics": {
                    "total_executions": snapshot.total_execs,
                    "total_failures": snapshot.total_failures,
                    "recovery_rate": format!("{:.1}%", snapshot.recovery_rate * 100.0),
                    "false_positive_rate": format!("{:.1}%", snapshot.false_positive_rate * 100.0),
                    "context_savings": format!("{:.1}%", snapshot.context_savings_pct),
                    "avg_recovery_time_ms": snapshot.avg_recovery_time_ms as u64,
                    "auto_retries_r22": snapshot.auto_retries_r22,
                    "auto_retries_r25": snapshot.auto_retries_r25,
                },
                "patterns": {
                    "total": total_patterns,
                    "with_fixes": pattern_fixes,
                    "average_fix_success_rate": avg_success,
                    "by_recovery_code": by_code
                },
                "recovery_attempts": {
                    "total": recovery_counts.iter().map(|(_, c)| c).sum::<i64>(),
                    "by_code": recovery_counts
                },
                "recent_invocations": recent_json
            });
            (stats, false)
        }
        "rsh_read_file" => {
            let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match paths::validate_and_read_file(path) {
                Ok((resolved_path, content)) => (
                    json!({
                        "path": resolved_path.to_string_lossy(),
                        "content": content,
                        "line_count": content.lines().count(),
                    }),
                    false,
                ),
                Err(e) => (json!({"error": format!("File read blocked: {}", e)}), true),
            }
        }
        "rsh_write_file" => {
            let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match paths::validate_and_create_file(path, content) {
                Ok(resolved_path) => {
                    // Audit the write so the tool-call trace covers mutating tools.
                    let store = {
                        let s = state.lock().await;
                        s.store.clone()
                    };
                    let _ = audit_tool_call(
                        &store,
                        &call_ctx,
                        "rsh_write_file",
                        path,
                        json!({ "path": path, "bytes": content.len() })
                            .to_string()
                            .as_str(),
                        0,
                        "R10",
                    )
                    .await;
                    (
                        json!({
                            "path": resolved_path.to_string_lossy(),
                            "bytes_written": content.len(),
                        }),
                        false,
                    )
                }
                Err(e) => (json!({"error": format!("File write blocked: {}", e)}), true),
            }
        }
        _ => (json!({ "error": format!("Unknown tool: {}", name) }), true),
    }
}

fn parse_recovery_code(code: &str) -> RecoveryCode {
    match code {
        "R10" => RecoveryCode::R10,
        "R20" => RecoveryCode::R20,
        "R21" => RecoveryCode::R21,
        "R22" => RecoveryCode::R22,
        "R23" => RecoveryCode::R23,
        "R24" => RecoveryCode::R24,
        "R25" => RecoveryCode::R25,
        "R26" => RecoveryCode::R26,
        "R27" => RecoveryCode::R27,
        "R28" => RecoveryCode::R28,
        "R29" => RecoveryCode::R29,
        _ => RecoveryCode::R30,
    }
}

/// Write a best-effort audit row for a non-`rsh_exec` tool call so the audit
/// log covers the whole tool surface, not just command execution. Failures are
/// swallowed (audit is telemetry, never on the request's critical path).
async fn audit_tool_call(
    store: &crate::memory::Store,
    call_ctx: &CallContext,
    tool: &str,
    raw_command: &str,
    args_json: &str,
    exit_code: i32,
    recovery_code: &str,
) {
    let normalized = crate::utils::normalize_command(raw_command);
    let hash = crate::utils::hash_command(&normalized);
    let _ = store
        .log_audit(crate::memory::AuditRecord {
            command_hash: &hash,
            command_template: &normalized,
            cwd: None,
            exit_code,
            recovery_code,
            validation_passed: true,
            session_id: Some(&call_ctx.session_id),
            request_id: call_ctx.request_id_str(),
            tool: Some(tool),
            raw_command: Some(raw_command),
            args_json: Some(args_json),
            wall_ms: None,
        })
        .await;
}

/// Current wall-clock time in seconds since the Unix epoch, for budget windows.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// If the session-scoped budget decision is Exhausted, write an R29 audit row
/// and return the structured refusal result for the caller to return.
async fn budget_refusal(
    decision: &crate::budget::BudgetDecision,
    req: &ExecRequest,
    call_ctx: &CallContext,
    store: &crate::memory::Store,
) -> Option<ExecResult> {
    match decision {
        crate::budget::BudgetDecision::Allowed => None,
        crate::budget::BudgetDecision::Exhausted { cap, used, limit } => {
            let _ = audit_tool_call(
                store,
                call_ctx,
                "rsh_exec",
                &req.command,
                &format!("{{\"budget\":\"{}\"}}", cap),
                -1,
                "R29",
            )
            .await;
            Some(budget_exhausted_result(&req.command, cap, *used, *limit))
        }
    }
}

/// Build the structured R29 (Budget Exhausted) result returned to the agent.
fn budget_exhausted_result(command: &str, cap: &str, used: u64, limit: u64) -> ExecResult {
    ExecResult {
        status: "failed".to_string(),
        recovery_code: "R29".to_string(),
        recovery_class: RecoveryCode::R29.class_name().to_string(),
        original_command: command.to_string(),
        execution_id: uuid::Uuid::new_v4().to_string(),
        output_id: None,
        suggestion: serde_json::json!({
            "action": "wait_or_raise_limit",
            "confidence": "high",
            "reason": format!(
                "Budget cap '{}' reached ({} of {}). Wait for the window to reset or raise the cap in [budget].",
                cap, used, limit
            )
        }),
        output: OutputInfo {
            stdout: String::new(),
            stderr: format!(
                "rsh: budget exhausted: cap '{}' at {}/{}",
                cap, used, limit
            ),
            exit_code: -1,
            truncated: false,
            binary_summary: None,
        },
        next_action: Some(NextAction {
            tool: "rsh_recover".to_string(),
            params: serde_json::json!({
                "recovery_code": "R29",
                "original_command": command,
                "context": format!("budget cap {} reached", cap),
                "stderr": "",
            }),
            reason: "Budget cap reached. The command was not executed. Wait for the window to reset or ask the operator to raise the cap.".to_string(),
        }),
        compaction_hint: None,
        platform: None,
        warnings: vec![],
        auto_retry: None,
    }
}

#[derive(Debug, serde::Deserialize)]
struct ExecRequestWrapper {
    command: String,
    cwd: Option<String>,
    timeout: Option<u64>,
    env: Option<HashMap<String, String>>,
    retry: Option<bool>,
    #[serde(default)]
    binary_handling: crate::exec::BinaryHandling,
    /// Human approval for a high-risk command (R28). Forwarded by the agent
    /// after the host's permission UI confirms the command is safe to run.
    #[serde(default)]
    approve: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct RecoverRequest {
    #[serde(default)]
    execution_id: Option<String>,
    #[serde(default)]
    recovery_code: Option<String>,
    #[serde(default)]
    original_command: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    stderr: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CompactRequest {
    #[serde(rename = "output_id")]
    output_id: Option<String>,
    file: Option<String>,
    #[serde(rename = "view")]
    view: Option<String>,
    path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct FeedbackRequest {
    original_command: String,
    #[serde(default)]
    stderr: Option<String>,
    fix_command: String,
    success: bool,
}
