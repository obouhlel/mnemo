//! Polices installées sur le système : celle choisie pour le texte des notes, l'interface ou le code
//! (`appearance.json` d'Obsidian : `textFontFamily`, `interfaceFontFamily`, `monospaceFontFamily`), et une
//! police d'emoji en couleur. La liste des polices est lue une seule fois, au premier besoin.

use std::sync::OnceLock;

/// Une face de police chargée en mémoire.
pub struct Face {
    pub data: Vec<u8>,
    pub index: u32,
    /// la graisse / le style demandés existent (sinon : la face normale de la famille)
    pub exact: bool,
}

fn db() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Premier nom d'une liste CSS (`"Fira Code", monospace` → `Fira Code`).
pub fn first_family(list: &str) -> Option<String> {
    list.split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']).trim())
        .find(|f| !f.is_empty() && !GENERIC.contains(&f.to_ascii_lowercase().as_str()) && !f.starts_with("var("))
        .map(str::to_string)
}

/// Familles génériques CSS : pas une police précise, Mnemo garde les siennes.
const GENERIC: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-sans-serif",
    "ui-serif",
    "ui-monospace",
    "inherit",
    "initial",
    "unset",
    "??",
];

fn weight_distance(face: &fontdb::FaceInfo, bold: bool) -> u16 {
    let want = if bold { 700 } else { 400 };
    face.weight.0.abs_diff(want)
}

/// Face d'une famille installée (nom insensible à la casse) ; `None` si la famille n'existe pas.
pub fn find(family: &str, bold: bool, italic: bool) -> Option<Face> {
    let db = db();
    let faces: Vec<&fontdb::FaceInfo> =
        db.faces().filter(|f| f.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family))).collect();
    let italic_ok = |f: &fontdb::FaceInfo| (f.style != fontdb::Style::Normal) == italic;
    let best = faces
        .iter()
        .filter(|f| italic_ok(f))
        .min_by_key(|f| weight_distance(f, bold))
        .or_else(|| faces.iter().min_by_key(|f| (f.style != fontdb::Style::Normal, weight_distance(f, false))))?;
    let exact = italic_ok(best) && weight_distance(best, bold) <= 100;
    let index = best.index;
    let data = db.with_face_data(best.id, |data, _| data.to_vec())?;
    Some(Face { data, index, exact })
}

/// Police d'emoji en couleur du système (Noto Color Emoji, Apple Color Emoji, Segoe UI Emoji…).
pub fn color_emoji() -> Option<Face> {
    ["Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji", "Twemoji", "Twitter Color Emoji", "JoyPixels"]
        .iter()
        .find_map(|name| find(name, false, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_family_lists() {
        assert_eq!(first_family("\"Fira Code\", monospace").as_deref(), Some("Fira Code"));
        assert_eq!(first_family("CaskaydiaMono Nerd Font").as_deref(), Some("CaskaydiaMono Nerd Font"));
        assert_eq!(first_family("sans-serif, ??").as_deref(), None);
        assert_eq!(first_family("var(--font-default), Inter").as_deref(), Some("Inter"));
        assert_eq!(first_family("").as_deref(), None);
    }

    #[test]
    fn unknown_family_is_none() {
        assert!(find("Police Qui N'existe Pas 42", false, false).is_none());
    }
}
