# Reshell Improvement Design Spec

**Approach 3 — Phased Roadmap (Deployability → Capability → Containment)**

**Date:** 2026-06-14  
**Author:** OpenCode Build Agent  
**Status:** Approved for implementation

---

## 1. Overview & Goals

Reshell currently works as a deterministic shell-execution middleware, but several caveats limit its usefulness for OpenCode: the SSE transport is a stub, pattern memory is trapped on one machine, there is no real filesystem/network isolation, and some aspirational features are only partially implemented.

This design sequences improvements into three independent phases so each can ship and deliver value on its own.

**Primary goals:**
1. Make Reshell trivial for OpenCode to discover, install, and connect to.
2. Make pattern memory portable and team-shareable.
3. Improve output handling and recovery for modern development toolchains.
4. Provide optional, non-root filesystem and network containment.

**Success criteria:**
- OpenCode can connect via stdio **or** SSE without building from source.
- A team can commit learned fixes alongside a project.
- Large/binary outputs are handled predictably.
- Dangerous commands can be run inside an opt-in sandbox that works on Linux, macOS, and Windows (via Docker).

---

## 2. Non-Goals

- **LLM-based recovery.** The zero-LLM constraint remains; no OpenAI/Claude calls in the recovery path.
- **Kernel modules or custom seccomp-bpf.** Use existing primitives (Landlock, namespaces, Docker) instead.
- **Windows-native sandbox.** Windows containment will use WSL2/Docker, not a from-scratch sandbox.
- **Cloud sync service.** Pattern sync will be file-based (project DB + optional Git), not a hosted service.

---

## 3. Phase 1 — Deployability & Connectivity

### 3.1 Functional MCP-over-SSE

**Current state:** `src/mcp/sse.rs` starts an HTTP server but does not route JSON-RPC to the tool handler.

**Design:**
- Reuse the existing `McpServer` state (`Store`, `Metrics`) and tool router from `src/mcp/tools.rs`.
- Add an SSE session manager:
  - `GET /mcp/sse` opens an SSE stream, assigns a `session_id`, and emits an `endpoint` event pointing to `POST /mcp/messages?session_id=...`.
  - `POST /mcp/messages?session_id=...` accepts JSON-RPC requests and routes them through the same `handle_tool_call` / `handle_list_tools` logic used by stdio.
  - Responses are sent as SSE events with `event: message` and `data: <jsonrpc response>`.
- Use a bounded in-memory channel per session (e.g., `tokio::sync::mpsc` with capacity 64) to queue outbound events.
- Keep stdio as the default; SSE is opt-in via `rsh mcp --transport sse --port 3000`.

**Key changes:**
- Extract tool dispatch into `src/mcp/router.rs` shared by stdio and SSE.
- `SseServer` holds `Arc<Router>` instead of a stub handler.

### 3.2 Prebuilt Binaries & Distribution

**Current state:** Users must build from source.

**Design:**
- Add a GitHub Actions release workflow:
  - Build statically linked Linux binaries (musl).
  - Build macOS x86_64 + aarch64 binaries.
  - Build Windows x86_64 binary (MSVC).
  - Attach `.tar.gz`/`.zip` artifacts and a checksum file.
- Add a Homebrew tap repo (`azri-cs/homebrew-reshell`) with a generated formula.
- Update README installation instructions.
- Optional: publish to crates.io so `cargo install reshell` works.

**Key changes:**
- New `.github/workflows/release.yml`.
- New `homebrew/reshell.rb` template.
- Cross-compilation targets in CI.

### 3.3 Project-Local Pattern Database

**Current state:** `Store::new()` always resolves to `~/.reshell/patterns.db`.

**Design:**
- Add lookup order:
  1. `RSH_PATTERN_DB` environment variable.
  2. `.reshell/patterns.db` in the current working directory (or `cwd` from `ExecRequest`).
  3. `~/.reshell/patterns.db`.
- CLI `rsh exec` and MCP `rsh_exec` honor the same lookup.
- Add `rsh_stats` field showing which DB file is active.
- Provide a migration/merge helper: `rsh merge-patterns --from <path> --into <path>`.

**Key changes:**
- `Store::new()` → `Store::new(cwd: Option<&Path>)`.
- Update all callers in `main.rs`, `tools.rs`, tests.

---

## 4. Phase 2 — Resilience & Capability

### 4.1 Richer jq-like JSON Extraction

**Current state:** `src/compact/jq.rs` supports only simple dot/bracket paths.

**Design:**
- Replace the hand-rolled parser with a small, auditable JSONPath engine supporting:
  - Child keys: `.key`, `.["escaped key"]`
  - Array indices: `.array[0]`, `.array[-1]`
  - Wildcards: `.*`, `[*]`
  - Filters: `?key`, `[?(@.price < 10)]` (subset)
  - Slices: `.array[0:5]`
- Keep the existing `rsh compact --jq <path>` UX.
- Return clear errors for unsupported operations.

**Key changes:**
- New `src/compact/jsonpath.rs` module.
- Deprecate `src/compact/jq.rs` or make it a thin compatibility shim.

### 4.2 Binary Output Detection & Enforcement

**Current state:** Binary output is detected but still returned if small.

**Design:**
- Always detect binary output using MIME/null-byte heuristics.
- When binary output is detected:
  - Return a structured summary: MIME type, byte count, SHA-256 hash, first/last bytes (hex).
  - Never emit raw binary into MCP text content.
- Add `rsh_exec` option: `binary_handling: "summary" | "reject" | "allow"` (default `"summary"`).
- For CLI `rsh exec`, default to `summary`; allow `--binary-allow`.

**Key changes:**
- `src/utils.rs` binary detection already exists; wire it into runner output building.
- Update `ExecResult` schema with `binary_summary` field.

### 4.3 Broader Recovery Patterns

**Current state:** Dependency extraction covers npm, pip, cargo, gem, go, docker, apt, composer.

**Design:**
- Add extractors for:
  - **pnpm / yarn:** `pnpm: command not found`, `yarn: command not found`, lockfile parsing.
  - **uv / poetry:** missing `uv`/`poetry`, Python version mismatches.
  - **Gradle / Maven:** Java build tool diagnostics.
  - **Bun / Deno:** modern JS runtimes.
- Expand bashism translation table in `src/recover/bashisms.rs` for:
  - `&>` redirect
  - `declare -n` namerefs
  - `$'{...}'` quoting
  - `(( ))` arithmetic
- Add user-extensible `~/.reshell/patterns.toml` and project-level `.reshell/patterns.toml`.

**Key changes:**
- Extend `src/recover/deps.rs`.
- Add config merging for user + project patterns.

---

## 5. Phase 3 — Containment & Safety

### 5.1 Non-Root Filesystem Sandbox

**Current state:** OverlayFS sandbox exists but requires root/CAP_SYS_ADMIN and is Linux-only.

**Design:**
- **Linux:** Use `Landlock LSM` for path-based access control (works without root on kernel ≥ 5.13) as the default containment. Keep OverlayFS as an opt-in stricter mode.
- **macOS:** Use an app sandbox profile or documented Docker mode. Default to Docker-based containment.
- **Windows:** Use WSL2 or Docker Desktop.
- Provide a unified `Sandbox` trait:
  ```rust
  trait Sandbox {
      fn prepare(&self, cwd: &Path) -> Result<SandboxHandle>;
      fn allowed_paths(&self) -> &[PathBuf];
      fn network_policy(&self) -> NetworkPolicy;
  }
  ```
- Add `rsh exec --sandbox=landlock|overlay|docker|none`.
- MCP `rsh_exec` gains `sandbox: string` option.

**Key changes:**
- New `src/sandbox/landlock.rs` (Linux).
- New `src/sandbox/docker.rs` (cross-platform).
- Refactor `src/sandbox/overlay.rs` behind the trait.

### 5.2 Network Isolation

**Current state:** No network restrictions.

**Design:**
- Add `network_policy` enum: `inherit` (default), `localhost_only`, `none`.
- **Linux:** Use network namespaces (`unshare -n`) plus a `lo`-only veth pair for `localhost_only`.
- **Docker mode:** Pass `--network none` or `--network host` accordingly.
- **Landlock-only mode:** Network is not restricted; emit a warning.
- Add validator check: block commands that try to override network restrictions (`curl --unix-socket` still allowed in `localhost_only`).

**Key changes:**
- `src/exec/runner.rs` applies network policy before spawning.
- New `src/sandbox/network.rs`.

### 5.3 Hardened Validator

**Current state:** Regex-based validator and simple AST counter.

**Design:**
- Add a denylist of known bypass patterns from literature (subshell obfuscation, hex-encoded commands, `eval` via backticks).
- Integrate a lightweight shell parser (e.g., `shell-words` + custom) to catch structural obfuscation beyond regex.
- Add negative tests for each bypass.
- Increase fuzzing coverage: 10 minutes per run in CI, with corpus seeding.
- Add audit mode: `rsh check --verbose` explains why a command was blocked/allowed.

**Key changes:**
- Extend `src/exec/validator.rs` and `src/exec/analyze.rs`.
- New `src/exec/audit.rs`.
- Update `.github/workflows/ci.yml` fuzz duration.

---

## 6. Architecture Changes

The current architecture stays largely intact. The main additions are:

1. **Shared MCP Router** (`src/mcp/router.rs`) used by stdio and SSE.
2. **Store Resolution** now takes a `cwd` parameter.
3. **Sandbox Trait** abstracts Landlock, OverlayFS, Docker, and no-op modes.
4. **Network Policy** applied in the runner before process spawn.
5. **JSONPath Engine** replaces the minimal jq parser.

No existing public tool schemas change in a breaking way; new fields are additive.

---

## 7. Data Flow

For a sandboxed `rsh_exec` call:

```
Agent → MCP Router → tools::rsh_exec
                    → Runner::run
                      → validator (pre-check)
                      → env::Detector
                      → Sandbox::prepare
                      → network policy applied
                      → tokio::process::Command
                      → scrubber
                      → classify
                      → compact
                      → recover
                      → Store::save_pattern / update_fix_outcome
                    → JSON response
```

---

## 8. Error Handling

- **SSE transport errors:** Return JSON-RPC error objects; log session failures to stderr.
- **Sandbox preparation failures:** Fail open with a warning unless `sandbox=strict` is set.
- **Landlock unavailable:** Fall back to OverlayFS if privileged, otherwise warn and run unsandboxed.
- **Project DB missing:** Create it automatically (same as global DB).
- **JSONPath parse errors:** Return clear `INVALID_JSONPATH` error with position.

---

## 9. Testing Strategy

| Layer               | Tests                                                                             |
| ------------------- | --------------------------------------------------------------------------------- |
| SSE transport       | Integration test: open SSE stream, send `tools/list`, verify response event         |
| Project-local DB    | Unit test: `Store::new(Some(cwd))` resolves correctly; merge helper test            |
| JSONPath            | Property tests: round-trip against `serde_json::Value`; snapshot tests for examples |
| Binary handling     | Unit tests with fixtures: ELF, PNG, PDF, UTF-8 text                               |
| Recovery patterns   | Per-tool integration tests for pnpm/uv/Gradle failures                            |
| Landlock sandbox    | Linux-only integration tests with temp dirs; verify read/write allow/deny         |
| Network isolation   | Integration test: `curl https://example.com` fails in `none` policy                   |
| Validator hardening | Negative tests for each bypass pattern; fuzz CI                                   |

---

## 10. Risks & Mitigations

| Risk                                    | Mitigation                                                     |
| --------------------------------------- | -------------------------------------------------------------- |
| Landlock not available on older kernels | Graceful fallback to OverlayFS or unsandboxed with warning     |
| SSE adds attack surface                 | Bind to `127.0.0.1` only; validate session IDs; bound channels   |
| Docker sandbox requires Docker          | Document prerequisite; provide `none` fallback                   |
| JSONPath engine bloats binary           | Make it a compile-time feature if binary size matters          |
| Project DB pollutes repos               | Default to `.reshell/` in `.gitignore`; keep global DB as fallback |

---

## 11. Approaches Considered

| Approach | Scope | Pros | Cons |
| -------- | ----- | ---- | ---- |
| Deployability First | SSE, binaries, project DB | Fast value, low risk | Leaves isolation gaps |
| Safety First | Real sandbox + network | Addresses biggest caveat | High effort, platform-specific |
| Phased Roadmap (selected) | Deployability → Capability → Containment | Balanced, incremental, ships early | Full containment arrives last |
