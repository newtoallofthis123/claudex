use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use anyhow::Context as _;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub handoff_dir: Option<PathBuf>,
    #[serde(default)]
    pub roots: Roots,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Roots {
    #[serde(default)]
    pub claude: Vec<PathBuf>,
    #[serde(default)]
    pub codex: Vec<PathBuf>,
}

pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("claudex/config.toml")
}

pub fn load() -> anyhow::Result<Config> {
    load_from(&default_path())
}

pub fn load_from(path: &Path) -> anyhow::Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read config `{}`", path.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("could not parse config `{}` as TOML", path.display()))
}

pub fn effective_handoff_dir(cfg: &Config) -> PathBuf {
    cfg.handoff_dir
        .clone()
        .map(expand_tilde)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".handoffs"))
}

pub fn effective_claude_roots(configured: &[PathBuf]) -> Vec<PathBuf> {
    if !configured.is_empty() {
        return configured.iter().map(expand_tilde).collect();
    }
    if let Ok(env) = std::env::var("CLAUDE_CONFIG_DIR") {
        return vec![PathBuf::from(env).join("projects")];
    }
    vec![dirs::home_dir().unwrap().join(".claude/projects")]
}

pub fn effective_codex_roots(configured: &[PathBuf]) -> Vec<PathBuf> {
    if !configured.is_empty() {
        return configured.iter().map(expand_tilde).collect();
    }
    if let Ok(env) = std::env::var("CODEX_HOME") {
        return vec![PathBuf::from(env).join("sessions")];
    }
    vec![dirs::home_dir().unwrap().join(".codex/sessions")]
}

fn expand_tilde(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    if let Ok(stripped) = p.strip_prefix("~") {
        dirs::home_dir().unwrap().join(stripped)
    } else {
        p.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn missing_config_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        let cfg = load_from(&path).unwrap();
        assert!(cfg.handoff_dir.is_none());
        assert!(cfg.roots.claude.is_empty());
        assert!(cfg.roots.codex.is_empty());
    }

    #[test]
    fn configured_roots_win_over_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("CLAUDE_CONFIG_DIR");
        unsafe {
            std::env::set_var("CLAUDE_CONFIG_DIR", "/from/env");
        }
        let configured = vec![PathBuf::from("/from/config")];
        let roots = effective_claude_roots(&configured);
        assert_eq!(roots, vec![PathBuf::from("/from/config")]);
        restore_env("CLAUDE_CONFIG_DIR", old);
    }

    #[test]
    fn env_wins_over_home_fallback_claude() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("CLAUDE_CONFIG_DIR");
        unsafe {
            std::env::set_var("CLAUDE_CONFIG_DIR", "/from/env/claude");
        }
        let roots = effective_claude_roots(&[]);
        assert_eq!(roots, vec![PathBuf::from("/from/env/claude/projects")]);
        restore_env("CLAUDE_CONFIG_DIR", old);
    }

    #[test]
    fn env_wins_over_home_fallback_codex() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", "/from/env/codex");
        }
        let roots = effective_codex_roots(&[]);
        assert_eq!(roots, vec![PathBuf::from("/from/env/codex/sessions")]);
        restore_env("CODEX_HOME", old);
    }

    #[test]
    fn tilde_is_expanded() {
        let home = dirs::home_dir().unwrap();
        let configured = vec![PathBuf::from("~/custom/claude")];
        let roots = effective_claude_roots(&configured);
        assert_eq!(roots, vec![home.join("custom/claude")]);
    }

    #[test]
    fn loads_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "handoff_dir = \"/tmp/handoffs\"\n[roots]\nclaude = [\"/a\"]\ncodex = [\"/b\"]\n"
        )
        .unwrap();
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.handoff_dir, Some(PathBuf::from("/tmp/handoffs")));
        assert_eq!(cfg.roots.claude, vec![PathBuf::from("/a")]);
        assert_eq!(cfg.roots.codex, vec![PathBuf::from("/b")]);
    }

    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        unsafe {
            match old {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
