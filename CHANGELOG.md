# Changelog

All notable changes to Reshell will be documented in this file.

## [0.1.0] - Unreleased

### Added
- Deterministic shell command execution with structured JSON output
- Failure taxonomy (R10–R30) with regex-driven classification
- Zero-LLM recovery engine with per-class suggestion templates
- Output compaction: head + structural skeleton + tail truncation
- SQLite-backed pattern memory with learned fix reuse
- MCP server over framed stdio transport (Content-Length headers)
- Safety sandbox: pre-exec validation, allowlist, secret scrubbing
- 9 MCP tools: rsh_exec, rsh_env, rsh_recover, rsh_compact, rsh_read_file, rsh_write_file, rsh_check, rsh_feedback, rsh_stats
- Bashism-to-POSIX translation (10+ patterns)
- Command alternatives mapping (18 tools)
- Missing dependency extraction from stderr (npm, pip, cargo, etc.)
- Language-aware output skeleton extraction (12 languages)
- JSON structural summary compaction
- Linux seccomp syscall filtering (opt-in)
- AST-level command obfuscation analysis
- OverlayFS filesystem isolation sandbox (Linux)
- Shell completion generation (bash, zsh, fish)
- SSE transport for MCP server
- jq-like JSON path extraction
- Success metrics telemetry
- Pattern LRU eviction
- MIME-based binary output detection
- Fuzzing targets (validator, classifier, scrubber)
- Docker image for containerized deployment
- Multi-platform CI (Linux, macOS, Windows)
- Homebrew formula

### Security
- 35+ environment variables filtered from child processes
- Entropy-based secret detection in stderr/stdout
- Path traversal blocking for file read/write tools
- Dangerous command pattern blocking (25+ regexes)
- Interactive command and interpreter blocking
