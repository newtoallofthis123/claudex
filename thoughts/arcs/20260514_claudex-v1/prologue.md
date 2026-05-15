# claudex v1 — Rust CLI for Claude↔Codex handoffs

**Date:** 2026-05-14
**Status:** enacting
**Worklog:** worklog.md

## Context

Greenfield Rust project. `docs/technical-prd.md` fully specifies the product: a local CLI that reads a saved Claude Code or Codex transcript, converts it through a neutral conversation model, writes a Markdown handoff under `~/.handoffs`, and starts the target agent with a short catch-up prompt. The handoff Markdown is the durable artifact; launch failure makes the operation fail, but the written file path is still printed and the artifact is left in place.

Provider transcript shapes were sampled from the user's machine during preflight (see locked answers). The provider boundary owns all source-specific JSONL parsing — code above the provider trait only sees the neutral `Conversation`/`Block` model.

The arc splits the build into one foundation chapter and four feature chapters. Foundation must land first; the two provider chapters and the renderer chapter can then run in parallel; the final integration chapter wires CLI commands, selection, launcher, and end-to-end tests on top of all of them.

## Locked answers from preflight

- Q: Build language and stack? → A: Rust, single binary, crates per PRD (`clap`, `serde`, `serde_json`, `anyhow`, `thiserror`, `dirs`, `time`, `toml`, `insta` for snapshots).
- Q: Claude transcript shape (verified)? → A: JSONL at `~/.claude/projects/<slug>/<session-uuid>.jsonl`. Event `type` values seen: `user`, `assistant`, `attachment`, `file-history-snapshot`, `permission-mode`, `last-prompt`. `user`/`assistant` carry `message.content` as either a plain string or an array of typed blocks (`text`, `tool_use`, `tool_result`). Metadata fields on conversation lines: `sessionId`, `cwd`, `timestamp`, `gitBranch`, `uuid`, `parentUuid`. Session id is the filename stem.
- Q: Codex transcript shape (verified)? → A: JSONL at `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`. Outer `type` values: `session_meta`, `turn_context`, `event_msg`, `response_item`. `payload.type` (on event_msg / response_item): `user_message`, `agent_message`, `message`, `function_call`, `function_call_output`, `custom_tool_call`, `custom_tool_call_output`, `mcp_tool_call_end`, `reasoning`, `token_count`, `task_started`, `task_complete`, `turn_aborted`, `thread_rolled_back`, `patch_apply_end`. `session_meta.payload` carries `id`, `cwd`, `timestamp`, `originator`, `cli_version`. Session id is the `session_meta.payload.id`; file path is reachable by glob over `rollout-*.jsonl`.
- Q: How should the parser respond to unknown / non-conversational events? → A: Convert them into compact `UnknownEvent` blocks preserving `type` and compact raw JSON rather than crash; render-time truncation adds the visible marker when needed. Structurally broken JSONL (a line that does not parse as JSON) is a hard failure that reports path + line number.
- Q: Should v1 support `--no-launch` as a public flag? (PRD open question) → A: Public flag on `handoff`. Reason: it is needed for the integration tests anyway and is genuinely useful for scripting.
- Q: Should `inspect` print the full rendered handoff by default? (PRD open question) → A: Default to a preview (first ~80 lines plus tail summary). Add `--full` for the full rendered Markdown.
- Q: Where does the `fzf` boundary live? → A: Selection layer calls out to `fzf` through an injectable `Selector` trait so tests can stub it. If `fzf` is missing on PATH, fail fast with a clear error pointing the user at non-interactive alternatives.
- Q: Launch process semantics? → A: `claude "<prompt>"` and `codex "<prompt>"` via `std::process::Command` with stdio inherited so the user lands directly in the new interactive session. The catch-up prompt is the positional argument verbatim. Missing executable, spawn failure, or non-zero target exit surfaces a clear `LaunchError` after the handoff file has been written; the operation fails and exits non-zero, but the file path is still printed.
- Q: How are tool calls and their outputs paired across providers? → A: Providers are responsible for emitting paired `ToolCall` and `ToolResult` blocks in transcript order. The neutral model does not enforce pairing; the renderer treats them as independent ordered blocks. This matches the "prefer preserving order over perfectly classifying every event" rule in the PRD.

## Chapters

- **A — Scaffold, types, config** [strict] — Cargo project, module skeleton, `Agent`/`SessionRef`/`Conversation`/`Block` types, `Provider` trait declaration, config loading with env-aware default roots.
- **B — Renderer, truncation, handoff store** [conscious] — `render.rs`, truncation policy, `handoff_store.rs` file naming + safe write, snapshot tests from hand-authored neutral fixtures.
- **C — Claude provider** [conscious] — `providers/claude.rs`: root discovery, session listing, id resolution, JSONL → neutral conversation parser with fixture-backed snapshot tests.
- **D — Codex provider** [conscious] — `providers/codex.rs`: root discovery (incl. `CODEX_HOME`), session listing across date-partitioned tree, id resolution, JSONL → neutral conversation parser with fixture-backed snapshot tests.
- **E — CLI commands, selection, launcher, integration tests** [conscious] — `cli.rs` dispatch, `list` / `inspect` / `handoff` / `settings` wiring, `--last` + `--interactive` (injectable `Selector`), `launch.rs`, settings mutations, fixture-driven integration tests including `handoff --no-launch`.

Dependencies: B/C/D depend only on A. E depends on A+B+C+D.
