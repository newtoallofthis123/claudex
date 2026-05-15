use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::{tempdir, TempDir};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn fixtures_claude() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude")
}

fn fixtures_codex() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex")
}

/// Set up a tempdir-rooted XDG_CONFIG_HOME with a config.toml that points
/// claude at the fixture root and writes handoffs under `handoffs/` inside the
/// same tempdir.
fn fixture_env() -> (TempDir, PathBuf) {
    let tmp = tempdir().unwrap();
    let cfg_dir = tmp.path().join("claudex");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    let handoff_dir = tmp.path().join("handoffs");
    let cfg_text = format!(
        "handoff_dir = \"{}\"\n\n[roots]\nclaude = [\"{}\"]\ncodex = [\"{}\"]\n",
        handoff_dir.display(),
        fixtures_claude().display(),
        fixtures_codex().display(),
    );
    std::fs::write(cfg_dir.join("config.toml"), cfg_text).unwrap();
    (tmp, handoff_dir)
}

fn claudex(home: &Path) -> Command {
    let mut c = Command::cargo_bin("claudex").unwrap();
    c.env("XDG_CONFIG_HOME", home);
    c.env_remove("CLAUDE_CONFIG_DIR");
    c.env_remove("CODEX_HOME");
    c
}

#[cfg(unix)]
fn fake_fzf_that_cancels(dir: &Path) -> PathBuf {
    let path = dir.join("fzf");
    std::fs::write(&path, "#!/bin/sh\ncat >/dev/null\nexit 130\n").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[test]
fn list_claude_lists_fixtures() {
    let (tmp, _) = fixture_env();
    let out = claudex(tmp.path())
        .args(["list", "claude"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    // Expect each fixture id to appear.
    for id in [
        "fixture-simple",
        "fixture-tools",
        "fixture-unknown",
        "fixture-multibyte",
        "fixture-invalid",
    ] {
        assert!(stdout.contains(id), "missing id `{id}` in:\n{stdout}");
    }

    // Newest first: fixture-invalid (2026-05-13) precedes fixture-multibyte
    // (2026-05-09).
    let invalid_pos = stdout.find("fixture-invalid").unwrap();
    let multibyte_pos = stdout.find("fixture-multibyte").unwrap();
    assert!(invalid_pos < multibyte_pos, "wrong order:\n{stdout}");
}

#[test]
fn list_last_prints_one_row() {
    let (tmp, _) = fixture_env();
    let out = claudex(tmp.path())
        .args(["list", "claude", "--last"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "expected one row, got:\n{stdout}");
    assert!(
        lines[0].starts_with("fixture-invalid\t"),
        "got: {}",
        lines[0]
    );
}

#[test]
fn list_verbose_appends_path() {
    let (tmp, _) = fixture_env();
    let out = claudex(tmp.path())
        .args(["list", "claude", "--last", "--verbose"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let line = stdout.lines().next().unwrap();
    // tab-separated, fifth column should be the transcript path.
    let cols: Vec<&str> = line.split('\t').collect();
    assert_eq!(cols.len(), 5, "expected 5 columns, got: {line}");
    assert!(
        cols[4].ends_with("fixture-invalid.jsonl"),
        "got: {}",
        cols[4]
    );
}

#[test]
fn inspect_preview() {
    let (tmp, _) = fixture_env();
    let out = claudex(tmp.path())
        .args(["inspect", "claude:fixture-simple"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("source: claude"));
    assert!(stdout.contains("session_id: fixture-simple"));
    assert!(stdout.contains("--- preview ---"));
    assert!(stdout.contains("blocks:"));
}

#[test]
fn inspect_full_includes_body() {
    let (tmp, _) = fixture_env();
    let out = claudex(tmp.path())
        .args(["inspect", "claude:fixture-simple", "--full"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("source: claude"));
    // A known line from the rendered body.
    assert!(
        stdout.contains("hello, can you help me build something?"),
        "missing user text:\n{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn inspect_cancelled_interactive_error_is_concise_with_backtrace_enabled() {
    let (tmp, _) = fixture_env();
    let fake_bin = tempdir().unwrap();
    fake_fzf_that_cancels(fake_bin.path());

    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!(
        "{}:{}",
        fake_bin.path().display(),
        existing_path.to_string_lossy()
    );

    let assert = claudex(tmp.path())
        .args(["inspect"])
        .env("PATH", path)
        .env("RUST_BACKTRACE", "1")
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();

    assert!(
        stderr.contains("claudex: no selection made"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Stack backtrace") && !stderr.contains("std::backtrace"),
        "stderr included a backtrace:\n{stderr}"
    );
    assert!(
        !stderr.contains("src/select.rs"),
        "stderr included a source location:\n{stderr}"
    );
}

#[test]
fn handoff_no_launch_writes_file() {
    let (tmp, handoff_dir) = fixture_env();
    let out = claudex(tmp.path())
        .args(["handoff", "claude:fixture-simple", "codex", "--no-launch"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let line = stdout.lines().next().unwrap();
    assert!(line.starts_with("wrote: "), "got: {line}");
    let path = PathBuf::from(line.trim_start_matches("wrote: "));
    assert!(
        path.starts_with(&handoff_dir),
        "{path:?} not under {handoff_dir:?}"
    );
    let name = path.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("claude-to-codex-"), "name: {name}");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.starts_with("source: claude\ntarget: codex\n"),
        "body:\n{body}"
    );
}

#[test]
fn handoff_same_agent_rejected() {
    let (tmp, _) = fixture_env();
    let assert = claudex(tmp.path())
        .args(["handoff", "claude:fixture-simple", "claude", "--no-launch"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("source and target cannot both be `claude`"),
        "stderr: {stderr}"
    );
}
