# Contributing to claudex

Thanks for taking the time to improve `claudex`.

The project is intentionally small: a local Rust CLI that converts Claude Code and Codex transcripts into stable Markdown handoffs. Contributions should keep that shape clear.

## Development Setup

Install Rust stable, then run:

```bash
cargo build
cargo test --locked
```

If you use Nix:

```bash
nix develop
```

The repository also includes a `justfile`:

```bash
just ci
```

## Local Checks

Before opening a pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

Or use:

```bash
just ci
```

## Design Guidelines

- Preserve conversations faithfully before adding cleverness.
- Keep handoffs readable as plain Markdown.
- Prefer small, scriptable commands over hidden state.
- Keep provider-specific transcript handling inside provider modules.
- Do not write fake native sessions into Claude Code or Codex history.
- Do not include private transcript fixtures unless they are sanitized and minimal.

## Pull Requests

Good pull requests are focused and easy to verify.

Please include:

- what changed
- why it changed
- how you tested it
- any transcript format assumptions you had to make

Parser changes should include fixture coverage when practical. CLI behavior changes should include integration tests.

## Reporting Bugs

When reporting parser or handoff bugs, avoid sharing private transcripts directly. A minimal sanitized JSONL fixture is ideal.

Useful details:

- `claudex --version`
- operating system
- source agent (`claude` or `codex`)
- command that failed
- error output
- whether the provider recently changed its session storage path or transcript schema
- sanitized transcript shape, if relevant

## Feature Requests

Feature requests should explain the workflow they unlock. `claudex` is most interested in features that make handoffs more faithful, inspectable, or useful without making the core flow opaque.
