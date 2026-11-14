//! Miroir d'un dossier de coffre (Obsidian) sur le disque.
//!
//! Quand un dossier est lié, le worker le compare régulièrement à la base (`Cmd::VaultScan`) :
//! - les fichiers `.md` du dossier absents de la base sont importés ;
//! - les notes de la base absentes du dossier y sont écrites (notes créées dans Mnemo comprises) ;
//! - une modification d'un côté est recopiée de l'autre : contenu, renommage, suppression
//!   (les fichiers supprimés depuis Mnemo vont dans `.trash/` du coffre, comme dans Obsidian) ;
//! - les dossiers vides sont créés / supprimés des deux côtés.
//!
//! L'état connu (`Known` : version en base et date de modification du fichier au dernier passage) sert à savoir
//! de quel côté vient chaque changement. Il est enregistré dans `.mnemo/sync.json`, avec le texte de chaque note
//! à ce moment-là dans `.mnemo/base/` : il survit donc à un redémarrage et à une coupure de la base.
//! Si les deux côtés ont changé depuis la dernière synchronisation, les deux textes sont fusionnés (fusion à
//! 3 voies, comme dans l'éditeur). S'ils ont changé les mêmes lignes, la base l'emporte et le texte du disque est
//! conservé dans une note « (conflit disque) ». Rien n'est noté comme synchronisé avant la validation de la
//! transaction : un passage interrompu est simplement refait.
//! Les dossiers cachés (`.obsidian`, `.trash`, `.git`…) et les fichiers autres que `.md` sont ignorés.

use super::*;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Default, Clone, Copy)]
pub struct VaultStats {
    /// fichiers du disque ajoutés à la base
    pub imported: usize,
    /// notes mises à jour depuis le disque
    pub pulled: usize,
    /// fichiers écrits / déplacés sur le disque
    pub written: usize,
    /// suppressions (d'un côté ou de l'autre)
    pub removed: usize,
    /// fichiers ignorés (nom invalide pour Mnemo, encodage non UTF-8)
    pub skipped: usize,
}

impl VaultStats {
    pub fn changed(&self) -> bool {
        self.imported + self.pulled + self.written + self.removed > 0
    }
}

pub(super) struct Vault {
    root: PathBuf,
    known: HashMap<Uuid, Known>,
    /// dossiers (en minuscules) présents des deux côtés au dernier passage
    folders: HashSet<String>,
    /// premier passage depuis la connexion (résumé affiché une fois)
    first: bool,
    /// un passage a déjà abouti avec cette base (maintenant ou avant un redémarrage) : une absence d'un côté
    /// est alors une suppression à recopier, plus une première rencontre
    synced: bool,
    /// identifiant de la base avec laquelle l'état connu a été établi
    db_id: String,
    db_checked: bool,
    /// icônes / réglages des plugins tels qu'au dernier passage (base de la fusion à 3 voies)
    kv_known: HashMap<String, Map<String, Value>>,
    /// pièces jointes au dernier passage : chemin → (empreinte, date de modification du fichier)
    file_known: HashMap<String, (String, Option<SystemTime>)>,
}

/// Contenu de `.mnemo/sync.json`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct SavedState {
    db: String,
    notes: HashMap<Uuid, Known>,
    folders: HashSet<String>,
    kv: HashMap<String, Map<String, Value>>,
    files: HashMap<String, (String, Option<SystemTime>)>,
}

fn state_file(root: &Path) -> PathBuf {
    root.join(crate::local::shared::STATE_DIR).join("sync.json")
}

fn base_dir(root: &Path) -> PathBuf {
    root.join(crate::local::shared::STATE_DIR).join("base")
}

/// Texte d'une note au dernier passage (base de la fusion à 3 voies).
fn read_base(root: &Path, id: Uuid) -> Option<String> {
    std::fs::read_to_string(base_dir(root).join(format!("{id}.md"))).ok()
}

impl Vault {
    pub(super) fn new(root: PathBuf) -> Self {
        let saved: Option<SavedState> =
            std::fs::read_to_string(state_file(&root)).ok().and_then(|s| serde_json::from_str(&s).ok());
        let synced = saved.is_some();
        let s = saved.unwrap_or_default();
        Self {
            root,
            known: s.notes,
            folders: s.folders,
            first: true,
            synced,
            db_id: s.db,
            db_checked: false,
            kv_known: s.kv,
            file_known: s.files,
        }
    }

    /// Connexion coupée ou changée : l'état connu est gardé (c'est le dernier état commun), mais le prochain
    /// passage vérifie qu'il s'agit bien de la même base.
    pub(super) fn reset(&mut self) {
        self.first = true;
        self.db_checked = false;
    }

    /// Autre base que celle de l'état connu : tout est à recomparer comme une première rencontre.
    fn forget(&mut self) {
        self.known.clear();
        self.folders.clear();
        self.kv_known.clear();
        self.file_known.clear();
        self.synced = false;
        let _ = std::fs::remove_dir_all(base_dir(&self.root));
    }

    fn save(&self) {
        let state = SavedState {
            db: self.db_id.clone(),
            notes: self.known.clone(),
            folders: self.folders.clone(),
            kv: self.kv_known.clone(),
            files: self.file_known.clone(),
        };
        if let Ok(s) = serde_json::to_string(&state) {
            let _ = crate::local::shared::write_atomic(&state_file(&self.root), &s);
        }
    }
}

/// Changements de l'état connu des pièces jointes, appliqués une fois la transaction validée.
type FileUpdates = Vec<(String, Option<(String, Option<SystemTime>)>)>;

/// Taille maximale d'une pièce jointe synchronisée.
const MAX_FILE: u64 = 50 * 1024 * 1024;

/// Que faire d'une pièce jointe, d'après son empreinte au dernier passage (`known`), en local et en base.
#[derive(Debug, PartialEq)]
enum FilePlan {
    Nothing,
    Upload,
    Download,
    DeleteRemote,
    DeleteLocal,
    /// changée des deux côtés : la base l'emporte, la version locale est gardée à côté
    Conflict,
}

fn plan_file(known: Option<&str>, local: Option<&str>, remote: Option<&str>) -> FilePlan {
    if local == remote {
        return FilePlan::Nothing;
    }
    let Some(k) = known else {
        // première rencontre : union, la base l'emporte si les deux existent
        return match (local, remote) {
            (Some(_), None) => FilePlan::Upload,
            (None, Some(_)) => FilePlan::Download,
            _ => FilePlan::Conflict,
        };
    };
    match (local != Some(k), remote != Some(k)) {
        (true, false) if local.is_none() => FilePlan::DeleteRemote,
        (true, false) => FilePlan::Upload,
        (false, true) if remote.is_none() => FilePlan::DeleteLocal,
        (false, true) => FilePlan::Download,
        // les deux ont changé : une suppression d'un côté ne gagne pas sur une modification de l'autre
        _ => match (local, remote) {
            (Some(_), None) => FilePlan::Upload,
            (None, Some(_)) => FilePlan::Download,
            _ => FilePlan::Conflict,
        },
    }
}

/// « Dossier/photo.png » → « Dossier/photo (conflit disque).png »
fn conflict_name(path: &str) -> String {
    match path.rsplit_once('.') {
        Some((s, e)) if !e.contains('/') => format!("{s} (conflit disque).{e}"),
        _ => format!("{path} (conflit disque)"),
    }
}

/// Pièces jointes : dossier ↔ table `files`.
async fn sync_files(
    t: &tokio_postgres::Transaction<'_>,
    v: &mut Vault,
    root: &Path,
    stats: &mut VaultStats,
) -> Res<FileUpdates> {
    let mut updates: FileUpdates = vec![];
    let local: HashMap<String, AttFile> =
        walk_files(root).into_iter().filter(|f| f.size <= MAX_FILE).map(|f| (f.rel.clone(), f)).collect();
    // un chemin de la base qui sortirait du dossier (« .. », chemin absolu, lecteur Windows) est ignoré
    let remote: HashMap<String, String> = t
        .query("SELECT path, hash FROM files", &[])
        .await?
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get(1)))
        .filter(|(p, _)| safe_rel(p))
        .collect();
    let mut paths: Vec<&String> = local.keys().chain(remote.keys()).collect();
    paths.sort();
    paths.dedup();
    for path in paths {
        let known = v.file_known.get(path).cloned();
        let lf = local.get(path);
        // empreinte locale : recalculée seulement si le fichier a bougé depuis le dernier passage
        let mut bytes: Option<Vec<u8>> = None;
        let lhash = match (lf, &known) {
            (None, _) => None,
            (Some(f), Some((h, m))) if *m == f.mtime => Some(h.clone()),
            (Some(f), _) => {
                let Ok(b) = std::fs::read(&f.abs) else { continue };
                let h = file_hash(&b);
                bytes = Some(b);
                Some(h)
            }
        };
        let rhash = remote.get(path).cloned();
        let plan = plan_file(known.as_ref().map(|k| k.0.as_str()), lhash.as_deref(), rhash.as_deref());
        let abs = root.join(path);
        match plan {
            FilePlan::Nothing => {}
            FilePlan::Upload => {
                let b = match bytes.take() {
                    Some(b) => b,
                    None => match std::fs::read(&abs) {
                        Ok(b) => b,
                        Err(_) => continue,
                    },
                };
                t.execute(
                    "INSERT INTO files (path, data, hash, size) VALUES ($1, $2, $3, $4)
                     ON CONFLICT (path) DO UPDATE SET data = EXCLUDED.data, hash = EXCLUDED.hash, size = EXCLUDED.size,
                       updated_at = now()",
                    &[path, &b, &lhash.clone().unwrap_or_default(), &(b.len() as i64)],
                )
                .await?;
                stats.imported += 1;
            }
            FilePlan::Download | FilePlan::Conflict => {
                if plan == FilePlan::Conflict && abs.exists() {
                    // la version locale est gardée à côté, sans écraser une copie de conflit plus ancienne
                    let mut copy = root.join(conflict_name(path));
                    if copy.exists() {
                        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
                        copy = root.join(
                            conflict_name(path).replace("(conflit disque)", &format!("(conflit disque {stamp})")),
                        );
                    }
                    if std::fs::rename(&abs, &copy).is_err() {
                        continue;
                    }
                }
                let data: Vec<u8> = t.query_one("SELECT data FROM files WHERE path = $1", &[path]).await?.get(0);
                if write_bytes(&abs, &data).is_ok() {
                    stats.written += 1;
                }
            }
            FilePlan::DeleteRemote => {
                t.execute("DELETE FROM files WHERE path = $1", &[path]).await?;
                stats.removed += 1;
            }
            FilePlan::DeleteLocal => {
                let mut dest = root.join(".trash").join(path);
                if dest.exists() {
                    dest = root.join(".trash").join(format!("{} {path}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
                }
                if let Some(d) = dest.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                if std::fs::rename(&abs, &dest).is_ok() {
                    stats.removed += 1;
                }
            }
        }
        // état connu après ce passage : ce qui est maintenant des deux côtés
        let now_local = mtime(&abs);
        let final_hash = match plan {
            FilePlan::Nothing | FilePlan::Upload => lhash.or(rhash),
            FilePlan::Download | FilePlan::Conflict => rhash,
            FilePlan::DeleteRemote | FilePlan::DeleteLocal => None,
        };
        let new = match final_hash {
            Some(h) if abs.exists() => Some((h, now_local)),
            _ => None,
        };
        if new != known {
            updates.push((path.clone(), new));
        }
    }
    Ok(updates)
}

/// Chemin relatif qui reste dans le dossier des notes et hors de ses dossiers cachés (.mnemo, .obsidian, .trash).
fn safe_rel(path: &str) -> bool {
    !path.contains(['\\', ':'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && !s.starts_with('.') && s.trim() == s && !s.chars().any(char::is_control))
}

fn write_bytes(abs: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let tmp = abs.with_file_name(format!(".{name}.mnemo-tmp"));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, abs)
}

/// Clés à écrire dans la base après une fusion (`None` : à supprimer).
type KvPush = Vec<(String, Option<Value>)>;

/// Fusion à 3 voies d'un fichier d'état (icônes, plugins) avec la table correspondante.
/// Renvoie le résultat et les clés à écrire dans la base (`None` : à supprimer). Si les deux côtés ont changé
/// la même clé, la base l'emporte (comme pour les notes). Sans base connue (premier passage) : union.
fn merge_kv(
    base: Option<&Map<String, Value>>,
    local: &Map<String, Value>,
    remote: &Map<String, Value>,
) -> (Map<String, Value>, KvPush) {
    let keys: std::collections::BTreeSet<&String> = local.keys().chain(remote.keys()).collect();
    let mut result = Map::new();
    let mut push = vec![];
    for k in keys {
        let (l, r) = (local.get(k), remote.get(k));
        let chosen = if l == r {
            l
        } else {
            match base {
                None => r.or(l),
                Some(b) if l == b.get(k) => r, // seule la base a changé
                Some(b) if r == b.get(k) => l, // seul le local a changé
                Some(_) => r,                  // les deux : la base l'emporte
            }
        };
        if chosen != r {
            push.push((k.clone(), chosen.cloned()));
        }
        if let Some(v) = chosen {
            result.insert(k.clone(), v.clone());
        }
    }
    (result, push)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Known {
    path: String,
    version: i32,
    mtime: Option<SystemTime>,
}

#[derive(Debug)]
pub(crate) struct DiskFile {
    pub(crate) rel: String,
    pub(crate) abs: PathBuf,
    pub(crate) mtime: Option<SystemTime>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// `a/b/c` → `<root>/a/b/c.md` (sans `set_extension`, qui couperait « v1.2 »).
pub(crate) fn note_file(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    let mut segs: Vec<&str> = rel.split('/').collect();
    let last = segs.pop().unwrap_or_default();
    for s in segs {
        p.push(s);
    }
    p.push(format!("{last}.md"));
    p
}

fn dir_path(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for s in rel.split('/') {
        p.push(s);
    }
    p
}

/// Pièce jointe : tout fichier visible du dossier qui n'est pas une note (image, PDF, whiteboard…).
#[derive(Debug, Clone)]
pub(crate) struct AttFile {
    /// chemin relatif complet, extension comprise (« Pièces jointes/photo.png »)
    pub(crate) rel: String,
    pub(crate) abs: PathBuf,
    pub(crate) mtime: Option<SystemTime>,
    pub(crate) size: u64,
}

/// Pièces jointes du dossier (les dossiers cachés comme .obsidian, .trash, .mnemo sont ignorés).
pub(crate) fn walk_files(root: &Path) -> Vec<AttFile> {
    fn rec(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<AttFile>) {
        if depth > 32 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if ft.is_dir() {
                rec(&e.path(), &rel, depth + 1, out);
            } else if ft.is_file() && !name.to_lowercase().ends_with(".md") {
                let meta = e.metadata().ok();
                out.push(AttFile {
                    rel,
                    abs: e.path(),
                    mtime: meta.as_ref().and_then(|m| m.modified().ok()),
                    size: meta.map(|m| m.len()).unwrap_or(0),
                });
            }
        }
    }
    let mut out = vec![];
    rec(root, "", 0, &mut out);
    out.sort_by_key(|f| f.rel.to_lowercase());
    out
}

/// Empreinte stable d'un fichier (FNV-1a 64 bits, en hexadécimal) : la même sur tous les appareils,
/// application mobile comprise, pour savoir si une pièce jointe a changé.
pub fn file_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Parcourt le coffre : fichiers `.md` (clé = chemin sans extension, en minuscules),
/// dossiers visibles et nombre d'entrées ignorées.
pub(crate) fn walk(root: &Path) -> (HashMap<String, DiskFile>, Vec<String>, usize) {
    let mut files = HashMap::new();
    let mut dirs = vec![];
    let mut skipped = 0;
    fn rec(
        dir: &Path,
        prefix: &str,
        depth: usize,
        files: &mut HashMap<String, DiskFile>,
        dirs: &mut Vec<String>,
        skipped: &mut usize,
    ) {
        if depth > 32 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                *skipped += 1;
                continue;
            };
            if name.starts_with('.') {
                continue; // .obsidian, .trash, .git, fichiers temporaires
            }
            let Ok(ft) = e.file_type() else { continue };
            let abs = e.path();
            let (is_dir, is_file) = if ft.is_symlink() {
                // on suit les liens vers des fichiers, pas vers des dossiers (boucles possibles)
                (false, std::fs::metadata(&abs).is_ok_and(|m| m.is_file()))
            } else {
                (ft.is_dir(), ft.is_file())
            };
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if is_dir {
                if normalize_path(&rel).as_deref() != Ok(rel.as_str()) {
                    *skipped += 1;
                    continue;
                }
                dirs.push(rel.clone());
                rec(&abs, &rel, depth + 1, files, dirs, skipped);
            } else if is_file {
                let Some(stem) = rel.strip_suffix(".md").or_else(|| rel.strip_suffix(".MD")) else { continue };
                if normalize_path(stem).as_deref() != Ok(stem) {
                    *skipped += 1;
                    continue;
                }
                let mtime = mtime(&abs);
                files.insert(stem.to_lowercase(), DiskFile { rel: stem.to_string(), abs, mtime });
            }
        }
    }
    rec(root, "", 0, &mut files, &mut dirs, &mut skipped);
    (files, dirs, skipped)
}

fn read(f: &DiskFile) -> Option<String> {
    std::fs::read(&f.abs).ok().and_then(|b| String::from_utf8(b).ok())
}

/// Écrit via un fichier temporaire caché puis renomme : Obsidian ne voit jamais de fichier à moitié écrit.
pub(crate) fn write_file(abs: &Path, content: &str) -> std::io::Result<Option<SystemTime>> {
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("note.md");
    let tmp = abs.with_file_name(format!(".{name}.mnemo-tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, abs)?;
    Ok(mtime(abs))
}

/// Écrit le fichier d'une note seulement s'il n'a pas bougé depuis le parcours du dossier (`seen` : sa date à ce
/// moment-là, `None` s'il n'existait pas). `Ok(None)` : il vient de changer (enregistrement en cours dans
/// l'éditeur), il n'est pas touché et sera repris au passage suivant.
fn write_if_unchanged(
    shared: &crate::local::shared::SharedState,
    abs: &Path,
    seen: Option<SystemTime>,
    content: &str,
) -> std::io::Result<Option<Option<SystemTime>>> {
    let _io = shared.io_lock.lock().expect("io lock");
    if mtime(abs) != seen {
        return Ok(None);
    }
    write_file(abs, content).map(Some)
}

/// Garde un texte dans une nouvelle note à côté de l'originale : « Note (conflit disque) », puis
/// « Note (conflit disque 2) »… Elle sera envoyée à la base comme toute nouvelle note.
fn conflict_note(root: &Path, path: &str, text: &str) -> std::io::Result<()> {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{path} (conflit disque)") } else { format!("{path} (conflit disque {n})") };
        let abs = note_file(root, &name);
        if !abs.exists() {
            return write_file(&abs, text).map(|_| ());
        }
        n += 1;
    }
}

/// Garde un texte écarté à la première rencontre dans `.trash/` (rien n'est détruit sans trace).
fn keep_in_trash(root: &Path, path: &str, text: &str) {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let _ = write_file(&note_file(&root.join(".trash"), &format!("{path} (remplacée {stamp})")), text);
}

/// Fusion à 3 voies ; `None` si les deux côtés ont changé les mêmes lignes.
fn merge3(base: &str, disk: &str, db: &str) -> Option<String> {
    let blocks = crate::merge::build(base, disk, db);
    if crate::merge::conflict_count(&blocks).0 > 0 {
        return None;
    }
    crate::merge::assemble(&blocks)
}

/// Met le contenu d'une note à jour dans la base si elle y est toujours à la version attendue.
/// `None` : elle a changé entre-temps (autre appareil), rien n'est écrit.
async fn push(t: &tokio_postgres::Transaction<'_>, id: Uuid, text: &str, version: i32) -> Res<Option<i32>> {
    Ok(t.query_opt(
        "UPDATE notes SET content = $2 WHERE id = $1 AND version = $3 RETURNING version",
        &[&id, &text, &version],
    )
    .await?
    .map(|r| r.get(0)))
}

/// Déplace un fichier dans `<coffre>/.trash/`, comme la corbeille d'Obsidian.
fn trash(root: &Path, f: &DiskFile) {
    let mut dest = note_file(&root.join(".trash"), &f.rel);
    if dest.exists() {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        dest = note_file(&root.join(".trash"), &format!("{} {stamp}", f.rel));
    }
    if let Some(dir) = dest.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::rename(&f.abs, &dest).is_err() {
        let _ = std::fs::remove_file(&f.abs);
    }
}

/// Tous les dossiers parents (en minuscules) d'un chemin de note.
fn ancestors(path: &str, out: &mut HashSet<String>) {
    let mut p = path;
    while let Some(i) = p.rfind('/') {
        p = &p[..i];
        out.insert(p.to_lowercase());
    }
}

enum Plan {
    /// la base a changé : réécrire le fichier
    Write,
    /// le fichier a changé : mettre la base à jour si le texte diffère
    Pull,
    /// les deux ont changé
    Both,
    /// première rencontre, présent des deux côtés : le plus récent l'emporte (l'autre texte va dans .trash)
    Reconcile(chrono::DateTime<chrono::Utc>),
    /// fichier supprimé du disque
    DeleteNote,
}

impl Worker {
    pub(super) async fn vault_scan(&mut self, em: &Emitter) -> Res<()> {
        let Some(mut v) = self.vault.take() else { return Ok(()) };
        let r = self.vault_scan_in(&mut v, em).await;
        self.vault = Some(v);
        r
    }

    async fn vault_scan_in(&mut self, v: &mut Vault, em: &Emitter) -> Res<()> {
        if !v.root.is_dir() {
            if v.first {
                v.first = false;
                em.emit(Msg::Error(format!("Dossier du coffre introuvable : {}", v.root.display())));
            }
            return Ok(());
        }
        let root = v.root.clone();
        // l'état connu ne vaut que pour la base avec laquelle il a été établi
        if !v.db_checked {
            let db_id: String =
                self.conn().query_one("SELECT value FROM mnemo_meta WHERE key = 'db_id'", &[]).await?.get(0);
            if v.db_id != db_id {
                v.forget();
                v.db_id = db_id;
            }
            v.db_checked = true;
        }
        let mut dirty = false;
        let (mut disk, dirs, skipped) = walk(&root);
        let mut stats = VaultStats { skipped, ..Default::default() };

        let rows = self.conn().query("SELECT id, path, version, updated_at FROM notes", &[]).await?;
        let db_ids: HashSet<Uuid> = rows.iter().map(|r| r.get(0)).collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut plans: Vec<(Uuid, String, Plan)> = vec![];

        for r in &rows {
            let (id, path, version): (Uuid, String, i32) = (r.get(0), r.get(1), r.get(2));
            let key = path.to_lowercase();
            // chemin que Mnemo n'accepte pas (« .. », « \\ », « # »… écrit par un autre client ou en SQL) :
            // la note reste dans la base, mais rien n'est écrit ni supprimé pour elle sur ce disque
            if normalize_path(&path).as_deref() != Ok(path.as_str()) {
                seen.insert(key);
                stats.skipped += 1;
                continue;
            }
            if let Some(k) = v.known.get_mut(&id) {
                if k.path != path {
                    // renommée / déplacée dans Mnemo : on déplace le fichier
                    let old_key = k.path.to_lowercase();
                    if (old_key == key || !disk.contains_key(&key))
                        && let Some(old) = disk.remove(&old_key)
                    {
                        let abs = note_file(&root, &path);
                        if let Some(dir) = abs.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        // l'id suit le fichier avant qu'il bouge : le stockage local garde la même note
                        self.shared.ids.lock().expect("ids").rename(&k.path, &path, false, true);
                        if std::fs::rename(&old.abs, &abs).is_ok() {
                            stats.written += 1;
                            let m = mtime(&abs);
                            if old.mtime == k.mtime {
                                k.mtime = m;
                            }
                            disk.insert(key.clone(), DiskFile { rel: path.clone(), abs, mtime: m });
                        } else {
                            disk.insert(old_key, old);
                        }
                    }
                    k.path = path.clone();
                    dirty = true;
                }
                seen.insert(key.clone());
                let db_changed = version != k.version;
                let plan = match disk.get(&key) {
                    None if db_changed => Plan::Write,
                    None => Plan::DeleteNote,
                    Some(f) => match (db_changed, f.mtime != k.mtime) {
                        (false, false) => continue,
                        (true, false) => Plan::Write,
                        (false, true) => Plan::Pull,
                        (true, true) => Plan::Both,
                    },
                };
                plans.push((id, path, plan));
            } else {
                seen.insert(key.clone());
                let plan = if disk.contains_key(&key) { Plan::Reconcile(r.get(3)) } else { Plan::Write };
                plans.push((id, path, plan));
            }
        }

        // version et contenu en base des notes concernées, lus ensemble en une requête
        let need: Vec<Uuid> = plans.iter().map(|p| p.0).collect();
        let mut contents: HashMap<Uuid, (i32, String)> = HashMap::new();
        if !need.is_empty() {
            for r in self.conn().query("SELECT id, version, content FROM notes WHERE id = ANY($1)", &[&need]).await? {
                contents.insert(r.get(0), (r.get(1), r.get(2)));
            }
        }

        // notes supprimées dans Mnemo depuis le dernier passage
        let gone: Vec<Uuid> = v.known.keys().filter(|id| !db_ids.contains(id)).copied().collect();
        for id in gone {
            let k = v.known.remove(&id).expect("known");
            let _ = std::fs::remove_file(base_dir(&root).join(format!("{id}.md")));
            dirty = true;
            let key = k.path.to_lowercase();
            if seen.contains(&key) {
                continue; // une autre note occupe désormais ce chemin
            }
            if let Some(f) = disk.get(&key)
                && f.mtime == k.mtime
            {
                trash(&root, f);
                disk.remove(&key);
                seen.insert(key);
                stats.removed += 1;
            }
            // modifié sur le disque entre-temps : il sera réimporté ci-dessous
        }

        let tag = format!("{}#disk", self.client_id);
        let shared = self.shared.clone();
        let t = tagged_tx(self.client.as_mut().expect("connected"), &tag).await?;
        // dossiers qui contiennent des notes : ils existent implicitement des deux côtés
        let mut note_dirs: HashSet<String> = HashSet::new();
        for r in &rows {
            ancestors(r.get::<_, &str>(1), &mut note_dirs);
        }

        // état connu après ce passage (`None` : note supprimée) avec le texte commun aux deux côtés ;
        // appliqué seulement une fois la transaction validée
        let mut pending: Vec<(Uuid, Option<(Known, String)>)> = vec![];

        for (id, path, plan) in plans {
            let abs = note_file(&root, &path);
            // supprimée de la base depuis la première requête : traitée au passage suivant
            let Some((version, db_text)) = contents.remove(&id) else { continue };
            let disk_file = disk.get(&path.to_lowercase());
            let on_disk = disk_file.and_then(|f| f.mtime);
            let known = |version: i32, mtime: Option<SystemTime>, text: &str| {
                (id, Some((Known { path: path.clone(), version, mtime }, text.to_string())))
            };
            // la note garde en local l'identifiant qu'elle a dans la base
            shared.ids.lock().expect("ids").set(&path, id, true);
            match plan {
                Plan::Write => match write_if_unchanged(&shared, &abs, on_disk, &db_text) {
                    Ok(Some(m)) => {
                        stats.written += 1;
                        pending.push(known(version, m, &db_text));
                    }
                    Ok(None) => {}
                    Err(e) => em.emit(Msg::Error(format!("Écriture de {} impossible : {e}", abs.display()))),
                },
                Plan::DeleteNote => {
                    // seulement si la note n'a pas changé ailleurs entre-temps (sinon elle sera réécrite)
                    let n = t.execute("DELETE FROM notes WHERE id = $1 AND version = $2", &[&id, &version]).await?;
                    if n > 0 {
                        pending.push((id, None));
                        stats.removed += 1;
                    }
                }
                Plan::Pull | Plan::Both | Plan::Reconcile(_) => {
                    let Some(f) = disk_file else { continue };
                    let Some(text) = read(f) else {
                        stats.skipped += 1;
                        continue;
                    };
                    if text == db_text {
                        // rien à recopier (fichier simplement « touché », ou même changement des deux côtés)
                        pending.push(known(version, f.mtime, &text));
                        continue;
                    }
                    match plan {
                        Plan::Pull => {
                            // `None` : la base a changé pendant ce passage, fusion au suivant
                            if let Some(nv) = push(&t, id, &text, version).await? {
                                stats.pulled += 1;
                                pending.push(known(nv, f.mtime, &text));
                            }
                        }
                        Plan::Both => {
                            let merged = read_base(&root, id).and_then(|base| merge3(&base, &text, &db_text));
                            match merged {
                                // changements à des endroits différents : les deux côtés reçoivent la fusion
                                Some(m) => {
                                    let nv =
                                        if m == db_text { Some(version) } else { push(&t, id, &m, version).await? };
                                    let Some(nv) = nv else { continue };
                                    stats.pulled += 1;
                                    if m == text {
                                        pending.push(known(nv, f.mtime, &m));
                                    } else if let Ok(Some(mt)) = write_if_unchanged(&shared, &abs, f.mtime, &m) {
                                        stats.written += 1;
                                        pending.push(known(nv, mt, &m));
                                    }
                                }
                                // mêmes lignes changées des deux côtés : la base l'emporte, le texte du disque est
                                // d'abord gardé dans une note à part
                                None => {
                                    let _io = shared.io_lock.lock().expect("io lock");
                                    if mtime(&abs) == f.mtime
                                        && conflict_note(&root, &path, &text).is_ok()
                                        && let Ok(mt) = write_file(&abs, &db_text)
                                    {
                                        stats.written += 1;
                                        pending.push(known(version, mt, &db_text));
                                    }
                                }
                            }
                        }
                        Plan::Reconcile(updated) => {
                            // première rencontre, textes différents : le plus récent l'emporte, l'autre va dans .trash
                            let disk_newer =
                                f.mtime.map(chrono::DateTime::<chrono::Utc>::from).is_some_and(|m| m > updated);
                            if disk_newer {
                                if let Some(nv) = push(&t, id, &text, version).await? {
                                    keep_in_trash(&root, &path, &db_text);
                                    stats.pulled += 1;
                                    pending.push(known(nv, f.mtime, &text));
                                }
                            } else {
                                let _io = shared.io_lock.lock().expect("io lock");
                                if mtime(&abs) == f.mtime {
                                    keep_in_trash(&root, &path, &text);
                                    if let Ok(mt) = write_file(&abs, &db_text) {
                                        stats.written += 1;
                                        pending.push(known(version, mt, &db_text));
                                    }
                                }
                            }
                        }
                        Plan::Write | Plan::DeleteNote => unreachable!(),
                    }
                }
            }
        }

        // fichiers nouveaux sur le disque : import
        let mut fresh: Vec<&DiskFile> = disk.iter().filter(|(k, _)| !seen.contains(*k)).map(|(_, f)| f).collect();
        fresh.sort_by(|a, b| a.rel.cmp(&b.rel));
        for f in fresh {
            let Some(text) = read(f) else {
                stats.skipped += 1;
                continue;
            };
            // la note garde dans la base l'identifiant qu'elle a en local ; si cet id désigne déjà une autre note
            // de la base (copie restée d'une note renommée ailleurs), le fichier devient une note à part
            let mut id = shared.ids.lock().expect("ids").get_or_create(&f.rel);
            if db_ids.contains(&id) {
                id = Uuid::new_v4();
                shared.ids.lock().expect("ids").set(&f.rel, id, true);
            }
            let row = t
                .query_opt(
                    "INSERT INTO notes (id, path, content) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING id, version",
                    &[&id, &f.rel, &text],
                )
                .await?;
            if let Some(row) = row {
                let id: Uuid = row.get(0);
                pending.push((id, Some((Known { path: f.rel.clone(), version: row.get(1), mtime: f.mtime }, text))));
                stats.imported += 1;
                ancestors(&f.rel, &mut note_dirs);
            }
        }

        // dossiers
        let db_folders: Vec<String> =
            t.query("SELECT path FROM folders", &[]).await?.iter().map(|r| r.get(0)).collect();
        let db_set: HashMap<String, &String> = db_folders.iter().map(|f| (f.to_lowercase(), f)).collect();
        let disk_set: HashMap<String, &String> = dirs.iter().map(|d| (d.to_lowercase(), d)).collect();
        let mut kept: HashSet<String> = note_dirs.clone();
        let mut to_remove: Vec<&String> = vec![];
        for (lk, d) in &disk_set {
            if db_set.contains_key(lk) {
                kept.insert(lk.clone());
            } else if note_dirs.contains(lk) {
                // dossier qui contient des notes : il existe déjà implicitement dans Mnemo
            } else if v.folders.contains(lk) && v.synced {
                to_remove.push(d); // supprimé dans Mnemo
            } else {
                t.execute("INSERT INTO folders (path) VALUES ($1) ON CONFLICT DO NOTHING", &[d]).await?;
                kept.insert(lk.clone());
            }
        }
        // les plus profonds d'abord : remove_dir n'efface que les dossiers vides
        to_remove.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
        for d in to_remove {
            if std::fs::remove_dir(dir_path(&root, d)).is_ok() {
                stats.removed += 1;
            }
        }
        for (lk, f) in &db_set {
            if disk_set.contains_key(lk) {
                continue;
            }
            if v.folders.contains(lk) && v.synced {
                t.execute("DELETE FROM folders WHERE lower(path) = $1", &[lk]).await?; // supprimé sur le disque
                stats.removed += 1;
            } else if std::fs::create_dir_all(dir_path(&root, f)).is_ok() {
                kept.insert(lk.clone());
                stats.written += 1;
            }
        }
        // icônes et réglages des plugins : .mnemo/icons.json et plugins.json ↔ tables de la base
        let mut kv_pending: Vec<(&str, Map<String, Value>)> = vec![];
        for name in ["icons", "plugins"] {
            let remote: Map<String, Value> = if name == "icons" {
                t.query("SELECT path, icon FROM path_icons", &[])
                    .await?
                    .iter()
                    .map(|r| (r.get::<_, String>(0), Value::String(r.get(1))))
                    .collect()
            } else {
                t.query("SELECT id, enabled, settings FROM plugin_state", &[])
                    .await?
                    .iter()
                    .map(|r| {
                        let v = serde_json::json!({ "enabled": r.get::<_, bool>(1), "settings": r.get::<_, Value>(2) });
                        (r.get::<_, String>(0), v)
                    })
                    .collect()
            };
            let push = {
                let _guard = shared.state_lock.lock().expect("state lock");
                let local = crate::local::shared::read_state(&root, name);
                let (result, push) = merge_kv(v.kv_known.get(name), &local, &remote);
                if result != local {
                    crate::local::shared::write_state(&root, name, &result);
                    stats.written += 1;
                }
                if v.kv_known.get(name) != Some(&result) {
                    kv_pending.push((name, result));
                }
                push
            };
            for (key, val) in push {
                stats.pulled += 1;
                match (name, val) {
                    ("icons", Some(Value::String(icon))) => {
                        t.execute(
                            "INSERT INTO path_icons (path, icon) VALUES ($1, $2) ON CONFLICT (path) DO UPDATE SET icon = EXCLUDED.icon",
                            &[&key, &icon],
                        )
                        .await?;
                    }
                    ("icons", _) => {
                        t.execute("DELETE FROM path_icons WHERE path = $1", &[&key]).await?;
                    }
                    (_, Some(v)) => {
                        let enabled = v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
                        let settings = v.get("settings").cloned().unwrap_or_else(|| serde_json::json!({}));
                        t.execute(
                            "INSERT INTO plugin_state (id, enabled, settings) VALUES ($1, $2, $3)
                             ON CONFLICT (id) DO UPDATE SET enabled = EXCLUDED.enabled, settings = EXCLUDED.settings",
                            &[&key, &enabled, &settings],
                        )
                        .await?;
                    }
                    (_, None) => {
                        t.execute("DELETE FROM plugin_state WHERE id = $1", &[&key]).await?;
                    }
                }
            }
        }
        let file_updates = sync_files(&t, v, &root, &mut stats).await?;
        t.commit().await?;

        // la transaction est validée : l'état connu peut avancer
        dirty |= !pending.is_empty() || !kv_pending.is_empty() || !file_updates.is_empty() || v.folders != kept;
        dirty |= !v.synced;
        for (id, k) in pending {
            let base = base_dir(&root).join(format!("{id}.md"));
            match k {
                Some((k, text)) => {
                    let _ = crate::local::shared::write_atomic(&base, &text);
                    v.known.insert(id, k);
                }
                None => {
                    let _ = std::fs::remove_file(&base);
                    v.known.remove(&id);
                }
            }
        }
        for (name, map) in kv_pending {
            v.kv_known.insert(name.to_string(), map);
        }
        for (path, k) in file_updates {
            match k {
                Some(k) => v.file_known.insert(path, k),
                None => v.file_known.remove(&path),
            };
        }
        v.folders = kept;
        v.synced = true;
        if dirty {
            v.save();
        }

        let first = std::mem::replace(&mut v.first, false);
        if first || stats.changed() {
            em.emit(Msg::VaultSynced { stats, first });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_file_keeps_dots() {
        let p = note_file(Path::new("/v"), "dossier/notes v1.2");
        assert_eq!(p, PathBuf::from("/v/dossier/notes v1.2.md"));
    }

    #[test]
    fn walk_skips_hidden_and_invalid() {
        let root = std::env::temp_dir().join(format!("mnemo-walk-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(root.join("Projets/Vide")).unwrap();
        std::fs::write(root.join(".obsidian/app.md"), "x").unwrap();
        std::fs::write(root.join("Projets/Plan.md"), "plan").unwrap();
        std::fs::write(root.join("Accueil.md"), "salut").unwrap();
        std::fs::write(root.join("image.png"), [0u8]).unwrap();
        std::fs::write(root.join("bad[1].md"), "x").unwrap();
        let (files, mut dirs, skipped) = walk(&root);
        dirs.sort();
        assert_eq!(files.len(), 2);
        assert_eq!(files["projets/plan"].rel, "Projets/Plan");
        assert!(files.contains_key("accueil"));
        assert_eq!(dirs, vec!["Projets".to_string(), "Projets/Vide".to_string()]);
        assert_eq!(skipped, 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn file_plans() {
        use FilePlan::*;
        assert_eq!(plan_file(None, Some("a"), None), Upload);
        assert_eq!(plan_file(None, None, Some("a")), Download);
        assert_eq!(plan_file(None, Some("a"), Some("b")), Conflict);
        assert_eq!(plan_file(Some("a"), Some("a"), Some("a")), Nothing);
        assert_eq!(plan_file(Some("a"), Some("b"), Some("a")), Upload);
        assert_eq!(plan_file(Some("a"), Some("a"), Some("b")), Download);
        assert_eq!(plan_file(Some("a"), None, Some("a")), DeleteRemote);
        assert_eq!(plan_file(Some("a"), Some("a"), None), DeleteLocal);
        assert_eq!(plan_file(Some("a"), Some("b"), None), Upload); // modifiée ici, supprimée ailleurs : gardée
        assert_eq!(plan_file(Some("a"), Some("b"), Some("c")), Conflict);
        assert_eq!(conflict_name("D/photo.png"), "D/photo (conflit disque).png");
        assert!(safe_rel("Pièces jointes/photo 1.png"));
        for bad in ["../x", "a/../../x", "/etc/x", "C:/x", "a\\b", ".mnemo/ids.json", "a//b"] {
            assert!(!safe_rel(bad), "{bad}");
        }
    }

    #[test]
    fn kv_merge() {
        let m = |pairs: &[(&str, &str)]| -> Map<String, Value> {
            pairs.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect()
        };
        // premier passage : union, la base l'emporte sur une même clé
        let (r, push) = merge_kv(None, &m(&[("a", "1"), ("b", "local")]), &m(&[("b", "base"), ("c", "3")]));
        assert_eq!(r, m(&[("a", "1"), ("b", "base"), ("c", "3")]));
        assert_eq!(push, vec![("a".to_string(), Some(Value::String("1".into())))]);
        // ensuite : chaque côté propage ses changements, suppressions comprises
        let base = m(&[("a", "1"), ("b", "2")]);
        let (r, push) = merge_kv(Some(&base), &m(&[("a", "1"), ("b", "2"), ("n", "new")]), &m(&[("a", "1")]));
        assert_eq!(r, m(&[("a", "1"), ("n", "new")])); // b supprimée dans la base, n ajoutée en local
        assert_eq!(push, vec![("n".to_string(), Some(Value::String("new".into())))]);
        let (_, push) = merge_kv(Some(&base), &m(&[("a", "1")]), &base);
        assert_eq!(push, vec![("b".to_string(), None)]); // supprimée en local
    }

    #[test]
    fn ancestors_lists_parents() {
        let mut s = HashSet::new();
        ancestors("A/B/c", &mut s);
        assert!(s.contains("a") && s.contains("a/b") && s.len() == 2);
    }
}
