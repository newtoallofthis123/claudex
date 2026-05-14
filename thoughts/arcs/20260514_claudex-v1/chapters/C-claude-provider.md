# Chapter C — Claude provider

**Type:** conscious
**Depends on:** A

## Executive summary

Implement `providers::claude::ClaudeProvider` end to end: discover roots from configured paths + `CLAUDE_CONFIG_DIR` + home fallback, walk `~/.claude/projects/<slug>/*.jsonl` to list sessions, resolve a session id to a transcript path, and parse the JSONL into the neutral `Conversation`. Snapshot-test the parsed-then-rendered output against fixture transcripts checked into the repo. The Claude format is messy in places; this chapter is conscious because the parser must make judgement calls about which events are conversational, which are metadata, and which are unknown.

## Files touched

- `src/providers/claude.rs`
- `tests/claude_provider.rs`
- `tests/fixtures/claude/<project>/<session>.jsonl` — at least three fixture transcripts (see C.5)
- `tests/snapshots/...` — `insta` snapshot files

## Success criteria

- `ClaudeProvider::list_sessions()` returns one `SessionSummary` per `*.jsonl` file under any effective root, populated with `id` (filename stem), `path`, `cwd` (from the first user/assistant line that carries it), `started_at` (first timestamp), `updated_at` (last timestamp), `title` (first ≤ 80 chars of the first user message with non-empty text content).
- `ClaudeProvider::resolve_session("<id>")` finds the unique `<id>.jsonl` across all effective roots; ambiguity is reported via `ProviderError::SessionNotFound` augmented with a message listing matches; missing id is `SessionNotFound`.
- `parse_transcript` produces a `Conversation` whose blocks preserve transcript order, classify `user`/`assistant` messages correctly (including content-array shape), surface `tool_use`/`tool_result` as separate blocks, drop `permission-mode` / `file-history-snapshot` / `last-prompt` as `SystemEvent`s (compact), and emit `UnknownEvent` for any other `type` value rather than failing.
- Structurally invalid JSONL (unparseable line) returns `ProviderError::InvalidJsonl` with path + 1-indexed line number.
- Fixture-driven snapshot tests pass.

## Observed Claude JSONL shape (from preflight)

Each line has a top-level `type`. Conversational lines (`user`, `assistant`) carry:

```jsonc
{
  "parentUuid": "...",
  "type": "user",                          // or "assistant"
  "message": {
    "role": "user",                        // or "assistant"
    "content": "string"                    // OR array of typed blocks (see below)
  },
  "uuid": "...",
  "timestamp": "2026-05-14T17:39:32.755Z",
  "cwd": "/Users/noob/Projects/claudex",
  "sessionId": "...",
  "gitBranch": "main"
}
```

`message.content` can be:
- a plain string (older or simple turns), or
- an array of blocks: `{type: "text", text: "..."}`, `{type: "tool_use", name, input}`, `{type: "tool_result", content, is_error?}` (content is itself a string OR an array of `{type: "text", text}`).

Non-conversational top-level types observed: `permission-mode`, `file-history-snapshot`, `last-prompt`, `attachment`. There may be others not yet observed — treat unknown `type` values as `UnknownEvent`, not errors.

## Phases

### C.1 — Root discovery

- **Goal:** Implement `agent()` and `effective_roots()` using `config::effective_claude_roots`.
- **Files & changes:** `providers/claude.rs`.
- **Code:**

  ```rust
  pub struct ClaudeProvider { configured_roots: Vec<PathBuf> }

  impl ClaudeProvider {
      pub fn new(configured_roots: Vec<PathBuf>) -> Self { Self { configured_roots } }
  }

  impl Provider for ClaudeProvider {
      fn agent(&self) -> Agent { Agent::Claude }
      fn effective_roots(&self, _configured: &[PathBuf]) -> Vec<PathBuf> {
          crate::config::effective_claude_roots(&self.configured_roots)
      }
      // ...
  }
  ```

  Note: the trait takes `configured_roots` redundantly (matches PRD trait shape) but the provider already owns its own. Ignore the parameter — keep the trait signature for compatibility with any future caller that wants to inject roots.

### C.2 — Session listing

- **Goal:** Walk `<root>/<project-slug>/*.jsonl`. For each file: open, scan lines until we have enough to populate the summary, then close.
- **Files & changes:** `providers/claude.rs`.
- **Implementation strategy:**
  - For each effective root that exists, `read_dir` recursively only one level deep (project-slug dirs). Inside each project dir, list `*.jsonl` files. Skip the `memory` subdirectory (observed during preflight).
  - For each transcript file, lazily read line-by-line:
    - `started_at` = first parseable `timestamp` on any conversational line.
    - `cwd` = first non-empty `cwd` value seen.
    - `title` = take the first `user` line where `message.content` reduces to a non-empty text. Strip newlines, take the first 80 chars, trim. If the content is `<command-message>...</command-message>...` (a slash-command invocation), keep raw — these are real first messages.
    - `updated_at` = last parseable `timestamp`. To avoid re-reading, scan the whole file but stop short of full parsing — only `serde_json::from_str::<TimestampLine>` with a tiny struct `{ #[serde(default)] timestamp: Option<String> }`.
  - Return `Vec<SessionSummary>` sorted by `started_at` descending (newest first) so `--last` is a simple `.first()`.

- **Error policy:** A bad transcript file should not nuke the list. If a file fails to read or has zero parseable lines, log to stderr and skip it. Hard failures only when the root path itself is unreadable — even then, return the entries from other roots if any.

### C.3 — Session resolution

- **Goal:** Map a session id to a `ResolvedSession`. The id is the filename stem.
- **Files & changes:** `providers/claude.rs`.
- **Implementation:** Iterate effective roots, glob `<root>/*/<id>.jsonl`. On hit, return `ResolvedSession { agent: Claude, id, path }`. Zero hits → `SessionNotFound`. Multiple hits across roots → return the first (sorted by `updated_at` desc if cheap; otherwise lexicographic root order) but log the ambiguity to stderr.

### C.4 — Transcript parser

- **Goal:** Stream the JSONL and emit `Block`s in order.
- **Files & changes:** `providers/claude.rs`.
- **Algorithm:**

  ```text
  for each (idx, line) in file.lines().enumerate():
      let value: serde_json::Value = serde_json::from_str(line)
          .map_err(|e| InvalidJsonl { path, line: idx + 1, source: e })?;
      match value["type"].as_str() {
          Some("user")      => push_message(blocks, idx, value, Role::Human),
          Some("assistant") => push_message(blocks, idx, value, Role::Agent),
          Some(t @ ("permission-mode" | "file-history-snapshot" | "last-prompt" | "attachment"))
              => blocks.push(SystemEvent { label: t.to_string(), detail: compact(value) }),
          Some(other) => blocks.push(UnknownEvent { raw_type: other, raw_excerpt: truncate_to(UNKNOWN_EVENT_LIMIT, value) }),
          None        => blocks.push(UnknownEvent { raw_type: "<missing>", raw_excerpt: truncate_to(UNKNOWN_EVENT_LIMIT, value) }),
      }
  ```

  `push_message` handles the string-vs-array shape of `message.content`:
  - **string:** emit one `HumanMessage`/`AgentMessage` with `text = string`.
  - **array:** iterate items. For each item:
    - `text` → emit text block (concatenate consecutive text items from the same line into one block? **No.** Emit them in order as separate blocks only if there are intervening tool blocks; otherwise concatenate with a single newline. Implementor: join consecutive `text` items into one text block per side. This avoids artificial paragraph splits.)
    - `tool_use` → emit `ToolCall { name, input: serde_json::to_string_pretty(&input)? }`
    - `tool_result` → flatten `content` (string OR `[{type: text, text}]`) into a single string, emit `ToolResult { output, truncated: None }`. Truncation is applied at render time, not parse time.
    - any other item `type` → emit `UnknownEvent`.

  Metadata harvest as we go: first `cwd`, first `timestamp` for `Conversation.started_at`. `session_id` = filename stem (already known). `transcript_path` = the file path.

### C.5 — Fixtures + snapshot tests

- **Goal:** Lock parser+renderer composition against checked-in transcripts.
- **Files & changes:** `tests/fixtures/claude/<project-slug>/<session>.jsonl`, `tests/claude_provider.rs`.
- **Fixtures (hand-trim to ~10–30 lines each so they're reviewable):**
  1. `simple-dialogue.jsonl` — user message, assistant message, user message, assistant message. All `message.content` as plain strings.
  2. `tool-call.jsonl` — user message + assistant turn that contains a `text` item followed by a `tool_use` item, then a `user` line carrying a `tool_result`.
  3. `unknown-event.jsonl` — same as `simple-dialogue` but with one synthetic `{"type":"future-feature","payload":{"x":1}}` line in the middle.
  4. (optional, recommended) `multibyte.jsonl` — content containing CJK / emoji to verify char-count truncation.

  Source the fixtures by copy-and-trim from real transcripts, then **redact** absolute paths and replace `sessionId` / `uuid` values with stable strings (e.g. `fixture-1`). The fixture's filename stem must equal the redacted session id.

- **Tests:**
  - `list_sessions` over the fixture root returns exactly the expected ids, ordered newest first by `started_at`.
  - `resolve_session("fixture-1")` returns the right path.
  - `parse_transcript` of each fixture, then `render(...)`, then `insta::assert_snapshot!` against the locked text. Use a fixed `created_at` so headers are deterministic.
  - `parse_transcript` of a deliberately broken line (e.g., a fixture named `invalid.jsonl` whose 3rd line is `{not json`) returns `InvalidJsonl { line: 3, .. }`.
