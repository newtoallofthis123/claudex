# Chapter E — CLI commands, selection, launcher, integration tests

**Type:** conscious
**Depends on:** A, B, C, D

## Executive summary

Wire everything together. Implement command bodies for `list`, `inspect`, `handoff`, and `settings`; implement `--last` / `--interactive` selection (with an injectable `Selector` trait so tests can avoid running `fzf`); implement the target launcher; and back the whole surface with fixture-based integration tests, including `handoff --no-launch` for end-to-end verification without spawning real agents. This chapter is conscious because the command shapes have many small decisions (output format, exit codes, error wording) that need local judgement.

## Files touched

- `src/cli.rs` (replace `todo!()` bodies)
- `src/select.rs`
- `src/launch.rs`
- `src/settings.rs`
- `src/handoff_store.rs` (only if a small caller-facing helper needs to be added — avoid otherwise)
- `tests/cli_integration.rs`
- `tests/settings.rs`
- `tests/launcher.rs`

## Success criteria

- `baton list claude` and `baton list codex` print one row per session: `id  started_at  cwd  title` (tab-separated). `--verbose` adds `path`. Order: newest first. `--last` prints only the top row.
- `baton list <agent> --interactive` pipes rows to `fzf` and prints the selected `<agent>:<id>` reference. With no `fzf` on PATH, fails with a clear error.
- `baton inspect <ref>` prints metadata header (source, session_id, transcript path, cwd, message/tool block counts) and a preview (default: first 80 lines of the rendered handoff). `--full` prints the entire rendered handoff. `--last <agent>` and `--interactive <agent>` re-use the selection layer.
- `baton handoff <source-ref> <target-agent>` parses the ref, resolves it, parses the transcript, renders the handoff, writes it under the configured handoff dir, prints the final path, and then launches the target agent. With `--no-launch`, skip the launch step and exit 0. Refusing to handoff when source agent == target agent (clear error).
- Launch failure (missing executable, spawn failure, or non-zero target exit) prints the handoff path, prints a clear error, and exits non-zero. The written file is left in place.
- `baton settings show` prints the parsed TOML config plus the resolved effective values for handoff dir and per-agent roots. `path` prints the config file path. `edit` opens `$EDITOR` on the config file (creating it if missing). `get <key>`, `set <key> <value>`, `add-root`, `remove-root`, and `reset-root` each operate on the TOML config through a load-modify-write round-trip and print the new effective config diff (or just "ok").
- Integration tests cover the golden paths against fixture transcript roots.

## Phases

### E.1 — Selection layer

- **Goal:** `select::resolve(...)` picks one `SessionSummary` from a list using one of: explicit id, `--last`, or interactive (`fzf`). Tests inject a stub selector.
- **Files & changes:** `src/select.rs`.
- **Code:**

  ```rust
  pub trait Selector { fn pick(&self, rows: &[String]) -> anyhow::Result<Option<usize>>; }

  pub struct FzfSelector;
  impl Selector for FzfSelector { /* spawn fzf, pipe rows on stdin, parse selected line back to its index */ }

  pub enum Selection { Explicit(String), Last, Interactive }

  pub fn resolve(provider: &dyn Provider, selection: Selection, selector: &dyn Selector) -> anyhow::Result<SessionSummary> {
      let mut sessions = provider.list_sessions()?;
      match selection {
          Selection::Explicit(id) => sessions.into_iter().find(|s| s.id == id)
              .ok_or_else(|| anyhow::anyhow!("session id `{id}` not found")),
          Selection::Last => sessions.into_iter().next().ok_or_else(|| anyhow::anyhow!("no sessions available")),
          Selection::Interactive => {
              let rows: Vec<String> = sessions.iter().map(format_row).collect();
              let idx = selector.pick(&rows)?.ok_or_else(|| anyhow::anyhow!("no selection made"))?;
              Ok(sessions.into_iter().nth(idx).unwrap())
          }
      }
  }
  ```

  `FzfSelector`: detect `fzf` via `which::which` or by spawning and catching `ErrorKind::NotFound`. On missing `fzf`, return `anyhow::anyhow!("fzf not found on PATH — try `--last` or pass an explicit session id")`.

### E.2 — `list` command

- **Goal:** Implement the list dispatch.
- **Files & changes:** `cli.rs`.
- **Implementation:**
  - Parse `agent` arg via `Agent::parse`; reject unknowns with a clear error.
  - Construct provider with `providers::for_agent(agent, &config)`.
  - Call `list_sessions()`. If `--last`, take the first; print one row. If `--interactive`, pass to `FzfSelector` via `select::resolve(..., Selection::Interactive, ...)` and print `<agent>:<id>` of the chosen row. Otherwise print all rows.
  - Row format: `id\t<started_at or ->\t<cwd or ->\t<title or ->`. `--verbose` appends `\t<path>`.
  - Empty list: print nothing and exit 0.

### E.3 — `inspect` command

- **Goal:** Show what would be handed off, without writing or launching.
- **Files & changes:** `cli.rs`.
- **Implementation:**
  - Determine the selection (explicit `<agent>:<id>` arg → `Selection::Explicit`; `--last <agent>` → `Selection::Last`; `--interactive <agent>` → `Selection::Interactive`). Exactly one must be specified — otherwise clap-level error.
  - Resolve to a `SessionSummary` (use the provider's `resolve_session` path for explicit refs and `select::resolve` for `--last`/`--interactive`).
  - Provider `parse_transcript` → `Conversation`.
  - Print metadata header:

    ```text
    source: claude
    session_id: <id>
    transcript: <path>
    cwd: <path or ->
    blocks: <human=…, agent=…, tool_calls=…, tool_results=…, system=…, unknown=…>
    ```

  - Render the handoff with `render::render(...)`. If `--full`, print the whole rendered text after the header. Otherwise: print `--- preview ---`, then the first 80 lines, then `--- end preview (…N more lines hidden, use --full)` if there are more lines.
  - No file is written. No launch.

### E.4 — `handoff` command

- **Goal:** Generate the handoff Markdown and start the target.
- **Files & changes:** `cli.rs`, plus call into `launch.rs`.
- **Implementation order:**
  1. Resolve selection → `SessionSummary` for the source agent.
  2. Parse target via `Agent::parse`. If `source_agent == target`, error: `"source and target cannot both be `<agent>`"`.
  3. Provider `parse_transcript` → `Conversation`.
  4. `render::render(conv, target, now)` where `now = OffsetDateTime::now_local().unwrap_or_else(|_| now_utc())`.
  5. `HandoffStore::new(effective_handoff_dir).write(...)` → `path`.
  6. Print `wrote: <path>` to stdout.
  7. If `--no-launch`, return Ok(()).
  8. Otherwise build the catch-up prompt from `launch::catch_up_prompt(&path)` and call `Launcher::launch(target, &prompt)`. On launch error or non-zero target exit, print `launch failed: <err>` to stderr and exit code 2. The handoff file remains.

### E.5 — `launch.rs`

- **Goal:** Tiny launcher abstraction + default `ProcessLauncher` that spawns `claude` / `codex` with the prompt as the positional argument and inherits stdio.
- **Files & changes:** `src/launch.rs`.
- **Code:**

  ```rust
  pub struct LaunchResult { pub status: std::process::ExitStatus }

  #[derive(Debug, thiserror::Error)]
  pub enum LaunchError {
      #[error("executable for `{0}` not found on PATH")]
      ExecutableNotFound(String),
      #[error("launching `{cmd}` failed: {source}")]
      Spawn { cmd: String, #[source] source: std::io::Error },
      #[error("`{cmd}` exited with status {status}")]
      NonZeroExit { cmd: String, status: std::process::ExitStatus },
  }

  pub trait Launcher { fn launch(&self, target: Agent, prompt: &str) -> Result<LaunchResult, LaunchError>; }

  pub struct ProcessLauncher;

  impl Launcher for ProcessLauncher {
      fn launch(&self, target: Agent, prompt: &str) -> Result<LaunchResult, LaunchError> {
          let cmd = target.as_str();
          let mut child = std::process::Command::new(cmd)
              .arg(prompt)
              .stdin(std::process::Stdio::inherit())
              .stdout(std::process::Stdio::inherit())
              .stderr(std::process::Stdio::inherit())
              .spawn()
              .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound {
                  LaunchError::ExecutableNotFound(cmd.to_string())
              } else {
                  LaunchError::Spawn { cmd: cmd.to_string(), source: e }
              })?;
          let status = child.wait().map_err(|e| LaunchError::Spawn { cmd: cmd.to_string(), source: e })?;
          if !status.success() {
              return Err(LaunchError::NonZeroExit { cmd: cmd.to_string(), status });
          }
          Ok(LaunchResult { status })
      }
  }

  pub fn catch_up_prompt(handoff_path: &std::path::Path) -> String {
      format!(
          "You are catching up from a previous Claude Code/Codex conversation.\n\n\
           Read this handoff file:\n{}\n\n\
           Use it as context and continue naturally. Tool output may be truncated; inspect the workspace directly when exact details matter.",
          handoff_path.display()
      )
  }
  ```

### E.6 — `settings` command

- **Goal:** Boring, scriptable mutations to `~/.config/baton/config.toml`.
- **Files & changes:** `src/settings.rs` + dispatch in `cli.rs`.
- **Implementation:**
  - `path` → print `config::default_path()`.
  - `show` → load the raw TOML `Config`, then compute effective handoff dir + effective roots per agent, print both sections in plain text.
  - `edit` → ensure the file exists (create with `Config::default()` serialized to TOML if missing), then `Command::new(std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())).arg(path).status()`.
  - `get <key>` → load config and print the supported key's TOML value. Supported keys in v1: `handoff_dir`, `roots.claude`, `roots.codex`.
  - `set handoff_dir <path>` → load, mutate `cfg.handoff_dir = Some(<path>)`, write TOML.
  - `set roots.claude <toml-array>` / `set roots.codex <toml-array>` → load, parse the value as a TOML array of strings, replace that root list, write TOML. Keep `add-root` / `remove-root` for single-path edits.
  - `add-root <agent> <path>` → load, push into `cfg.roots.<agent>` if not already present, write.
  - `remove-root <agent> <path>` → load, remove matching entry, write.
  - `reset-root <agent>` → load, set `cfg.roots.<agent> = vec![]`, write.
  - Write: serialize with `toml::to_string_pretty`, atomic write (write to `<path>.tmp` + `rename`). Create parent dir if missing.
  - Unknown `get <key>` / `set <key>` values → error listing the supported keys.

### E.7 — Integration tests

- **Goal:** Drive the binary end to end with fixture roots.
- **Files & changes:** `tests/cli_integration.rs`, `tests/settings.rs`, `tests/launcher.rs`. Use `assert_cmd` (add as a dev-dep) or invoke `baton::cli::run_with(args, env)` if we expose a test entry. Recommend `assert_cmd` for clarity.
  - Add `assert_cmd = "2"` and `predicates = "3"` to `dev-dependencies`.
- **Cases:**
  1. `list_claude_lists_fixtures` — arranges `XDG_CONFIG_HOME` to point at a temp config that names the fixture root. Runs `baton list claude` and asserts the fixture session ids appear, newest first.
  2. `list_last` — `baton list claude --last` prints exactly one row, the newest.
  3. `inspect_preview` — `baton inspect claude:fixture-1` prints metadata + preview; not the full body.
  4. `inspect_full` — same but `--full` includes a known later line.
  5. `handoff_no_launch_writes_file` — `baton handoff claude:fixture-1 codex --no-launch` writes a file matching the naming pattern under the configured handoff dir; the file content starts with `source: claude\ntarget: codex\n`.
  6. `handoff_same_agent_rejected` — `baton handoff claude:fixture-1 claude` exits non-zero with a clear message.
  7. `settings_roundtrip` — `baton settings set handoff_dir /tmp/handoffs`, `settings get handoff_dir`, `settings add-root claude /tmp/foo`, then `settings show` lists `/tmp/foo`; `reset-root claude` removes it.
  8. `launcher_missing_exec` — call `ProcessLauncher.launch(Agent::Claude, "...")` with `PATH=` set to an empty temp dir; assert `LaunchError::ExecutableNotFound`.
  9. `launcher_nonzero_exit` — point `ProcessLauncher` at a test executable or shim that exits non-zero; assert `LaunchError::NonZeroExit`.
- **Config injection:** Do not add a public `--config` flag for v1. Tests should isolate configuration through `XDG_CONFIG_HOME` and normal TOML files.
