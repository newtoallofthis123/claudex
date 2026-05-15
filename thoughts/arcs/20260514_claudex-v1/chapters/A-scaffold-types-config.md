# Chapter A — Scaffold, types, config

**Type:** strict
**Depends on:** none

## Executive summary

Initialise the Rust crate, lay down the module tree from the PRD, and define the foundational types and traits every other chapter compiles against: `Agent`, `SessionRef`, `Conversation`, `Block`, `Provider`, plus config loading with env-aware default roots. Pure scaffolding — no rendering, no parsing of real transcripts, no CLI commands beyond a clap skeleton.

## Files touched

- `Cargo.toml`
- `Cargo.lock` (generated)
- `.gitignore`
- `src/main.rs`
- `src/lib.rs`
- `src/cli.rs`
- `src/session_ref.rs`
- `src/model.rs`
- `src/config.rs`
- `src/providers/mod.rs`
- `src/providers/claude.rs` (stub only)
- `src/providers/codex.rs` (stub only)
- `src/render.rs` (empty stub)
- `src/handoff_store.rs` (empty stub)
- `src/launch.rs` (empty stub)
- `src/settings.rs` (empty stub)
- `src/select.rs` (empty stub)

## Success criteria

- `cargo build` succeeds.
- `cargo test` runs unit tests for `session_ref` and `config` and they pass.
- `baton --help` prints subcommand list (`list`, `inspect`, `handoff`, `settings`); subcommand handlers may be `todo!()`.
- `SessionRef::parse` accepts `claude:<id>` and `codex:<id>`, rejects everything else with a specific error variant.
- `config::load` resolves effective Claude/Codex roots and `handoff_dir` from (1) config file, (2) `CLAUDE_CONFIG_DIR` / `CODEX_HOME` env vars, (3) home-directory fallback.

## Phases

### A.1 — Cargo project + dependencies

- **Goal:** Bootable Rust crate with the dependency set the PRD recommends.
- **Files & changes:** Run `cargo init --name baton --bin`. Create `src/lib.rs`. In `Cargo.toml`, add deps. Add a `.gitignore` excluding `/target`.
- **Code:**

  ```toml
  [package]
  name = "baton"
  version = "0.1.0"
  edition = "2021"

  [dependencies]
  clap = { version = "4", features = ["derive"] }
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  anyhow = "1"
  thiserror = "1"
  dirs = "5"
  time = { version = "0.3", features = ["formatting", "parsing", "serde-human-readable", "local-offset", "macros"] }
  toml = "0.8"

  [dev-dependencies]
  insta = { version = "1", features = ["yaml"] }
  tempfile = "3"

  [[bin]]
  name = "baton"
  path = "src/main.rs"

  [lib]
  path = "src/lib.rs"
  ```

  `src/main.rs`:

  ```rust
  fn main() -> anyhow::Result<()> {
      baton::cli::run()
  }
  ```

  `src/lib.rs`:

  ```rust
  pub mod cli;
  pub mod config;
  pub mod handoff_store;
  pub mod launch;
  pub mod model;
  pub mod providers;
  pub mod render;
  pub mod select;
  pub mod session_ref;
  pub mod settings;
  ```

### A.2 — `Agent` and `SessionRef`

- **Goal:** Typed agent enum and `<agent>:<id>` reference parser with rejection of unknown agents and malformed input.
- **Files & changes:** `src/model.rs` (define `Agent`), `src/session_ref.rs` (parse + Display + unit tests).
- **Code:**

  ```rust
  // model.rs (excerpt)
  #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum Agent { Claude, Codex }

  impl Agent {
      pub fn as_str(self) -> &'static str { match self { Agent::Claude => "claude", Agent::Codex => "codex" } }
      pub fn parse(s: &str) -> Option<Self> {
          match s { "claude" => Some(Agent::Claude), "codex" => Some(Agent::Codex), _ => None }
      }
  }
  ```

  ```rust
  // session_ref.rs
  use crate::model::Agent;

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct SessionRef { pub agent: Agent, pub id: String }

  #[derive(Debug, thiserror::Error)]
  pub enum SessionRefError {
      #[error("session reference must be `<agent>:<session_id>`")]
      Malformed,
      #[error("unknown agent `{0}` (expected `claude` or `codex`)")]
      UnknownAgent(String),
      #[error("session id cannot be empty")]
      EmptyId,
  }

  impl SessionRef {
      pub fn parse(s: &str) -> Result<Self, SessionRefError> {
          let (a, id) = s.split_once(':').ok_or(SessionRefError::Malformed)?;
          let agent = Agent::parse(a).ok_or_else(|| SessionRefError::UnknownAgent(a.to_string()))?;
          if id.is_empty() { return Err(SessionRefError::EmptyId); }
          Ok(SessionRef { agent, id: id.to_string() })
      }
  }
  ```

  Add unit tests covering: valid `claude:abc`, `codex:abc`, malformed (no colon), unknown agent (`foo:abc`), empty id (`claude:`).

### A.3 — Neutral conversation model

- **Goal:** Define `Conversation`, `Block`, and supporting structs verbatim per PRD §"Internal Conversation Model". No behavior beyond the types.
- **Files & changes:** Extend `src/model.rs`.
- **Code:**

  ```rust
  use std::path::PathBuf;
  use time::OffsetDateTime;

  #[derive(Debug, Clone)]
  pub struct Conversation {
      pub source: Agent,
      pub session_id: String,
      pub transcript_path: PathBuf,
      pub cwd: Option<PathBuf>,
      pub started_at: Option<OffsetDateTime>,
      pub blocks: Vec<Block>,
  }

  #[derive(Debug, Clone)]
  pub enum Block {
      HumanMessage(TextBlock),
      AgentMessage(TextBlock),
      ToolCall(ToolCallBlock),
      ToolResult(ToolResultBlock),
      SystemEvent(SystemEventBlock),
      UnknownEvent(UnknownEventBlock),
  }

  #[derive(Debug, Clone)]
  pub struct TextBlock { pub text: String, pub source_event_index: usize }

  #[derive(Debug, Clone)]
  pub struct ToolCallBlock { pub name: String, pub input: String, pub source_event_index: usize }

  #[derive(Debug, Clone)]
  pub struct ToolResultBlock { pub output: String, pub truncated: Option<TruncationInfo>, pub source_event_index: usize }

  #[derive(Debug, Clone)]
  pub struct TruncationInfo { pub original_chars: usize, pub shown_chars: usize }

  #[derive(Debug, Clone)]
  pub struct SystemEventBlock { pub label: String, pub detail: String, pub source_event_index: usize }

  #[derive(Debug, Clone)]
  pub struct UnknownEventBlock { pub raw_type: String, pub raw_excerpt: String, pub source_event_index: usize }

  #[derive(Debug, Clone)]
  pub struct SessionSummary {
      pub agent: Agent,
      pub id: String,
      pub path: PathBuf,
      pub cwd: Option<PathBuf>,
      pub started_at: Option<OffsetDateTime>,
      pub updated_at: Option<OffsetDateTime>,
      pub title: Option<String>,
  }

  #[derive(Debug, Clone)]
  pub struct ResolvedSession { pub agent: Agent, pub id: String, pub path: PathBuf }
  ```

### A.4 — `Provider` trait

- **Goal:** Declare the provider abstraction so chapters B/C/D/E compile against it. Stub implementations return `todo!()`.
- **Files & changes:** `src/providers/mod.rs` (trait, error type, factory `for_agent(...)` returning `Box<dyn Provider>`). `src/providers/claude.rs` and `src/providers/codex.rs` each contain a `pub struct ClaudeProvider;` / `pub struct CodexProvider;` with `impl Provider` whose method bodies are `todo!("chapter C/D")`.
- **Code:**

  ```rust
  // providers/mod.rs
  use crate::model::{Agent, Conversation, ResolvedSession, SessionSummary};
  use std::path::PathBuf;

  pub mod claude;
  pub mod codex;

  #[derive(Debug, thiserror::Error)]
  pub enum ProviderError {
      #[error("transcript root not found: {0}")] RootNotFound(PathBuf),
      #[error("session id `{0}` not found")] SessionNotFound(String),
      #[error("session id `{id}` matched multiple transcripts: {matches:?}")] AmbiguousSession { id: String, matches: Vec<PathBuf> },
      #[error("transcript unreadable at {path}: {source}")] TranscriptUnreadable { path: PathBuf, #[source] source: std::io::Error },
      #[error("invalid JSONL at {path}:{line}: {source}")] InvalidJsonl { path: PathBuf, line: usize, #[source] source: serde_json::Error },
  }

  pub trait Provider {
      fn agent(&self) -> Agent;
      fn effective_roots(&self, configured_roots: &[PathBuf]) -> Vec<PathBuf>;
      fn list_sessions(&self) -> Result<Vec<SessionSummary>, ProviderError>;
      fn resolve_session(&self, id: &str) -> Result<ResolvedSession, ProviderError>;
      fn parse_transcript(&self, session: &ResolvedSession) -> Result<Conversation, ProviderError>;
  }

  pub fn for_agent(agent: Agent, config: &crate::config::Config) -> Box<dyn Provider> {
      match agent {
          Agent::Claude => Box::new(claude::ClaudeProvider::new(config.roots.claude.clone())),
          Agent::Codex => Box::new(codex::CodexProvider::new(config.roots.codex.clone())),
      }
  }
  ```

  Stub structs hold `configured_roots: Vec<PathBuf>` and a `new(...)` constructor. Method bodies are `todo!()` except `agent()` and `effective_roots()`, which chapter A may leave as `todo!()` if it complicates the build — chapters C/D will overwrite them.

### A.5 — Config loading

- **Goal:** Implement `config::Config` with TOML deserialization, default-path discovery, env-aware effective-root resolution.
- **Files & changes:** `src/config.rs` with `Config`, `Roots`, `load()`, `default_path()`, `effective_handoff_dir()`. Unit tests using `tempfile` and overridden env vars.
- **Code:**

  ```rust
  use std::path::{Path, PathBuf};
  use serde::{Deserialize, Serialize};

  #[derive(Debug, Clone, Serialize, Deserialize, Default)]
  pub struct Config {
      #[serde(default)]
      pub handoff_dir: Option<PathBuf>,
      #[serde(default)]
      pub roots: Roots,
  }

  #[derive(Debug, Clone, Default, Serialize, Deserialize)]
  pub struct Roots {
      #[serde(default)] pub claude: Vec<PathBuf>,
      #[serde(default)] pub codex: Vec<PathBuf>,
  }

  pub fn default_path() -> PathBuf {
      dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("baton/config.toml")
  }

  pub fn load() -> anyhow::Result<Config> { load_from(&default_path()) }

  pub fn load_from(path: &Path) -> anyhow::Result<Config> {
      if !path.exists() { return Ok(Config::default()); }
      let text = std::fs::read_to_string(path)?;
      Ok(toml::from_str(&text)?)
  }

  pub fn effective_handoff_dir(cfg: &Config) -> PathBuf {
      cfg.handoff_dir.clone()
          .map(expand_tilde)
          .unwrap_or_else(|| dirs::home_dir().unwrap().join(".handoffs"))
  }

  pub fn effective_claude_roots(configured: &[PathBuf]) -> Vec<PathBuf> {
      if !configured.is_empty() { return configured.iter().map(expand_tilde).collect(); }
      if let Ok(env) = std::env::var("CLAUDE_CONFIG_DIR") { return vec![PathBuf::from(env).join("projects")]; }
      vec![dirs::home_dir().unwrap().join(".claude/projects")]
  }

  pub fn effective_codex_roots(configured: &[PathBuf]) -> Vec<PathBuf> {
      if !configured.is_empty() { return configured.iter().map(expand_tilde).collect(); }
      if let Ok(env) = std::env::var("CODEX_HOME") { return vec![PathBuf::from(env).join("sessions")]; }
      vec![dirs::home_dir().unwrap().join(".codex/sessions")]
  }

  fn expand_tilde(p: impl AsRef<Path>) -> PathBuf {
      let p = p.as_ref();
      if let Ok(stripped) = p.strip_prefix("~") {
          dirs::home_dir().unwrap().join(stripped)
      } else { p.to_path_buf() }
  }
  ```

  Tests (in `#[cfg(test)]` mod): missing config file → defaults; configured roots win over env; env wins over home fallback; `~` is expanded.

### A.6 — CLI skeleton

- **Goal:** clap subcommand surface compiling; every handler is `todo!()` so the build succeeds and `--help` shows the v1 contract.
- **Files & changes:** `src/cli.rs` with `clap::Parser` derive defining `list`, `inspect`, `handoff`, `settings` subcommands and their flags exactly as the PRD §"CLI Contract" describes.
- **Code (sketch — fill out flags per PRD):**

  ```rust
  use clap::{Parser, Subcommand};

  #[derive(Parser)]
  #[command(name = "baton", version)]
  pub struct Cli { #[command(subcommand)] pub command: Command }

  #[derive(Subcommand)]
  pub enum Command {
      List { agent: String, #[arg(long)] last: bool, #[arg(long)] interactive: bool, #[arg(short, long)] verbose: bool },
      Inspect {
          /// session reference like `claude:<id>` — omit when using --last or --interactive
          session: Option<String>,
          #[arg(long, value_name = "AGENT")] last: Option<String>,
          #[arg(long, value_name = "AGENT")] interactive: Option<String>,
          #[arg(long)] full: bool,
      },
      Handoff {
          source: Option<String>,
          target: String,
          #[arg(long, value_name = "AGENT")] last: Option<String>,
          #[arg(long, value_name = "AGENT")] interactive: Option<String>,
          #[arg(long)] no_launch: bool,
      },
      Settings { #[command(subcommand)] action: SettingsAction },
  }

  #[derive(Subcommand)]
  pub enum SettingsAction {
      Path,
      Show,
      Edit,
      Get { key: String },
      Set { key: String, value: String },
      AddRoot { agent: String, path: std::path::PathBuf },
      RemoveRoot { agent: String, path: std::path::PathBuf },
      ResetRoot { agent: String },
  }

  pub fn run() -> anyhow::Result<()> {
      let cli = Cli::parse();
      match cli.command {
          Command::List { .. } => todo!("chapter E"),
          Command::Inspect { .. } => todo!("chapter E"),
          Command::Handoff { .. } => todo!("chapter E"),
          Command::Settings { .. } => todo!("chapter E"),
      }
  }
  ```

  This is the surface only — chapter E owns the dispatch bodies.
