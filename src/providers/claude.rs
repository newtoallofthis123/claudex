use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::model::{
    Agent, Block, Conversation, ResolvedSession, SessionSummary, SystemEventBlock, TextBlock,
    ToolCallBlock, ToolResultBlock, UnknownEventBlock,
};
use crate::providers::{Provider, ProviderError};

pub struct ClaudeProvider {
    pub configured_roots: Vec<PathBuf>,
}

impl ClaudeProvider {
    pub fn new(configured_roots: Vec<PathBuf>) -> Self {
        Self { configured_roots }
    }
}

/// Compact tiny header used for cheap timestamp scans.
#[derive(Debug, Deserialize)]
struct TimestampLine {
    #[serde(default)]
    timestamp: Option<String>,
}

fn parse_ts(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

fn compact_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| String::from("<unserializable>"))
}

/// Reduce `message.content` items of type=text into a single joined string for a title.
fn first_text_from_content(content: &serde_json::Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    } else if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
        let joined = parts.join("\n");
        if joined.trim().is_empty() {
            None
        } else {
            Some(joined)
        }
    } else {
        None
    }
}

fn make_title(raw: &str) -> String {
    let cleaned: String = raw.replace(['\n', '\r'], " ");
    let trimmed = cleaned.trim();
    let mut out = String::new();
    for (i, c) in trimmed.chars().enumerate() {
        if i >= 80 {
            break;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// Flatten `tool_result.content` (string OR array of {type:text,text}) into one string.
fn flatten_tool_result_content(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(t.to_string());
                }
            } else {
                parts.push(compact_json(item));
            }
        }
        return parts.join("\n");
    }
    compact_json(content)
}

/// Walk one project directory (one level), collecting *.jsonl files.
/// Skips the `memory` subdirectory.
fn list_jsonl_in_project_dir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
    out
}

fn list_project_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        if p.file_name().and_then(|s| s.to_str()) == Some("memory") {
            continue;
        }
        out.push(p);
    }
    out
}

fn summarize_transcript(path: &Path, agent: Agent) -> Option<SessionSummary> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("claudex: skipping {}: {}", path.display(), e);
            return None;
        }
    };
    let reader = BufReader::new(file);

    let mut started_at: Option<OffsetDateTime> = None;
    let mut updated_at: Option<OffsetDateTime> = None;
    let mut cwd: Option<PathBuf> = None;
    let mut title: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        // Cheap timestamp parse first.
        if let Ok(ts) = serde_json::from_str::<TimestampLine>(&line) {
            if let Some(ref s) = ts.timestamp {
                if let Some(parsed) = parse_ts(s) {
                    if started_at.is_none() {
                        started_at = Some(parsed);
                    }
                    updated_at = Some(parsed);
                }
            }
        }

        // For cwd / title we need the full Value.
        if cwd.is_some() && title.is_some() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if cwd.is_none() {
            if let Some(c) = value.get("cwd").and_then(|c| c.as_str()) {
                if !c.is_empty() {
                    cwd = Some(PathBuf::from(c));
                }
            }
        }
        if title.is_none() && value.get("type").and_then(|t| t.as_str()) == Some("user") {
            if let Some(content) = value.get("message").and_then(|m| m.get("content")) {
                if let Some(text) = first_text_from_content(content) {
                    title = Some(make_title(&text));
                }
            }
        }
    }

    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    if id.is_empty() {
        return None;
    }

    Some(SessionSummary {
        agent,
        id,
        path: path.to_path_buf(),
        cwd,
        started_at,
        updated_at,
        title,
    })
}

fn push_message_blocks(
    blocks: &mut Vec<Block>,
    idx: usize,
    value: &serde_json::Value,
    role_human: bool,
) {
    let content = match value.get("message").and_then(|m| m.get("content")) {
        Some(c) => c,
        None => return,
    };

    // Plain string -> single text block.
    if let Some(s) = content.as_str() {
        let block = TextBlock {
            text: s.to_string(),
            source_event_index: idx,
        };
        blocks.push(if role_human {
            Block::HumanMessage(block)
        } else {
            Block::AgentMessage(block)
        });
        return;
    }

    let Some(arr) = content.as_array() else {
        return;
    };

    // Walk array; coalesce consecutive `text` items into a single text block.
    let mut text_buf: Vec<String> = Vec::new();
    let flush_text =
        |buf: &mut Vec<String>, blocks: &mut Vec<Block>| {
            if buf.is_empty() {
                return;
            }
            let joined = buf.join("\n");
            buf.clear();
            let tb = TextBlock {
                text: joined,
                source_event_index: idx,
            };
            blocks.push(if role_human {
                Block::HumanMessage(tb)
            } else {
                Block::AgentMessage(tb)
            });
        };

    for item in arr {
        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match ty {
            "text" => {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    text_buf.push(t.to_string());
                }
            }
            "tool_use" => {
                flush_text(&mut text_buf, blocks);
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_value = item.get("input").cloned().unwrap_or(serde_json::Value::Null);
                let input = serde_json::to_string_pretty(&input_value)
                    .unwrap_or_else(|_| compact_json(&input_value));
                blocks.push(Block::ToolCall(ToolCallBlock {
                    name,
                    input,
                    source_event_index: idx,
                }));
            }
            "tool_result" => {
                flush_text(&mut text_buf, blocks);
                let inner = item.get("content").cloned().unwrap_or(serde_json::Value::Null);
                let output = flatten_tool_result_content(&inner);
                blocks.push(Block::ToolResult(ToolResultBlock {
                    output,
                    truncated: None,
                    source_event_index: idx,
                }));
            }
            other => {
                flush_text(&mut text_buf, blocks);
                blocks.push(Block::UnknownEvent(UnknownEventBlock {
                    raw_type: other.to_string(),
                    raw_excerpt: compact_json(item),
                    source_event_index: idx,
                }));
            }
        }
    }
    flush_text(&mut text_buf, blocks);
}

impl Provider for ClaudeProvider {
    fn agent(&self) -> Agent {
        Agent::Claude
    }

    fn effective_roots(&self, _configured_roots: &[PathBuf]) -> Vec<PathBuf> {
        crate::config::effective_claude_roots(&self.configured_roots)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>, ProviderError> {
        let roots = self.effective_roots(&self.configured_roots);
        if roots.is_empty() {
            return Err(ProviderError::RootNotFound(PathBuf::from("")));
        }

        let configured_present = !self.configured_roots.is_empty();
        let mut summaries: Vec<SessionSummary> = Vec::new();
        let mut any_existed = false;
        let mut missing_configured: Option<PathBuf> = None;

        for root in &roots {
            if !root.exists() {
                if configured_present && missing_configured.is_none() {
                    missing_configured = Some(root.clone());
                }
                continue;
            }
            any_existed = true;
            for proj in list_project_dirs(root) {
                for jsonl in list_jsonl_in_project_dir(&proj) {
                    if let Some(s) = summarize_transcript(&jsonl, Agent::Claude) {
                        summaries.push(s);
                    }
                }
            }
        }

        if let Some(missing) = missing_configured {
            // Configured root that doesn't exist is a hard error.
            return Err(ProviderError::RootNotFound(missing));
        }
        if !any_existed {
            return Err(ProviderError::RootNotFound(roots.into_iter().next().unwrap()));
        }

        // Sort by started_at descending; None sorts last.
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
        for root in &roots {
            if !root.exists() {
                continue;
            }
            for proj in list_project_dirs(root) {
                let candidate = proj.join(format!("{id}.jsonl"));
                if candidate.is_file() {
                    matches.push(candidate);
                }
            }
        }
        matches.sort();
        matches.dedup();
        match matches.len() {
            0 => Err(ProviderError::SessionNotFound(id.to_string())),
            1 => Ok(ResolvedSession {
                agent: Agent::Claude,
                id: id.to_string(),
                path: matches.into_iter().next().unwrap(),
            }),
            _ => Err(ProviderError::AmbiguousSession {
                id: id.to_string(),
                matches,
            }),
        }
    }

    fn parse_transcript(
        &self,
        session: &ResolvedSession,
    ) -> Result<Conversation, ProviderError> {
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
            let value: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| ProviderError::InvalidJsonl {
                    path: path.clone(),
                    line: idx + 1,
                    source: e,
                })?;

            if cwd.is_none() {
                if let Some(c) = value.get("cwd").and_then(|c| c.as_str()) {
                    if !c.is_empty() {
                        cwd = Some(PathBuf::from(c));
                    }
                }
            }
            if started_at.is_none() {
                if let Some(ts) = value.get("timestamp").and_then(|t| t.as_str()) {
                    if let Some(parsed) = parse_ts(ts) {
                        started_at = Some(parsed);
                    }
                }
            }

            let ty = value.get("type").and_then(|t| t.as_str());
            match ty {
                Some("user") => push_message_blocks(&mut blocks, idx, &value, true),
                Some("assistant") => push_message_blocks(&mut blocks, idx, &value, false),
                Some(t @ ("permission-mode" | "file-history-snapshot" | "last-prompt"
                | "attachment")) => {
                    blocks.push(Block::SystemEvent(SystemEventBlock {
                        label: t.to_string(),
                        detail: compact_json(&value),
                        source_event_index: idx,
                    }));
                }
                Some(other) => {
                    blocks.push(Block::UnknownEvent(UnknownEventBlock {
                        raw_type: other.to_string(),
                        raw_excerpt: compact_json(&value),
                        source_event_index: idx,
                    }));
                }
                None => {
                    blocks.push(Block::UnknownEvent(UnknownEventBlock {
                        raw_type: "<missing>".to_string(),
                        raw_excerpt: compact_json(&value),
                        source_event_index: idx,
                    }));
                }
            }
        }

        Ok(Conversation {
            source: Agent::Claude,
            session_id: session.id.clone(),
            transcript_path: path.clone(),
            cwd,
            started_at,
            blocks,
        })
    }
}
