//! Jeux d'icônes du plugin Iconize d'Obsidian (Remix, Simple Icons, Font Awesome, Tabler…) : Iconize les
//! télécharge dans le coffre (`iconPacksPath`, `.obsidian/icons` par défaut), en dossiers de SVG ou en `.zip`.
//! Une icône s'y nomme par le préfixe du jeu suivi du nom du fichier (`SiGithub` : `simple-icons/github.svg`).
//! Les icônes Lucide (`Li…`) sont dessinées avec la police intégrée de Mnemo, pas avec ces fichiers.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Préfixe Iconize d'un jeu : `simple-icons` → `Si`, `font-awesome-solid` → `Fas`, `coolicons` → `Co`.
pub fn pack_prefix(pack: &str) -> String {
    let mut parts = pack.split('-').filter(|p| !p.is_empty());
    let Some(first) = parts.next() else { return String::new() };
    let mut out: String = first.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    if pack.contains('-') {
        for p in parts {
            out.extend(p.chars().next().map(|c| c.to_ascii_lowercase()));
        }
    } else {
        out.extend(first.chars().nth(1).map(|c| c.to_ascii_lowercase()));
    }
    out
}

/// Nom d'une icône dans son jeu : `arrow-right` → `ArrowRight`, `github` → `Github`.
pub fn normalize(stem: &str) -> String {
    stem.split(['-', '_', ' '])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

/// Où lire une icône.
#[derive(Debug, Clone)]
enum Source {
    File(PathBuf),
    Zip { archive: PathBuf, entry: String },
}

/// Index des icônes des jeux installés : nom Iconize complet (`SiGithub`) → fichier.
#[derive(Debug, Default, Clone)]
pub struct Index {
    icons: HashMap<String, Source>,
}

impl Index {
    /// Lit les jeux d'icônes du coffre dont `obsidian_dir` est le dossier `.obsidian`.
    pub fn load(obsidian_dir: &Path) -> Self {
        let mut index = Index::default();
        let Some(vault) = obsidian_dir.parent() else { return index };
        let settings = std::fs::read_to_string(obsidian_dir.join("plugins/obsidian-icon-folder/data.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let rel = settings
            .as_ref()
            .and_then(|v| v.pointer("/settings/iconPacksPath"))
            .and_then(|v| v.as_str())
            .unwrap_or(".obsidian/icons");
        // chemin relatif au coffre, sans sortir du coffre
        if Path::new(rel).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return index;
        }
        let Ok(entries) = std::fs::read_dir(vault.join(rel)) else { return index };
        for e in entries.flatten() {
            let path = e.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            if path.is_dir() {
                let prefix = pack_prefix(&name);
                for f in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    let p = f.path();
                    if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("svg"))
                        && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                    {
                        index.icons.entry(format!("{prefix}{}", normalize(stem))).or_insert(Source::File(p));
                    }
                }
            } else if let Some(pack) = name.strip_suffix(".zip") {
                index.add_zip(&path, &pack_prefix(pack));
            }
        }
        index
    }

    fn add_zip(&mut self, archive: &Path, prefix: &str) {
        let Ok(file) = std::fs::File::open(archive) else { return };
        let Ok(mut zip) = zip::ZipArchive::new(file) else { return };
        for i in 0..zip.len() {
            let Ok(f) = zip.by_index(i) else { continue };
            let Ok(entry) = f.name().map(|n| n.to_string()) else { continue };
            if !entry.to_ascii_lowercase().ends_with(".svg") {
                continue;
            }
            let stem = entry.rsplit('/').next().unwrap_or(&entry);
            let stem = &stem[..stem.len() - 4];
            self.icons
                .entry(format!("{prefix}{}", normalize(stem)))
                .or_insert(Source::Zip { archive: archive.to_path_buf(), entry });
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.icons.contains_key(name)
    }

    /// SVG de l'icône, prêt à teinter (voir [`prepare_svg`]).
    pub fn svg(&self, name: &str) -> Option<Vec<u8>> {
        const MAX: u64 = 512 * 1024;
        let bytes = match self.icons.get(name)? {
            Source::File(p) => {
                if std::fs::metadata(p).ok()?.len() > MAX {
                    return None;
                }
                std::fs::read(p).ok()?
            }
            Source::Zip { archive, entry } => {
                let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).ok()?).ok()?;
                let f = zip.by_name(entry).ok()?;
                let mut out = vec![];
                f.take(MAX).read_to_end(&mut out).ok()?;
                out
            }
        };
        Some(prepare_svg(&String::from_utf8_lossy(&bytes)))
    }
}

/// Dessin en blanc (`currentColor` et remplissage par défaut), pour être teint à la couleur du texte.
pub fn prepare_svg(svg: &str) -> Vec<u8> {
    let mut s = svg.replace("currentColor", "#ffffff").replace("currentcolor", "#ffffff");
    if let Some(start) = s.find("<svg") {
        let end = s[start..].find('>').map(|e| start + e).unwrap_or(s.len());
        if !s[start..end].contains("fill=") {
            s.insert_str(start + 4, " fill=\"#ffffff\"");
        }
    }
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_match_iconize() {
        assert_eq!(pack_prefix("simple-icons"), "Si");
        assert_eq!(pack_prefix("font-awesome-solid"), "Fas");
        assert_eq!(pack_prefix("font-awesome-brands"), "Fab");
        assert_eq!(pack_prefix("remix-icons"), "Ri");
        assert_eq!(pack_prefix("icon-brew"), "Ib");
        assert_eq!(pack_prefix("coolicons"), "Co");
        assert_eq!(pack_prefix("octicons"), "Oc");
        assert_eq!(normalize("arrow-right"), "ArrowRight");
        assert_eq!(normalize("github"), "Github");
        assert_eq!(normalize("Keyboard"), "Keyboard");
    }

    #[test]
    fn svg_is_whitened() {
        let s = String::from_utf8(prepare_svg("<svg viewBox=\"0 0 24 24\"><path d=\"M0\"/></svg>")).unwrap();
        assert!(s.starts_with("<svg fill=\"#ffffff\" viewBox"));
        let s = String::from_utf8(prepare_svg("<svg fill=\"none\" stroke=\"currentColor\"></svg>")).unwrap();
        assert_eq!(s, "<svg fill=\"none\" stroke=\"#ffffff\"></svg>");
    }

    #[test]
    fn packs_in_folders_and_zips() {
        let root = std::env::temp_dir().join(format!("mnemo-iconize-{}", uuid::Uuid::new_v4()));
        let obs = root.join(".obsidian");
        std::fs::create_dir_all(obs.join("icons/simple-icons")).unwrap();
        std::fs::write(obs.join("icons/simple-icons/github.svg"), "<svg><path/></svg>").unwrap();
        {
            let f = std::fs::File::create(obs.join("icons/remix-icons.zip")).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file("remix-icons/home-line.svg", opts).unwrap();
            std::io::Write::write_all(&mut z, b"<svg fill=\"currentColor\"></svg>").unwrap();
            z.finish().unwrap();
        }
        let idx = Index::load(&obs);
        assert!(idx.contains("SiGithub"));
        assert!(String::from_utf8(idx.svg("SiGithub").unwrap()).unwrap().contains("fill=\"#ffffff\""));
        assert_eq!(idx.svg("RiHomeLine").unwrap(), b"<svg fill=\"#ffffff\"></svg>");
        assert!(idx.svg("SiGitlab").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
