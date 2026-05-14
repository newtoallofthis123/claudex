# Chapter B — Renderer, truncation, handoff store

**Type:** conscious
**Depends on:** A

## Executive summary

Convert a neutral `Conversation` into a stable Markdown handoff, apply the v1 truncation rules, and persist the artifact to disk under `~/.handoffs` with a deterministic, collision-resistant filename. Everything here is pure with respect to provider transcripts — the chapter uses hand-authored `Conversation` fixtures, not real JSONL. Snapshot tests pin the rendered output. The renderer must remain reusable by `inspect` (chapter E) without writing to disk.

## Files touched

- `src/render.rs`
- `src/handoff_store.rs`
- `tests/render_snapshots.rs`
- `tests/handoff_store.rs`
- `tests/fixtures/neutral/*.rs` (hand-authored `Conversation` builders) — or inline in the test file; choose what reads cleanest.

## Success criteria

- `render::render(&Conversation)` returns `String` exactly matching the PRD §"Handoff Rendering" default shape — lowercase role labels, top metadata block, blank-line separators, preserved order.
- Truncation defaults match PRD §"Truncation Rules": human/agent text uncut; tool input ≤ 4000 chars; tool output ≤ 2000 chars; unknown event ≤ 1000 chars. Every truncation emits `[truncated: showing first N chars of M]`.
- `handoff_store::write(...)` creates `~/.handoffs` if missing, writes UTF-8, refuses to overwrite an existing file (returns `HandoffStoreError::AlreadyExists`), returns the final path.
- Filename format: `<source>-to-<target>-<YYYYMMDD>-<HHMMSS>-<short-id>.md`, where `<short-id>` is the first 8 chars of the source session id (sanitised to `[A-Za-z0-9_-]`). On collision, append `-2`, `-3`, … until unique.
- `cargo test` snapshot tests pass covering: basic human+agent dialogue, tool call with short output, tool call with truncated output, mixed ordering, unknown event preservation, missing `cwd`.

## Phases

### B.1 — Truncation primitive

- **Goal:** A single helper that enforces a char budget and returns the possibly-truncated string + optional `TruncationInfo`. Used by tool input, tool output, and unknown event rendering.
- **Files & changes:** `render.rs` — `fn truncate(text: &str, max_chars: usize) -> (String, Option<TruncationInfo>)`. Count *characters* (Unicode scalar values), not bytes.
- **Code:**

  ```rust
  use crate::model::TruncationInfo;

  pub(crate) fn truncate(text: &str, max_chars: usize) -> (String, Option<TruncationInfo>) {
      let total = text.chars().count();
      if total <= max_chars { return (text.to_string(), None); }
      let shown: String = text.chars().take(max_chars).collect();
      (shown, Some(TruncationInfo { original_chars: total, shown_chars: max_chars }))
  }

  pub const TOOL_INPUT_LIMIT: usize = 4000;
  pub const TOOL_OUTPUT_LIMIT: usize = 2000;
  pub const UNKNOWN_EVENT_LIMIT: usize = 1000;
  ```

  Unit tests: under-limit (no truncation), exact-limit, over-limit, multibyte safety.

### B.2 — Markdown renderer

- **Goal:** Implement `render(&Conversation) -> String` producing the PRD default shape. Header: source, target, session_id, cwd (omitted when `None`), transcript, created_at. Body: blocks separated by blank lines.
- **Files & changes:** `render.rs` — public `render(conv: &Conversation, target: Agent, created_at: OffsetDateTime) -> String`. Use `std::fmt::Write` into a `String`.
- **Format contract (read carefully — snapshot tests pin this):**

  ```text
  source: <source>
  target: <target>
  session_id: <id>
  cwd: <path>                 # omitted when None
  transcript: <path>
  created_at: <RFC3339-with-offset>

  human:
  <text>

  agent:
  <text>

  tool:
  name: <name>
  input:
  <truncated-or-full input>

  output:
  [truncated: showing first 2000 chars of 18422]
  <truncated output>

  system:
  <label>: <detail>           # one block for SystemEvent

  unknown:
  type: <raw_type>
  <truncated raw_excerpt>
  ```

  Notes:
  - `cwd:` line is omitted entirely when `conv.cwd` is `None`.
  - `ToolCall` and `ToolResult` are independent blocks (per locked answer). When a tool result has truncation, the `[truncated: ...]` marker is the first line of the output body, then a blank line, then the truncated content. (Matches the PRD example.)
  - `created_at` is RFC3339 with offset, e.g. `2026-05-14T22:40:00+05:30`. Pass it in as a parameter so snapshot tests can pin a fixed instant.

- **Code (skeleton):**

  ```rust
  pub fn render(conv: &Conversation, target: Agent, created_at: OffsetDateTime) -> String {
      let mut out = String::new();
      writeln!(out, "source: {}", conv.source.as_str()).unwrap();
      writeln!(out, "target: {}", target.as_str()).unwrap();
      writeln!(out, "session_id: {}", conv.session_id).unwrap();
      if let Some(cwd) = &conv.cwd { writeln!(out, "cwd: {}", cwd.display()).unwrap(); }
      writeln!(out, "transcript: {}", conv.transcript_path.display()).unwrap();
      writeln!(out, "created_at: {}", format_rfc3339(created_at)).unwrap();
      for block in &conv.blocks {
          out.push('\n');
          render_block(&mut out, block);
      }
      out
  }
  ```

  Implementor: keep one helper per `Block` variant. Apply `truncate` inside the tool-input / tool-output / unknown-event helpers.

### B.3 — Snapshot tests

- **Goal:** Lock renderer output for every block variant and every truncation path against `insta` snapshots.
- **Files & changes:** `tests/render_snapshots.rs`. Build `Conversation` values inline (no JSONL). Use `insta::assert_snapshot!`. Fixed `created_at` = `datetime!(2026-05-14 22:40:00 +05:30)` from the `time::macros::datetime` macro.
- **Cases (one snapshot per case):**
  1. `basic_human_agent` — two `HumanMessage` + two `AgentMessage`.
  2. `tool_call_small_output` — one `ToolCall` + one `ToolResult` (no truncation).
  3. `tool_call_truncated_output` — `ToolResult` with output longer than `TOOL_OUTPUT_LIMIT`; assert the `[truncated: …]` marker appears with the exact counts.
  4. `mixed_ordering` — interleaved human/agent/tool/tool result, confirming block order is preserved verbatim.
  5. `unknown_event_preserved` — one `UnknownEventBlock` with a raw excerpt; verify renderer emits `unknown:` block with type label.
  6. `missing_cwd` — `Conversation.cwd = None`; assert no `cwd:` line in header.

  Use `insta::assert_snapshot!(name, rendered)` and commit `.snap` files under `tests/snapshots/`.

### B.4 — Handoff store

- **Goal:** Deterministic filename + safe write. No silent overwrites. No clobbering. Returns the final path so callers can print it.
- **Files & changes:** `src/handoff_store.rs`. Public surface:

  ```rust
  pub struct HandoffStore { dir: PathBuf }

  #[derive(Debug, thiserror::Error)]
  pub enum HandoffStoreError {
      #[error("could not create handoff dir {0}: {1}")] DirCreate(PathBuf, #[source] std::io::Error),
      #[error("handoff file already exists: {0}")] AlreadyExists(PathBuf),
      #[error("could not write handoff file {0}: {1}")] Write(PathBuf, #[source] std::io::Error),
  }

  impl HandoffStore {
      pub fn new(dir: PathBuf) -> Self { Self { dir } }
      pub fn write(&self, source: Agent, target: Agent, session_id: &str, created_at: OffsetDateTime, body: &str) -> Result<PathBuf, HandoffStoreError> { /* ... */ }
      pub(crate) fn filename(source: Agent, target: Agent, session_id: &str, created_at: OffsetDateTime, dedup: u32) -> String { /* ... */ }
  }
  ```

  Implementation:
  - `std::fs::create_dir_all(&self.dir)` first (map error to `DirCreate`).
  - Build a base filename `<source>-to-<target>-<YYYYMMDD>-<HHMMSS>-<short>.md`.
  - `<short>` = first 8 chars of `session_id` after sanitising to keep only `[A-Za-z0-9_-]`; pad/truncate to exactly 8 chars; if empty after sanitisation, use `"session"`.
  - Use `OpenOptions::new().create_new(true).write(true)` to refuse overwrites.
  - On `AlreadyExists`, retry with `-2`, `-3`, … up to a small cap (say 100). Beyond that, return `AlreadyExists` with the last attempted path.

### B.5 — Handoff store tests

- **Goal:** Verify filename shape, dedup, dir creation, and overwrite refusal.
- **Files & changes:** `tests/handoff_store.rs`.
- **Cases:**
  1. Writes into a `tempfile::tempdir()` and returns a path inside it.
  2. Filename matches `claude-to-codex-20260514-224000-abcdefgh.md`.
  3. Pre-existing file with the same base name triggers `-2` suffix.
  4. Creating the dir succeeds when it didn't previously exist.
  5. Sanitisation: a session id containing `/` or `:` does not appear literally in the filename.
