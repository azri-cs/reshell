# Reshell (`rsh`) — Resilient Shell Execution Middleware for AI Agents

**Reshell** is a deterministic execution middleware that sits between an AI agent (Claude Code, OpenCode, Cursor, etc.) and the operating system shell. It transforms opaque command failures into structured, classified, and recoverable events — **without ever calling an LLM to fix a shell error**.

---

## Features

- **Deterministic Failure Classification** — Failures map to a taxonomy (`R20`–`R27`, `R30`) via regex-driven pattern matching and exit codes.
- **Zero-LLM Recovery Engine** — Hardcoded, template-based recovery suggestions (install missing tools, fix permissions, POSIX-ify syntax, etc.).
- **Output Compaction** — Prevents context-window pollution by truncating large outputs into head + structural skeleton + tail.
- **Pattern Memory** — SQLite-backed learning from previous failures to provide high-confidence instant fixes on recurrence.
- **Safety Sandbox** — Pre-exec validation blocks dangerous and interactive commands, optional command allowlist via `~/.reshell/allowlist.toml`, and stderr secret scrubbing. There is no filesystem or network isolation.
- **MCP Server** — Exposes 9 tools (`rsh_exec`, `rsh_env`, `rsh_recover`, `rsh_compact`, `rsh_read_file`, `rsh_write_file`, `rsh_check`, `rsh_feedback`, `rsh_stats`) and 2 prompts over **framed MCP transport** (`Content-Length` header + JSON body) on stdio.

---

## Installation

### Prerequisites

- Rust toolchain (`rustc` + `cargo`) installed via `rustup`.
- `git` installed to clone the repository.

You can install Rust/Cargo with:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

### Prebuilt binaries

Download the latest release for your platform from the [GitHub Releases](https://github.com/azri-cs/reshell/releases) page:

```bash
# Linux (x86_64)
curl -L -o rsh.tar.gz https://github.com/azri-cs/reshell/releases/latest/download/rsh-x86_64-unknown-linux-musl.tar.gz
tar xzf rsh.tar.gz
sudo mv rsh /usr/local/bin/

# macOS (Apple Silicon)
curl -L -o rsh.tar.gz https://github.com/azri-cs/reshell/releases/latest/download/rsh-aarch64-apple-darwin.tar.gz
tar xzf rsh.tar.gz
sudo mv rsh /usr/local/bin/
```

### Homebrew

```bash
brew tap azri-cs/reshell
brew install reshell
```

### From Source (requires Rust toolchain)

```bash
git clone https://github.com/azri-cs/reshell.git
cd reshell
cargo build --release
# Binary will be at target/release/rsh
```

### Add to `$PATH`

```bash
cp target/release/rsh ~/.local/bin/
# or
sudo cp target/release/rsh /usr/local/bin/
```

---

## Agent Configuration

Reshell exposes an **MCP server** on stdio (framed transport with `Content-Length` headers, per the MCP specification). Configure your agent to invoke `rsh mcp`.

### Claude Code

Recommended project-local config (`.mcp.json`):

```json
{
  "reshell": {
    "command": "rsh",
    "args": ["mcp"]
  }
}
```

If you are using a settings file with an `mcpServers` object, use the same server definition under `mcpServers.reshell`.

### OpenCode

Add to your `.opencode/mcp.json` or workspace settings:

```json
{
  "reshell": {
    "type": "local",
    "command": ["rsh", "mcp"],
    "enabled": true
  }
}
```

### Cursor

Add to `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "reshell": {
      "type": "stdio",
      "command": "rsh",
      "args": ["mcp"]
    }
  }
}
```

After adding the configuration, restart your agent. The agent will discover nine tools and two prompts.

---

## Tools Reference

### `rsh_exec`

Execute a shell command with resilient failure handling.

**Input Schema:**
```json
{
  "command": "ls -la",
  "cwd": "/tmp",
  "timeout": 120,
  "env": {},
  "retry": true,
  "binary_handling": "summary",
  "approve": false
}
```

If a call returns `recovery_code: "R28"` (Approval Required), re-issue the **same** command with `"approve": true` after a human approves the operation (see [Optional human-in-the-loop approval](#optional-human-in-the-loop-approval-r28)).

**Example Response (Success):**
```json
{
  "status": "success",
  "recovery_code": "R10",
  "recovery_class": "Success",
  "original_command": "ls -la",
  "suggestion": { "action": "none", "confidence": "high", "reason": "Command succeeded" },
  "output": { "stdout": "...", "stderr": "", "exit_code": 0, "truncated": false }
}
```

**Example Response (Command Not Found — R22):**
```json
{
  "status": "failed",
  "recovery_code": "R22",
  "recovery_class": "Command Not Found",
  "original_command": "gh pr view",
  "suggestion": {
    "action": "install_missing_tool",
    "command": "brew install gh",
    "confidence": "high",
    "reason": "`gh` not found in $PATH."
  },
  "output": { "stdout": "", "stderr": "gh: command not found", "exit_code": 127, "truncated": false }
}
```

**Additional fields (often present on failure or large output):**

| Field | Meaning |
|-------|---------|
| `output_id` | ID of stored stdout in the pattern DB (use with `rsh_compact` when `truncated` is true). |
| `next_action` | Suggested follow-up MCP tool name, parameters, and reason (e.g. call `rsh_recover`). On failure, `params` includes **`stderr`** (normalized, capped) so `rsh_recover` can reuse the same **learned patterns** as `rsh_exec`. |
| `compaction_hint` | When stdout was truncated, how to call `rsh_compact` with `output_id` and a suggested view. |
| `platform` | Host platform string for pattern matching context. |
| `warnings` | Non-fatal notices (e.g. security-related). |

### `rsh_env`

Detect and describe the current shell environment (shell type, OS, available tools, package manager).

**Input Schema:** `{}`

### `rsh_recover`

Apply a deterministic recovery strategy for a known failure class. When **`stderr`** is set (e.g. copy `next_action.params.stderr` from a failed `rsh_exec`), lookups use the same SQLite pattern memory as exec; otherwise only `context` is used (often the short classification reason).

**Input Schema:**
```json
{
  "recovery_code": "R22",
  "original_command": "gh pr view",
  "context": "",
  "stderr": ""
}
```

`stderr` is optional. Prefer passing it when you have it so learned fixes (`reuse_learned_fix`) can match.

### `rsh_compact`

Retrieve a compacted view of a large file **or** stdout previously stored from `rsh_exec` (via `output_id`).

**Input Schema (file on disk):**
```json
{
  "file": "/var/log/syslog",
  "view": "skeleton"
}
```

**Input Schema (stored command output):**
```json
{
  "output_id": "<uuid-from-rsh_exec>",
  "view": "errors_only"
}
```

Views: `full`, `skeleton`, `diff`, `errors_only`.

### `rsh_check`

Session health check and short onboarding: verifies the server is up and summarizes the `rsh_exec` → `rsh_recover` → `rsh_compact` workflow. **Input Schema:** `{}`

### `rsh_read_file`

Read a file through the safety sandbox (blocks path traversal and sensitive paths).

**Input Schema:**
```json
{
  "path": "/path/to/file.txt"
}
```

### `rsh_write_file`

Write content to a file through the safety sandbox (creates parent directories, blocks sensitive paths).

**Input Schema:**
```json
{
  "path": "/path/to/file.txt",
  "content": "file contents here"
}
```

### `rsh_feedback`

Record the outcome of a fix attempt to update pattern memory. Call this after trying a recovery suggestion.

**Input Schema:**
```json
{
  "recovery_code": "R22",
  "original_command": "gh pr view",
  "suggested_command": "brew install gh && gh pr view",
  "success": true
}
```

### `rsh_stats`

Get pattern memory statistics and runtime metrics: recovery attempts, pattern counts by recovery code, fix success rates, execution metrics.

**Input Schema:** `{}`

---

## Scope & Non-Goals

Reshell is a **resilient shell-execution layer**. It governs the tool-call failure modes that originate at the shell surface — opaque command failures, retry loops, output overflow, unsafe commands, runaway budgets, and unreviewed high-risk operations — and turns them into structured, recoverable events without ever calling an LLM.

It is **not** a complete solution to "the AI agent tool-call problem." The following concerns are deliberately out of scope and should be handled by complementary layers (the host agent, a separate tool gateway, or OS-level isolation):

| Concern | Why it's out of scope | Where it belongs |
|---------|----------------------|------------------|
| **Model-layer tool selection** — agents hallucinating tool names, returning empty responses, or picking the wrong tool | Reshell only structures what happens *after* a real shell command runs; it cannot influence the model's choice of tool | The model / agent runtime |
| **Non-shell API/HTTP tool misuse** — FireCrawl, `web_search`, custom HTTP tools, authenticated API calls | Reshell is shell-execution middleware; it has no visibility into other tools' transports | A separate tool-call gateway or the host's permission layer |
| **Cross-tool token budgeting** — per-session/hourly/daily token ceilings across *all* tools | Reshell budgets only the shell surface it controls (via `[budget]`) | The host agent's budget/guardrail layer |
| **Full checkpoint/resume of agent execution graphs** — durable state for resuming a corrupted long run | Pattern memory persists *fixes* (command → learned correction), not the agent's execution graph | The agent framework |
| **Per-tool least-privilege credentials for non-shell tools** (OWASP ASI02 Guidelines 1, 2, 6, 7) | Applies to API/auth tools outside the shell surface | A credentials/identity layer at the host |

**What Reshell *does* own:** deterministic shell error classification (R10–R30), zero-LLM recovery strategies, learned pattern memory for recurring shell failures, output compaction (head/skeleton/diff/errors-only + JSONPath extraction), MIME-aware binary output handling, pre-exec safety validation with risk-tiered approval (`R28`), secret scrubbing on shell stderr/stdout, full-argument audit & correlation-id tracing, and per-session/hourly/daily *shell-invocation* budgeting (`R29`). For everything above, pair Reshell with the appropriate complementary layer rather than expecting it to cover the full tool-call surface.

---

## CLI Usage (Direct Mode)

Reshell can also be used directly from the terminal without an agent.

```bash
# Execute a command with structured JSON on stdout (default timeout 120s, retry on R25 enabled)
rsh exec --command "ls -la"
rsh exec --command "npm test" --cwd ./myapp --timeout 300
rsh exec --command "printenv FOO" -E FOO=bar

# Detect environment
rsh env

# Compact a large file, or a stored output from a prior exec
rsh compact --file /var/log/syslog
rsh compact --output-id "<uuid>" --view errors_only
```

### Additional CLI flags

```bash
# Enable OverlayFS sandbox (Linux only)
rsh exec --command "npm install" --sandbox

# Generate shell completions
rsh completions bash
rsh completions zsh
rsh completions fish

# Extract JSON field
rsh compact --file package.json --jq ".dependencies"

# SSE transport for MCP
rsh mcp --transport sse --port 3000
```

### Execution model

Commands are run as `sh -c '<command>'` by default. If `retry` is true (the default) and the first run is classified as **R25** (environment mismatch), Reshell may re-run the same command using your login shell from `$SHELL` (e.g. bash or zsh) when it differs from `sh`. See `src/env/detector.rs` and `src/exec/runner.rs` for details.

### Optional command allowlist

Advanced deployments can restrict which command names are permitted by creating `~/.reshell/allowlist.toml` (blocklist remains the default if the file is missing or invalid). See comments in `src/sandbox/allowlist.rs` for the TOML shape.

### Optional budget guardrail

To cap how many shell calls an agent can make, add a `[budget]` section to `~/.reshell/config.toml`. All values default to `0` (unlimited), so the guardrail is inert until you set one:

```toml
[budget]
max_invocations_per_session = 200   # calls per MCP server process; 0 = unlimited
max_output_bytes_per_session = 10485760
max_wall_secs_per_session = 600
max_invocations_per_hour = 0        # persisted across restarts via budget_ledger
max_invocations_per_day = 0
```

When a cap is reached, `rsh_exec` returns recovery code **`R29` (Budget Exhausted)** and the command is **not** executed. Session caps (invocations, bytes, wall) reset when the server restarts; hourly/daily caps persist in the SQLite `budget_ledger` table. Note: output bytes are charged *after* execution (size is unknowable in advance), so a bytes cap may refuse the *next* call rather than the one that exceeds it. The guardrail is enforced in the shared `Router`, so it applies to **both** stdio and SSE transports.

### Optional human-in-the-loop approval (R28)

High-risk-but-legitimate commands — recursive deletes outside `/tmp`, `git push --force`, `docker system prune`, `sudo`-bearing commands, or anything matching a configurable pattern — return **`R28` (Approval Required)** instead of executing silently. The MCP host (Claude Code / OpenCode / Cursor) intercepts the tool call for its own permission UI and, once a human approves, re-issues the same command with `approve: true`:

```json
{ "command": "git push --force origin main", "approve": true }
```

Configure the triggers in `~/.reshell/config.toml`:

```toml
[safety]
auto_approve = false                 # if true, skip the R28 gate entirely
review_patterns = ["git reset --hard"]   # extra regexes that trigger review
```

The built-in review triggers (on unless `auto_approve = true`): `rm -r` outside `/tmp`/`/var/tmp`, `git push --force`, `docker (system|volume) prune`, and any command containing `sudo`. Commands that are outright dangerous (`rm -rf /`, `mkfs`, fork bombs, etc.) are still hard-blocked as `R27` and never reach the approval gate.

### Optional full-argument audit & tracing

Every tool call is logged to the `audit_log` SQLite table with a per-process **session id**, the JSON-RPC **request id**, the (secret-scrubbed) **raw arguments**, and **wall-clock duration** — so every invocation is traceable end-to-end. The last 10 invocations surface in `rsh_stats` as `recent_invocations`. Arguments are routed through the secret scrubber before persistence, so tokens/keys in `env` or args are never written to disk. This is on by default (no config needed); the columns are additive and nullable, so older databases upgrade in place.

---

## Failure Taxonomy

| Code | Class | Trigger | Example |
|------|-------|---------|---------|
| `R10` | **Success** | Exit 0 | — |
| `R20` | **Syntax Error** | Exit 2, usage text | `invalid option`, `unrecognized argument` |
| `R21` | **Permission Denied** | Exit 126, 128 | `Permission denied` |
| `R22` | **Command Not Found** | Exit 127 | `command not found` |
| `R23` | **Timeout** | SIGKILL / timeout | Process exceeded time limit |
| `R24` | **Subcommand Failure** | Exit 1 + pattern | `npm ERR!`, `pytest failed`, `make: ***` |
| `R25` | **Environment Mismatch** | Shell mismatch | Bash-ism in Zsh |
| `R26` | **Output Overflow** | stdout > threshold | Truncated output |
| `R27` | **Blocked / Safety Violation** | Validator / allowlist | Dangerous pattern, interactive editor, or allowlist deny |
| `R28` | **Approval Required** | Validator risk-tier (high-risk command) | Recursive delete outside `/tmp`, `git push --force`, `sudo`-bearing command — re-issue with `approve: true` |
| `R29` | **Budget Exhausted** | Budget guardrail | A configured session/hourly/daily cap was reached; the command was not executed |
| `R30` | **Fatal / Unknown** | Non-matching | Requires escalation |

---

## Architecture

```
AI Agent (Claude/OpenCode/Cursor)
    |
    | MCP stdio
    v
+---------------------------+
| Reshell MCP Server        |
|  - rsh_exec               |
|  - rsh_env                |
|  - rsh_recover            |
|  - rsh_compact            |
|  - rsh_check              |
+---------------------------+
    |
    v
+---------------------------+
| Command Validator         |
| Environment Detector      |
| Failure Classifier        |
| Recovery Engine           |
| Output Compactor          |
| Pattern Memory (SQLite)   |
| Safety Sandbox            |
+---------------------------+
    |
    v
   Shell (Bash / Zsh)
```

---

## Testing

```bash
# Run unit tests embedded in modules
cargo test

# Run integration tests (spawns real MCP server process)
cargo test --test integration_tests

# Run benchmarks
cargo bench
```

Integration tests verify:
- Happy path command execution via CLI and MCP.
- Classification of `R22` (Command Not Found).
- Blocking of dangerous and interactive commands (`R27`).
- Environment detection.
- MCP `initialize`, `tools/list`, `tools/call` protocol compliance, including `rsh_check`.

---

## Roadmap

- [x] Foundation — Command execution, timeout, structured JSON output
- [x] Failure Taxonomy — Regex-driven classifiers (R20–R30)
- [x] Recovery Engine — Deterministic suggestions per class
- [x] Output Compaction — Head/skeleton/tail truncation
- [x] Pattern Memory — SQLite-backed persistence
- [x] MCP Server — framed stdio transport (Content-Length headers) for Claude Code / OpenCode / Cursor
- [x] Optional command allowlist — `~/.reshell/allowlist.toml`
- [x] OverlayFS sandbox — Filesystem isolation (Linux, opt-in)
- [x] Shell completions — bash, zsh, fish
- [x] SSE transport — HTTP SSE endpoint for MCP
- [x] jq-like extraction — JSON path queries in compact
- [x] Docker image — Multi-stage container build
- [x] Success metrics — Recovery rate, context savings, latency telemetry
- [ ] Distribution — `cargo install`, Homebrew formula, crates.io publishing

---

## License

MIT — See [LICENSE](./LICENSE) for details.
