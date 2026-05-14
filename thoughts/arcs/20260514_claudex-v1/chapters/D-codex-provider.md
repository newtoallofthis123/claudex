# Chapter D — Codex provider

**Type:** conscious
**Depends on:** A

## Executive summary

Implement `providers::codex::CodexProvider`: discover roots from configured paths + `CODEX_HOME` + home fallback, walk the `<root>/YYYY/MM/DD/rollout-*.jsonl` tree to list sessions, resolve session ids to transcript paths, and parse Codex JSONL — which has a notably different envelope shape from Claude — into the neutral `Conversation`. Snapshot-test parser+renderer composition against fixture transcripts.

## Files touched

- `src/providers/codex.rs`
- `tests/codex_provider.rs`
- `tests/fixtures/codex/<YYYY>/<MM>/<DD>/rollout-*.jsonl` — at least three fixtures
- `tests/snapshots/...`

## Success criteria

- `CodexProvider::list_sessions()` returns one `SessionSummary` per `rollout-*.jsonl`, populated from the `session_meta` envelope (`id`, `cwd`, `timestamp` → `started_at`) plus a `title` derived from the first observed user message and an `updated_at` from the last envelope timestamp.
- `CodexProvider::resolve_session("<id>")` finds the rollout whose `session_meta.payload.id == <id>`. v1 may use a filename-glob shortcut: rollout filenames include the id (`rollout-<ts>-<id>.jsonl`). Verify the match by reading the first line and confirming `session_meta.payload.id`.
- `parse_transcript` produces an ordered `Conversation` whose blocks classify:
  - `event_msg` / `response_item` with `payload.type = user_message` → `HumanMessage`
  - `payload.type = agent_message` → `AgentMessage`
  - `payload.type = message` → inspect `payload.role`: `user`/`assistant` map to Human/Agent, anything else → `SystemEvent`
  - `payload.type in {function_call, custom_tool_call, mcp_tool_call_end}` → `ToolCall`
  - `payload.type in {function_call_output, custom_tool_call_output, patch_apply_end}` → `ToolResult`
  - `payload.type in {reasoning, token_count, task_started, task_complete, turn_aborted, thread_rolled_back}` → `SystemEvent`
  - any other → `UnknownEvent`
  - `session_meta` / `turn_context` → `SystemEvent` (compact)
- Structurally invalid JSONL → `ProviderError::InvalidJsonl { path, line }`.
- Fixture-driven snapshot tests pass.

## Observed Codex JSONL shape (from preflight)

Outer envelope variants:

```jsonc
{ "timestamp": "...", "type": "session_meta", "payload": { "id": "...", "cwd": "...", "timestamp": "...", "originator": "...", ... } }
{ "timestamp": "...", "type": "turn_context", "payload": { /* model config */ } }
{ "timestamp": "...", "type": "event_msg",   "payload": { "type": "agent_message", "message": "..." } }
{ "timestamp": "...", "type": "response_item", "payload": { "type": "message", "role": "user"|"assistant", "content": [ ... ] } }
{ "timestamp": "...", "type": "response_item", "payload": { "type": "function_call", "name": "...", "arguments": "..." } }
{ "timestamp": "...", "type": "response_item", "payload": { "type": "function_call_output", "output": "..." } }
{ "timestamp": "...", "type": "event_msg",   "payload": { "type": "reasoning", "summary": [...] } }
{ "timestamp": "...", "type": "event_msg",   "payload": { "type": "token_count", ... } }
```

`response_item` with `payload.type == "message"` carries `payload.content` as an array of OpenAI-style content parts (`{type: "input_text", text}`, `{type: "output_text", text}`); flatten by concatenating all text parts.

`function_call` `payload.arguments` is a JSON-encoded string. Surface it as the tool input verbatim (don't double-parse — the renderer will pretty-print).

## Phases

### D.1 — Root discovery

- **Goal:** Implement `agent()` and `effective_roots()` using `config::effective_codex_roots`.
- **Code:** Mirror chapter C.1 with `CodexProvider`.

### D.2 — Session listing

- **Goal:** Walk `<root>/YYYY/MM/DD/rollout-*.jsonl`. For each file: read the first non-empty line, parse as `session_meta`; if successful, populate a `SessionSummary`.
- **Files & changes:** `providers/codex.rs`.
- **Implementation:**
  - For each effective root that exists: `read_dir` year-level, then month-level, then day-level. Skip non-numeric names defensively.
  - For each `rollout-*.jsonl`: open, read first line, `serde_json::from_str::<MetaLine>`. `MetaLine` has `{ payload: { id, cwd: Option, timestamp: Option<String> } }`. Map ISO timestamp → `OffsetDateTime` via `time::OffsetDateTime::parse(.., &Rfc3339)`.
  - To populate `title` and `updated_at` cheaply: scan the rest of the file with a tiny struct `{ timestamp: Option<String>, type: Option<String>, payload: Option<{type, message, role, content}> }`. First user-text → title (first 80 chars). Last `timestamp` → `updated_at`.
  - Sort by `started_at` desc.

- **Error policy:** Same as Claude — bad single file ≠ fatal; skip and continue.

### D.3 — Session resolution

- **Goal:** Map id → path.
- **Implementation:**
  - Fast path: filename-glob `**/rollout-*-<id>.jsonl` under each effective root. On match, read first line and verify `session_meta.payload.id == id` to defend against id collisions across filename conventions.
  - If no filename match, fall back to scanning all rollouts and reading the first line until one matches. v1 doesn't need an index.
  - Zero matches → `SessionNotFound`. Multiple matches → log to stderr, return the first by directory mtime.

### D.4 — Transcript parser

- **Goal:** Stream JSONL → ordered `Block`s. Be defensive about payload shapes — Codex evolves quickly.
- **Implementation:**

  ```text
  for each (idx, line) in file:
      let value = serde_json::from_str(line).map_err(InvalidJsonl)?;
      let outer = value["type"].as_str();
      let payload_type = value["payload"]["type"].as_str();
      match (outer, payload_type) {
          (Some("session_meta"), _) | (Some("turn_context"), _)
              => push SystemEvent { label: outer, detail: compact(payload) },
          (_, Some("user_message")) => HumanMessage { text: payload["message"].as_str() or flatten payload["content"] },
          (_, Some("agent_message")) => AgentMessage { text: same },
          (_, Some("message")) => {
              let role = payload["role"].as_str();
              let text = flatten_content_parts(payload["content"]);
              match role {
                  Some("user") => HumanMessage,
                  Some("assistant") => AgentMessage,
                  _ => SystemEvent { label: "message", detail: text },
              }
          },
          (_, Some(t @ ("function_call" | "custom_tool_call" | "mcp_tool_call_end"))) => ToolCall {
              name: payload["name"].as_str().unwrap_or(t).to_string(),
              input: payload["arguments"].as_str()
                  .map(str::to_string)
                  .unwrap_or_else(|| serde_json::to_string_pretty(&payload).unwrap()),
          },
          (_, Some("function_call_output" | "custom_tool_call_output" | "patch_apply_end")) => ToolResult {
              output: payload["output"].as_str().map(str::to_string)
                  .unwrap_or_else(|| serde_json::to_string_pretty(&payload).unwrap()),
              truncated: None,
          },
          (_, Some(t @ ("reasoning" | "token_count" | "task_started" | "task_complete" | "turn_aborted" | "thread_rolled_back"))) => SystemEvent { label: t, detail: compact(payload) },
          _ => UnknownEvent { raw_type: payload_type or outer, raw_excerpt: compact(value) },
      }
  ```

  Helpers:
  - `flatten_content_parts(v)` walks an array of `{type, text}` items and joins all text values with `"\n"`. Non-text items are noted with `[<type> omitted]`.
  - `compact(v)` = `serde_json::to_string(&v)` (single-line). Render-time truncation handles size.

  Track metadata as the file streams: `session_meta.payload.cwd` → `Conversation.cwd`, `session_meta.payload.timestamp` → `Conversation.started_at`. `session_id` = the verified id (resolution provided it). `transcript_path` = the file path.

### D.5 — Fixtures + snapshot tests

- **Goal:** Lock parser+renderer composition against checked-in transcripts.
- **Files & changes:** `tests/fixtures/codex/2026/05/14/rollout-*.jsonl`, `tests/codex_provider.rs`.
- **Fixtures (hand-trim from real rollouts; redact paths/ids):**
  1. `rollout-simple.jsonl` — `session_meta` + two `event_msg` `user_message`s + two `event_msg` `agent_message`s.
  2. `rollout-tool.jsonl` — `session_meta` + `response_item` `message`(user) + `response_item` `function_call` + `response_item` `function_call_output` + `response_item` `message`(assistant).
  3. `rollout-unknown.jsonl` — `session_meta` + one normal user message + one synthetic `{"type":"event_msg","payload":{"type":"future-thing","x":1}}` to verify `UnknownEvent` fallback.
- **Tests:** Same shape as chapter C.5 — `list_sessions`, `resolve_session`, `parse_transcript` then `render` then snapshot, and an `InvalidJsonl` test using a deliberately broken line.
