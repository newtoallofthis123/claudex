use std::path::PathBuf;

use claudex::model::{Agent, Block, Conversation};
use claudex::providers::claude::ClaudeProvider;
use claudex::providers::{Provider, ProviderError};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude")
}

fn provider() -> ClaudeProvider {
    ClaudeProvider::new(vec![fixtures_root()])
}

/// Produce a deterministic textual dump of a parsed Conversation. We avoid
/// the renderer here because chapter B is implementing it concurrently;
/// this dump is independent of presentation.
fn dump_conversation(c: &Conversation) -> String {
    let mut out = String::new();
    out.push_str(&format!("source: {}\n", c.source.as_str()));
    out.push_str(&format!("session_id: {}\n", c.session_id));
    out.push_str(&format!(
        "cwd: {}\n",
        c.cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".into())
    ));
    out.push_str(&format!(
        "started_at: {}\n",
        c.started_at
            .map(|t| t.to_string())
            .unwrap_or_else(|| "<none>".into())
    ));
    out.push_str("blocks:\n");
    for (i, b) in c.blocks.iter().enumerate() {
        match b {
            Block::HumanMessage(t) => {
                out.push_str(&format!("  [{i}] HumanMessage(idx={}): {:?}\n", t.source_event_index, t.text));
            }
            Block::AgentMessage(t) => {
                out.push_str(&format!("  [{i}] AgentMessage(idx={}): {:?}\n", t.source_event_index, t.text));
            }
            Block::ToolCall(tc) => {
                out.push_str(&format!(
                    "  [{i}] ToolCall(idx={}, name={}): {}\n",
                    tc.source_event_index, tc.name, tc.input
                ));
            }
            Block::ToolResult(tr) => {
                out.push_str(&format!(
                    "  [{i}] ToolResult(idx={}): {:?}\n",
                    tr.source_event_index, tr.output
                ));
            }
            Block::SystemEvent(s) => {
                out.push_str(&format!(
                    "  [{i}] SystemEvent(idx={}, label={})\n",
                    s.source_event_index, s.label
                ));
            }
            Block::UnknownEvent(u) => {
                out.push_str(&format!(
                    "  [{i}] UnknownEvent(idx={}, raw_type={})\n",
                    u.source_event_index, u.raw_type
                ));
            }
        }
    }
    out
}

#[test]
fn list_sessions_returns_all_fixtures_newest_first() {
    let p = provider();
    let sessions = p.list_sessions().expect("list_sessions");
    let ids: Vec<String> = sessions.iter().map(|s| s.id.clone()).collect();
    // started_at order:
    //   fixture-invalid 2026-05-13
    //   fixture-unknown 2026-05-12
    //   fixture-tools   2026-05-11
    //   fixture-simple  2026-05-10
    //   fixture-multibyte 2026-05-09
    assert_eq!(
        ids,
        vec![
            "fixture-invalid",
            "fixture-unknown",
            "fixture-tools",
            "fixture-simple",
            "fixture-multibyte",
        ]
    );
    let s = &sessions[3];
    assert_eq!(s.id, "fixture-simple");
    assert_eq!(s.agent, Agent::Claude);
    assert_eq!(s.cwd.as_deref(), Some(std::path::Path::new("/fixtures/project-alpha")));
    assert_eq!(
        s.title.as_deref(),
        Some("hello, can you help me build something?")
    );
}

#[test]
fn resolve_session_finds_unique_id() {
    let p = provider();
    let r = p.resolve_session("fixture-simple").expect("resolve");
    assert_eq!(r.agent, Agent::Claude);
    assert_eq!(r.id, "fixture-simple");
    assert!(r.path.ends_with("project-alpha/fixture-simple.jsonl"));
}

#[test]
fn resolve_session_missing() {
    let p = provider();
    let err = p.resolve_session("nope").unwrap_err();
    match err {
        ProviderError::SessionNotFound(id) => assert_eq!(id, "nope"),
        other => panic!("expected SessionNotFound, got {other:?}"),
    }
}

#[test]
fn resolve_session_ambiguous_across_roots() {
    // Build a second root that contains a duplicate id and configure both.
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("dup-project");
    std::fs::create_dir_all(&proj).unwrap();
    let dup = proj.join("fixture-simple.jsonl");
    std::fs::write(&dup, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x\"},\"timestamp\":\"2026-05-10T10:00:00.000Z\"}\n").unwrap();

    let p = ClaudeProvider::new(vec![fixtures_root(), tmp.path().to_path_buf()]);
    let err = p.resolve_session("fixture-simple").unwrap_err();
    match err {
        ProviderError::AmbiguousSession { id, matches } => {
            assert_eq!(id, "fixture-simple");
            assert_eq!(matches.len(), 2);
            assert!(matches.windows(2).all(|w| w[0] <= w[1]), "matches must be sorted");
        }
        other => panic!("expected AmbiguousSession, got {other:?}"),
    }
}

#[test]
fn parse_transcript_simple_dialogue() {
    let p = provider();
    let r = p.resolve_session("fixture-simple").unwrap();
    let c = p.parse_transcript(&r).unwrap();
    insta::assert_snapshot!("simple-dialogue", dump_conversation(&c));
}

#[test]
fn parse_transcript_tool_call() {
    let p = provider();
    let r = p.resolve_session("fixture-tools").unwrap();
    let c = p.parse_transcript(&r).unwrap();
    insta::assert_snapshot!("tool-call", dump_conversation(&c));
}

#[test]
fn parse_transcript_unknown_event_is_not_error() {
    let p = provider();
    let r = p.resolve_session("fixture-unknown").unwrap();
    let c = p.parse_transcript(&r).unwrap();
    // Must contain exactly one UnknownEvent for raw_type "future-feature".
    let unknown_count = c
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::UnknownEvent(u) if u.raw_type == "future-feature"))
        .count();
    assert_eq!(unknown_count, 1);
    insta::assert_snapshot!("unknown-event", dump_conversation(&c));
}

#[test]
fn parse_transcript_multibyte() {
    let p = provider();
    let r = p.resolve_session("fixture-multibyte").unwrap();
    let c = p.parse_transcript(&r).unwrap();
    insta::assert_snapshot!("multibyte", dump_conversation(&c));
}

#[test]
fn parse_transcript_invalid_jsonl_reports_1_indexed_line() {
    let p = provider();
    let r = p.resolve_session("fixture-invalid").unwrap();
    let err = p.parse_transcript(&r).unwrap_err();
    match err {
        ProviderError::InvalidJsonl { path, line, .. } => {
            assert!(path.ends_with("fixture-invalid.jsonl"));
            assert_eq!(line, 3);
        }
        other => panic!("expected InvalidJsonl, got {other:?}"),
    }
}

#[test]
fn list_sessions_missing_configured_root_errors() {
    let p = ClaudeProvider::new(vec![PathBuf::from("/definitely/does/not/exist/claudex")]);
    let err = p.list_sessions().unwrap_err();
    assert!(matches!(err, ProviderError::RootNotFound(_)));
}
