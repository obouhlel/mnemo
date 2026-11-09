//! État partagé entre le stockage local et la synchronisation avec PostgreSQL (même processus) :
//! identifiant de chaque note (le même en local et en base) et verrou des fichiers d'état `.mnemo/*.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Dossier des fichiers internes de Mnemo dans le dossier des notes (caché : ignoré par le parcours des notes).
pub const STATE_DIR: &str = ".mnemo";

/// Chemin de note (en minuscules) → identifiant, enregistré dans `.mnemo/ids.json`.
/// La synchronisation y écrit l'identifiant de la base pour qu'une note garde le même id partout.
#[derive(Default)]
pub struct IdMap {
    map: HashMap<String, Uuid>,
    file: Option<PathBuf>,
    /// incrémenté à chaque changement venu de la synchronisation (le stockage local relit alors ses ids)
    pub generation: u64,
}

impl IdMap {
    pub fn load(&mut self, root: &Path) {
        let file = root.join(STATE_DIR).join("ids.json");
        self.map = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        self.file = Some(file);
        self.generation += 1;
    }

    pub fn get(&self, path: &str) -> Option<Uuid> {
        self.map.get(&path.to_lowercase()).copied()
    }

    /// Identifiant du chemin, créé s'il n'existe pas encore.
    pub fn get_or_create(&mut self, path: &str) -> Uuid {
        if let Some(id) = self.get(path) {
            return id;
        }
        let id = Uuid::new_v4();
        self.map.insert(path.to_lowercase(), id);
        self.save();
        id
    }

    /// Fixe l'identifiant d'un chemin (adopté depuis la base). `from_sync` : prévenir le stockage local.
    pub fn set(&mut self, path: &str, id: Uuid, from_sync: bool) {
        if self.map.insert(path.to_lowercase(), id) != Some(id) {
            if from_sync {
                self.generation += 1;
            }
            self.save();
        }
    }

    /// Renommage : l'identifiant suit le chemin (notes d'un dossier comprises si `prefix`).
    pub fn rename(&mut self, from: &str, to: &str, prefix: bool, from_sync: bool) {
        let (f, t) = (from.to_lowercase(), to.to_lowercase());
        let moved: Vec<(String, Uuid)> = self
            .map
            .iter()
            .filter(|(k, _)| **k == f || (prefix && k.starts_with(&format!("{f}/"))))
            .map(|(k, v)| (format!("{t}{}", &k[f.len()..]), *v))
            .collect();
        if moved.is_empty() {
            return;
        }
        self.map.retain(|k, _| !(*k == f || (prefix && k.starts_with(&format!("{f}/")))));
        self.map.extend(moved);
        if from_sync {
            self.generation += 1;
        }
        self.save();
    }

    fn save(&self) {
        if let Some(f) = &self.file {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string(&self.map) {
                let _ = write_atomic(f, &s);
            }
        }
    }
}

pub struct Shared {
    pub ids: Mutex<IdMap>,
    /// pris pendant toute lecture-modification-écriture de `.mnemo/icons.json` / `plugins.json`
    pub state_lock: Mutex<()>,
}

pub type SharedState = Arc<Shared>;

pub fn new_shared() -> SharedState {
    Arc::new(Shared { ids: Mutex::new(IdMap::default()), state_lock: Mutex::new(()) })
}

/// Écrit via un fichier temporaire caché puis renomme (jamais de fichier à moitié écrit).
pub fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let tmp = path.with_file_name(format!(".{name}.mnemo-tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Fichier d'état `.mnemo/<name>.json` lu comme objet JSON (vide s'il n'existe pas).
pub fn read_state(root: &Path, name: &str) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(root.join(STATE_DIR).join(format!("{name}.json")))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn write_state(root: &Path, name: &str, map: &serde_json::Map<String, serde_json::Value>) {
    let text = serde_json::to_string_pretty(map).unwrap_or_else(|_| "{}".into());
    let _ = write_atomic(&root.join(STATE_DIR).join(format!("{name}.json")), &text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_renames() {
        let mut m = IdMap::default();
        let a = m.get_or_create("Dir/A");
        let b = m.get_or_create("Dir/Sub/B");
        assert_eq!(m.get("dir/a"), Some(a));
        m.rename("Dir", "Archive/Dir", true, false);
        assert_eq!(m.get("Archive/Dir/A"), Some(a));
        assert_eq!(m.get("archive/dir/sub/b"), Some(b));
        assert_eq!(m.get("Dir/A"), None);
        let g = m.generation;
        m.set("Archive/Dir/A", b, true);
        assert_eq!((m.get("Archive/Dir/A"), m.generation), (Some(b), g + 1));
    }
}
