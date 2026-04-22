# AGENTS.md

## Start here
- This is a single-package Rust crate (`Cargo.toml`) that builds one binary: `rsh`. No workspace, repo-local CI, or custom lint/format config was found; use plain Cargo commands.
- Broad globs will hit generated files under `target/` (already present in the repo). Search `src/`, `tests/`, and `benches/` explicitly, and never edit `target/`.

## Real entrypoints
- `src/main.rs` is the only CLI entrypoint. Subcommands dispatch to:
  - `mcp` -> `src/mcp/{server,tools}.rs`
  - `exec` -> `src/exec/{validator,runner}.rs`
  - `env` -> `src/env/detector.rs`
  - `compact` -> `src/compact/*`
- `src/lib.rs` only re-exports modules; there are no additional packages or hidden binaries.

## Commands you can trust
- `cargo run -- exec --command "echo hello"`
- `cargo run -- env`
- `cargo run -- compact --file tests/fixtures/large_log.txt`
- `cargo run -- mcp`
- `cargo test`
- `cargo test --test integration_tests`
- `cargo test test_cli_exec_echo --test integration_tests -- --exact`
- `cargo bench --bench compaction_bench`

## Trust code over docs
- `README.md` and `RESHELL_PLAN.md` are ahead of the implementation in a few places; verify behavior in `src/**` before claiming a feature exists.
- `src/exec/runner.rs` always executes via `sh -c`. Even though the docs discuss Bash/Zsh, current command semantics are POSIX `sh` unless you change the runner.
- The MCP server is newline-delimited JSON-RPC over stdio (`src/mcp/server.rs` reads `stdin.lines()`), not header-framed stdio MCP. Keep tests/clients aligned unless you upgrade the transport everywhere.
- `compact` only works from a file today. `output_id` and `view` exist in the CLI/tool schemas but are ignored in `src/main.rs` and `src/mcp/tools.rs`.
- `retry` is parsed in CLI/tool inputs but unused by `Runner`.
- Pattern memory is only partially wired: runs create `~/.reshell/patterns.db`, but the hot path currently only calls `save_output`; learned pattern lookup/update is not used.
- The advertised “safety sandbox” is currently just pre-exec command blocking plus stderr secret scrubbing; `src/sandbox/` does not implement filesystem or network isolation.

## Testing / state gotchas
- `tests/integration_tests.rs` is end-to-end: it spawns the built `rsh` binary via `CARGO_BIN_EXE_rsh` and talks to the MCP server over stdio. If you change CLI flags, MCP payloads, or response shapes, update these tests too.
- Local runs/tests write persistent state to `~/.reshell/patterns.db`; clean that manually if stateful behavior starts affecting a repro.
- Compaction fixtures live in `tests/fixtures/`.
