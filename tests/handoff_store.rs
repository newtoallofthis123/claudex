use std::fs;

use claudex::handoff_store::HandoffStore;
use claudex::model::Agent;
use tempfile::tempdir;
use time::macros::datetime;
use time::OffsetDateTime;

fn fixed_created_at() -> OffsetDateTime {
    datetime!(2026-05-14 22:40:00 +05:30)
}

#[test]
fn writes_inside_target_dir() {
    let tmp = tempdir().unwrap();
    let store = HandoffStore::new(tmp.path().to_path_buf());
    let path = store
        .write(
            Agent::Claude,
            Agent::Codex,
            "abcdefgh",
            fixed_created_at(),
            "hello",
        )
        .unwrap();
    assert!(
        path.starts_with(tmp.path()),
        "{path:?} not under {:?}",
        tmp.path()
    );
    let body = fs::read_to_string(&path).unwrap();
    assert_eq!(body, "hello");
}

#[test]
fn filename_matches_canonical_shape() {
    let tmp = tempdir().unwrap();
    let store = HandoffStore::new(tmp.path().to_path_buf());
    let path = store
        .write(
            Agent::Claude,
            Agent::Codex,
            "abcdefghIJKL",
            fixed_created_at(),
            "body",
        )
        .unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(name, "claude-to-codex-20260514-224000-abcdefgh.md");
}

#[test]
fn collision_triggers_dedup_suffix() {
    let tmp = tempdir().unwrap();
    let store = HandoffStore::new(tmp.path().to_path_buf());
    let first = store
        .write(
            Agent::Claude,
            Agent::Codex,
            "abcdefgh",
            fixed_created_at(),
            "first",
        )
        .unwrap();
    let second = store
        .write(
            Agent::Claude,
            Agent::Codex,
            "abcdefgh",
            fixed_created_at(),
            "second",
        )
        .unwrap();
    let third = store
        .write(
            Agent::Claude,
            Agent::Codex,
            "abcdefgh",
            fixed_created_at(),
            "third",
        )
        .unwrap();
    assert_eq!(
        first.file_name().unwrap().to_str().unwrap(),
        "claude-to-codex-20260514-224000-abcdefgh.md"
    );
    assert_eq!(
        second.file_name().unwrap().to_str().unwrap(),
        "claude-to-codex-20260514-224000-abcdefgh-2.md"
    );
    assert_eq!(
        third.file_name().unwrap().to_str().unwrap(),
        "claude-to-codex-20260514-224000-abcdefgh-3.md"
    );
    assert_eq!(fs::read_to_string(&first).unwrap(), "first");
    assert_eq!(fs::read_to_string(&second).unwrap(), "second");
    assert_eq!(fs::read_to_string(&third).unwrap(), "third");
}

#[test]
fn creates_missing_dir() {
    let tmp = tempdir().unwrap();
    let nested = tmp.path().join("nested").join("handoffs");
    assert!(!nested.exists());
    let store = HandoffStore::new(nested.clone());
    let path = store
        .write(
            Agent::Codex,
            Agent::Claude,
            "abcdefgh",
            fixed_created_at(),
            "body",
        )
        .unwrap();
    assert!(nested.is_dir());
    assert!(path.starts_with(&nested));
}

#[test]
fn session_id_special_chars_are_sanitised() {
    let tmp = tempdir().unwrap();
    let store = HandoffStore::new(tmp.path().to_path_buf());
    let path = store
        .write(
            Agent::Claude,
            Agent::Codex,
            "ab/cd:ef gh!",
            fixed_created_at(),
            "body",
        )
        .unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    assert!(!name.contains('/'));
    assert!(!name.contains(':'));
    assert!(!name.contains(' '));
    assert!(!name.contains('!'));
    // The 8-char short id should be drawn from the kept characters.
    assert!(name.contains("-abcdefgh.md"), "unexpected name: {name}");
}
