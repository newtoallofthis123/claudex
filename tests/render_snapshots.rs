//! Snapshot tests pinning the Markdown handoff format.

use std::path::PathBuf;

use claudex::model::{
    Agent, Block, Conversation, SystemEventBlock, TextBlock, ToolCallBlock, ToolResultBlock,
    UnknownEventBlock,
};
use claudex::render::{render, TOOL_OUTPUT_LIMIT};
use time::macros::datetime;
use time::OffsetDateTime;

fn fixed_created_at() -> OffsetDateTime {
    datetime!(2026-05-14 22:40:00 +05:30)
}

fn base_conversation(blocks: Vec<Block>) -> Conversation {
    Conversation {
        source: Agent::Claude,
        session_id: "abc123".to_string(),
        transcript_path: PathBuf::from("/Users/noob/.claude/sessions/abc123.jsonl"),
        cwd: Some(PathBuf::from("/Users/noob/Projects/example")),
        started_at: None,
        blocks,
    }
}

fn human(text: &str) -> Block {
    Block::HumanMessage(TextBlock {
        text: text.to_string(),
        source_event_index: 0,
    })
}

fn agent(text: &str) -> Block {
    Block::AgentMessage(TextBlock {
        text: text.to_string(),
        source_event_index: 0,
    })
}

fn tool_call(name: &str, input: &str) -> Block {
    Block::ToolCall(ToolCallBlock {
        name: name.to_string(),
        input: input.to_string(),
        source_event_index: 0,
    })
}

fn tool_result(output: &str) -> Block {
    Block::ToolResult(ToolResultBlock {
        output: output.to_string(),
        truncated: None,
        source_event_index: 0,
    })
}

#[test]
fn basic_human_agent() {
    let conv = base_conversation(vec![
        human("Can you inspect the auth flow?"),
        agent("I will trace the auth path end to end."),
        human("Thanks — start with the login handler."),
        agent("Reading src/auth/login.rs now."),
    ]);
    let rendered = render(&conv, Agent::Codex, fixed_created_at());
    insta::assert_snapshot!("basic_human_agent", rendered);
}

#[test]
fn tool_call_small_output() {
    let conv = base_conversation(vec![
        human("List repo files."),
        tool_call("Bash", "ls src"),
        tool_result("auth.rs\nmain.rs\nrender.rs\n"),
        agent("Three files in src."),
    ]);
    let rendered = render(&conv, Agent::Codex, fixed_created_at());
    insta::assert_snapshot!("tool_call_small_output", rendered);
}

#[test]
fn tool_call_truncated_output() {
    // Build an output deliberately longer than TOOL_OUTPUT_LIMIT.
    let line = "abcdefghij"; // 10 chars
    let repeats = (TOOL_OUTPUT_LIMIT / line.chars().count()) + 50;
    let big: String = line.repeat(repeats);
    assert!(big.chars().count() > TOOL_OUTPUT_LIMIT);

    let conv = base_conversation(vec![tool_call("Bash", "rg login src"), tool_result(&big)]);
    let rendered = render(&conv, Agent::Codex, fixed_created_at());
    let expected_marker = format!(
        "[truncated: showing first {} chars of {}]",
        TOOL_OUTPUT_LIMIT,
        big.chars().count()
    );
    assert!(
        rendered.contains(&expected_marker),
        "expected marker `{expected_marker}` in rendered output"
    );
    insta::assert_snapshot!("tool_call_truncated_output", rendered);
}

#[test]
fn mixed_ordering() {
    let conv = base_conversation(vec![
        human("Check the deploy script."),
        agent("Looking now."),
        tool_call("Bash", "cat deploy.sh"),
        tool_result("#!/usr/bin/env bash\nset -e\n"),
        agent("Deploy script is two lines."),
        human("Anything risky?"),
        tool_call("Bash", "grep -n rm deploy.sh"),
        tool_result(""),
        agent("Nothing risky."),
    ]);
    let rendered = render(&conv, Agent::Codex, fixed_created_at());
    insta::assert_snapshot!("mixed_ordering", rendered);
}

#[test]
fn unknown_event_preserved() {
    let conv = base_conversation(vec![
        human("Run a custom tool."),
        Block::UnknownEvent(UnknownEventBlock {
            raw_type: "experimental.custom_event".to_string(),
            raw_excerpt: "{\"foo\":\"bar\",\"baz\":42}".to_string(),
            source_event_index: 0,
        }),
        Block::SystemEvent(SystemEventBlock {
            label: "note".to_string(),
            detail: "tool registry refreshed".to_string(),
            source_event_index: 0,
        }),
    ]);
    let rendered = render(&conv, Agent::Codex, fixed_created_at());
    insta::assert_snapshot!("unknown_event_preserved", rendered);
}

#[test]
fn missing_cwd() {
    let mut conv = base_conversation(vec![human("Quick question."), agent("Sure.")]);
    conv.cwd = None;
    let rendered = render(&conv, Agent::Codex, fixed_created_at());
    assert!(
        !rendered.contains("cwd:"),
        "rendered output must omit cwd line: {rendered}"
    );
    insta::assert_snapshot!("missing_cwd", rendered);
}
