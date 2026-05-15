use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use time::OffsetDateTime;

use crate::model::SessionSummary;

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

/// A cwd-equality filter for session listings. Carries both the raw
/// path (for display in error hints) and a canonicalized form (for
/// matching), since user-supplied paths may differ from a session's
/// recorded cwd by trailing slashes, `..`, or symlinks.
pub struct Scope {
    raw: PathBuf,
    canon: PathBuf,
}

impl Scope {
    pub fn new(raw: PathBuf) -> Self {
        let canon = std::fs::canonicalize(&raw).unwrap_or_else(|_| raw.clone());
        Self { raw, canon }
    }

    pub fn retain(&self, sessions: &mut Vec<SessionSummary>) {
        sessions.retain(|s| match s.cwd.as_ref() {
            Some(c) => self.matches(c),
            None => false,
        });
    }

    fn matches(&self, cwd: &Path) -> bool {
        // Fast path: exact equality avoids a syscall on the common case
        // where the session's recorded cwd is already canonical.
        if cwd == self.canon || cwd == self.raw {
            return true;
        }
        std::fs::canonicalize(cwd)
            .map(|c| c == self.canon)
            .unwrap_or(false)
    }

    pub fn hint(&self) {
        eprintln!(
            "no sessions for {}; try --all-sessions or --pwd <path>",
            self.raw.display()
        );
    }
}

/// Drive the picker against a pre-filtered list. Caller is responsible
/// for non-empty sessions.
pub fn pick_interactive(
    sessions: Vec<SessionSummary>,
    selector: &dyn Selector,
    now: OffsetDateTime,
) -> anyhow::Result<SessionSummary> {
    let rows: Vec<String> = sessions.iter().map(|s| format_picker_row(s, now)).collect();
    let idx = selector
        .pick(&rows)?
        .ok_or_else(|| anyhow::anyhow!("no selection made"))?;
    Ok(sessions.into_iter().nth(idx).unwrap())
}

fn field_time(s: &SessionSummary, now: OffsetDateTime) -> String {
    match s.started_at.as_ref() {
        Some(t) => humanize_time(*t, now),
        None => "-".to_string(),
    }
}

fn humanize_time(then: OffsetDateTime, now: OffsetDateTime) -> String {
    let delta = now - then;
    let secs = delta.whole_seconds();
    if secs < 0 {
        // Future timestamp can happen with clock skew — show a date
        // rather than "-Nm ago".
        return format_date(then);
    }
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = delta.whole_minutes();
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = delta.whole_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = delta.whole_days();
    if days < 7 {
        return format!("{days}d ago");
    }
    format_date(then)
}

fn format_date(t: OffsetDateTime) -> String {
    let fmt = time::macros::format_description!("[year]-[month]-[day]");
    t.format(&fmt).unwrap_or_else(|_| "-".to_string())
}

fn field_cwd(s: &SessionSummary) -> String {
    s.cwd
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn field_command(s: &SessionSummary) -> String {
    // Newlines are scrubbed here (not in csv_escape) because the
    // list-row format is tab-separated and has no way to quote them.
    match s.title.as_deref() {
        Some(t) => t
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect(),
        None => "-".to_string(),
    }
}

/// Tab-separated row for `claudex list`. Columns: id, command, time,
/// and dir when `show_dir` is true (i.e. `--all-sessions`).
pub fn format_list_row(s: &SessionSummary, show_dir: bool, now: OffsetDateTime) -> String {
    let base = format!("{}\t{}\t{}", s.id, field_command(s), field_time(s, now));
    if show_dir {
        format!("{}\t{}", base, field_cwd(s))
    } else {
        base
    }
}

/// CSV row for the interactive picker (fzf). Columns: id, command,
/// time, dir. CSV (not TSV) because session titles may already contain
/// tabs from copy-pasted shell output.
pub fn format_picker_row(s: &SessionSummary, now: OffsetDateTime) -> String {
    let fields = [
        s.id.clone(),
        field_command(s),
        field_time(s, now),
        field_cwd(s),
    ];
    fields
        .iter()
        .map(|f| csv_escape(f))
        .collect::<Vec<_>>()
        .join(",")
}

fn csv_escape(field: &str) -> String {
    // `field_command` already scrubs newlines, and the other fields
    // (id, formatted time, cwd) cannot contain them, so we only need
    // to quote for commas and embedded quotes.
    if field.contains(',') || field.contains('"') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        field.to_string()
    }
}
