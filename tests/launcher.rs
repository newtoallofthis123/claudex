use std::path::PathBuf;

use claudex::launch::{catch_up_prompt, LaunchError, Launcher, ProcessLauncher};
use claudex::model::Agent;
use tempfile::tempdir;

static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn missing_exec_returns_not_found() {
    let _guard = PATH_LOCK.lock().unwrap();

    // Point PATH at an empty temp dir so the agent executable cannot be found.
    let empty = tempdir().unwrap();
    let prev = std::env::var_os("PATH");
    unsafe {
        std::env::set_var("PATH", empty.path());
    }
    let result = ProcessLauncher.launch(Agent::Claude, "anything");
    restore_path(prev);
    let err = result.expect_err("expected launch failure");
    assert!(
        matches!(err, LaunchError::ExecutableNotFound(_)),
        "expected ExecutableNotFound, got {err:?}"
    );
}

#[test]
fn nonzero_exit_is_reported() {
    let _guard = PATH_LOCK.lock().unwrap();

    // Shim `claude` to `false` by giving PATH a dir with a `claude` script.
    let dir = tempdir().unwrap();
    let shim = dir.path().join("claude");
    std::fs::write(&shim, "#!/bin/sh\nexit 7\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&shim).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&shim, perms).unwrap();
    }

    let prev = std::env::var_os("PATH");
    unsafe {
        std::env::set_var("PATH", dir.path());
    }
    let result = ProcessLauncher.launch(Agent::Claude, "ignored");
    restore_path(prev);
    let err = result.expect_err("expected launch failure");
    match err {
        LaunchError::NonZeroExit { cmd, status } => {
            assert_eq!(cmd, "claude");
            assert!(!status.success());
        }
        other => panic!("expected NonZeroExit, got {other:?}"),
    }
}

#[test]
fn catch_up_prompt_mentions_path() {
    let p = PathBuf::from("/tmp/handoff.md");
    let prompt = catch_up_prompt(&p);
    assert!(prompt.contains("/tmp/handoff.md"));
    assert!(prompt.contains("handoff"));
}

fn restore_path(prev: Option<std::ffi::OsString>) {
    unsafe {
        match prev {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }
}
