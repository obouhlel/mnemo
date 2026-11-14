//! Stockage local des notes : un dossier de fichiers Markdown, la référence de Mnemo.
//!
//! Un thread répond aux mêmes commandes (`Cmd`) que le worker PostgreSQL pour tout ce qui concerne les
//! notes, dossiers, recherche, rétroliens, tags, graphe, icônes et réglages des plugins, et renvoie les mêmes
//! messages (`Msg`). Toutes les secondes, il compare l'index au disque : un fichier modifié par un autre
//! éditeur, ou par la synchronisation avec la base (`db/vault_sync.rs`), devient un événement `Msg::LocalEvent`,
//! comme un NOTIFY de PostgreSQL. La base de données n'est donc pas nécessaire pour utiliser Mnemo :
//! elle ne sert qu'à synchroniser plusieurs appareils.

pub mod index;
pub mod shared;

use crate::db::{Cmd, LoadPurpose, Msg};
use crate::model::*;
use index::Doc;
use shared::{STATE_DIR, SharedState, read_state, write_atomic, write_state};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// Intervalle de comparaison avec le disque.
const RESCAN: Duration = Duration::from_millis(900);

struct LocalNote {
    path: String,
    content: String,
    version: i32,
    updated_at: chrono::DateTime<chrono::Utc>,
    mtime: Option<SystemTime>,
}

struct Store {
    root: Option<PathBuf>,
    notes: HashMap<Uuid, LocalNote>,
    dirs: BTreeSet<String>,
    client_id: String,
    out: Sender<Msg>,
    notify: Arc<dyn Fn() + Send + Sync>,
    shared: SharedState,
    ids_generation: u64,
    /// dates de modification des fichiers d'état (icônes, plugins) pour voir les changements extérieurs
    state_mtimes: HashMap<&'static str, Option<SystemTime>>,
    /// pièces jointes (chemin relatif → date de modification, taille)
    files: std::collections::BTreeMap<String, (Option<SystemTime>, u64)>,
}

/// Lance le thread du stockage local ; les commandes arrivent par `rx`.
pub fn spawn(
    client_id: String,
    out: Sender<Msg>,
    notify: Arc<dyn Fn() + Send + Sync>,
    shared: SharedState,
    rx: Receiver<Cmd>,
) {
    std::thread::Builder::new()
        .name("mnemo-local".into())
        .spawn(move || {
            let mut s = Store {
                root: None,
                notes: HashMap::new(),
                dirs: BTreeSet::new(),
                client_id,
                out,
                notify,
                shared,
                ids_generation: 0,
                state_mtimes: HashMap::new(),
                files: Default::default(),
            };
            let mut last = Instant::now();
            loop {
                match rx.recv_timeout(RESCAN) {
                    Ok(cmd) => s.handle(cmd),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                if last.elapsed() >= RESCAN {
                    s.rescan();
                    last = Instant::now();
                }
            }
        })
        .expect("spawn local thread");
}

/// Écrit des octets via un fichier temporaire caché puis renomme.
fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let tmp = path.with_file_name(format!(".{name}.mnemo-tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Store {
    fn emit(&self, msg: Msg) {
        let _ = self.out.send(msg);
        (self.notify)();
    }

    fn event(&self, kind: &str, op: &str, id: Option<Uuid>, path: &str, version: Option<i32>, mine: bool) {
        self.emit(Msg::LocalEvent(VaultEvent {
            kind: kind.into(),
            op: op.into(),
            id: id.map(|i| serde_json::Value::String(i.to_string())).unwrap_or(serde_json::Value::String(path.into())),
            path: Some(path.into()),
            old_path: None,
            version,
            updated_by: mine.then(|| self.client_id.clone()),
        }));
    }

    fn error(&self, e: impl Into<String>) {
        self.emit(Msg::Error(e.into()));
    }

    fn file(&self, path: &str) -> Option<PathBuf> {
        self.root.as_ref().map(|r| crate::db::note_file(r, path))
    }

    fn note(&self, id: Uuid) -> Option<Note> {
        self.notes.get(&id).map(|n| Note {
            id,
            path: n.path.clone(),
            content: n.content.clone(),
            version: n.version,
            updated_by: None,
            updated_at: n.updated_at,
        })
    }

    fn by_path(&self, path: &str) -> Option<Uuid> {
        let l = path.to_lowercase();
        self.notes.iter().find(|(_, n)| n.path.to_lowercase() == l).map(|(id, _)| *id)
    }

    fn docs(&self) -> Vec<Doc<'_>> {
        self.notes.iter().map(|(id, n)| Doc { id: *id, path: &n.path, content: &n.content }).collect()
    }

    /// Écrit le contenu d'une note sur le disque et met l'index à jour (nouvelle version).
    fn write_note(&mut self, id: Uuid, path: &str, content: String) -> std::io::Result<()> {
        let file = self.file(path).ok_or_else(|| std::io::Error::other("aucun dossier de notes"))?;
        write_atomic(&file, &content)?;
        let m = mtime(&file);
        let n = self.notes.entry(id).or_insert_with(|| LocalNote {
            path: path.to_string(),
            content: String::new(),
            version: 0,
            updated_at: chrono::Utc::now(),
            mtime: None,
        });
        n.path = path.to_string();
        n.content = content;
        n.version += 1;
        n.updated_at = chrono::Utc::now();
        n.mtime = m;
        let mut acc = String::new();
        for seg in path.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            self.dirs.insert(acc.clone());
        }
        Ok(())
    }

    fn set_root(&mut self, root: PathBuf) {
        let _ = std::fs::create_dir_all(&root);
        self.shared.ids.lock().expect("ids").load(&root);
        self.ids_generation = self.shared.ids.lock().expect("ids").generation;
        self.root = Some(root);
        self.notes.clear();
        self.dirs.clear();
        self.state_mtimes.clear();
        self.files.clear();
        self.rescan_inner(false);
        self.emit(Msg::LocalReady);
    }

    /// Compare l'index au disque. `report` : envoyer un événement par changement (pas au premier chargement).
    fn rescan_inner(&mut self, report: bool) {
        let Some(root) = self.root.clone() else { return };
        // identifiants changés par la synchronisation (note adoptant l'id de la base)
        let generation = self.shared.ids.lock().expect("ids").generation;
        if generation != self.ids_generation {
            self.ids_generation = generation;
            let ids = self.shared.ids.lock().expect("ids");
            let remap: Vec<(Uuid, Uuid)> = self
                .notes
                .iter()
                .filter_map(|(id, n)| ids.get(&n.path).filter(|nid| nid != id).map(|nid| (*id, nid)))
                .collect();
            drop(ids);
            for (old, new) in remap {
                if let Some(n) = self.notes.remove(&old) {
                    self.notes.insert(new, n);
                    if report {
                        self.emit(Msg::IdChanged { old, new });
                    }
                }
            }
        }
        let (disk, dirs, _) = crate::db::walk(&root);
        let by_path: HashMap<String, Uuid> = self.notes.iter().map(|(id, n)| (n.path.to_lowercase(), *id)).collect();
        // fichiers déjà indexés : modifiés ?
        let mut seen: std::collections::HashSet<Uuid> = Default::default();
        let mut fresh: Vec<&crate::db::DiskFile> = vec![];
        for (key, f) in &disk {
            let Some(id) = by_path.get(key).copied() else {
                fresh.push(f);
                continue;
            };
            seen.insert(id);
            let n = self.notes.get_mut(&id).expect("note");
            if n.mtime == f.mtime {
                continue;
            }
            n.mtime = f.mtime;
            let Ok(text) = std::fs::read_to_string(&f.abs) else { continue };
            if text != n.content || n.path != f.rel {
                n.content = text;
                n.path = f.rel.clone();
                n.version += 1;
                n.updated_at = chrono::Utc::now();
                let v = n.version;
                if report {
                    self.event("note", "update", Some(id), &f.rel, Some(v), false);
                }
            }
        }
        // fichiers apparus (créés à côté, ou déplacés par la synchronisation : ils gardent alors leur id)
        fresh.sort_by(|a, b| a.rel.cmp(&b.rel));
        for f in fresh {
            let Ok(text) = std::fs::read_to_string(&f.abs) else { continue };
            let mut id = self.shared.ids.lock().expect("ids").get_or_create(&f.rel);
            if seen.contains(&id) {
                // id déjà porté par un autre fichier : celui-ci devient une note à part
                id = Uuid::new_v4();
                self.shared.ids.lock().expect("ids").set(&f.rel, id, false);
            }
            let op = if self.notes.contains_key(&id) { "update" } else { "insert" };
            let version = self.notes.get(&id).map(|n| n.version + 1).unwrap_or(1);
            seen.insert(id);
            self.notes.insert(
                id,
                LocalNote {
                    path: f.rel.clone(),
                    content: text,
                    version,
                    updated_at: f.mtime.map(chrono::DateTime::from).unwrap_or_else(chrono::Utc::now),
                    mtime: f.mtime,
                },
            );
            if report {
                self.event("note", op, Some(id), &f.rel, Some(version), false);
            }
        }
        // fichiers disparus
        let gone: Vec<Uuid> = self.notes.keys().filter(|id| !seen.contains(id)).copied().collect();
        for id in gone {
            if let Some(n) = self.notes.remove(&id)
                && report
            {
                self.event("note", "delete", Some(id), &n.path, None, false);
            }
        }
        let dirs: BTreeSet<String> = dirs.into_iter().collect();
        if dirs != self.dirs {
            self.dirs = dirs;
            if report {
                self.event("folder", "update", None, "", None, false);
            }
        }
        // pièces jointes ajoutées, modifiées ou supprimées (par un autre éditeur ou la synchronisation)
        let files: std::collections::BTreeMap<String, (Option<SystemTime>, u64)> =
            crate::db::walk_files(&root).into_iter().map(|f| (f.rel, (f.mtime, f.size))).collect();
        if files != self.files {
            self.files = files;
            if report {
                self.event("file", "update", None, "", None, false);
            }
        }
        // icônes / plugins modifiés par la synchronisation
        for (name, kind) in [("icons", "icon"), ("plugins", "plugin")] {
            let m = mtime(&root.join(STATE_DIR).join(format!("{name}.json")));
            if self.state_mtimes.insert(name, m) != Some(m) && report {
                self.event(kind, "update", None, name, None, false);
            }
        }
    }

    fn rescan(&mut self) {
        self.rescan_inner(true);
    }

    fn state<T>(&self, f: impl FnOnce(&Path) -> T) -> Option<T> {
        let root = self.root.clone()?;
        let _guard = self.shared.state_lock.lock().expect("state lock");
        Some(f(&root))
    }

    /// Icônes : déplace (renommage) ou supprime (`to` = None) les entrées d'un chemin et de son contenu.
    fn move_icons(&mut self, from: &str, to: Option<&str>) {
        let changed = self.state(|root| {
            let mut m = read_state(root, "icons");
            let f = from.to_lowercase();
            let keys: Vec<String> = m.keys().filter(|k| **k == f || k.starts_with(&format!("{f}/"))).cloned().collect();
            for k in &keys {
                let v = m.remove(k).expect("key");
                if let Some(t) = to {
                    m.insert(format!("{}{}", t.to_lowercase(), &k[f.len()..]), v);
                }
            }
            if !keys.is_empty() {
                write_state(root, "icons", &m);
            }
            !keys.is_empty()
        });
        if changed == Some(true) {
            self.state_mtimes.remove("icons");
        }
    }

    fn trash(&self, rel: &str, is_dir: bool) -> std::io::Result<()> {
        let root = self.root.as_ref().ok_or_else(|| std::io::Error::other("aucun dossier"))?;
        let src = if is_dir { root.join(rel) } else { crate::db::note_file(root, rel) };
        let mut dest =
            if is_dir { root.join(".trash").join(rel) } else { crate::db::note_file(&root.join(".trash"), rel) };
        if dest.exists() {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let name = format!("{} {stamp}", dest.file_name().and_then(|n| n.to_str()).unwrap_or("x"));
            dest = dest.with_file_name(name);
        }
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::rename(&src, &dest)
    }

    fn handle(&mut self, cmd: Cmd) {
        if let Cmd::SetRoot(root) = cmd {
            self.set_root(root);
            return;
        }
        if self.root.is_none() {
            return;
        }
        match cmd {
            Cmd::ListNotes => {
                let mut notes: Vec<NoteMeta> = self
                    .notes
                    .iter()
                    .map(|(id, n)| NoteMeta {
                        id: *id,
                        path: n.path.clone(),
                        version: n.version,
                        updated_at: n.updated_at,
                    })
                    .collect();
                notes.sort_by_key(|n| n.path.to_lowercase());
                self.emit(Msg::Notes(notes));
            }
            Cmd::ListFolders => self.emit(Msg::Folders(self.dirs.iter().cloned().collect())),
            Cmd::Load { id, purpose } => match self.note(id) {
                Some(note) => self.emit(Msg::Loaded { note, purpose }),
                None if purpose == LoadPurpose::Open => self.error("Note introuvable"),
                None => {}
            },
            Cmd::OpenLink(link) => {
                let found = index::resolve(&self.docs(), &link).map(|d| d.id);
                match found {
                    Some(id) => {
                        let note = self.note(id).expect("note");
                        self.emit(Msg::Loaded { note, purpose: LoadPurpose::Open });
                    }
                    None => match normalize_path(link.trim().trim_end_matches(".md")) {
                        Ok(path) => {
                            self.handle(Cmd::Create { path, content: String::new(), open: true, if_missing: true })
                        }
                        Err(e) => self.error(e),
                    },
                }
            }
            Cmd::Create { path, content, open, if_missing } => {
                if let Some(id) = self.by_path(&path) {
                    if if_missing {
                        if open {
                            let note = self.note(id).expect("note");
                            self.emit(Msg::Loaded { note, purpose: LoadPurpose::Open });
                        }
                    } else {
                        self.error(format!("« {path} » existe déjà"));
                    }
                    return;
                }
                let id = self.shared.ids.lock().expect("ids").get_or_create(&path);
                match self.write_note(id, &path, content) {
                    Ok(()) => {
                        let note = self.note(id).expect("note");
                        self.event("note", "insert", Some(id), &path, Some(note.version), true);
                        self.emit(Msg::Created { note, open });
                    }
                    Err(e) => self.error(format!("Création de « {path} » impossible : {e}")),
                }
            }
            Cmd::Save { id, content, base_version } => {
                let Some(cur) = self.note(id) else {
                    self.error("La note a été supprimée entre-temps");
                    return;
                };
                if cur.version != base_version {
                    self.emit(Msg::Conflict(cur));
                    return;
                }
                if cur.content == content {
                    self.emit(Msg::Saved(cur));
                    return;
                }
                match self.write_note(id, &cur.path, content) {
                    Ok(()) => {
                        let note = self.note(id).expect("note");
                        self.event("note", "update", Some(id), &note.path, Some(note.version), true);
                        self.emit(Msg::Saved(note));
                    }
                    Err(e) => self.error(format!("Enregistrement impossible : {e}")),
                }
            }
            Cmd::Rename { id, new_path } => self.rename_note(id, new_path),
            Cmd::Delete(id) => {
                let Some(path) = self.notes.get(&id).map(|n| n.path.clone()) else { return };
                if let Err(e) = self.trash(&path, false) {
                    self.error(format!("Suppression impossible : {e}"));
                    return;
                }
                self.notes.remove(&id);
                self.move_icons(&path, None);
                self.event("note", "delete", Some(id), &path, None, true);
                self.emit(Msg::Deleted(id));
            }
            Cmd::CreateFolder(path) => {
                let root = self.root.clone().expect("root");
                match std::fs::create_dir_all(root.join(&path)) {
                    Ok(()) => {
                        self.dirs.insert(path.clone());
                        self.event("folder", "insert", None, &path, None, true);
                    }
                    Err(e) => self.error(format!("Création du dossier impossible : {e}")),
                }
            }
            Cmd::RenameFolder { from, to } => self.rename_folder(from, to),
            Cmd::DeleteFolder(path) => {
                if let Err(e) = self.trash(&path, true) {
                    self.error(format!("Suppression du dossier impossible : {e}"));
                    return;
                }
                let p = path.to_lowercase();
                let inside = |q: &str| q.to_lowercase() == p || q.to_lowercase().starts_with(&format!("{p}/"));
                let gone: Vec<(Uuid, String)> =
                    self.notes.iter().filter(|(_, n)| inside(&n.path)).map(|(id, n)| (*id, n.path.clone())).collect();
                for (id, np) in gone {
                    self.notes.remove(&id);
                    self.event("note", "delete", Some(id), &np, None, true);
                }
                self.dirs.retain(|d| !inside(d));
                self.move_icons(&path, None);
                self.event("folder", "delete", None, &path, None, true);
            }
            Cmd::Search(q) => {
                let hits = index::search(&self.docs(), &q);
                self.emit(Msg::SearchResults { query: q, hits });
            }
            Cmd::Backlinks(id) => {
                let docs = self.docs();
                let Some(target) = docs.iter().find(|d| d.id == id) else { return };
                let links = index::backlinks(&docs, target);
                self.emit(Msg::Backlinks { id, links });
            }
            Cmd::Tags => self.emit(Msg::Tags(index::tag_counts(&self.docs()))),
            Cmd::Graph => self.emit(Msg::Graph(index::graph(&self.docs()))),
            Cmd::LoadPlugins => {
                let map = self.state(|root| read_state(root, "plugins")).unwrap_or_default();
                let states = map
                    .into_iter()
                    .map(|(id, v)| {
                        let enabled = v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
                        let settings = v.get("settings").and_then(|s| s.as_object()).cloned().unwrap_or_default();
                        (id, PluginState { enabled, settings })
                    })
                    .collect();
                self.emit(Msg::Plugins(states));
            }
            Cmd::SetPlugin { id, state } => {
                self.state(|root| {
                    let mut m = read_state(root, "plugins");
                    m.insert(
                        id.clone(),
                        serde_json::json!({ "enabled": state.enabled, "settings": serde_json::Value::Object(state.settings) }),
                    );
                    write_state(root, "plugins", &m);
                });
                self.state_mtimes.remove("plugins");
                self.event("plugin", "update", None, &id, None, true);
            }
            Cmd::ListIcons => {
                let map = self.state(|root| read_state(root, "icons")).unwrap_or_default();
                let icons = map.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect();
                self.emit(Msg::Icons(icons));
            }
            Cmd::ListFiles => self.emit(Msg::Files(self.files.keys().cloned().collect())),
            Cmd::SaveFile { path, bytes, insert } => {
                let root = self.root.clone().expect("root");
                // nom unique : « photo.png », « photo 1.png », « photo 2.png »…
                let (stem, ext) = match path.rsplit_once('.') {
                    Some((s, e)) if !s.is_empty() && !e.contains('/') => (s.to_string(), format!(".{e}")),
                    _ => (path.clone(), String::new()),
                };
                let mut rel = path.clone();
                let mut n = 1;
                while root.join(&rel).exists() {
                    rel = format!("{stem} {n}{ext}");
                    n += 1;
                }
                match write_bytes_atomic(&root.join(&rel), &bytes) {
                    Ok(()) => {
                        let abs = root.join(&rel);
                        self.files.insert(rel.clone(), (mtime(&abs), bytes.len() as u64));
                        self.event("file", "insert", None, &rel, None, true);
                        self.emit(Msg::FileSaved { path: rel, insert });
                    }
                    Err(e) => self.error(format!("Enregistrement de « {rel} » impossible : {e}")),
                }
            }
            Cmd::WriteFile { path, bytes } => {
                let abs = self.root.clone().expect("root").join(&path);
                match write_bytes_atomic(&abs, &bytes) {
                    Ok(()) => {
                        self.files.insert(path.clone(), (mtime(&abs), bytes.len() as u64));
                        self.event("file", "update", None, &path, None, true);
                    }
                    Err(e) => self.error(format!("Enregistrement de « {path} » impossible : {e}")),
                }
            }
            Cmd::DeleteFile(path) => {
                let root = self.root.clone().expect("root");
                let mut dest = root.join(".trash").join(&path);
                if dest.exists() {
                    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                    dest = root.join(".trash").join(format!("{stamp} {path}"));
                }
                if let Some(d) = dest.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                match std::fs::rename(root.join(&path), &dest) {
                    Ok(()) => {
                        self.files.remove(&path);
                        self.move_icons(&path, None);
                        self.event("file", "delete", None, &path, None, true);
                    }
                    Err(e) => self.error(format!("Suppression impossible : {e}")),
                }
            }
            Cmd::SetIcon { path, icon } => {
                self.state(|root| {
                    let mut m = read_state(root, "icons");
                    match &icon {
                        Some(i) => m.insert(path.to_lowercase(), serde_json::Value::String(i.clone())),
                        None => m.remove(&path.to_lowercase()),
                    };
                    write_state(root, "icons", &m);
                });
                self.state_mtimes.remove("icons");
                self.event("icon", "update", None, &path, None, true);
            }
            other => self.error(format!("Commande non prise en charge en local : {other:?}")),
        }
    }

    fn rename_note(&mut self, id: Uuid, new_path: String) {
        let Some(old) = self.notes.get(&id).map(|n| n.path.clone()) else { return };
        if old == new_path {
            return;
        }
        if self.by_path(&new_path).is_some_and(|other| other != id) {
            self.error(format!("« {new_path} » existe déjà"));
            return;
        }
        let root = self.root.clone().expect("root");
        let (src, dest) = (crate::db::note_file(&root, &old), crate::db::note_file(&root, &new_path));
        if let Some(d) = dest.parent()
            && let Err(e) = std::fs::create_dir_all(d)
        {
            self.error(format!("Renommage impossible : {e}"));
            return;
        }
        self.shared.ids.lock().expect("ids").rename(&old, &new_path, false, false);
        if let Err(e) = std::fs::rename(&src, &dest) {
            self.shared.ids.lock().expect("ids").rename(&new_path, &old, false, false);
            self.error(format!("Renommage impossible : {e}"));
            return;
        }
        {
            let n = self.notes.get_mut(&id).expect("note");
            n.path = new_path.clone();
            n.version += 1;
            n.mtime = mtime(&dest);
        }
        self.move_icons(&old, Some(&new_path));
        let relinked = self.relink(id, &old, &new_path);
        let note = self.note(id).expect("note");
        self.event("note", "update", Some(id), &new_path, Some(note.version), true);
        self.emit(Msg::Renamed { note, relinked });
    }

    /// Réécrit les `[[liens]]` des autres notes après un renommage ; renvoie le nombre de notes modifiées.
    fn relink(&mut self, id: Uuid, old: &str, new: &str) -> usize {
        let changes = index::relink(&self.docs(), id, old, new);
        let mut n = 0;
        for (nid, content) in changes {
            let path = self.notes[&nid].path.clone();
            if self.write_note(nid, &path, content).is_ok() {
                let v = self.notes[&nid].version;
                self.event("note", "update", Some(nid), &path, Some(v), true);
                n += 1;
            }
        }
        n
    }

    fn rename_folder(&mut self, from: String, to: String) {
        let (f, t) = (from.to_lowercase(), to.to_lowercase());
        if t == f || t.starts_with(&format!("{f}/")) {
            self.error("Impossible de déplacer un dossier dans lui-même");
            return;
        }
        let root = self.root.clone().expect("root");
        if root.join(&to).exists() {
            self.error(format!("« {to} » existe déjà"));
            return;
        }
        if let Some(d) = root.join(&to).parent() {
            let _ = std::fs::create_dir_all(d);
        }
        self.shared.ids.lock().expect("ids").rename(&from, &to, true, false);
        if let Err(e) = std::fs::rename(root.join(&from), root.join(&to)) {
            self.shared.ids.lock().expect("ids").rename(&to, &from, true, false);
            self.error(format!("Renommage du dossier impossible : {e}"));
            return;
        }
        let moved: Vec<(Uuid, String, String)> = self
            .notes
            .iter()
            .filter(|(_, n)| n.path.to_lowercase().starts_with(&format!("{f}/")))
            .map(|(id, n)| (*id, n.path.clone(), format!("{to}{}", &n.path[from.len()..])))
            .collect();
        for (id, _, np) in &moved {
            let file = crate::db::note_file(&root, np);
            let n = self.notes.get_mut(id).expect("note");
            n.path = np.clone();
            n.version += 1;
            n.mtime = mtime(&file);
        }
        self.dirs = self
            .dirs
            .iter()
            .map(|d| {
                let l = d.to_lowercase();
                if l == f || l.starts_with(&format!("{f}/")) { format!("{to}{}", &d[from.len()..]) } else { d.clone() }
            })
            .collect();
        self.move_icons(&from, Some(&to));
        for (id, old, np) in moved {
            self.relink(id, &old, &np);
            let v = self.notes[&id].version;
            self.event("note", "update", Some(id), &np, Some(v), true);
        }
        self.event("folder", "update", None, &to, None, true);
    }
}
