use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use crate::model::Agent;

const DEDUP_LIMIT: u32 = 100;
const SHORT_ID_LEN: usize = 8;

const DATE_FMT: &[FormatItem<'static>] = format_description!("[year][month][day]");
const TIME_FMT: &[FormatItem<'static>] = format_description!("[hour][minute][second]");

#[derive(Debug)]
pub struct HandoffStore {
    dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum HandoffStoreError {
    #[error("could not create handoff dir {0}: {1}")]
    DirCreate(PathBuf, #[source] std::io::Error),
    #[error("handoff file already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("could not write handoff file {0}: {1}")]
    Write(PathBuf, #[source] std::io::Error),
}

impl HandoffStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write `body` to a deterministically-named file under `self.dir`,
    /// retrying with `-2`, `-3`, … suffixes on collision. Returns the final
    /// path written.
    pub fn write(
        &self,
        source: Agent,
        target: Agent,
        session_id: &str,
        created_at: OffsetDateTime,
        body: &str,
    ) -> Result<PathBuf, HandoffStoreError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| HandoffStoreError::DirCreate(self.dir.clone(), e))?;

        let mut last_path = self.dir.clone();
        for dedup in 1..=DEDUP_LIMIT {
            let name = Self::filename(source, target, session_id, created_at, dedup);
            let path = self.dir.join(&name);
            last_path = path.clone();
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(mut f) => {
                    f.write_all(body.as_bytes())
                        .map_err(|e| HandoffStoreError::Write(path.clone(), e))?;
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(HandoffStoreError::Write(path, e)),
            }
        }
        Err(HandoffStoreError::AlreadyExists(last_path))
    }

    /// Build the canonical handoff filename. `dedup == 1` yields the base
    /// name; higher values append `-2`, `-3`, … before `.md`.
    pub(crate) fn filename(
        source: Agent,
        target: Agent,
        session_id: &str,
        created_at: OffsetDateTime,
        dedup: u32,
    ) -> String {
        let date = created_at.format(DATE_FMT).expect("date format");
        let time = created_at.format(TIME_FMT).expect("time format");
        let short = short_id(session_id);
        let base = format!(
            "{}-to-{}-{}-{}-{}",
            source.as_str(),
            target.as_str(),
            date,
            time,
            short,
        );
        if dedup <= 1 {
            format!("{base}.md")
        } else {
            format!("{base}-{dedup}.md")
        }
    }
}

/// Reduce `session_id` to exactly `SHORT_ID_LEN` characters drawn from
/// `[A-Za-z0-9_-]`. Empty input falls back to `"session"` padded with `0`.
fn short_id(session_id: &str) -> String {
    let mut sanitised: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(SHORT_ID_LEN)
        .collect();
    if sanitised.is_empty() {
        sanitised.push_str("session");
    }
    while sanitised.chars().count() < SHORT_ID_LEN {
        sanitised.push('0');
    }
    sanitised
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_truncates() {
        assert_eq!(short_id("abcdefghijkl"), "abcdefgh");
    }

    #[test]
    fn short_id_pads_when_short() {
        let s = short_id("abc");
        assert_eq!(s.chars().count(), SHORT_ID_LEN);
        assert!(s.starts_with("abc"));
    }

    #[test]
    fn short_id_sanitises() {
        let s = short_id("a/b:c d");
        assert!(!s.contains('/'));
        assert!(!s.contains(':'));
        assert!(!s.contains(' '));
    }

    #[test]
    fn short_id_fallback_when_empty() {
        let s = short_id("/:/:");
        assert_eq!(s, "session0");
    }
}
