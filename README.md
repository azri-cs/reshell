# Reshell (`rsh`) — Resilient Shell Execution Middleware for AI Agents

**Reshell** is a deterministic execution middleware that sits between an AI agent (Claude Code, OpenCode, Cursor, etc.) and the operating system shell. It transforms opaque command failures into structured, classified, and recoverable events — **without ever calling an LLM to fix a shell error**.

---

## Features

- **Deterministic Failure Classification** — Failures map to a taxonomy (`R20`–`R27`, `R30`) via regex-driven pattern matching and exit codes.
- **Zero-LLM Recovery Engine** — Hardcoded, template-based recovery suggestions (install missing tools, fix permissions, POSIX-ify syntax, etc.).
- **Output Compaction** — Prevents context-window pollution by truncating large outputs into head + structural skeleton + tail.
- **Pattern Memory** — SQLite-backed learning from previous failures to provide high-confidence instant fixes on recurrence.
- **Safety Sandbox** — Pre-exec validation blocks dangerous and interactive commands, optional command allowlist via `~/.reshell/allowlist.toml`, and stderr secret scrubbing. There is no filesystem or network isolation.
- **MCP Server** — Exposes 7 tools (`rsh_exec`, `rsh_env`, `rsh_recover`, `rsh_compact`, `rsh_check`, `rsh_feedback`, `rsh_stats`) over **framed MCP transport** (`Content-Length` header + JSON body) on stdio.

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

After adding the configuration, restart your agent. The agent will discover five tools: `rsh_exec`, `rsh_env`, `rsh_recover`, `rsh_compact`, and `rsh_check`.

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
  "retry": true
}
```

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

### Execution model

Commands are run as `sh -c '<command>'` by default. If `retry` is true (the default) and the first run is classified as **R25** (environment mismatch), Reshell may re-run the same command using your login shell from `$SHELL` (e.g. bash or zsh) when it differs from `sh`. See `src/env/detector.rs` and `src/exec/runner.rs` for details.

### Optional command allowlist

Advanced deployments can restrict which command names are permitted by creating `~/.reshell/allowlist.toml` (blocklist remains the default if the file is missing or invalid). See comments in `src/sandbox/allowlist.rs` for the TOML shape.

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
- [x] Failure Taxonomy — Regex-driven classifiers (`R20`–`R30`)
- [x] Recovery Engine — Deterministic suggestions per class
- [x] Output Compaction — Head/skeleton/tail truncation
- [x] Pattern Memory — SQLite-backed persistence
- [x] MCP Server — framed stdio transport (Content-Length headers) for Claude Code / OpenCode / Cursor
- [x] Optional command allowlist — `~/.reshell/allowlist.toml` (see `src/sandbox/allowlist.rs`)
- [ ] Safety Hardening — OverlayFS or equivalent filesystem isolation; Linux seccomp syscall filtering (stub in `src/sandbox/seccomp.rs`)
- [ ] Distribution — `cargo install`, Homebrew formula

---

## License

MIT — See [LICENSE](./LICENSE) for details.
