use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::model::Agent;

/// Resolve the config-file path, honouring `XDG_CONFIG_HOME` when set so that
/// integration tests can sandbox configuration without touching the real
/// `dirs::config_dir()`. Falls back to `config::default_path()`.
pub fn config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("claudex/config.toml");
    }
    config::default_path()
}

/// Load the config at `path`, returning `Config::default()` when the file is
/// missing.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    config::load_from(path)
}

/// Convenience: load the config from `config_path()`.
pub fn load_default() -> anyhow::Result<Config> {
    load(&config_path())
}

/// Atomically write `cfg` as TOML to `path` (write to `<path>.tmp` + rename).
pub fn write(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn ensure_exists(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        write(path, &Config::default())?;
    }
    Ok(())
}

/// Return a list of supported keys for `get` / `set`.
pub fn supported_keys() -> &'static [&'static str] {
    &["handoff_dir", "roots.claude", "roots.codex"]
}

pub fn get_value(cfg: &Config, key: &str) -> anyhow::Result<String> {
    match key {
        "handoff_dir" => Ok(cfg
            .handoff_dir
            .as_ref()
            .map(|p| toml::Value::String(p.display().to_string()).to_string())
            .unwrap_or_default()),
        "roots.claude" => Ok(toml_array(&cfg.roots.claude)),
        "roots.codex" => Ok(toml_array(&cfg.roots.codex)),
        other => Err(anyhow::anyhow!(
            "unknown key `{other}` (supported: {})",
            supported_keys().join(", ")
        )),
    }
}

pub fn set_value(cfg: &mut Config, key: &str, value: &str) -> anyhow::Result<()> {
    match key {
        "handoff_dir" => {
            cfg.handoff_dir = Some(PathBuf::from(value));
            Ok(())
        }
        "roots.claude" => {
            cfg.roots.claude = parse_toml_path_array(value)?;
            Ok(())
        }
        "roots.codex" => {
            cfg.roots.codex = parse_toml_path_array(value)?;
            Ok(())
        }
        other => Err(anyhow::anyhow!(
            "unknown key `{other}` (supported: {})",
            supported_keys().join(", ")
        )),
    }
}

pub fn add_root(cfg: &mut Config, agent: Agent, path: PathBuf) {
    let list = roots_mut(cfg, agent);
    if !list.iter().any(|p| p == &path) {
        list.push(path);
    }
}

pub fn remove_root(cfg: &mut Config, agent: Agent, path: &Path) {
    let list = roots_mut(cfg, agent);
    list.retain(|p| p != path);
}

pub fn reset_root(cfg: &mut Config, agent: Agent) {
    *roots_mut(cfg, agent) = Vec::new();
}

fn roots_mut(cfg: &mut Config, agent: Agent) -> &mut Vec<PathBuf> {
    match agent {
        Agent::Claude => &mut cfg.roots.claude,
        Agent::Codex => &mut cfg.roots.codex,
    }
}

fn toml_array(paths: &[PathBuf]) -> String {
    let items: Vec<toml::Value> = paths
        .iter()
        .map(|p| toml::Value::String(p.display().to_string()))
        .collect();
    toml::Value::Array(items).to_string()
}

fn parse_toml_path_array(s: &str) -> anyhow::Result<Vec<PathBuf>> {
    let value: toml::Value = s
        .parse()
        .map_err(|e| anyhow::anyhow!("could not parse `{s}` as a TOML array of strings: {e}"))?;
    let arr = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("expected a TOML array, got `{s}`"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("expected an array of strings"))?;
        out.push(PathBuf::from(s));
    }
    Ok(out)
}
