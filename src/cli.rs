use std::io::Write as _;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use time::OffsetDateTime;

use crate::config::{self, Config};
use crate::handoff_store::HandoffStore;
use crate::launch::{self, Launcher, ProcessLauncher};
use crate::model::{Agent, Block, Conversation, SessionSummary};
use crate::providers;
use crate::render;
use crate::select::{self, FzfSelector, Selection};
use crate::session_ref::SessionRef;
use crate::settings;

#[derive(Parser)]
#[command(name = "claudex", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    List {
        agent: String,
        #[arg(long)]
        last: bool,
        #[arg(long)]
        interactive: bool,
        #[arg(short, long)]
        verbose: bool,
    },
    Inspect {
        /// session reference like `claude:<id>` — omit when using --last or --interactive
        session: Option<String>,
        #[arg(long, value_name = "AGENT")]
        last: Option<String>,
        #[arg(long, value_name = "AGENT")]
        interactive: Option<String>,
        #[arg(long)]
        full: bool,
    },
    Handoff {
        /// either `<source-ref>` (e.g. `claude:abc`) followed by `<target>`,
        /// or just `<target>` when using `--last`/`--interactive`.
        first: String,
        second: Option<String>,
        #[arg(long, value_name = "AGENT")]
        last: Option<String>,
        #[arg(long, value_name = "AGENT")]
        interactive: Option<String>,
        #[arg(long)]
        no_launch: bool,
    },
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
}

#[derive(Subcommand)]
pub enum SettingsAction {
    Path,
    Show,
    Edit,
    Get {
        key: String,
    },
    Set {
        key: String,
        value: String,
    },
    AddRoot {
        agent: String,
        path: std::path::PathBuf,
    },
    RemoveRoot {
        agent: String,
        path: std::path::PathBuf,
    },
    ResetRoot {
        agent: String,
    },
}

const INSPECT_PREVIEW_LINES: usize = 80;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ExitError {
    code: i32,
    message: String,
}

impl ExitError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> i32 {
        self.code
    }
}

pub fn exit_code(error: &anyhow::Error) -> i32 {
    error
        .downcast_ref::<ExitError>()
        .map(ExitError::code)
        .unwrap_or(1)
}

pub fn run_to_exit_code() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    match cli.command {
        Command::List {
            agent,
            last,
            interactive,
            verbose,
        } => cmd_list(&agent, last, interactive, verbose).map(|_| 0),
        Command::Inspect {
            session,
            last,
            interactive,
            full,
        } => cmd_inspect(session, last, interactive, full).map(|_| 0),
        Command::Handoff {
            first,
            second,
            last,
            interactive,
            no_launch,
        } => {
            // When `--last` or `--interactive` is used, the only positional is
            // the target agent. Otherwise we expect `<source-ref> <target>`.
            let (source, target) = match second {
                Some(t) => (Some(first), t),
                None => (None, first),
            };
            cmd_handoff(source, &target, last, interactive, no_launch)
        }
        Command::Settings { action } => cmd_settings(action).map(|_| 0),
    }
}

fn parse_agent(s: &str) -> anyhow::Result<Agent> {
    Agent::parse(s)
        .ok_or_else(|| anyhow::anyhow!("unknown agent `{s}` (expected `claude` or `codex`)"))
}

fn cmd_list(agent_str: &str, last: bool, interactive: bool, verbose: bool) -> anyhow::Result<()> {
    if last && interactive {
        return Err(anyhow::anyhow!(
            "`--last` and `--interactive` are mutually exclusive"
        ));
    }
    let agent = parse_agent(agent_str)?;
    let cfg = settings::load_default().context("could not load settings")?;
    let provider = providers::for_agent(agent, &cfg);
    let sessions = provider
        .list_sessions()
        .with_context(|| format!("could not list {} sessions", agent.as_str()))?;

    if interactive {
        if sessions.is_empty() {
            return Err(anyhow::anyhow!("no sessions available"));
        }
        let rows: Vec<String> = sessions.iter().map(select::format_row).collect();
        let selector = FzfSelector;
        let idx = <FzfSelector as select::Selector>::pick(&selector, &rows)?
            .ok_or_else(|| anyhow::anyhow!("no selection made"))?;
        let chosen = &sessions[idx];
        println!("{}:{}", agent.as_str(), chosen.id);
        return Ok(());
    }

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if last {
        if let Some(s) = sessions.first() {
            writeln!(out, "{}", format_list_row(s, verbose))?;
        }
        return Ok(());
    }
    for s in &sessions {
        writeln!(out, "{}", format_list_row(s, verbose))?;
    }
    Ok(())
}

fn format_list_row(s: &SessionSummary, verbose: bool) -> String {
    let base = select::format_row(s);
    if verbose {
        format!("{}\t{}", base, s.path.display())
    } else {
        base
    }
}

fn cmd_inspect(
    session: Option<String>,
    last: Option<String>,
    interactive: Option<String>,
    full: bool,
) -> anyhow::Result<()> {
    let cfg = settings::load_default().context("could not load settings")?;
    let (agent, summary) = resolve_selection(
        session.as_deref(),
        last.as_deref(),
        interactive.as_deref(),
        &cfg,
    )?;
    let provider = providers::for_agent(agent, &cfg);
    let resolved = provider.resolve_session(&summary.id).with_context(|| {
        format!(
            "could not resolve {} session `{}`",
            agent.as_str(),
            summary.id
        )
    })?;
    let conv = provider.parse_transcript(&resolved).with_context(|| {
        format!(
            "could not parse {} transcript `{}`",
            agent.as_str(),
            resolved.path.display()
        )
    })?;

    print_inspect_header(&conv);

    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let rendered = render::render(&conv, agent_swap(agent), now);

    if full {
        print!("{rendered}");
        if !rendered.ends_with('\n') {
            println!();
        }
        return Ok(());
    }

    let lines: Vec<&str> = rendered.lines().collect();
    let preview_len = lines.len().min(INSPECT_PREVIEW_LINES);
    println!("--- preview ---");
    for line in &lines[..preview_len] {
        println!("{line}");
    }
    if lines.len() > preview_len {
        let hidden = lines.len() - preview_len;
        println!("--- end preview ({hidden} more lines hidden, use --full)");
    } else {
        println!("--- end preview ---");
    }
    Ok(())
}

/// Pick a "target" agent for the render header during inspect. The handoff
/// target is unknown at inspect time, so we render as if going to the
/// opposite agent — this is a preview only and never written to disk.
fn agent_swap(a: Agent) -> Agent {
    match a {
        Agent::Claude => Agent::Codex,
        Agent::Codex => Agent::Claude,
    }
}

fn print_inspect_header(conv: &Conversation) {
    let mut human = 0usize;
    let mut agent_msg = 0usize;
    let mut tool_calls = 0usize;
    let mut tool_results = 0usize;
    let mut system = 0usize;
    let mut unknown = 0usize;
    for b in &conv.blocks {
        match b {
            Block::HumanMessage(_) => human += 1,
            Block::AgentMessage(_) => agent_msg += 1,
            Block::ToolCall(_) => tool_calls += 1,
            Block::ToolResult(_) => tool_results += 1,
            Block::SystemEvent(_) => system += 1,
            Block::UnknownEvent(_) => unknown += 1,
        }
    }
    println!("source: {}", conv.source.as_str());
    println!("session_id: {}", conv.session_id);
    println!("transcript: {}", conv.transcript_path.display());
    let cwd = conv
        .cwd
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".to_string());
    println!("cwd: {cwd}");
    println!(
        "blocks: human={human}, agent={agent_msg}, tool_calls={tool_calls}, tool_results={tool_results}, system={system}, unknown={unknown}"
    );
}

fn resolve_selection(
    explicit: Option<&str>,
    last_agent: Option<&str>,
    interactive_agent: Option<&str>,
    cfg: &Config,
) -> anyhow::Result<(Agent, SessionSummary)> {
    let modes = [
        explicit.is_some(),
        last_agent.is_some(),
        interactive_agent.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if modes > 1 {
        return Err(anyhow::anyhow!(
            "specify at most one of <session-ref>, --last <agent>, or --interactive <agent>"
        ));
    }
    let interactive_agent = if modes == 0 {
        Some("claude")
    } else {
        interactive_agent
    };

    if let Some(s) = explicit {
        let r = SessionRef::parse(s)?;
        let provider = providers::for_agent(r.agent, cfg);
        let sessions = provider
            .list_sessions()
            .with_context(|| format!("could not list {} sessions", r.agent.as_str()))?;
        let summary = sessions
            .into_iter()
            .find(|x| x.id == r.id)
            .ok_or_else(|| anyhow::anyhow!("session id `{}` not found", r.id))?;
        return Ok((r.agent, summary));
    }
    if let Some(a) = last_agent {
        let agent = parse_agent(a)?;
        let provider = providers::for_agent(agent, cfg);
        let s = select::resolve(provider.as_ref(), Selection::Last, &FzfSelector)?;
        return Ok((agent, s));
    }
    let a = interactive_agent.unwrap();
    let agent = parse_agent(a)?;
    let provider = providers::for_agent(agent, cfg);
    let s = select::resolve(provider.as_ref(), Selection::Interactive, &FzfSelector)?;
    Ok((agent, s))
}

fn cmd_handoff(
    source: Option<String>,
    target_str: &str,
    last: Option<String>,
    interactive: Option<String>,
    no_launch: bool,
) -> anyhow::Result<i32> {
    let target = parse_agent(target_str)?;
    let cfg = settings::load_default().context("could not load settings")?;
    let (source_agent, summary) = resolve_selection(
        source.as_deref(),
        last.as_deref(),
        interactive.as_deref(),
        &cfg,
    )?;

    if source_agent == target {
        return Err(anyhow::anyhow!(
            "source and target cannot both be `{}`",
            target.as_str()
        ));
    }

    let provider = providers::for_agent(source_agent, &cfg);
    let resolved = provider.resolve_session(&summary.id).with_context(|| {
        format!(
            "could not resolve {} session `{}`",
            source_agent.as_str(),
            summary.id
        )
    })?;
    let conv = provider.parse_transcript(&resolved).with_context(|| {
        format!(
            "could not parse {} transcript `{}`",
            source_agent.as_str(),
            resolved.path.display()
        )
    })?;

    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let rendered = render::render(&conv, target, now);

    let store = HandoffStore::new(config::effective_handoff_dir(&cfg));
    let path = store.write(source_agent, target, &conv.session_id, now, &rendered)?;
    println!("wrote: {}", path.display());

    if no_launch {
        return Ok(0);
    }

    let prompt = launch::catch_up_prompt(&path);
    let launcher = ProcessLauncher;
    match launcher.launch(target, &prompt) {
        Ok(_) => Ok(0),
        Err(e) => Err(ExitError::new(2, format!("launch failed: {e}")).into()),
    }
}

fn cmd_settings(action: SettingsAction) -> anyhow::Result<()> {
    let path = settings::config_path();
    match action {
        SettingsAction::Path => {
            println!("{}", path.display());
            Ok(())
        }
        SettingsAction::Show => {
            let cfg = settings::load(&path)?;
            println!("# config: {}", path.display());
            print!("{}", toml::to_string_pretty(&cfg)?);
            println!();
            println!("# effective");
            println!(
                "handoff_dir = {}",
                config::effective_handoff_dir(&cfg).display()
            );
            let claude_roots = config::effective_claude_roots(&cfg.roots.claude);
            println!("roots.claude = [");
            for r in &claude_roots {
                println!("  {},", r.display());
            }
            println!("]");
            let codex_roots = config::effective_codex_roots(&cfg.roots.codex);
            println!("roots.codex = [");
            for r in &codex_roots {
                println!("  {},", r.display());
            }
            println!("]");
            Ok(())
        }
        SettingsAction::Edit => {
            settings::ensure_exists(&path)?;
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let status = std::process::Command::new(&editor)
                .arg(&path)
                .status()
                .with_context(|| format!("could not launch editor `{editor}`"))?;
            if !status.success() {
                return Err(anyhow::anyhow!("editor `{editor}` exited with {status}"));
            }
            Ok(())
        }
        SettingsAction::Get { key } => {
            let cfg = settings::load(&path)?;
            println!("{}", settings::get_value(&cfg, &key)?);
            Ok(())
        }
        SettingsAction::Set { key, value } => {
            let mut cfg = settings::load(&path)?;
            settings::set_value(&mut cfg, &key, &value)?;
            settings::write(&path, &cfg)?;
            println!("ok");
            Ok(())
        }
        SettingsAction::AddRoot { agent, path: root } => {
            let agent = parse_agent(&agent)?;
            let mut cfg = settings::load(&path)?;
            settings::add_root(&mut cfg, agent, root);
            settings::write(&path, &cfg)?;
            println!("ok");
            Ok(())
        }
        SettingsAction::RemoveRoot { agent, path: root } => {
            let agent = parse_agent(&agent)?;
            let mut cfg = settings::load(&path)?;
            settings::remove_root(&mut cfg, agent, &root);
            settings::write(&path, &cfg)?;
            println!("ok");
            Ok(())
        }
        SettingsAction::ResetRoot { agent } => {
            let agent = parse_agent(&agent)?;
            let mut cfg = settings::load(&path)?;
            settings::reset_root(&mut cfg, agent);
            settings::write(&path, &cfg)?;
            println!("ok");
            Ok(())
        }
    }
}
