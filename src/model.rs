use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct NoteMeta {
    pub id: Uuid,
    pub path: String,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Note {
    pub id: Uuid,
    pub path: String,
    pub content: String,
    pub version: i32,
    pub updated_by: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: Uuid,
    pub path: String,
    /// Snippet where matches are wrapped in \u{1} ... \u{2}
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct Backlink {
    pub id: Uuid,
    pub path: String,
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GraphData {
    /// (id, label, is_ghost)
    pub nodes: Vec<(String, String, bool)>,
    pub edges: Vec<(String, String)>,
}

/// Payload sent by the Postgres triggers on channel `vault_events`.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEvent {
    pub kind: String,
    pub op: String,
    #[serde(default)]
    pub id: serde_json::Value,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub version: Option<i32>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

impl VaultEvent {
    pub fn note_id(&self) -> Option<Uuid> {
        self.id.as_str().and_then(|s| Uuid::parse_str(s).ok())
    }
}

#[derive(Debug, Clone, Default)]
pub struct PluginState {
    pub enabled: bool,
    pub settings: serde_json::Map<String, serde_json::Value>,
}

pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn parent(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

/// Normalizes a user-entered note path. Returns an error message in French.
pub fn normalize_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_suffix(".md").or_else(|| trimmed.strip_suffix(".MD")).unwrap_or(trimmed);
    let p = trimmed.split('/').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/");
    if p.is_empty() {
        return Err("Le nom ne peut pas être vide".into());
    }
    if p.chars().count() > 500 {
        return Err("Nom trop long".into());
    }
    if p.chars().any(|c| matches!(c, '[' | ']' | '|' | '#' | '^' | '\\') || c.is_control()) {
        return Err("Caractères interdits : [ ] | # ^ \\".into());
    }
    if p.split('/').any(|s| s == "." || s == "..") {
        return Err("Segment de chemin invalide".into());
    }
    // un nom qui commence par un point est un fichier caché : il n'apparaîtrait pas dans l'explorateur
    if p.split('/').any(|s| s.starts_with('.')) {
        return Err("Un nom ne peut pas commencer par un point".into());
    }
    // limite des systèmes de fichiers : 255 octets par nom (avec « .md »)
    if p.split('/').any(|s| s.len() > 250) {
        return Err("Nom trop long".into());
    }
    Ok(p)
}

/// Vérifie qu'un nouveau nom existe sur tous les systèmes (les notes voyagent entre Linux, macOS, Windows et
/// Android) : pas de `: * ? " < >`, pas de point ou d'espace final, pas de nom réservé par Windows.
/// Les fichiers déjà présents dans le dossier ne sont pas concernés : ils restent lisibles tels quels.
pub fn portable_name(path: &str) -> Result<(), String> {
    for seg in path.split('/') {
        if seg.chars().any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>')) {
            return Err("Caractères interdits dans un nom : : * ? \" < >".into());
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return Err("Un nom ne peut pas finir par un point ou un espace".into());
        }
        let stem = seg.split('.').next().unwrap_or(seg).to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.ends_with(|c: char| c.is_ascii_digit() && c != '0'));
        if reserved {
            return Err(format!("« {seg} » est un nom réservé par Windows"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize() {
        assert_eq!(normalize_path(" a// b /c.md ").unwrap(), "a/b/c");
        assert!(normalize_path("  / ").is_err());
        assert!(normalize_path("x[y]").is_err());
        assert!(normalize_path("a/../b").is_err());
        assert!(normalize_path(".brouillon").is_err() && normalize_path("a/.cache/b").is_err());
        assert!(normalize_path("v1.2 notes").is_ok());
        assert!(portable_name("Réunions/Point d'étape v1.2").is_ok());
        for bad in ["Quoi ?", "a:b", "Projets./x", "fin ", "CON", "dir/nul.txt", "COM1", "a*b", "<x>"] {
            assert!(portable_name(bad).is_err(), "{bad}");
        }
        assert!(portable_name("COM0").is_ok() && portable_name("Console").is_ok());
        assert_eq!(basename("a/b/c"), "c");
        assert_eq!(parent("a/b/c"), Some("a/b"));
        assert_eq!(parent("c"), None);
    }
}
