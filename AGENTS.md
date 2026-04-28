# AGENTS.md

## Start here
- Single-package Rust crate (`Cargo.toml`) → one binary: `rsh`. No workspace, no CI config, no rustfmt/clippy config. Use plain Cargo commands.
- `target/` is present in the repo. Search `src/`, `tests/`, and `benches/` explicitly; never edit `target/`.

## Entrypoints
- `src/main.rs` — CLI entrypoint. Subcommands:
  - `mcp` → `src/mcp/{server,tools}.rs`
  - `exec` → `src/exec/{validator,runner}.rs`
  - `env` → `src/env/detector.rs`
  - `compact` → `src/compact/*`
- `src/lib.rs` — re-exports modules only; no additional packages or hidden binaries.

## Commands you can trust
- `cargo run -- exec --command "echo hello"`
- `cargo run -- env`
- `cargo run -- compact --file tests/fixtures/large_log.txt`
- `cargo run -- compact --output-id <UUID> --view errors_only`
- `cargo run -- mcp`
- `cargo test` — all tests (unit + integration)
- `cargo test --test integration_tests` — end-to-end only
- `cargo test test_cli_exec_echo --test integration_tests -- --exact`
- `cargo bench --bench compaction_bench`

## Execution model (important)
- `detector.execution_shell()` is hardcoded to `"sh"` (see `src/env/detector.rs`). Primary execution always uses `sh -c`.
- **Retry path**: when `--retry` is true (default) and the first attempt classifies as `R25` (environment mismatch), the runner re-executes using `detector.recovery_shell()` — which is `$SHELL` if it's not `sh` (e.g. `/bin/bash`, `/bin/zsh`). See `src/exec/runner.rs` lines 57–85.
- The retry wraps the command: `<fallback_shell> -c '<original_command>'` via `posix_retry_request`.

## Trust code over docs
- `README.md` and `RESHELL_PLAN.md` can drift; verify against `src/**` before claiming a feature exists. Taxonomy codes: `src/classify/taxonomy.rs`. MCP tool names and schemas: `src/mcp/tools.rs` (`list_tools`).
- Aspirational / not implemented: OverlayFS sandbox, binary output detection, jq-like extraction, SSE transport (see `RESHELL_PLAN.md`).
- The MCP server is **newline-delimited JSON-RPC over stdio** (`src/mcp/server.rs` reads `stdin.lines()`), not header-framed stdio MCP. Keep tests/clients aligned unless you upgrade the transport.
- The "safety sandbox" is pre-exec validation (patterns, interactive commands), optional `~/.reshell/allowlist.toml` command allowlist (`src/sandbox/allowlist.rs`), and stderr secret scrubbing (`src/sandbox/scrubber.rs`). No filesystem or network isolation exists.

## Pattern memory
- State lives at `~/.reshell/patterns.db` (SQLite, created automatically via `rusqlite` with bundled feature).
- On failure, the runner looks up `find_pattern(command_template, stderr)` and reuses learned fixes with `fix_success_rate >= 0.5`.
- On non-success, non-R10 results where no pattern exists, a new pattern is saved. `save_pattern` upserts by `(command_template, stderr_pattern)` and increments `usage_count`.

## Testing / state gotchas
- **Integration tests** (`tests/integration_tests.rs`) are end-to-end: they spawn the built `rsh` binary via `CARGO_BIN_EXE_rsh` and talk to the MCP server over stdio. If you change CLI flags, MCP payloads, or response shapes, update these tests.
- Each integration test calls `unique_home_dir()` to isolate `~/.reshell/patterns.db` into a temp directory — tests should not pollute each other.
- Unit tests in `src/exec/runner.rs` use `tempfile::tempdir()` for the same reason.
- If running `cargo run -- exec` manually, state persists at `~/.reshell/patterns.db`; clean it manually if reproducibility matters.
- Compaction fixtures: `tests/fixtures/{large_log.txt, json_output.txt}`.
- `insta` (dev-dep) is available for snapshot testing.
