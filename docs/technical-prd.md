# claudex Technical PRD

## Purpose

`claudex` is a local CLI for handing active work between Claude Code and Codex.

The tool reads a saved source transcript, converts it into a stable internal conversation model, writes a Markdown handoff file, and starts the target agent with a short prompt that points at that file.

The core promise is:

```text
unstable source transcript -> stable Markdown handoff -> fresh target session
```

The handoff file is the product's center of gravity. It should be durable, inspectable, grep-friendly, and useful even if the launch step fails.

## Product Constraints

- Preserve the conversation before adding intelligence.
- Keep handoff generation mechanical by default.
- Keep commands independent so users can list, inspect, generate, and launch separately.
- Prefer Markdown artifacts over hidden state or opaque JSON output.
- Treat Claude Code and Codex transcript formats as private, unstable implementation details.

## Recommended Stack

Build `claudex` as a Rust CLI.

Rust is the preferred v1 language because the product is a small local utility that benefits from a single binary, strong filesystem handling, streaming JSONL parsing, and straightforward distribution.

Recommended crates:

```text
clap          CLI parsing
serde         typed structures
serde_json    JSONL event parsing
anyhow        application-level errors
thiserror     provider and parser error types, if needed
dirs          home/config path discovery
time          timestamps and file naming
```

## Architecture

The system should be adapter-driven:

```text
CLI command
  -> session reference parser
  -> source provider
  -> transcript parser
  -> neutral conversation model
  -> handoff renderer
  -> handoff store
  -> target launcher
```

The important boundary is the neutral conversation model. The implementation should never directly convert Claude JSONL into Codex JSONL, or Codex JSONL into Claude JSONL.

Instead:

```text
Claude JSONL -> neutral blocks -> Markdown handoff
Codex JSONL  -> neutral blocks -> Markdown handoff
```

This isolates source-specific transcript weirdness inside providers and keeps the rendered artifact stable.

## Module Layout

The initial implementation should use a small module structure:

```text
src/
  main.rs
  cli.rs
  session_ref.rs
  model.rs
  providers/
    mod.rs
    claude.rs
    codex.rs
  render.rs
  handoff.rs
  launch.rs
  config.rs
  settings.rs
  select.rs
```

### Module Responsibilities

- `cli.rs`: command parsing, argument validation, and command dispatch.
- `session_ref.rs`: references such as `claude:<session_id>` and `codex:<session_id>`.
- `model.rs`: stable internal handoff model shared by all providers and renderers.
- `providers/`: local session resolution and provider-specific JSONL parsing.
- `render.rs`: Markdown handoff rendering from neutral conversation data.
- `handoff_store.rs`: handoff file path selection and safe artifact writing.
- `launch.rs`: target agent launch with the catch-up prompt.
- `config.rs`: config loading, default path derivation, and truncation limits.
- `settings.rs`: `settings` subcommand mutations.
- `select.rs`: `--last` resolution and `fzf`-backed selection.

## CLI Contract

### List

```bash
claudex list claude
claudex list codex
claudex list claude --last
claudex list codex --interactive
```

Lists locally available sessions for one source agent.

Output should include enough information for a user to choose a session:

- session id
- timestamp, if available
- cwd, if available
- first useful human message or title, if cheaply available
- transcript path in verbose mode

`--last` should print the most recent session for the requested source according to provider metadata.

`--interactive` should pipe session rows into `fzf` and print the selected session reference. This is still a selection helper, not a full terminal UI owned by `claudex`.

### Inspect

```bash
claudex inspect claude:<session_id>
claudex inspect codex:<session_id>
claudex inspect --last claude
claudex inspect --interactive codex
```

Resolves and parses a session without launching another agent.

Default output should show:

- source agent
- session id
- transcript path
- cwd, if available
- message/tool block counts
- rendered preview of the handoff

`inspect` is the debugging and trust-building command. If a user is unsure what will be handed off, they should be able to run `inspect` first.

### Handoff

```bash
claudex handoff claude:<session_id> codex
claudex handoff codex:<session_id> claude
claudex handoff --last claude codex
claudex handoff --interactive codex claude
```

Creates a handoff file and starts the target agent.

The default flow:

1. Parse the source session reference.
2. Resolve the reference to a local transcript file.
3. Parse the source JSONL into neutral conversation blocks.
4. Render the blocks as a Markdown handoff.
5. Write the handoff Markdown file under `~/.handoffs`.
6. Start the target agent with a short catch-up prompt referencing the file.
7. Print the handoff file path and launch result.

If launch fails, the command should still succeed if the handoff file was written. The user can manually paste or reuse the file path.

### Settings

```bash
claudex settings path
claudex settings show
claudex settings edit
claudex settings set handoff_dir ~/.handoffs
claudex settings add-root claude ~/.claude/projects
claudex settings add-root codex ~/.codex/sessions
claudex settings remove-root claude ~/.claude/projects
claudex settings reset-root claude
claudex settings reset-root codex
```

The settings subcommand owns persistent user configuration. It should be boring and scriptable.

Settings requirements:

- `path` prints the effective config file path.
- `show` prints the effective config, including defaults and user overrides.
- `edit` opens the config file in `$EDITOR`.
- `set handoff_dir` sets the handoff output directory.
- `add-root` appends an agent transcript root.
- `remove-root` removes an agent transcript root.
- `reset-root` removes user overrides and returns that agent to discovered defaults.

## Session References

A session reference has this shape:

```text
<agent>:<session_id>
```

Supported agents in v1:

```text
claude
codex
```

The parser should reject unknown agents and malformed references before touching the filesystem.

The session id should be treated as an opaque string. Providers are responsible for mapping it to a local transcript path.

## Provider Interface

Each provider should expose the same behavior:

```rust
trait Provider {
    fn agent(&self) -> Agent;
    fn effective_roots(&self, configured_roots: &[PathBuf]) -> Vec<PathBuf>;
    fn list_sessions(&self) -> Result<Vec<SessionSummary>>;
    fn resolve_session(&self, id: &str) -> Result<ResolvedSession>;
    fn parse_transcript(&self, session: &ResolvedSession) -> Result<Conversation>;
}
```

Suggested supporting types:

```rust
struct SessionSummary {
    agent: Agent,
    id: String,
    path: PathBuf,
    cwd: Option<PathBuf>,
    started_at: Option<OffsetDateTime>,
    updated_at: Option<OffsetDateTime>,
    title: Option<String>,
}

struct ResolvedSession {
    agent: Agent,
    id: String,
    path: PathBuf,
}
```

The provider boundary owns all source-specific assumptions:

- where transcripts live
- which environment variables define provider home directories
- how session ids map to files
- which JSONL event shapes matter
- how tool calls and tool results are represented
- which metadata fields are reliable

Code outside the provider should not depend on raw Claude Code or Codex event shapes.

## Internal Conversation Model

The neutral model should be intentionally small.

```rust
struct Conversation {
    source: Agent,
    session_id: String,
    transcript_path: PathBuf,
    cwd: Option<PathBuf>,
    started_at: Option<OffsetDateTime>,
    blocks: Vec<Block>,
}

enum Block {
    HumanMessage(TextBlock),
    AgentMessage(TextBlock),
    ToolCall(ToolCallBlock),
    ToolResult(ToolResultBlock),
    SystemEvent(SystemEventBlock),
    UnknownEvent(UnknownEventBlock),
}
```

The renderer should be able to render every block variant. Unknown events should not crash parsing unless they indicate a corrupt transcript. Preserve enough information for debugging while keeping the default handoff readable.

Suggested block fields:

```rust
struct TextBlock {
    text: String,
    source_event_index: usize,
}

struct ToolCallBlock {
    name: String,
    input: String,
    source_event_index: usize,
}

struct ToolResultBlock {
    output: String,
    truncated: Option<TruncationInfo>,
    source_event_index: usize,
}

struct TruncationInfo {
    original_chars: usize,
    shown_chars: usize,
}
```

Provider parsers should prefer preserving order over perfectly classifying every event. A slightly generic ordered block is better than a clever but lossy reconstruction.

## Handoff Rendering

The rendered handoff should be Markdown, not JSON.

Default shape:

```text
source: claude
target: codex
session_id: abc123
cwd: /Users/noob/Projects/example
transcript: /Users/noob/.claude/...
created_at: 2026-05-14T22:40:00+05:30

human:
Can you inspect the auth flow?

agent:
I will trace the auth path end to end.

tool:
name: Bash
input:
rg "login" src

output:
[truncated: showing first 2000 chars of 18422]
...
```

Rendering rules:

- Use stable lowercase role labels.
- Keep metadata at the top.
- Separate blocks with blank lines.
- Preserve message order from the transcript.
- Keep human and agent prose readable.
- Render tool calls with explicit name and input.
- Render tool results with truncation markers when applicable.
- Render unknown events only when useful for debugging, and keep them compact.

The renderer should be snapshot-tested with fixtures for both providers.

## Truncation Rules

The default handoff should preserve conversational shape without flooding the target session.

Suggested v1 limits:

```text
human messages: no truncation by default
agent messages: no truncation by default
tool inputs: 4000 chars
tool outputs: 2000 chars
unknown events: 1000 chars
```

Truncation must always be marked.

Example:

```text
[truncated: showing first 2000 chars of 18422]
```

The launch prompt should remind the target agent that tool output may be truncated and that it should inspect the workspace directly when exact details matter.

V1 should start with fixed truncation defaults unless real usage proves they are wrong.

## Handoff Storage

Default output directory:

```text
~/.handoffs
```

File naming should be deterministic enough to understand and unique enough to avoid collisions.

Suggested format:

```text
~/.handoffs/claude-to-codex-20260514-224000-abc123.md
~/.handoffs/codex-to-claude-20260514-224000-def456.md
```

The writer should:

- create the directory if missing
- write UTF-8 Markdown
- avoid overwriting an existing file
- return the final path to the caller

V1 does not need a handoff index. The filesystem is the index.

## Target Launcher

The launcher starts the target agent with a short initial prompt.

Prompt template:

```text
You are catching up from a previous Claude Code/Codex conversation.

Read this handoff file:
<handoff_path>

Use it as context and continue naturally. Tool output may be truncated; inspect the workspace directly when exact details matter.
```

Launcher behavior should be isolated behind a small interface:

```rust
trait Launcher {
    fn launch(&self, target: Agent, prompt: &str) -> Result<LaunchResult>;
}
```

Default launch commands:

```bash
claude "<prompt>"
codex "<prompt>"
```

These should start a fresh interactive target session with the catch-up prompt as the initial user message. V1 should not default to non-interactive modes such as `claude -p` or `codex exec`, because the handoff is meant to continue work in the target agent, not produce a one-shot answer.

If an agent executable is missing or launch arguments differ by environment, return a clear error after writing the handoff file.

The launcher is deliberately less important than the artifact. A failed launch should not invalidate a successful handoff.

### Launch Research Notes

- Local `claude --help` reports `Usage: claude [options] [command] [prompt]` and says the positional prompt is `Your prompt`.
- Anthropic's Claude Code quickstart documents `claude "task"` as a one-time task and `claude "query"` as starting with an initial prompt.
- Local `codex --help` reports `Usage: codex [OPTIONS] [PROMPT]` and says `[PROMPT]` is an optional user prompt to start the session.

## Config

`claudex` should have a small TOML config file managed by the `settings` subcommand.

Default config path:

```text
~/.config/claudex/config.toml
```

Minimal config file shape:

```toml
handoff_dir = "~/.handoffs"

[roots]
claude = []
codex = []
```

Root precedence:

1. Configured roots from `~/.config/claudex/config.toml`.
2. Provider-specific environment variables.
3. Provider-specific home-directory fallback.

Root resolution should be provider-owned. The shared config layer should pass optional user-configured roots to the provider, and the provider should decide its effective roots using the precedence above.

Provider defaults:

- Claude provider: configured `roots.claude`; else `$CLAUDE_CONFIG_DIR/projects`; else `~/.claude/projects`.
- Codex provider: configured `roots.codex`; else `$CODEX_HOME/sessions`; else `~/.codex/sessions`.
- Handoff store: configured `handoff_dir`; else `~/.handoffs`.

The `settings show` command should display both the config file values and the final effective values after environment-aware discovery.

Config should support:

- handoff output directory
- Claude transcript roots
- Codex transcript roots

Config should not become a general feature flag bucket. Keep it focused on paths.

### Path Research Notes

- Claude Code documents application data under `~/.claude`, with transcripts at `projects/<project>/<session>.jsonl`; the same documentation says `CLAUDE_CONFIG_DIR` relocates `~/.claude` paths.
- Codex session rollout files are observed under `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`.
- Codex uses `$CODEX_HOME` as the effective Codex home when set, falling back to `~/.codex`.

## Error Handling

Errors should explain what failed and what the user can do next.

Important cases:

- malformed session reference
- unknown source or target agent
- source and target are the same
- transcript root not found
- session id not found
- transcript file unreadable
- invalid JSONL line
- unsupported event shape
- handoff directory cannot be created
- handoff file cannot be written
- target executable not found
- target launch failed

Parsing should be tolerant of unknown event shapes where possible. Unknown events can become compact `UnknownEvent` blocks instead of hard failures.

Invalid JSONL is different: a structurally unreadable transcript should fail with the path and line number.

## Testing Strategy

Testing should be fixture-first.

### Unit Tests

- session reference parsing
- provider session id resolution
- provider event parsing
- truncation logic
- renderer output
- handoff filename generation
- launch prompt generation

### Snapshot Tests

Use snapshot tests for rendered handoff files.

Fixtures should cover:

- basic human and agent messages
- tool call with small output
- tool call with truncated output
- mixed message and tool ordering
- unknown event preservation
- missing cwd metadata

### Integration Tests

Integration tests should run CLI commands against fixture transcript roots:

```bash
claudex list claude
claudex inspect claude:test-session
claudex handoff claude:test-session codex --no-launch
```

V1 should include a `--no-launch` flag or equivalent test hook so handoff generation can be verified without opening another agent.

## V1 Scope

V1 includes:

- Rust CLI
- `list`
- `inspect`
- `handoff`
- `settings`
- `--last` session selection
- `--interactive` session selection through `fzf`
- Claude Code provider
- Codex provider
- neutral conversation model
- Markdown handoff renderer
- Markdown handoff files
- configurable handoff directory
- configurable transcript roots
- short target launch prompt
- target launcher with clear failure behavior
- fixed truncation rules
- fixture-based tests

## Implementation Plan

1. Create CLI skeleton and session reference parser.
   Verify: parser tests pass for valid and invalid references.

2. Define config loading and environment-aware default roots.
   Verify: `CLAUDE_CONFIG_DIR` and `CODEX_HOME` influence effective roots when no user roots are configured.

3. Define `Agent`, `Conversation`, and `Block` model types.
   Verify: model compiles with renderer stubs.

4. Implement handoff renderer and truncation logic.
   Verify: snapshot tests pass against hand-authored neutral fixtures.

5. Implement handoff store.
   Verify: tests write unique files into a temporary directory.

6. Implement provider session listing and resolution.
   Verify: fixture transcript roots produce expected session summaries.

7. Implement `--last` and `--interactive` selection.
   Verify: `--last` chooses the newest fixture session and `fzf` selection can be tested behind an injectable selector.

8. Implement Claude Code transcript parser.
   Verify: Claude fixtures render into expected handoff snapshots.

9. Implement Codex transcript parser.
   Verify: Codex fixtures render into expected handoff snapshots.

10. Implement `inspect`.
   Verify: inspect output includes metadata, counts, and preview.

11. Implement `settings`.
    Verify: settings commands create, show, update, and reset path-focused config.

12. Implement `handoff --no-launch`.
    Verify: command writes a handoff file from fixture sessions.

13. Implement launcher.
    Verify: missing executable produces a clear error after handoff writing succeeds.

## Risks

- Claude Code and Codex may change transcript shapes.
- Local transcript paths may differ across versions or environments.
- Tool output can overwhelm the target session if truncation is too loose.
- Over-aggressive truncation can hide important context.
- Launch commands may vary across installations.

Mitigations:

- keep transcript parsing isolated by provider
- preserve unknown events compactly
- always write an inspectable artifact
- mark truncation explicitly
- treat launch as optional after successful file creation

## Open Questions

- Should v1 support `--no-launch` as a public flag, or keep it as a hidden test option?
- Should `inspect` print the full rendered handoff by default or use a preview with `--full`?

## Success Criteria

`claudex` is successful when:

- a user can find a recent Claude Code or Codex session from the CLI
- a user can inspect what will be handed off before launching anything
- a user can generate a readable handoff file from a source session
- the target agent starts with a short prompt pointing at that file
- the handoff file remains useful if launch fails
- parser changes stay isolated to provider modules when transcript shapes drift
- fixtures make renderer and parser regressions obvious
