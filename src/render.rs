use std::fmt::Write as _;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::model::{
    Agent, Block, Conversation, SystemEventBlock, TextBlock, ToolCallBlock, ToolResultBlock,
    TruncationInfo, UnknownEventBlock,
};

pub const TOOL_INPUT_LIMIT: usize = 4000;
pub const TOOL_OUTPUT_LIMIT: usize = 2000;
pub const UNKNOWN_EVENT_LIMIT: usize = 1000;

/// Truncate `text` to at most `max_chars` Unicode scalar values.
///
/// Returns the (possibly shortened) string and `Some(TruncationInfo)` when
/// truncation occurred.
pub(crate) fn truncate(text: &str, max_chars: usize) -> (String, Option<TruncationInfo>) {
    let total = text.chars().count();
    if total <= max_chars {
        return (text.to_string(), None);
    }
    let shown: String = text.chars().take(max_chars).collect();
    (
        shown,
        Some(TruncationInfo {
            original_chars: total,
            shown_chars: max_chars,
        }),
    )
}

/// Render a `Conversation` into the canonical Markdown handoff shape.
///
/// `created_at` is injected so callers (and snapshot tests) can pin a fixed
/// instant.
pub fn render(conv: &Conversation, target: Agent, created_at: OffsetDateTime) -> String {
    let mut out = String::new();
    writeln!(out, "source: {}", conv.source.as_str()).unwrap();
    writeln!(out, "target: {}", target.as_str()).unwrap();
    writeln!(out, "session_id: {}", conv.session_id).unwrap();
    if let Some(cwd) = &conv.cwd {
        writeln!(out, "cwd: {}", cwd.display()).unwrap();
    }
    writeln!(out, "transcript: {}", conv.transcript_path.display()).unwrap();
    writeln!(
        out,
        "created_at: {}",
        created_at.format(&Rfc3339).expect("RFC3339 formatting"),
    )
    .unwrap();

    for block in &conv.blocks {
        out.push('\n');
        render_block(&mut out, block);
    }

    out
}

fn render_block(out: &mut String, block: &Block) {
    match block {
        Block::HumanMessage(b) => render_text(out, "human", b),
        Block::AgentMessage(b) => render_text(out, "agent", b),
        Block::ToolCall(b) => render_tool_call(out, b),
        Block::ToolResult(b) => render_tool_result(out, b),
        Block::SystemEvent(b) => render_system(out, b),
        Block::UnknownEvent(b) => render_unknown(out, b),
    }
}

fn render_text(out: &mut String, label: &str, b: &TextBlock) {
    writeln!(out, "{label}:").unwrap();
    writeln!(out, "{}", b.text).unwrap();
}

fn render_tool_call(out: &mut String, b: &ToolCallBlock) {
    let (input, info) = truncate(&b.input, TOOL_INPUT_LIMIT);
    writeln!(out, "tool:").unwrap();
    writeln!(out, "name: {}", b.name).unwrap();
    writeln!(out, "input:").unwrap();
    if let Some(info) = info {
        writeln!(out, "{}", truncation_marker(&info)).unwrap();
        out.push('\n');
    }
    writeln!(out, "{input}").unwrap();
}

fn render_tool_result(out: &mut String, b: &ToolResultBlock) {
    // Honour any truncation already recorded on the block, otherwise apply the
    // default tool-output budget.
    let (body, info) = match &b.truncated {
        Some(info) => (b.output.clone(), Some(info.clone())),
        None => truncate(&b.output, TOOL_OUTPUT_LIMIT),
    };
    writeln!(out, "output:").unwrap();
    if let Some(info) = info {
        writeln!(out, "{}", truncation_marker(&info)).unwrap();
        out.push('\n');
    }
    writeln!(out, "{body}").unwrap();
}

fn render_system(out: &mut String, b: &SystemEventBlock) {
    writeln!(out, "system:").unwrap();
    writeln!(out, "{}: {}", b.label, b.detail).unwrap();
}

fn render_unknown(out: &mut String, b: &UnknownEventBlock) {
    let (excerpt, info) = truncate(&b.raw_excerpt, UNKNOWN_EVENT_LIMIT);
    writeln!(out, "unknown:").unwrap();
    writeln!(out, "type: {}", b.raw_type).unwrap();
    if let Some(info) = info {
        writeln!(out, "{}", truncation_marker(&info)).unwrap();
        out.push('\n');
    }
    writeln!(out, "{excerpt}").unwrap();
}

fn truncation_marker(info: &TruncationInfo) -> String {
    format!(
        "[truncated: showing first {} chars of {}]",
        info.shown_chars, info.original_chars
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_limit() {
        let (out, info) = truncate("hello", 10);
        assert_eq!(out, "hello");
        assert!(info.is_none());
    }

    #[test]
    fn truncate_exact_limit() {
        let (out, info) = truncate("hello", 5);
        assert_eq!(out, "hello");
        assert!(info.is_none());
    }

    #[test]
    fn truncate_over_limit() {
        let (out, info) = truncate("hello world", 5);
        assert_eq!(out, "hello");
        let info = info.expect("truncated");
        assert_eq!(info.shown_chars, 5);
        assert_eq!(info.original_chars, 11);
    }

    #[test]
    fn truncate_multibyte_is_char_safe() {
        // Each emoji is one scalar value but multiple bytes; truncation must
        // not split a code point.
        let s = "🦀🦀🦀🦀🦀";
        let (out, info) = truncate(s, 3);
        assert_eq!(out.chars().count(), 3);
        let info = info.expect("truncated");
        assert_eq!(info.shown_chars, 3);
        assert_eq!(info.original_chars, 5);
    }
}
