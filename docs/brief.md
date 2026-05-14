# claudex

`claudex` is a small handoff tool for moving active work between Claude Code and Codex without pretending their native session formats are interchangeable.

The goal is simple: when a conversation has useful momentum in one coding agent, start a fresh session in the other agent with enough faithful context to continue. The tool should make that handoff explicit, inspectable, and cheap.

## Core Idea

Claude Code and Codex both save local conversation history as JSONL, but they do not speak the same transcript format. A native session migration would be brittle: it would depend on private event shapes, local indexes, tool-call conventions, and resume behavior that can change between releases.

`claudex` takes a humbler route. It reads the source agent's saved transcript, walks the JSONL mechanically, writes a plain handoff file, and opens the target agent with a short catch-up prompt that points at that file.

The handoff is not a regenerated summary. It is a cleaned conversation record.

## Philosophy

### Preserve the conversation

The most important artifact is the conversation itself: human messages, agent replies, tool calls, and heavily truncated tool results. The handoff should avoid inventing sections like "current state" or "open threads" by default. Those may be useful later as optional modes, but the core product should be faithful before it is clever.

### Do not over-prompt the target agent

The target agent does not need a long instruction pack. It needs a simple orientation:

```text
You are catching up from a previous Claude Code/Codex conversation.

Read this handoff file:
~/.handoffs/...

Use it as context and continue naturally. Tool output may be truncated; inspect the workspace directly when exact details matter.
```

The handoff file carries the context. The launch prompt only explains how to relate to it.

### Make the bridge human-readable

The handoff file should be easy to open, skim, grep, and paste. JSON is useful internally, but noisy as the final artifact. The saved handoff should be plain text with simple role labels:

```text
source: claude
target: codex
session_id: abc123
cwd: /Users/noob/Projects/example

human:
Can you inspect the auth flow?

agent:
I will trace the auth path end to end.

tool:
name: Bash
input:
rg "login" src

output:
[truncated to 2000 chars]
...
```

This format is intentionally boring. Boring is good here: it is readable by humans, friendly to LLMs, and easy to generate from a transcript walker.

### Keep commands independent

`handoff`, `list`, and `inspect` should be separate tools. A user may want to browse sessions without handing anything off, inspect a transcript without launching another agent, or generate a handoff file without immediately starting a target session.

The tool should not assume the user's workflow.

## Product Shape

The main command creates a handoff from one agent to the other:

```bash
claudex handoff claude:<session_id> codex
claudex handoff codex:<session_id> claude
```

Supporting commands help users find and understand sessions:

```bash
claudex list claude
claudex list codex

claudex inspect claude:<session_id>
claudex inspect codex:<session_id>
```

The default handoff flow should:

1. Resolve the source session ID to a local transcript file.
2. Parse the source JSONL into neutral conversation blocks.
3. Render a plain handoff file in `~/.handoffs`.
4. Start the target agent with a short catch-up prompt referencing that file.

## Non-Goals

`claudex` should not start by writing fake native sessions into Claude Code or Codex history. That may become an experimental feature later, but it is not the core promise.

It should not summarize the conversation by default. Summaries are useful when explicitly requested, but default handoffs should be mechanically derived from the original transcript.

It should not hide the artifact. The handoff file is the product's center of gravity: a durable, inspectable bridge between tools.

## Design Bias

Prefer the smallest useful thing:

- faithful transcript walking over agent-generated summaries
- plain text over structured ceremony
- short prompts over heavy instructions
- explicit files over hidden state
- handoff over migration

`claudex` should feel like a sharp little local utility: no magic, no ceremony, just a clean way to carry momentum from one agent to another.
