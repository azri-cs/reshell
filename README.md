# Reshell (`rsh`) — Resilient Shell Execution Middleware for AI Agents

**Reshell** is a deterministic execution middleware that sits between an AI agent (Claude Code, OpenCode, Cursor, etc.) and the operating system shell. It transforms opaque command failures into structured, classified, and recoverable events — **without ever calling an LLM to fix a shell error**.

---

## Features

- **Deterministic Failure Classification** — Every non-zero exit code maps to a taxonomy (`R20`–`R30`) via regex-driven pattern matching.
- **Zero-LLM Recovery Engine** — Hardcoded, template-based recovery suggestions (install missing tools, fix permissions, POSIX-ify syntax, etc.).
- **Output Compaction** — Prevents context-window pollution by truncating large outputs into head + structural skeleton + tail.
- **Pattern Memory** — SQLite-backed learning from previous failures to provide high-confidence instant fixes on recurrence.
- **Safety Sandbox** — Blocks dangerous commands (`rm -rf /`), interactive editors (`vim`, `nano`), and scrubs secrets from stderr.
- **MCP Server** — Exposes `rsh_exec`, `rsh_env`, `rsh_recover`, and `rsh_compact` via Model Context Protocol (stdio transport).

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

Reshell exposes an **MCP server** over stdio. Configure your agent to invoke `rsh mcp`.

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

After adding the configuration, restart your agent. The agent will automatically discover the tools `rsh_exec`, `rsh_env`, `rsh_recover`, and `rsh_compact`.

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

### `rsh_env`

Detect and describe the current shell environment (shell type, OS, available tools, package manager).

**Input Schema:** `{}`

### `rsh_recover`

Apply a deterministic recovery strategy for a known failure class.

**Input Schema:**
```json
{
  "recovery_code": "R22",
  "original_command": "gh pr view",
  "context": ""
}
```

### `rsh_compact`

Retrieve a compacted view of a large file or previously stored output.

**Input Schema:**
```json
{
  "file": "/var/log/syslog",
  "view": "skeleton"
}
```

Views: `full`, `skeleton`, `diff`, `errors_only`.

---

## CLI Usage (Direct Mode)

Reshell can also be used directly from the terminal without an agent.

```bash
# Execute a command with structured output
rsh exec --command "ls -la"

# Detect environment
rsh env

# Compact a large file
rsh compact --file /var/log/syslog
```

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
- Blocking of dangerous and interactive commands.
- Environment detection.
- MCP `initialize`, `tools/list`, `tools/call` protocol compliance.

---

## Roadmap

- [x] Foundation — Command execution, timeout, structured JSON output
- [x] Failure Taxonomy — Regex-driven classifiers (`R20`–`R30`)
- [x] Recovery Engine — Deterministic suggestions per class
- [x] Output Compaction — Head/skeleton/tail truncation
- [x] Pattern Memory — SQLite-backed persistence
- [x] MCP Server — stdio transport for Claude Code / OpenCode / Cursor
- [ ] Safety Hardening — OverlayFS sandbox, allowlist
- [ ] Distribution — `cargo install`, Homebrew formula

---

## License

MIT — See [LICENSE](./LICENSE) for details.
