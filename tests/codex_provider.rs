use std::path::{Path, PathBuf};

use claudex::model::{Agent, Block, Conversation};
use claudex::providers::codex::CodexProvider;
use claudex::providers::{Provider, ProviderError};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex")
}

fn provider() -> CodexProvider {
    CodexProvider::new(vec![fixture_root()])
}

fn dump_conversation(c: &Conversation) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    writeln!(s, "session_id: {}", c.session_id).unwrap();
    writeln!(s, "agent: {:?}", c.source).unwrap();
    writeln!(
        s,
        "cwd: {}",
        c.cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    )
    .unwrap();
    writeln!(
        s,
        "started_at: {}",
        c.started_at
            .map(|t| t
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default())
            .unwrap_or_default()
    )
    .unwrap();
    writeln!(s, "blocks:").unwrap();
    for b in &c.blocks {
        match b {
            Block::HumanMessage(t) => {
                writeln!(s, "  [{}] HumanMessage: {:?}", t.source_event_index, t.text).unwrap()
            }
            Block::AgentMessage(t) => {
                writeln!(s, "  [{}] AgentMessage: {:?}", t.source_event_index, t.text).unwrap()
            }
            Block::ToolCall(t) => writeln!(
                s,
                "  [{}] ToolCall: name={:?} input={:?}",
                t.source_event_index, t.name, t.input
            )
            .unwrap(),
            Block::ToolResult(t) => writeln!(
                s,
                "  [{}] ToolResult: output={:?}",
                t.source_event_index, t.output
            )
            .unwrap(),
            Block::SystemEvent(e) => writeln!(
                s,
                "  [{}] SystemEvent: label={} detail={}",
                e.source_event_index, e.label, e.detail
            )
            .unwrap(),
            Block::UnknownEvent(e) => writeln!(
                s,
                "  [{}] UnknownEvent: raw_type={} raw_excerpt={}",
                e.source_event_index, e.raw_type, e.raw_excerpt
            )
            .unwrap(),
        }
    }
    s
}

fn snapshot_setting() -> insta::Settings {
    let mut s = insta::Settings::clone_current();
    s.set_snapshot_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"));
    s.set_prepend_module_to_snapshot(false);
    s
}

#[test]
fn list_sessions_returns_all_fixtures_newest_first() {
    let p = provider();
    let sessions = p.list_sessions().expect("list ok");
    // fixture-invalid only has 2 well-formed lines before bad json but summary
    // scan tolerates the bad line (skip) so it still appears.
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"fixture-simple"));
    assert!(ids.contains(&"fixture-tool"));
    assert!(ids.contains(&"fixture-unknown"));
    assert!(ids.contains(&"fixture-invalid"));

    // Sorted newest first by started_at.
    let starts: Vec<_> = sessions.iter().map(|s| s.started_at).collect();
    for w in starts.windows(2) {
        if let (Some(a), Some(b)) = (w[0], w[1]) {
            assert!(a >= b, "not sorted desc: {:?} vs {:?}", a, b);
        }
    }

    let simple = sessions.iter().find(|s| s.id == "fixture-simple").unwrap();
    assert_eq!(simple.cwd, Some(PathBuf::from("/work/proj")));
    assert_eq!(simple.title.as_deref(), Some("hello codex"));
    assert!(simple.updated_at.is_some());
    assert!(simple.updated_at >= simple.started_at);

    let tool = sessions.iter().find(|s| s.id == "fixture-tool").unwrap();
    assert_eq!(tool.title.as_deref(), Some("list files in cwd"));
}

#[test]
fn resolve_session_finds_by_id() {
    let p = provider();
    let r = p.resolve_session("fixture-tool").expect("resolve ok");
    assert_eq!(r.agent, Agent::Codex);
    assert_eq!(r.id, "fixture-tool");
    assert!(r.path.to_string_lossy().contains("fixture-tool"));
}

#[test]
fn resolve_unknown_session() {
    let p = provider();
    let err = p.resolve_session("does-not-exist").unwrap_err();
    matches!(err, ProviderError::SessionNotFound(_));
}

#[test]
fn parse_simple_snapshot() {
    let p = provider();
    let s = p.resolve_session("fixture-simple").unwrap();
    let c = p.parse_transcript(&s).unwrap();
    snapshot_setting().bind(|| {
        insta::assert_snapshot!("codex_simple", dump_conversation(&c));
    });
}

#[test]
fn parse_tool_snapshot() {
    let p = provider();
    let s = p.resolve_session("fixture-tool").unwrap();
    let c = p.parse_transcript(&s).unwrap();
    snapshot_setting().bind(|| {
        insta::assert_snapshot!("codex_tool", dump_conversation(&c));
    });
}

#[test]
fn parse_unknown_snapshot() {
    let p = provider();
    let s = p.resolve_session("fixture-unknown").unwrap();
    let c = p.parse_transcript(&s).unwrap();
    snapshot_setting().bind(|| {
        insta::assert_snapshot!("codex_unknown", dump_conversation(&c));
    });
}

#[test]
fn parse_invalid_jsonl_returns_invalid_jsonl_with_line() {
    let p = provider();
    let s = p.resolve_session("fixture-invalid").unwrap();
    let err = p.parse_transcript(&s).unwrap_err();
    match err {
        ProviderError::InvalidJsonl { line, .. } => assert_eq!(line, 3),
        other => panic!("expected InvalidJsonl, got {:?}", other),
    }
}

#[test]
fn classify_message_with_unknown_role_is_system_event() {
    // Build a tiny ad-hoc fixture in a temp dir.
    let dir = tempfile::tempdir().unwrap();
    let dpath = dir.path().join("2026/05/14");
    std::fs::create_dir_all(&dpath).unwrap();
    let file = dpath.join("rollout-x-fixture-roleweird.jsonl");
    std::fs::write(
        &file,
        concat!(
            r#"{"timestamp":"2026-05-14T09:00:00Z","type":"session_meta","payload":{"id":"fixture-roleweird","cwd":"/x","timestamp":"2026-05-14T09:00:00Z"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-14T09:00:01Z","type":"response_item","payload":{"type":"message","role":"system","content":[{"type":"input_text","text":"sysprompt"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    let p = CodexProvider::new(vec![dir.path().to_path_buf()]);
    let s = p.resolve_session("fixture-roleweird").unwrap();
    let c = p.parse_transcript(&s).unwrap();
    let has_sys = c
        .blocks
        .iter()
        .any(|b| matches!(b, Block::SystemEvent(e) if e.label == "message"));
    assert!(
        has_sys,
        "expected SystemEvent for message with role=system, got {:?}",
        c.blocks
    );
}
