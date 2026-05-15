use assert_cmd::Command;
use tempfile::tempdir;

fn claudex(home: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("claudex").unwrap();
    c.env("XDG_CONFIG_HOME", home);
    c.env_remove("CLAUDE_CONFIG_DIR");
    c.env_remove("CODEX_HOME");
    c
}

#[test]
fn settings_path_uses_xdg() {
    let tmp = tempdir().unwrap();
    let out = claudex(tmp.path())
        .args(["settings", "path"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.trim().ends_with("claudex/config.toml"),
        "got: {stdout}"
    );
    assert!(
        stdout.trim().starts_with(tmp.path().to_str().unwrap()),
        "got: {stdout}"
    );
}

#[test]
fn settings_roundtrip() {
    let tmp = tempdir().unwrap();

    claudex(tmp.path())
        .args(["settings", "set", "handoff_dir", "/tmp/handoffs"])
        .assert()
        .success();

    let out = claudex(tmp.path())
        .args(["settings", "get", "handoff_dir"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("/tmp/handoffs"), "got: {stdout}");

    claudex(tmp.path())
        .args(["settings", "add-root", "claude", "/tmp/foo"])
        .assert()
        .success();

    let out = claudex(tmp.path())
        .args(["settings", "show"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("/tmp/foo"),
        "show stdout missing root:\n{stdout}"
    );

    claudex(tmp.path())
        .args(["settings", "reset-root", "claude"])
        .assert()
        .success();

    let out = claudex(tmp.path())
        .args(["settings", "show"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    // The configured TOML section should no longer list /tmp/foo. (Effective
    // section may show a default root, which is fine.)
    let configured_section = stdout.split("# effective").next().unwrap_or("");
    assert!(
        !configured_section.contains("/tmp/foo"),
        "reset-root left configured root in place:\n{configured_section}"
    );
}

#[test]
fn add_root_is_idempotent() {
    let tmp = tempdir().unwrap();
    for _ in 0..2 {
        claudex(tmp.path())
            .args(["settings", "add-root", "codex", "/tmp/once"])
            .assert()
            .success();
    }
    let out = claudex(tmp.path())
        .args(["settings", "show"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let occurrences = stdout.matches("/tmp/once").count();
    // One in configured TOML, possibly one in effective. We want exactly two
    // (or one if the effective renders as relative). Just assert it's > 0 and
    // not duplicated within the configured section.
    let configured_section = stdout.split("# effective").next().unwrap_or("");
    assert_eq!(
        configured_section.matches("/tmp/once").count(),
        1,
        "expected exactly one occurrence in configured, got {occurrences}:\n{stdout}"
    );
}

#[test]
fn unknown_get_key_errors() {
    let tmp = tempdir().unwrap();
    claudex(tmp.path())
        .args(["settings", "get", "nope"])
        .assert()
        .failure();
}
