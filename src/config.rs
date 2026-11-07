use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Edit,
    Split,
    Preview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// postgres://user:password@host:5432/db  (add ?sslmode=require for remote/managed servers)
    pub database_url: String,
    pub dark_mode: bool,
    pub font_size: f32,
    pub view_mode: ViewMode,
    pub last_note: Option<String>,
    pub show_left: bool,
    pub show_right: bool,
    /// Override of the plugins folder (defaults to <config dir>/plugins)
    pub plugins_dir: Option<PathBuf>,
    /// `.obsidian` folder of an existing vault, to reuse its themes / snippets / appearance.json
    pub obsidian_dir: Option<PathBuf>,
    /// Vault folder on disk mirrored with the database (Obsidian vault): imported, then kept in sync both ways
    pub vault_dir: Option<PathBuf>,
    /// Follow that vault's appearance.json (theme, snippets, font size, accent, light/dark)
    pub follow_appearance: bool,
    /// Selected CSS theme (by name) when not following appearance.json
    pub css_theme: Option<String>,
    /// Enabled CSS snippets (by name) when not following appearance.json
    pub css_snippets: Vec<String>,
    /// GitHub personal access token (MNEMO_GITHUB_TOKEN has priority). Stored locally only.
    pub github_token: Option<String>,
    /// API root: https://api.github.com or https://<host>/api/v3 for GitHub Enterprise
    pub github_api: String,
    /// Synchronise projects linked to a repository every 5 minutes while the dashboard is open
    pub github_auto_sync: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "postgres://postgres:postgres@localhost:5432/vault".into(),
            dark_mode: true,
            font_size: 16.0,
            view_mode: ViewMode::Edit,
            last_note: None,
            show_left: true,
            show_right: true,
            plugins_dir: None,
            obsidian_dir: None,
            vault_dir: None,
            follow_appearance: true,
            css_theme: None,
            css_snippets: vec![],
            github_token: None,
            github_api: "https://api.github.com".into(),
            github_auto_sync: true,
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MNEMO_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("dev", "mnemo", "Mnemo")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".mnemo"))
}

impl Config {
    fn file() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> Self {
        let mut cfg: Config = std::fs::read_to_string(Self::file())
            .ok()
            .and_then(|s| toml::from_str(&s).map_err(|e| eprintln!("[config] invalid config.toml: {e}")).ok())
            .unwrap_or_default();
        if let Ok(t) = std::env::var("MNEMO_GITHUB_TOKEN")
            && !t.trim().is_empty()
        {
            cfg.github_token = Some(t);
        }
        if let Ok(url) = std::env::var("MNEMO_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL")) {
            cfg.database_url = url;
        }
        cfg
    }

    pub fn save(&self) {
        let path = Self::file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match toml::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&path, s) {
                    eprintln!("[config] cannot write {}: {e}", path.display());
                }
            }
            Err(e) => eprintln!("[config] serialize error: {e}"),
        }
    }

    pub fn plugins_dir(&self) -> PathBuf {
        if let Ok(dir) = std::env::var("MNEMO_PLUGINS_DIR") {
            return PathBuf::from(dir);
        }
        self.plugins_dir.clone().unwrap_or_else(|| config_dir().join("plugins"))
    }
}

/// Hides the password of a connection string for display.
pub fn redact_url(url: &str) -> String {
    if let Some(at) = url.rfind('@')
        && let Some(scheme_end) = url.find("://")
    {
        let creds = &url[scheme_end + 3..at];
        if let Some(colon) = creds.find(':') {
            return format!("{}{}:•••{}", &url[..scheme_end + 3], &creds[..colon], &url[at..]);
        }
    }
    url.to_string()
}
