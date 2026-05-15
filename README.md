# claudex

[![CI](https://github.com/newtoallofthis123/claudex/actions/workflows/ci.yml/badge.svg)](https://github.com/newtoallofthis123/claudex/actions/workflows/ci.yml)

Stop losing context when you move between Claude Code and Codex.

`claudex` is a local CLI that turns an existing Claude Code or Codex conversation into a readable Markdown handoff, then optionally launches the other agent with a short catch-up prompt pointing at that file.

It does not try to fake a native session migration. That would be brittle, opaque, and tied to private transcript internals. `claudex` takes the useful path instead:

```text
Claude Code or Codex transcript -> stable Markdown handoff -> fresh target session
```

The result is a handoff you can inspect, grep, edit, paste, archive, or reuse.

## Why claudex

Modern coding agents are good at different moments of the work. You may plan in one, implement in another, review from a different angle, or restart a stalled thread with a cleaner context window.

The annoying part is the handoff. Copying chat snippets loses tool calls, file paths, terminal output, and the sequence of decisions that made the work make sense.

`claudex` gives you a small local bridge:

- preserve the actual conversation instead of summarizing it away
- keep handoffs as plain Markdown files under your control
- inspect what will be sent before launching another agent
- use Claude Code and Codex together without pretending they share a session format
- recover gracefully when launching fails, because the handoff file is still written first

## Where it fits

Use `claudex` when you want to:

- move implementation work from Claude Code to Codex
- move a Codex investigation into Claude Code for an interactive pairing pass
- hand off a long debugging thread without rebuilding context by hand
- preserve an inspectable trail before switching tools
- compare how two agents reason over the same prior work
- archive a useful coding conversation as a durable Markdown artifact

It is especially useful for local development, refactors, incident/debugging sessions, code reviews, research spikes, and multi-agent workflows where context continuity matters more than magic.

## What it does

`claudex` reads local transcript files created by Claude Code or Codex, converts them into a neutral conversation model, renders a Markdown handoff, and can launch the target agent with a prompt like:

```text
You are catching up from a previous Claude Code/Codex conversation.

Read this handoff file:
~/.handoffs/...

Use it as context and continue naturally. Tool output may be truncated; inspect the workspace directly when exact details matter.
```

The handoff includes source metadata, working directory, transcript path, human messages, agent messages, tool calls, tool results, system events, and unknown events where useful for debugging.

## Installation

### Requirements

- Rust stable
- Claude Code CLI installed as `claude`, if you want to launch Claude from `claudex`
- Codex CLI installed as `codex`, if you want to launch Codex from `claudex`
- `fzf`, optional, for interactive session picking

### Install From Source

```bash
git clone https://github.com/newtoallofthis123/claudex.git
cd claudex
cargo install --path . --locked
```

This installs the `claudex` binary into `~/.cargo/bin` by default.

You can also use the included task runner:

```bash
just install
```

### Build Locally

```bash
cargo build
cargo build --release
```

The release binary will be written to:

```text
target/release/claudex
```

### Nix Development Shell

If you use Nix flakes:

```bash
nix develop
```

The dev shell includes Rust, `rustfmt`, `clippy`, `cargo-watch`, `cargo-nextest`, `just`, and `fzf`.

## Quick Start

List recent Claude Code sessions for the current project:

```bash
claudex list claude
```

List Codex sessions:

```bash
claudex list codex
```

Inspect a session before handing it off:

```bash
claudex inspect claude:<session_id>
```

Select a session to handoff from Claude Code to Codex:

```bash
claudex handoff claude codex
```

Create a handoff from Claude Code to Codex:

```bash
claudex handoff claude:<session_id> codex
```

Create the handoff file without launching the target agent:

```bash
claudex handoff claude:<session_id> codex --no-launch
```

Use the latest source session:

```bash
claudex handoff --last claude codex
```

Pick a session interactively with `fzf`:

```bash
claudex handoff --interactive codex claude
```

## Commands

### `list`

```bash
claudex list claude
claudex list codex
claudex list claude --last
claudex list codex --interactive
claudex list claude --all-sessions
claudex list codex --verbose
```

`list` shows locally available sessions for one agent. By default, it filters to sessions whose recorded working directory matches your current directory. Use `--all-sessions` to search across every known project, or `--pwd <path>` to search a specific project.

Use `--verbose` to include transcript paths.

### `inspect`

```bash
claudex inspect claude:<session_id>
claudex inspect codex:<session_id>
claudex inspect --last claude
claudex inspect --interactive codex
claudex inspect claude:<session_id> --full
```

`inspect` resolves and parses a session without launching another agent. It prints source metadata, block counts, and a preview of the Markdown handoff. Use `--full` to print the full rendered handoff.

### `handoff`

```bash
claudex handoff claude:<session_id> codex
claudex handoff codex:<session_id> claude
claudex handoff --last claude codex
claudex handoff --interactive codex claude
claudex handoff claude:<session_id> codex --no-launch
```

`handoff` writes a Markdown handoff file and, unless `--no-launch` is set, starts the target agent with a catch-up prompt.

By default, handoff files are written to:

```text
~/.handoffs
```

If launching the target agent fails, `claudex` still prints the written handoff path and exits non-zero. You can then open the target agent yourself and point it at the file.

### `settings`

```bash
claudex settings path
claudex settings show
claudex settings edit
claudex settings get handoff_dir
claudex settings set handoff_dir ~/.handoffs
claudex settings get roots.claude
claudex settings set roots.codex '["~/.codex/sessions"]'
claudex settings add-root claude ~/.claude/projects
claudex settings add-root codex ~/.codex/sessions
claudex settings remove-root claude ~/.claude/projects
claudex settings reset-root claude
```

Settings are stored as TOML.

## Configuration

The default config path follows your platform config directory. On most Unix-like systems, it is:

```text
~/.config/claudex/config.toml
```

You can print the exact path with:

```bash
claudex settings path
```

Example config:

```toml
handoff_dir = "~/.handoffs"

[roots]
claude = ["~/.claude/projects"]
codex = ["~/.codex/sessions"]
```

If roots are not configured, `claudex` discovers defaults from:

- `CLAUDE_CONFIG_DIR`, using `<CLAUDE_CONFIG_DIR>/projects`
- `CODEX_HOME`, using `<CODEX_HOME>/sessions`
- `~/.claude/projects`
- `~/.codex/sessions`

`XDG_CONFIG_HOME` is respected for the `claudex` settings file, which is useful for tests and sandboxed setups.

## Handoff Format

Handoffs are intentionally boring Markdown:

```text
source: claude
target: codex
session_id: abc123
cwd: /path/to/project
transcript: /path/to/transcript.jsonl
created_at: 2026-05-15T10:00:00Z

human:
Can you inspect the auth flow?

agent:
I will trace the auth path end to end.

tool:
name: Bash
input:
rg "login" src

output:
[truncated: showing first 2000 chars of 8421]
...
```

Plain text is the feature. It is readable by humans, friendly to agents, resilient to upstream transcript changes, and easy to debug.

## Privacy

`claudex` is local-first. It reads local agent transcripts and writes local Markdown handoff files. It does not upload transcripts anywhere.

Handoff files can contain sensitive prompts, file paths, command output, secrets pasted into chats, or private source snippets. Treat `~/.handoffs` like private development data. Do not commit handoffs unless you have reviewed and intentionally sanitized them.

## Provider Compatibility

`claudex` depends on local session files written by Claude Code and Codex. Those providers own their session storage layout and JSONL event schemas, and they can change either without warning.

If Claude Code or Codex changes where sessions are stored, how session IDs map to files, or what transcript events look like, `claudex` may fail to list, inspect, parse, or hand off sessions until its provider adapter is updated.

If this happens, please report it at [github.com/newtoallofthis123/claudex/issues](https://github.com/newtoallofthis123/claudex/issues). Include the source agent, command, error output, and a minimal sanitized transcript sample when possible.

## Limitations

- Claude Code and Codex transcript formats are private implementation details and may break provider adapters when they change.
- `claudex` preserves conversation structure; it does not summarize by default.
- It does not write fake native sessions into either tool.
- Tool output is truncated to keep handoffs usable.
- Launching requires the target CLI executable to be installed and available on `PATH`.

## Development

For the product brief and technical design, see [docs/brief.md](docs/brief.md) and [docs/technical-prd.md](docs/technical-prd.md).

Run the full local gate:

```bash
just ci
```

Or run individual checks:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

Useful `just` recipes:

```bash
just build
just test
just lint
just fmt
just run -- list claude
```

## Contributing

Issues, bug reports, docs improvements, parser fixes, and focused feature proposals are welcome. The best contributions preserve the core design bias: faithful transcript walking, explicit files, and small local commands.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and contribution guidelines.

## License

MIT. See [LICENSE](LICENSE).
