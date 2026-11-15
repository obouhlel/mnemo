use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Edit,
    Split,
    Preview,
}

/// Mode clair / sombre choisi dans les paramètres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
    /// suit le réglage du système
    System,
}

/// Volet d'un panneau latéral. Chaque volet peut être glissé d'un panneau à l'autre, comme dans Obsidian.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    Files,
    Search,
    Tags,
    Bookmarks,
    Backlinks,
    Tickets,
    /// panneau d'un plugin (par son nom)
    Plugin(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Optional sync database: postgres://user:password@host:5432/db (add ?sslmode=require for remote servers).
    /// Empty: local only.
    pub database_url: String,
    /// Mode effectivement appliqué (déduit de `theme_mode`, du système ou du coffre Obsidian)
    pub dark_mode: bool,
    /// Clair / Sombre / Système ; absent des anciennes configurations : déduit de `dark_mode`
    pub theme_mode: Option<ThemeMode>,
    pub font_size: f32,
    /// Couleur d'accent choisie (#rrggbb) quand l'apparence du coffre n'est pas utilisée ; sinon celle du thème
    pub accent_color: Option<String>,
    /// Polices choisies (familles installées sur le système) ; `None` : celles du thème, sinon de Mnemo
    pub text_font: Option<String>,
    pub interface_font: Option<String>,
    pub mono_font: Option<String>,
    pub view_mode: ViewMode,
    pub last_note: Option<String>,
    pub show_left: bool,
    pub show_right: bool,
    /// volets du panneau de gauche et de droite, dans l'ordre des onglets
    pub left_panes: Vec<Pane>,
    pub right_panes: Vec<Pane>,
    /// graphe local (liens de la note ouverte) en bas du panneau de gauche, et sa hauteur
    pub local_graph: bool,
    pub local_graph_height: f32,
    /// Override of the plugins folder (defaults to <config dir>/plugins)
    pub plugins_dir: Option<PathBuf>,
    /// `.obsidian` folder of an existing vault, to reuse its themes / snippets / appearance.json
    pub obsidian_dir: Option<PathBuf>,
    /// Notes folder: Markdown files, the source of truth (e.g. an Obsidian vault). Mirrored with the database if any.
    pub vault_dir: Option<PathBuf>,
    /// « Utiliser l'apparence du coffre Obsidian » : son appearance.json (thème, snippets, taille, accent,
    /// clair / sombre, polices) et son app.json
    pub follow_appearance: bool,
    /// Ruban d'icônes à gauche (tableau de bord, graphe, whiteboard…), affiché même si le coffre Obsidian le
    /// masque : ses boutons n'ont pas d'autre place dans Mnemo
    pub show_ribbon: bool,
    /// vitesse de la lecture à voix haute (1.0 = normale)
    pub tts_rate: f32,
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
            theme_mode: None,
            font_size: 16.0,
            accent_color: None,
            text_font: None,
            interface_font: None,
            mono_font: None,
            view_mode: ViewMode::Edit,
            last_note: None,
            show_left: true,
            show_right: true,
            left_panes: vec![Pane::Files, Pane::Search, Pane::Tags, Pane::Bookmarks],
            right_panes: vec![Pane::Backlinks, Pane::Tickets],
            local_graph: true,
            local_graph_height: 240.0,
            plugins_dir: None,
            obsidian_dir: None,
            vault_dir: None,
            follow_appearance: true,
            show_ribbon: true,
            tts_rate: 1.0,
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

    /// Mode choisi ; les anciennes configurations n'ont que `dark_mode`.
    pub fn mode(&self) -> ThemeMode {
        self.theme_mode.unwrap_or(if self.dark_mode { ThemeMode::Dark } else { ThemeMode::Light })
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

/// Chemin lisible et court pour l'interface, `max` caractères au plus (le chemin complet va dans l'infobulle).
pub fn display_path(path: &Path, max: usize) -> String {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    short_path(path, home.as_deref(), max)
}

/// Remplace le dossier personnel par `~`, puis, si le chemin dépasse `max` caractères, ne garde que
/// la fin : `…/Obsidian/.obsidian`. Le dernier élément est toujours gardé en entier.
pub fn short_path(path: &Path, home: Option<&Path>, max: usize) -> String {
    let sep = std::path::MAIN_SEPARATOR_STR;
    let full = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~{sep}{}", rest.display()),
        None => path.display().to_string(),
    };
    if full.chars().count() <= max {
        return full;
    }
    let parts: Vec<String> = path
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let mut tail = String::new();
    for part in parts.iter().rev() {
        let next = if tail.is_empty() { part.clone() } else { format!("{part}{sep}{tail}") };
        // « …/ » compte pour deux caractères
        if !tail.is_empty() && next.chars().count() + 2 > max {
            break;
        }
        tail = next;
    }
    format!("…{sep}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths() {
        let home = Path::new("/home/ana");
        assert_eq!(short_path(Path::new("/home/ana/Notes"), Some(home), 36), "~/Notes");
        assert_eq!(short_path(Path::new("/home/ana"), Some(home), 36), "~");
        assert_eq!(short_path(Path::new("/tmp/x"), Some(home), 36), "/tmp/x");
        assert_eq!(short_path(Path::new("/tmp/x"), None, 36), "/tmp/x");
        assert_eq!(
            short_path(Path::new("/home/ana/Documents/Coffres/Obsidian/.obsidian"), Some(home), 24),
            "…/Obsidian/.obsidian"
        );
        // le dernier élément reste entier, même trop long
        assert_eq!(short_path(Path::new("/a/un-nom-vraiment-tres-long"), None, 10), "…/un-nom-vraiment-tres-long");
    }

    #[test]
    fn old_config_still_loads() {
        // configuration écrite par la 0.5.0, avant la refonte des paramètres : aucun des nouveaux champs
        let old = r#"
database_url = ""
dark_mode = false
font_size = 15.0
view_mode = "Split"
show_left = true
show_right = false
obsidian_dir = "/home/ana/Coffre/.obsidian"
vault_dir = "/home/ana/Coffre"
follow_appearance = true
css_theme = "Minimal"
css_snippets = ["a", "b"]
github_api = "https://api.github.com"
github_auto_sync = false
"#;
        let mut cfg: Config = toml::from_str(old).unwrap();
        assert_eq!(cfg.mode(), ThemeMode::Light);
        assert!(cfg.follow_appearance);
        assert_eq!(cfg.font_size, 15.0);
        assert_eq!(cfg.view_mode, ViewMode::Split);
        assert_eq!(cfg.css_theme.as_deref(), Some("Minimal"));
        assert_eq!(cfg.css_snippets, ["a", "b"]);
        assert_eq!(cfg.accent_color, None);
        assert_eq!(cfg.interface_font, None);
        // les nouveaux champs survivent à l'écriture
        cfg.theme_mode = Some(ThemeMode::System);
        cfg.accent_color = Some("#ff8800".into());
        let again: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(again.mode(), ThemeMode::System);
        assert_eq!(again.accent_color.as_deref(), Some("#ff8800"));
    }

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
