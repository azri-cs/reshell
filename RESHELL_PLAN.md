# Reshell: Resilient Shell Execution Middleware for AI Agents

**Status:** Draft Plan  
**Date:** 2026-04-21  
**Target Platforms:** Claude Code, OpenCode, Cursor (via MCP)  
**Target Shells:** Bash, Zsh  
**Recovery Strategy:** Fully Deterministic (Zero LLM Calls)  
**Context:** Interactive Coding Agents

---

## 1. Problem Statement

AI agents interacting with the CLI routinely fail when commands do not execute on the "happy path." Current agent behaviors include:

| Failure Mode | Symptom | Impact |
|--------------|---------|--------|
| Execution Deadlocks | Agent gets stuck in retry loops | Token waste, context pollution |
| Hallucinated Success | Agent reports task done when it failed | Silent failures, corrupted state |
| Context Degradation | Error traces bloat the context window | 20–30% reasoning performance drop |
| Environment Mismatch | Bash syntax on wrong shell, missing deps | Complete task abandonment |
| Cascading Failures | One tool failure corrupts shared state | Multi-agent coordination breaks |
| Non-interactive Termination | Process exits on first error | CI/CD and automation pipelines fail |

Research (PALADIN, AgentRx) confirms agents are overwhelmingly trained on successful trajectories, leaving them brittle when tools malfunction.

---

## 2. Vision

**Reshell (`rsh`)** is a deterministic execution middleware that sits between an AI agent and the operating system shell. It transforms opaque command failures into structured, classified, and recoverable events—without ever calling an LLM to fix a shell error.

The agent receives:
- A clear failure classification
- A deterministic recovery suggestion
- Compacted, context-efficient output
- Learned patterns from previous failures

---

## 3. Architecture

```
┌─────────────┐     ┌──────────────────────────────────────────┐     ┌──────────┐
│  AI Agent   │────▶│         Reshell Middleware (rsh)         │────▶│   Shell  │
│ (Claude/    │◀────│                                          │◀────│ (Bash/   │
│  OpenCode)  │     │  1. Command Validator                    │     │   Zsh)   │
└─────────────┘     │  2. Environment Detector                 │     └──────────┘
                    │  3. Failure Classifier (Taxonomy)        │
                    │  4. Recovery Strategy Engine             │
                    │  5. Output Compactor                     │
                    │  6. Pattern Memory (SQLite)              │
                    │  7. Safety Sandbox                       │
                    └──────────────────────────────────────────┘
```

---

## 4. Core Components

### 4.1 Command Validator (`validate`)
- **Static analysis** of command strings before execution
- Detects dangerous patterns (`rm -rf /`, `> /dev/sda`)
- Blocks interactive commands (`vim`, `less`, `nano`) with clear error
- Checks for obvious syntax errors (unmatched quotes, invalid operators)
- Enforces a configurable command timeout (default: 120s)

### 4.2 Environment Detector (`env`)
- Detects active shell (Bash vs Zsh) via `$SHELL` and `$BASH_VERSION`
- Discovers available tools and their versions (`git`, `node`, `docker`, etc.)
- Checks current working directory, user permissions, and `$PATH`
- Identifies missing dependencies before execution
- Maps common commands to shell-native equivalents where needed

### 4.3 Failure Classifier (`classify`)
A deterministic, regex and exit-code-driven taxonomy:

| Code | Class | Trigger | Examples |
|------|-------|---------|----------|
| `R10` | **Success** | Exit 0, no stderr | — |
| `R20` | **Syntax Error** | Exit 2, stderr pattern | `invalid option`, `usage:`, `unrecognized argument` |
| `R21` | **Permission Denied** | Exit 126, 128, `EACCES` | `Permission denied`, `Operation not permitted` |
| `R22` | **Command Not Found** | Exit 127, `ENOENT` | `command not found`, `No such file or directory` |
| `R23` | **Timeout** | Exit 124 (timeout), SIGKILL | Process exceeded time limit |
| `R24` | **Subcommand Failure** | Exit 1 with known patterns | `npm ERR!`, `pytest failed`, `make: ***` |
| `R25` | **Environment Mismatch** | Exit 1 + shell mismatch | Bash-ism in Zsh, missing env var |
| `R26` | **Output Overflow** | stdout > threshold | Truncated due to size limits |
| `R27` | **Blocked / Safety Violation** | Pre-exec validator or allowlist | Dangerous pattern, interactive editor, allowlist deny |
| `R30` | **Fatal / Unknown** | Non-matching failure | Requires human/agent escalation |

### 4.4 Recovery Strategy Engine (`recover`)
**Fully deterministic. No LLM calls.** Strategies are hardcoded, template-based, and data-driven.

| Class | Auto-Action | Suggested Fix (returned to agent) |
|-------|-------------|-----------------------------------|
| `R20` Syntax Error | Extract usage text | Return corrected flags from usage synopsis |
| `R21` Permission | Test `sudo -n` availability | Suggest `sudo` or `chmod` command |
| `R22` Not Found | Search `$PATH`, check package manager | Suggest `apt install`, `brew install`, etc. |
| `R23` Timeout | SIGKILL, return partial output | Suggest chunked execution or longer timeout |
| `R24` Subcommand | Parse tool-specific error logs | Suggest next diagnostic command (e.g., `npm ls`) |
| `R25` Env Mismatch | Detect bashism, suggest portable form | Suggest POSIX-compliant alternative |
| `R26` Overflow | Truncate, return structural skeleton | Suggest scoped command (`grep`, `head`) |
| `R27` Blocked | No execution | Explain block reason; adjust command or allowlist config |

The engine returns a structured JSON response to the agent:
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
    "reason": "`gh` not found in $PATH. Detected macOS + Homebrew."
  },
  "output": {
    "stdout": "",
    "stderr": "gh: command not found",
    "exit_code": 127,
    "truncated": false
  }
}
```

### 4.5 Output Compactor (`compact`)
Prevent context-window pollution:

| Scenario | Strategy |
|----------|----------|
| First large output | Head 100 lines + structural skeleton (function defs, class names, error lines) + tail 20 lines |
| Repeat read of same file | Return structural diff only (99% token savings) |
| Binary output | Reject with MIME-type summary |
| JSON/XML | Parse and return relevant fields via `jq`-like extraction |
| Log files | Extract ERROR/WARN lines + surrounding context |

### 4.6 Pattern Memory (`learn`)
SQLite-backed persistent store at `~/.reshell/patterns.db`:

```sql
CREATE TABLE patterns (
  id INTEGER PRIMARY KEY,
  command_hash TEXT NOT NULL,
  command_template TEXT NOT NULL,
  recovery_code TEXT NOT NULL,
  stderr_pattern TEXT NOT NULL,
  fix_command TEXT,
  fix_success_rate REAL DEFAULT 0.0,
  last_used TIMESTAMP,
  usage_count INTEGER DEFAULT 1
);
```

- On failure, query: *"Has this command template failed with this stderr pattern before?"*
- If a fix exists and `fix_success_rate > 0.5`, auto-suggest it with high confidence.
- If the agent accepts the suggestion and it succeeds, increment `usage_count` and `success_rate`.

### 4.7 Safety Sandbox (`sandbox`)
Optional, opt-in security layer:

| Feature | Implementation |
|---------|----------------|
| Filesystem Isolation | OverlayFS (Linux) or temporary working directory |
| Command Allowlist | Configurable regex list of permitted commands |
| Secret Scrubbing | Regex-based redaction of API keys, tokens, passwords from stderr before returning to agent |
| Network Restrictions | Optional `unshare` or firewall rules |

---

## 5. MCP Server Specification

Reshell exposes itself as an MCP (Model Context Protocol) server with the following tools:

### Tool: `rsh_exec`
```json
{
  "name": "rsh_exec",
  "description": "Execute a shell command with resilient failure handling",
  "inputSchema": {
    "type": "object",
    "properties": {
      "command": { "type": "string", "description": "The shell command to execute" },
      "cwd": { "type": "string", "description": "Working directory" },
      "timeout": { "type": "integer", "default": 120 },
      "env": { "type": "object", "additionalProperties": { "type": "string" } },
      "retry": { "type": "boolean", "default": true }
    },
    "required": ["command"]
  }
}
```

### Tool: `rsh_env`
```json
{
  "name": "rsh_env",
  "description": "Detect and describe the current shell environment",
  "inputSchema": {
    "type": "object",
    "properties": {}
  }
}
```

### Tool: `rsh_recover`
```json
{
  "name": "rsh_recover",
  "description": "Apply a deterministic recovery strategy for a known failure",
  "inputSchema": {
    "type": "object",
    "properties": {
      "recovery_code": { "type": "string" },
      "original_command": { "type": "string" },
      "context": { "type": "string" }
    },
    "required": ["recovery_code", "original_command"]
  }
}
```

### Tool: `rsh_compact`
```json
{
  "name": "rsh_compact",
  "description": "Retrieve a compacted view of a previously stored large output",
  "inputSchema": {
    "type": "object",
    "properties": {
      "output_id": { "type": "string" },
      "view": { "type": "string", "enum": ["full", "skeleton", "diff", "errors_only"] }
    },
    "required": ["output_id"]
  }
}
```

---

## 6. Technology Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| **Language** | **Rust** | Memory safety, zero-cost abstractions, excellent CLI ecosystem (`clap`, `tokio`, `serde`), single binary distribution |
| **Async Runtime** | `tokio` | Needed for timeout handling, process management, and concurrent operations |
| **CLI Framework** | `clap` + `serde` | Robust argument parsing and configuration |
| **Database** | `rusqlite` | Embedded, zero-config pattern memory |
| **MCP Transport** | `stdio` and `SSE` | Compatible with Claude Code, OpenCode, Cursor |
| **Process Management** | `tokio::process` | Cross-platform async process spawning |
| **Testing** | `tokio-test` + `insta` | Snapshot testing for output compaction |

*Alternative: Go was considered for simplicity, but Rust's type safety and performance characteristics are superior for a deterministic, safety-critical middleware.*

---

## 7. Development Roadmap

### Phase 1: Foundation (Weeks 1–2)
- [ ] Project scaffold (`cargo new`, `clap` setup, logging)
- [ ] Basic command execution wrapper (`tokio::process`)
- [ ] Exit code + stdout/stderr capture
- [ ] Timeout handling (`tokio::time::timeout`)
- [ ] Unit tests for happy path execution

**Deliverable:** `rsh exec "ls -la"` works and returns structured JSON.

### Phase 2: Failure Taxonomy (Weeks 3–4)
- [ ] Implement `R20`–`R26` classifiers
- [ ] Regex pattern library for stderr parsing
- [ ] Exit code mapping table
- [ ] Environment detector (shell type, `$PATH`, available tools)
- [ ] Comprehensive classifier tests (snapshot + unit)

**Deliverable:** Every non-zero exit code maps to a `RecoveryCode`.

### Phase 3: Recovery Engine (Weeks 5–6)
- [ ] Hardcoded recovery strategy map
- [ ] Command suggestion templates
- [ ] `$PATH` search for missing binaries
- [ ] Package manager detection (`apt`, `brew`, `pacman`, `yum`, `choco`)
- [ ] Bashism-to-POSIX translation table
- [ ] Integration tests for each recovery class

**Deliverable:** `rsh` suggests a fix for `R22` (Command Not Found) on the user's OS.

### Phase 4: Output Compaction (Week 7)
- [ ] Large output truncation (head + skeleton + tail)
- [ ] Structural skeleton extraction (regex-based: `fn `, `class `, `ERROR`)
- [ ] Repeat-read diff mode
- [ ] Binary output detection
- [ ] Log file filtering (ERROR/WARN extraction)

**Deliverable:** 10MB log file compacted to <2KB structural summary.

### Phase 5: Pattern Memory (Week 8)
- [ ] SQLite schema and migrations
- [ ] Pattern insertion and retrieval
- [ ] Command template hashing
- [ ] Success rate tracking
- [ ] LRU eviction for old patterns

**Deliverable:** Second occurrence of the same failure returns an instant, high-confidence fix.

### Phase 6: MCP Server (Week 9)
- [ ] MCP `stdio` transport implementation
- [ ] `rsh_exec`, `rsh_env`, `rsh_recover`, `rsh_compact` tools
- [ ] JSON-RPC request/response handling
- [ ] Connection to Claude Code / OpenCode / Cursor
- [ ] End-to-end integration test

**Deliverable:** Agent can call `rsh_exec` via MCP and receive structured failures.

### Phase 7: Safety & Hardening (Weeks 10–11)
- [ ] Command allowlist/blacklist
- [ ] Secret scrubbing (15+ regex patterns)
- [ ] OverlayFS sandbox prototype (Linux)
- [ ] Security audit and fuzzing
- [ ] Benchmark suite: 100+ failure scenarios

**Deliverable:** `rsh` passes security audit and benchmarks.

### Phase 8: Release (Week 12)
- [ ] `cargo install` distribution
- [ ] Homebrew formula
- [ ] Documentation and usage examples
- [ ] GitHub repository setup
- [ ] Community announcement

**Deliverable:** Public v0.1.0 release.

---

## 8. Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| **Recovery Rate** | >70% | % of failed commands for which `rsh` returns a useful suggestion |
| **False Positive Rate** | <5% | % of recovery suggestions that make the situation worse |
| **Context Savings** | >80% | Token reduction vs. raw stdout on large outputs |
| **Time-to-Recovery** | <5s | Median time from failure to structured suggestion |
| **Zero LLM Dependency** | 100% | No LLM API calls in the recovery path |

---

## 9. Directory Structure (Target)

```
reshell/
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   └── tools.rs
│   ├── exec/
│   │   ├── mod.rs
│   │   ├── runner.rs
│   │   └── validator.rs
│   ├── classify/
│   │   ├── mod.rs
│   │   ├── taxonomy.rs
│   │   └── patterns.rs
│   ├── recover/
│   │   ├── mod.rs
│   │   ├── strategies.rs
│   │   └── suggest.rs
│   ├── compact/
│   │   ├── mod.rs
│   │   ├── skeleton.rs
│   │   └── diff.rs
│   ├── memory/
│   │   ├── mod.rs
│   │   ├── store.rs
│   │   └── pattern.rs
│   ├── env/
│   │   ├── mod.rs
│   │   ├── detector.rs
│   │   └── platform.rs
│   ├── sandbox/
│   │   ├── mod.rs
│   │   └── scrubber.rs
│   └── utils.rs
├── tests/
│   ├── integration_tests.rs
│   └── fixtures/
│       ├── large_log.txt
│       └── json_output.txt
└── benches/
    └── compaction_bench.rs
```

---

## 10. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Classification misses edge cases | High | Medium | Extensive regex test suite + fallback to `R30` |
| Cross-shell compatibility gaps | Medium | Medium | Focus only on Bash/Zsh; document limitations |
| Performance on massive outputs | Medium | Low | Streaming compaction, configurable size limits |
| MCP protocol changes | Low | High | Abstract transport layer, easy to adapt |
| Over-aggressive secret scrubbing | Medium | Medium | Allowlist for non-secrets, user-configurable |

---

## 11. References

1. **PALADIN** — Self-Correcting Language Model Agents to Cure Tool-Failure Cases ([arXiv:2509.25238](https://arxiv.org/html/2509.25238v1))
2. **AgentRx** — Diagnosing AI Agent Failures from Execution Trajectories (Microsoft Research, 2026)
3. **Agentic CLI Design** — 7 Principles for Designing CLI as a Protocol for AI Agents (DEV Community)
4. **Claude Code Bash Tool** — Anthropic API Docs ([platform.claude.com](https://platform.claude.com/docs/en/agents-and-tools/tool-use/bash-tool))
5. **Gemini CLI Issue #8081** — Tool Self-Healing for Enhanced Agent Resilience (GitHub)
6. **`oo` / `tsh`** — Smart output classification and limiting for LLM agents
7. **`fix-cli`** — AI-powered command fixer with contract-based dispute resolution

---

## 12. Next Steps

1. Initialize the Rust project (`cargo init`)
2. Set up CI (GitHub Actions: `cargo test`, `cargo clippy`, `cargo fmt`)
3. Implement Phase 1 (Foundation)
4. Open a tracking issue for each phase
5. Begin recruiting contributors for pattern library expansion

---

*This plan was synthesized from research into Anthropic, Google, Microsoft, and open-source agent tooling ecosystems. It prioritizes determinism, safety, and context efficiency over LLM-powered heuristics.*
