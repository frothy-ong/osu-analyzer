//! Config file and lazer folder detection. Never touches the lazer folder.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Manual override for the lazer data folder (the one containing files/).
    #[serde(default)]
    pub lazer_path: String,
    /// In-game player name; replays by other players are excluded.
    #[serde(default)]
    pub player_name: String,
}

pub const DEFAULT_PLAYER: &str = "fr01hy";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn config_path() -> PathBuf {
    home().join(".config/osu-analyzer/config.toml")
}

pub fn cache_db_path() -> PathBuf {
    home().join(".local/share/osu-analyzer/cache.db")
}

/// Load the config; create it with defaults if missing.
pub fn load_or_create(path: &Path) -> Result<Config, String> {
    if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
        return toml::from_str(&text).map_err(|e| format!("parse {path:?}: {e}"));
    }
    let cfg = Config {
        lazer_path: String::new(),
        player_name: DEFAULT_PLAYER.to_string(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;
    }
    let text = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(cfg)
}

/// Candidate install locations, in priority order (SPEC section 5).
pub fn default_candidates() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.join(".local/share/osu"),
        h.join(".var/app/sh.ppy.osu/data/osu"),
    ]
}

/// Override wins; otherwise the first candidate that has a files/ folder.
pub fn resolve_lazer_path(cfg: &Config, candidates: &[PathBuf]) -> Option<PathBuf> {
    if !cfg.lazer_path.trim().is_empty() {
        return Some(PathBuf::from(cfg.lazer_path.trim()));
    }
    candidates.iter().find(|p| p.join("files").is_dir()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins() {
        let cfg = Config { lazer_path: "/tmp/x".into(), player_name: "a".into() };
        let got = resolve_lazer_path(&cfg, &[PathBuf::from("/nonexistent")]);
        assert_eq!(got, Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn no_override_no_folder_is_none() {
        let cfg = Config::default();
        assert_eq!(resolve_lazer_path(&cfg, &[PathBuf::from("/nonexistent")]), None);
    }

    #[test]
    fn config_roundtrip() {
        let cfg = Config { lazer_path: "/a b".into(), player_name: "n".into() };
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(toml::from_str::<Config>(&s).unwrap(), cfg);
    }
}
