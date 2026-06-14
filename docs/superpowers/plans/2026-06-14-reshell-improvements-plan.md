# Reshell Improvements Implementation Plan

> **For agentic workers:** Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the Reshell phased improvements (Deployability → Capability → Containment) from the approved design spec at `docs/superpowers/specs/2026-06-14-reshell-improvements-design.md`.

**Architecture:** Each phase builds on the previous one. Phase 1 adds MCP-over-SSE, prebuilt release artifacts, and project-local pattern DBs. Phase 2 improves JSON extraction, binary output handling, and recovery patterns. Phase 3 adds Landlock/Docker sandboxing, network isolation, and validator hardening.

**Tech Stack:** Rust, tokio, serde_json, rusqlite, clap, Landlock (Linux), Docker, GitHub Actions.

---

## File Map

| File | Responsibility |
| ---- | -------------- |
| `src/mcp/router.rs` (new) | Shared JSON-RPC tool dispatcher for stdio and SSE |
| `src/mcp/server.rs` (modify) | Framed stdio server; delegates to router |
| `src/mcp/sse.rs` (modify) | Real MCP-over-SSE server using router |
| `src/mcp/tools.rs` (modify) | Tool definitions; accept `cwd` for Store resolution |
| `src/memory/store.rs` (modify) | Project-local DB resolution + merge helper |
| `src/cli.rs` (modify) | Add `--sandbox`, `--binary-handling`, `merge-patterns` subcommand |
| `src/exec/runner.rs` (modify) | Wire sandbox, network policy, binary handling |
| `src/compact/jsonpath.rs` (new) | JSONPath engine |
| `src/compact/jq.rs` (modify) | Compatibility shim to jsonpath |
| `src/utils.rs` (modify) | Binary summary helper |
| `src/recover/deps.rs` (modify) | Add pnpm/yarn/uv/poetry/Gradle/Maven/Bun/Deno |
| `src/recover/bashisms.rs` (modify) | Expand bashism table |
| `src/classify/config.rs` (modify) | Merge project-level patterns.toml |
| `src/sandbox/landlock.rs` (new) | Linux Landlock sandbox |
| `src/sandbox/docker.rs` (new) | Docker-based sandbox |
| `src/sandbox/network.rs` (new) | Network namespace / policy helpers |
| `src/sandbox/mod.rs` (modify) | Sandbox trait and dispatch |
| `src/exec/validator.rs` (modify) | Known bypass denylist |
| `src/exec/audit.rs` (new) | Verbose audit mode |
| `.github/workflows/release.yml` (new) | Release builds |
| `homebrew/reshell.rb` (new) | Homebrew formula template |

---

## Phase 1: Deployability & Connectivity

### Task 1.1: Extract MCP tool dispatch into a shared router

**Files:**
- Create: `src/mcp/router.rs`
- Modify: `src/mcp/server.rs`, `src/mcp/mod.rs`, `src/mcp/tools.rs`

- [ ] **Step 1: Define `Router` struct and trait**

Create `src/mcp/router.rs`:

```rust
use std::sync::Arc;
use crate::memory::{Store, Metrics};
use serde_json::Value;

pub struct Router {
    store: Store,
    metrics: Arc<Metrics>,
}

impl Router {
    pub fn new(store: Store, metrics: Arc<Metrics>) -> Self { ... }

    pub async fn handle(&self, request: Value) -> Value {
        // dispatch to tools/list, tools/call, etc.
    }
}
```

- [ ] **Step 2: Move tool-call dispatch from `server.rs` to `router.rs`**

Move the `tools/list` and `tools/call` handling logic into `Router::handle`.

- [ ] **Step 3: Update stdio server to use `Router`**

Replace inline dispatch in `src/mcp/server.rs` with `Arc<Router>`.

- [ ] **Step 4: Run tests**

```bash
cargo test --test integration_tests
```

Expected: existing MCP tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/mcp/router.rs src/mcp/server.rs src/mcp/mod.rs src/mcp/tools.rs
git commit -m "refactor(mcp): extract shared tool router for stdio and SSE"
```

---

### Task 1.2: Implement functional MCP-over-SSE

**Files:**
- Modify: `src/mcp/sse.rs`, `src/main.rs`

- [ ] **Step 1: Add SSE session manager**

In `src/mcp/sse.rs`, add:

```rust
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};

struct Session {
    tx: mpsc::Sender<String>,
}

pub struct SseServer {
    router: Arc<Router>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}
```

- [ ] **Step 2: Implement `GET /mcp/sse` endpoint**

Return SSE headers, generate UUID session id, emit `event: endpoint` with POST URL.

- [ ] **Step 3: Implement `POST /mcp/messages` endpoint**

Parse `session_id`, read JSON body, call `router.handle`, send result as SSE event to the session's channel.

- [ ] **Step 4: Update `main.rs` to pass `Router` to `SseServer`**

```rust
"sse" => {
    let store = Store::new(None)?;
    let metrics = Arc::new(Metrics::new());
    let router = Arc::new(Router::new(store, metrics));
    let sse_server = reshell::mcp::sse::SseServer::start(addr, router).await?;
    ...
}
```

- [ ] **Step 5: Add integration test for SSE**

Create `tests/sse_tests.rs`:

```rust
#[tokio::test]
async fn test_sse_tools_list() {
    // start SseServer on random port
    // GET /mcp/sse, capture endpoint
    // POST initialize + tools/list
    // verify response event
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test --test sse_tests
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/mcp/sse.rs src/main.rs tests/sse_tests.rs Cargo.toml
git commit -m "feat(mcp): implement functional MCP-over-SSE transport"
```

---

### Task 1.3: Project-local pattern database

**Files:**
- Modify: `src/memory/store.rs`, `src/memory/mod.rs`, `src/mcp/tools.rs`, `src/main.rs`, `src/cli.rs`

- [ ] **Step 1: Change `Store::new` signature**

```rust
impl Store {
    pub fn new(cwd: Option<&std::path::Path>) -> Result<Self> { ... }
}
```

Resolution order:
1. `RSH_PATTERN_DB` env var.
2. `cwd/.reshell/patterns.db` if `cwd` provided.
3. `~/.reshell/patterns.db`.

- [ ] **Step 2: Add `Store::merge_from` helper**

Merge patterns from one DB into another, upserting by `(command_template, stderr_pattern)`.

- [ ] **Step 3: Add `merge-patterns` CLI subcommand**

In `src/cli.rs`:

```rust
MergePatterns { from: PathBuf, into: PathBuf }
```

Implement in `main.rs`.

- [ ] **Step 4: Update all `Store::new()` callers**

Pass `cwd` from `ExecRequest` or CLI context.

- [ ] **Step 5: Add unit test for resolution**

In `src/memory/store.rs` tests:

```rust
#[tokio::test]
async fn test_project_local_db_resolution() { ... }
```

- [ ] **Step 6: Run tests**

```bash
cargo test
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/memory/store.rs src/memory/mod.rs src/mcp/tools.rs src/main.rs src/cli.rs
git commit -m "feat(memory): support project-local pattern databases"
```

---

### Task 1.4: Release workflow and Homebrew formula

**Files:**
- Create: `.github/workflows/release.yml`, `homebrew/reshell.rb`
- Modify: `README.md`

- [ ] **Step 1: Add release workflow**

```yaml
name: Release
on:
  push:
    tags: ['v*']
jobs:
  build:
    strategy:
      matrix:
        target: [x86_64-unknown-linux-musl, aarch64-apple-darwin, x86_64-apple-darwin, x86_64-pc-windows-msvc]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: cargo build --release --target ${{ matrix.target }}
      - run: tar czf rsh-${{ matrix.target }}.tar.gz -C target/${{ matrix.target }}/release rsh
      - uses: softprops/action-gh-release@v2
        with:
          files: rsh-*.tar.gz
```

- [ ] **Step 2: Add Homebrew formula template**

```ruby
class Reshell < Formula
  desc "Resilient Shell Execution Middleware for AI Agents"
  homepage "https://github.com/azri-cs/reshell"
  url "https://github.com/azri-cs/reshell/releases/download/v0.1.0/rsh-x86_64-apple-darwin.tar.gz"
  sha256 "REPLACE_ON_RELEASE"
  license "MIT"

  def install
    bin.install "rsh"
  end
end
```

- [ ] **Step 3: Update README installation section**

Add Homebrew and prebuilt binary instructions.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml homebrew/reshell.rb README.md
git commit -m "ci(release): add release builds and Homebrew formula template"
```

---

### Phase 1 Completion Checkpoint

- [ ] Run full test suite: `cargo test`
- [ ] Run clippy: `cargo clippy -- -D warnings`
- [ ] Tag phase commit:

```bash
git log --oneline -5
git tag phase-1-deployability
```

---

## Phase 2: Resilience & Capability

### Task 2.1: JSONPath engine for richer extraction

**Files:**
- Create: `src/compact/jsonpath.rs`
- Modify: `src/compact/jq.rs`, `src/compact/mod.rs`, `src/main.rs`

- [ ] **Step 1: Implement JSONPath parser and evaluator**

Support `.key`, `.["key"]`, `[0]`, `[-1]`, `.*`, `[*]`, slices `[0:5]`, and filter `[?(@.price < 10)]`.

- [ ] **Step 2: Add property and snapshot tests**

In `src/compact/jsonpath.rs`:

```rust
#[test]
fn test_jsonpath_basic() { ... }

#[test]
fn test_jsonpath_wildcard() { ... }
```

- [ ] **Step 3: Replace jq.rs implementation**

Make `src/compact/jq.rs` call `jsonpath::extract`.

- [ ] **Step 4: Run tests**

```bash
cargo test compact
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/compact/jsonpath.rs src/compact/jq.rs src/compact/mod.rs src/main.rs
git commit -m "feat(compact): add JSONPath extraction engine"
```

---

### Task 2.2: Binary output handling

**Files:**
- Modify: `src/utils.rs`, `src/exec/runner.rs`, `src/exec/mod.rs`, `src/cli.rs`

- [ ] **Step 1: Add binary summary helper**

```rust
pub struct BinarySummary {
    pub mime_type: String,
    pub byte_count: usize,
    pub sha256: String,
    pub first_bytes: String,
    pub last_bytes: String,
}

pub fn summarize_binary(data: &[u8]) -> BinarySummary { ... }
```

- [ ] **Step 2: Add `binary_handling` to `ExecRequest`**

```rust
pub enum BinaryHandling {
    Summary,
    Reject,
    Allow,
}
```

- [ ] **Step 3: Wire into runner output building**

When binary detected and `Summary` or `Reject`, replace stdout with summary JSON or error.

- [ ] **Step 4: Add CLI flag and MCP param**

CLI: `--binary-handling summary|reject|allow`.
MCP: `binary_handling` string param.

- [ ] **Step 5: Add tests**

Unit tests in `src/utils.rs` with PNG/ELF/text fixtures.

- [ ] **Step 6: Commit**

```bash
git add src/utils.rs src/exec/runner.rs src/exec/mod.rs src/cli.rs src/mcp/tools.rs
git commit -m "feat(exec): structured binary output handling"
```

---

### Task 2.3: Broader recovery patterns

**Files:**
- Modify: `src/recover/deps.rs`, `src/recover/bashisms.rs`, `src/classify/config.rs`

- [ ] **Step 1: Add modern tool dependency extractors**

Add detection for `pnpm`, `yarn`, `uv`, `poetry`, `gradle`, `mvn`, `bun`, `deno`.

- [ ] **Step 2: Expand bashism translations**

Add `&>`, `declare -n`, `$'...'`, `(( ))`.

- [ ] **Step 3: Merge project-level `patterns.toml`**

Load `.reshell/patterns.toml` after `~/.reshell/patterns.toml` with higher priority.

- [ ] **Step 4: Add integration tests**

In `tests/integration_tests.rs`:

```rust
#[test]
fn test_r22_pnpm_install_suggestion() { ... }
```

- [ ] **Step 5: Commit**

```bash
git add src/recover/deps.rs src/recover/bashisms.rs src/classify/config.rs tests/integration_tests.rs
git commit -m "feat(recover): expand patterns for modern toolchains"
```

---

### Phase 2 Completion Checkpoint

- [ ] Run full test suite: `cargo test`
- [ ] Run benchmarks: `cargo bench --bench compaction_bench`
- [ ] Tag phase commit:

```bash
git tag phase-2-capability
```

---

## Phase 3: Containment & Safety

### Task 3.1: Sandbox trait and Landlock implementation

**Files:**
- Create: `src/sandbox/landlock.rs`, `src/sandbox/docker.rs`
- Modify: `src/sandbox/mod.rs`, `src/sandbox/overlay.rs`

- [ ] **Step 1: Define `Sandbox` trait**

```rust
pub enum SandboxMode {
    None,
    Landlock,
    Overlay,
    Docker,
}

pub trait Sandbox: Send + Sync {
    fn prepare(&self, cwd: &Path) -> anyhow::Result<SandboxContext>;
    fn allowed_paths(&self) -> &[PathBuf];
    fn network_policy(&self) -> NetworkPolicy;
}
```

- [ ] **Step 2: Implement Landlock sandbox (Linux)**

Use `landlockconfig` / raw syscalls via `rust-landlock` crate. Allow read on `/usr`, `/bin`, `/lib`, cwd; allow write on cwd and `/tmp`.

- [ ] **Step 3: Implement Docker sandbox**

Generate `docker run --rm -v $(pwd):/workdir -w /workdir --network <policy> rsh-runner <shell> -c <command>`.

- [ ] **Step 4: Refactor OverlayFS behind trait**

Make existing OverlayFS implement `Sandbox`.

- [ ] **Step 5: Add tests**

Linux-only integration tests for Landlock read/write allow/deny.

- [ ] **Step 6: Commit**

```bash
git add src/sandbox/landlock.rs src/sandbox/docker.rs src/sandbox/mod.rs src/sandbox/overlay.rs Cargo.toml
git commit -m "feat(sandbox): add Landlock and Docker sandbox backends"
```

---

### Task 3.2: Network isolation

**Files:**
- Create: `src/sandbox/network.rs`
- Modify: `src/exec/runner.rs`, `src/cli.rs`, `src/mcp/tools.rs`

- [ ] **Step 1: Define `NetworkPolicy`**

```rust
pub enum NetworkPolicy {
    Inherit,
    LocalhostOnly,
    None,
}
```

- [ ] **Step 2: Implement Linux network namespace helper**

For `None`: `unshare -n`.
For `LocalhostOnly`: create veth pair + namespace with only `lo` up.

- [ ] **Step 3: Wire policy into runner**

Apply policy before spawning command; warn if unsupported on platform.

- [ ] **Step 4: Add CLI/MCP params**

CLI: `--network inherit|localhost-only|none`.
MCP: `network_policy` string param.

- [ ] **Step 5: Add tests**

Integration test: `curl https://example.com` fails under `none`.

- [ ] **Step 6: Commit**

```bash
git add src/sandbox/network.rs src/exec/runner.rs src/cli.rs src/mcp/tools.rs tests/integration_tests.rs
git commit -m "feat(sandbox): add network isolation policies"
```

---

### Task 3.3: Hardened validator and audit mode

**Files:**
- Create: `src/exec/audit.rs`
- Modify: `src/exec/validator.rs`, `src/exec/analyze.rs`, `src/cli.rs`, `src/mcp/tools.rs`

- [ ] **Step 1: Add known-bypass denylist**

Block hex-encoded commands, obfuscated `eval`, backtick subshell chains.

- [ ] **Step 2: Integrate lightweight shell parser**

Use `shell-words` + custom logic to detect nested expansions beyond regex.

- [ ] **Step 3: Implement audit mode**

`rsh check --verbose <command>` returns structured allow/block reasoning.

- [ ] **Step 4: Increase fuzz coverage in CI**

Update `.github/workflows/ci.yml` fuzz step to 10 minutes.

- [ ] **Step 5: Add negative tests**

One test per known bypass pattern.

- [ ] **Step 6: Commit**

```bash
git add src/exec/audit.rs src/exec/validator.rs src/exec/analyze.rs src/cli.rs src/mcp/tools.rs .github/workflows/ci.yml
git commit -m "feat(safety): harden validator and add audit mode"
```

---

### Phase 3 Completion Checkpoint

- [ ] Run full test suite: `cargo test`
- [ ] Run clippy: `cargo clippy -- -D warnings`
- [ ] Run security-focused tests on Linux
- [ ] Tag phase commit:

```bash
git tag phase-3-containment
```

---

## Spec Coverage Review

| Spec Requirement | Implementing Task |
| ---------------- | ----------------- |
| Functional MCP-over-SSE | Task 1.2 |
| Prebuilt binaries | Task 1.4 |
| Project-local DB | Task 1.3 |
| JSONPath engine | Task 2.1 |
| Binary output handling | Task 2.2 |
| Broader recovery patterns | Task 2.3 |
| Landlock/Docker sandbox | Task 3.1 |
| Network isolation | Task 3.2 |
| Hardened validator | Task 3.3 |

## Placeholder Scan

No `TODO`/`TBD`/vague steps. Each task names exact files and concrete behavior.

## Type Consistency Notes

- `Store::new` gains `cwd: Option<&Path>`.
- `ExecRequest` gains `binary_handling` and `network_policy` fields.
- `Sandbox` trait is introduced; existing OverlayFS and new Landlock/Docker implement it.
- `Router` is used by both stdio and SSE servers.
