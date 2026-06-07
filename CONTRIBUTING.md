# Contributing to Reshell

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/reshell.git`
3. Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
4. Build: `cargo build`
5. Run tests: `cargo test`

## Development Workflow

- **Branch naming:** `feature/`, `bugfix/`, `refactor/`, `docs/`
- **Commits:** Atomic, descriptive, imperative mood
- **Tests:** Required for all new features; use TDD
- **Format:** `cargo fmt`
- **Lint:** `cargo clippy -- -D warnings`

## Project Structure

See `AGENTS.md` for architecture details and entrypoints.

## Testing

```bash
cargo test                    # Unit tests
cargo test --test integration_tests  # E2E tests
cargo bench                   # Benchmarks
```

## Code Review

All PRs require:
- All tests passing
- `cargo fmt` clean
- `cargo clippy` with zero warnings
- No hardcoded secrets
- Input validation on all user-facing surfaces

## Security

Report security vulnerabilities to the maintainer directly.
Do not open public issues for security bugs.
