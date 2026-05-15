use std::io::Write as _;

use anyhow::Context as _;

use crate::model::SessionSummary;
use crate::providers::Provider;

pub trait Selector {
    fn pick(&self, rows: &[String]) -> anyhow::Result<Option<usize>>;
}

pub struct FzfSelector;

impl Selector for FzfSelector {
    fn pick(&self, rows: &[String]) -> anyhow::Result<Option<usize>> {
        use std::process::{Command, Stdio};

        let mut child = match Command::new("fzf")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!(
                    "fzf not found on PATH — try `--last` or pass an explicit session id"
                ));
            }
            Err(e) => return Err(e).context("could not launch fzf"),
        };

        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("could not open fzf stdin"))?;
            for row in rows {
                stdin
                    .write_all(row.as_bytes())
                    .context("could not write session row to fzf")?;
                stdin
                    .write_all(b"\n")
                    .context("could not write session row to fzf")?;
            }
        }

        let output = child
            .wait_with_output()
            .context("fzf did not exit cleanly")?;
        if !output.status.success() {
            // exit code 130 = user cancelled; treat as no selection.
            return Ok(None);
        }
        let chosen = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        if chosen.is_empty() {
            return Ok(None);
        }
        Ok(rows.iter().position(|r| r == &chosen))
    }
}

pub enum Selection {
    Explicit(String),
    Last,
    Interactive,
}

pub fn format_row(s: &SessionSummary) -> String {
    let started = s
        .started_at
        .as_ref()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "-".to_string());
    let cwd = s
        .cwd
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".to_string());
    let title = s.title.clone().unwrap_or_else(|| "-".to_string());
    format!("{}\t{}\t{}\t{}", s.id, started, cwd, title)
}

pub fn resolve(
    provider: &dyn Provider,
    selection: Selection,
    selector: &dyn Selector,
) -> anyhow::Result<SessionSummary> {
    let sessions = provider
        .list_sessions()
        .with_context(|| format!("could not list {} sessions", provider.agent().as_str()))?;
    match selection {
        Selection::Explicit(id) => sessions
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow::anyhow!("session id `{id}` not found")),
        Selection::Last => sessions
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no sessions available")),
        Selection::Interactive => {
            let rows: Vec<String> = sessions.iter().map(format_row).collect();
            let idx = selector
                .pick(&rows)?
                .ok_or_else(|| anyhow::anyhow!("no selection made"))?;
            Ok(sessions.into_iter().nth(idx).unwrap())
        }
    }
}
