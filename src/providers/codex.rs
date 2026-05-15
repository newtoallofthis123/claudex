use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::model::{
    Agent, Block, Conversation, ResolvedSession, SessionSummary, SystemEventBlock, TextBlock,
    ToolCallBlock, ToolResultBlock, UnknownEventBlock,
};
use crate::providers::{Provider, ProviderError};

pub struct CodexProvider {
    pub configured_roots: Vec<PathBuf>,
}

impl CodexProvider {
    pub fn new(configured_roots: Vec<PathBuf>) -> Self {
        Self { configured_roots }
    }
}

impl Provider for CodexProvider {
    fn agent(&self) -> Agent {
        Agent::Codex
    }

    fn effective_roots(&self, _configured_roots: &[PathBuf]) -> Vec<PathBuf> {
        crate::config::effective_codex_roots(&self.configured_roots)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>, ProviderError> {
        let roots = self.effective_roots(&self.configured_roots);
        let configured_explicit = !self.configured_roots.is_empty();

        let mut summaries: Vec<SessionSummary> = Vec::new();
        let mut any_root_existed = false;
        let mut first_attempted: Option<PathBuf> = None;

        for root in &roots {
            if first_attempted.is_none() {
                first_attempted = Some(root.clone());
            }
            if !root.exists() {
                if configured_explicit {
                    return Err(ProviderError::RootNotFound(root.clone()));
                }
                continue;
            }
            any_root_existed = true;

            for rollout in iter_rollouts(root) {
                match summarize_rollout(&rollout) {
                    Ok(Some(s)) => summaries.push(s),
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!(
                            "claudex: skipping unreadable rollout {}: {}",
                            rollout.display(),
                            e
                        );
                    }
                }
            }
        }

        if !any_root_existed {
            return Err(ProviderError::RootNotFound(
                first_attempted.unwrap_or_else(|| PathBuf::from(".")),
            ));
        }

        summaries.sort_by(|a, b| match (a.started_at, b.started_at) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        });

        Ok(summaries)
    }

    fn resolve_session(&self, id: &str) -> Result<ResolvedSession, ProviderError> {
        let roots = self.effective_roots(&self.configured_roots);

        let mut matches: Vec<PathBuf> = Vec::new();

        // Fast path: filename includes id.
        for root in &roots {
            if !root.exists() {
                continue;
            }
            for rollout in iter_rollouts(root) {
                let name = rollout
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if name.contains(id) && verify_session_id(&rollout, id) {
                    matches.push(rollout);
                }
            }
        }

        // Fall back to scanning all rollouts.
        if matches.is_empty() {
            for root in &roots {
                if !root.exists() {
                    continue;
                }
                for rollout in iter_rollouts(root) {
                    if verify_session_id(&rollout, id) {
                        matches.push(rollout);
                    }
                }
            }
        }

        matches.sort();
        matches.dedup();

        match matches.len() {
            0 => Err(ProviderError::SessionNotFound(id.to_string())),
            1 => Ok(ResolvedSession {
                agent: Agent::Codex,
                id: id.to_string(),
                path: matches.into_iter().next().unwrap(),
            }),
            _ => Err(ProviderError::AmbiguousSession {
                id: id.to_string(),
                matches,
            }),
        }
    }

    fn parse_transcript(&self, session: &ResolvedSession) -> Result<Conversation, ProviderError> {
        let path = &session.path;
        let file = File::open(path).map_err(|e| ProviderError::TranscriptUnreadable {
            path: path.clone(),
            source: e,
        })?;
        let reader = BufReader::new(file);

        let mut blocks: Vec<Block> = Vec::new();
        let mut cwd: Option<PathBuf> = None;
        let mut started_at: Option<OffsetDateTime> = None;

        for (idx, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| ProviderError::TranscriptUnreadable {
                path: path.clone(),
                source: e,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|e| ProviderError::InvalidJsonl {
                    path: path.clone(),
                    line: idx + 1,
                    source: e,
                })?;

            let outer = value.get("type").and_then(|v| v.as_str());
            let payload = value.get("payload");
            let payload_type = payload.and_then(|p| p.get("type")).and_then(|v| v.as_str());

            // Metadata harvest.
            if outer == Some("session_meta") {
                if let Some(p) = payload {
                    if cwd.is_none() {
                        if let Some(c) = p.get("cwd").and_then(|v| v.as_str()) {
                            cwd = Some(PathBuf::from(c));
                        }
                    }
                    if started_at.is_none() {
                        if let Some(ts) = p.get("timestamp").and_then(|v| v.as_str()) {
                            started_at = OffsetDateTime::parse(ts, &Rfc3339).ok();
                        }
                    }
                }
            }

            let block = classify(idx, outer, payload_type, payload, &value);
            blocks.push(block);
        }

        Ok(Conversation {
            source: Agent::Codex,
            session_id: session.id.clone(),
            transcript_path: path.clone(),
            cwd,
            started_at,
            blocks,
        })
    }
}

fn classify(
    idx: usize,
    outer: Option<&str>,
    payload_type: Option<&str>,
    payload: Option<&Value>,
    full: &Value,
) -> Block {
    let payload_v = payload.cloned().unwrap_or(Value::Null);

    match (outer, payload_type) {
        (Some(label @ ("session_meta" | "turn_context")), _) => {
            Block::SystemEvent(SystemEventBlock {
                label: label.to_string(),
                detail: compact(&payload_v),
                source_event_index: idx,
            })
        }
        (_, Some("user_message")) => Block::HumanMessage(TextBlock {
            text: extract_message_text(&payload_v),
            source_event_index: idx,
        }),
        (_, Some("agent_message")) => Block::AgentMessage(TextBlock {
            text: extract_message_text(&payload_v),
            source_event_index: idx,
        }),
        (_, Some("message")) => {
            let role = payload_v.get("role").and_then(|v| v.as_str());
            let text = flatten_content_parts(payload_v.get("content"));
            match role {
                Some("user") => Block::HumanMessage(TextBlock {
                    text,
                    source_event_index: idx,
                }),
                Some("assistant") => Block::AgentMessage(TextBlock {
                    text,
                    source_event_index: idx,
                }),
                _ => Block::SystemEvent(SystemEventBlock {
                    label: "message".to_string(),
                    detail: text,
                    source_event_index: idx,
                }),
            }
        }
        (_, Some(t @ ("function_call" | "custom_tool_call" | "mcp_tool_call_end"))) => {
            let name = payload_v
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(t)
                .to_string();
            let input = payload_v
                .get("arguments")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&payload_v).unwrap_or_else(|_| "{}".to_string())
                });
            Block::ToolCall(ToolCallBlock {
                name,
                input,
                source_event_index: idx,
            })
        }
        (_, Some("function_call_output" | "custom_tool_call_output" | "patch_apply_end")) => {
            let output = payload_v
                .get("output")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&payload_v).unwrap_or_else(|_| "{}".to_string())
                });
            Block::ToolResult(ToolResultBlock {
                output,
                truncated: None,
                source_event_index: idx,
            })
        }
        (
            _,
            Some(
                t @ ("reasoning" | "token_count" | "task_started" | "task_complete"
                | "turn_aborted" | "thread_rolled_back"),
            ),
        ) => Block::SystemEvent(SystemEventBlock {
            label: t.to_string(),
            detail: compact(&payload_v),
            source_event_index: idx,
        }),
        _ => {
            let raw_type = payload_type.or(outer).unwrap_or("<missing>").to_string();
            Block::UnknownEvent(UnknownEventBlock {
                raw_type,
                raw_excerpt: compact(full),
                source_event_index: idx,
            })
        }
    }
}

fn extract_message_text(payload: &Value) -> String {
    if let Some(s) = payload.get("message").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    flatten_content_parts(payload.get("content"))
}

fn flatten_content_parts(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let Some(arr) = content.as_array() else {
        return compact(content);
    };
    let mut parts: Vec<String> = Vec::new();
    for item in arr {
        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
            parts.push(text.to_string());
        } else if let Some(ty) = item.get("type").and_then(|v| v.as_str()) {
            parts.push(format!("[{} omitted]", ty));
        } else {
            parts.push("[unknown content part omitted]".to_string());
        }
    }
    parts.join("\n")
}

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Walk `<root>/YYYY/MM/DD/rollout-*.jsonl`.
fn iter_rollouts(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let Ok(years) = fs::read_dir(root) else {
        return out;
    };
    for y in years.flatten() {
        if !is_numeric_dir(&y) {
            continue;
        }
        let Ok(months) = fs::read_dir(y.path()) else {
            continue;
        };
        for m in months.flatten() {
            if !is_numeric_dir(&m) {
                continue;
            }
            let Ok(days) = fs::read_dir(m.path()) else {
                continue;
            };
            for d in days.flatten() {
                if !is_numeric_dir(&d) {
                    continue;
                }
                let Ok(files) = fs::read_dir(d.path()) else {
                    continue;
                };
                for f in files.flatten() {
                    let p = f.path();
                    let name = match p.file_name().and_then(|s| s.to_str()) {
                        Some(n) => n,
                        None => continue,
                    };
                    if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

fn is_numeric_dir(entry: &fs::DirEntry) -> bool {
    let Ok(ft) = entry.file_type() else {
        return false;
    };
    if !ft.is_dir() {
        return false;
    }
    entry
        .file_name()
        .to_str()
        .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false)
}

#[derive(serde::Deserialize)]
struct MetaPayload {
    id: Option<String>,
}

#[derive(serde::Deserialize)]
struct MetaLine {
    #[serde(rename = "type")]
    ty: Option<String>,
    payload: Option<MetaPayload>,
}

fn read_session_meta(path: &Path) -> Option<MetaLine> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: MetaLine = serde_json::from_str(&line).ok()?;
        return Some(parsed);
    }
    None
}

fn verify_session_id(path: &Path, id: &str) -> bool {
    match read_session_meta(path) {
        Some(m) => {
            m.ty.as_deref() == Some("session_meta")
                && m.payload.and_then(|p| p.id).as_deref() == Some(id)
        }
        None => false,
    }
}

#[derive(serde::Deserialize)]
struct ScanPayload {
    #[serde(rename = "type")]
    ty: Option<String>,
    role: Option<String>,
    message: Option<serde_json::Value>,
    content: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct ScanLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    ty: Option<String>,
    payload: Option<ScanPayload>,
}

fn summarize_rollout(path: &Path) -> std::io::Result<Option<SessionSummary>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut id: Option<String> = None;
    let mut cwd: Option<PathBuf> = None;
    let mut started_at: Option<OffsetDateTime> = None;
    let mut updated_at: Option<OffsetDateTime> = None;
    let mut title: Option<String> = None;
    let mut saw_meta = false;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<ScanLine>(&line) else {
            continue;
        };

        if !saw_meta && parsed.ty.as_deref() == Some("session_meta") {
            // ScanPayload doesn't carry id/cwd; reparse as Value to pick them up.
            if let Ok(meta_val) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(p) = meta_val.get("payload") {
                    if let Some(v) = p.get("id").and_then(|v| v.as_str()) {
                        id = Some(v.to_string());
                    }
                    if let Some(v) = p.get("cwd").and_then(|v| v.as_str()) {
                        cwd = Some(PathBuf::from(v));
                    }
                    if let Some(v) = p.get("timestamp").and_then(|v| v.as_str()) {
                        started_at = OffsetDateTime::parse(v, &Rfc3339).ok();
                    }
                }
            }
            saw_meta = true;
        }

        if let Some(ts) = parsed.timestamp.as_deref() {
            if let Ok(t) = OffsetDateTime::parse(ts, &Rfc3339) {
                updated_at = Some(t);
            }
        }

        if title.is_none() {
            if let Some(p) = &parsed.payload {
                let candidate = match p.ty.as_deref() {
                    Some("user_message") => p
                        .message
                        .as_ref()
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| {
                            Some(flatten_content_parts(p.content.as_ref()))
                                .filter(|s| !s.is_empty())
                        }),
                    Some("message") if p.role.as_deref() == Some("user") => {
                        let t = flatten_content_parts(p.content.as_ref());
                        if t.is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    }
                    _ => None,
                };
                if let Some(t) = candidate {
                    title = Some(make_title(&t));
                }
            }
        }
    }

    let Some(id) = id else {
        return Ok(None);
    };

    Ok(Some(SessionSummary {
        agent: Agent::Codex,
        id,
        path: path.to_path_buf(),
        cwd,
        started_at,
        updated_at,
        title,
    }))
}

fn make_title(s: &str) -> String {
    let collapsed: String = s
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = collapsed.trim();
    trimmed.chars().take(80).collect()
}
