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
    /// Optional sync database: postgres://user:password@host:5432/db (add ?sslmode=require for remote servers).
    /// Empty: local only.
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
    /// Notes folder: Markdown files, the source of truth (e.g. an Obsidian vault). Mirrored with the database if any.
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
    /// Jeton GitHub tel qu'il est dans le fichier : celui de MNEMO_GITHUB_TOKEN n'y est jamais recopié.
    #[serde(skip)]
    file_token: Option<Option<String>>,
    /// Le fichier de configuration était illisible : message à montrer au lancement.
    #[serde(skip)]
    pub load_error: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // vide : Mnemo travaille en local ; une base PostgreSQL ne sert qu'à synchroniser plusieurs appareils
            database_url: String::new(),
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
            file_token: None,
            load_error: None,
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
        let file = Self::file();
        let mut cfg = match std::fs::read_to_string(&file) {
            Err(_) => Config::default(),
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(c) => c,
                Err(e) => {
                    // fichier invalide : gardé à côté, il ne sera pas écrasé par les valeurs par défaut
                    let backup = file.with_extension("toml.invalide");
                    let _ = std::fs::copy(&file, &backup);
                    eprintln!("[config] invalid config.toml: {e}");
                    Config {
                        load_error: Some(format!(
                            "config.toml est invalide ({}) : réglages par défaut utilisés, l'ancien fichier est gardé dans {}",
                            e.message(),
                            backup.display()
                        )),
                        ..Default::default()
                    }
                }
            },
        };
        if let Ok(t) = std::env::var("MNEMO_GITHUB_TOKEN")
            && !t.trim().is_empty()
        {
            cfg.file_token = Some(cfg.github_token.take());
            cfg.github_token = Some(t);
        }
        // seulement MNEMO_DATABASE_URL : un DATABASE_URL exporté pour un autre projet n'est pas pour Mnemo
        if let Ok(url) = std::env::var("MNEMO_DATABASE_URL") {
            cfg.database_url = url;
        }
        cfg
    }

    pub fn save(&self) {
        let path = Self::file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // le jeton venu de l'environnement reste dans l'environnement, sauf s'il a été changé dans les réglages
        let mut on_disk = self.clone();
        if let Some(file_token) = &self.file_token
            && std::env::var("MNEMO_GITHUB_TOKEN").ok() == self.github_token
        {
            on_disk.github_token = file_token.clone();
        }
        match toml::to_string_pretty(&on_disk) {
            Ok(s) => {
                if let Err(e) = write_private(&path, &s) {
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

/// Écrit le fichier de configuration (il contient le mot de passe de la base et le jeton GitHub) via un fichier
/// temporaire, lisible par l'utilisateur seulement.
fn write_private(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        std::io::Write::write_all(&mut f, content.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
}

/// Hides the password of a connection string for display (URL or `key=value` form).
pub fn redact_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        // les identifiants s'arrêtent au dernier « @ » avant le chemin
        let authority = rest.split(['/', '?']).next().unwrap_or(rest);
        if let Some(at) = authority.rfind('@')
            && let Some(colon) = authority[..at].find(':')
        {
            return format!("{}{}:•••{}", &url[..scheme_end + 3], &authority[..colon], &rest[at..]);
        }
        return url.to_string();
    }
    // forme « host=… user=… password=… »
    url.split_whitespace()
        .map(|kv| if kv.to_lowercase().starts_with("password=") { "password=•••" } else { kv })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_hidden() {
        assert_eq!(redact_url("postgres://moi:secret@hôte:5432/db?x=1"), "postgres://moi:•••@hôte:5432/db?x=1");
        assert_eq!(redact_url("postgres://moi@hôte/db"), "postgres://moi@hôte/db");
        assert_eq!(redact_url("host=h user=u password=secret dbname=d"), "host=h user=u password=••• dbname=d");
        // ne plante pas sur une saisie en cours
        assert_eq!(redact_url("moi@x://y"), "moi@x://y");
        assert_eq!(redact_url("postgres://a:b@"), "postgres://a:•••@");
    }
}
