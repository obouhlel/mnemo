//! Miroir d'un dossier de coffre (Obsidian) sur le disque.
//!
//! Quand un dossier est lié, le worker le compare régulièrement à la base (`Cmd::VaultScan`) :
//! - les fichiers `.md` du dossier absents de la base sont importés ;
//! - les notes de la base absentes du dossier y sont écrites (notes créées dans Mnemo comprises) ;
//! - une modification d'un côté est recopiée de l'autre : contenu, renommage, suppression
//!   (les fichiers supprimés depuis Mnemo vont dans `.trash/` du coffre, comme dans Obsidian) ;
//! - les dossiers vides sont créés / supprimés des deux côtés.
//!
//! L'état connu (`Known` : version en base, date de modification du fichier, empreinte du
//! contenu) sert à savoir de quel côté vient chaque changement. Si les deux côtés ont changé
//! depuis la dernière synchronisation, la base l'emporte et le texte du disque est conservé
//! dans une note « (conflit disque) ».
//! Les dossiers cachés (`.obsidian`, `.trash`, `.git`…) et les fichiers autres que `.md` sont ignorés.

use super::*;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
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
    first: bool,
    /// icônes / réglages des plugins tels qu'au dernier passage (base de la fusion à 3 voies)
    kv_known: HashMap<&'static str, Map<String, Value>>,
}

impl Vault {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root, known: HashMap::new(), folders: HashSet::new(), first: true, kv_known: HashMap::new() }
    }

    /// Oublie l'état connu : le prochain passage recompare tout (reconnexion à la base).
    pub(super) fn reset(&mut self) {
        self.known.clear();
        self.folders.clear();
        self.kv_known.clear();
        self.first = true;
    }
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

struct Known {
    path: String,
    version: i32,
    mtime: Option<SystemTime>,
    hash: u64,
}

#[derive(Debug)]
pub(crate) struct DiskFile {
    pub(crate) rel: String,
    pub(crate) abs: PathBuf,
    pub(crate) mtime: Option<SystemTime>,
}

fn hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
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
    /// première rencontre, présent des deux côtés : le plus récent l'emporte
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
        let (mut disk, dirs, skipped) = walk(&root);
        let mut stats = VaultStats { skipped, ..Default::default() };

        let rows = self.conn().query("SELECT id, path, version, updated_at FROM notes", &[]).await?;
        let db_ids: HashSet<Uuid> = rows.iter().map(|r| r.get(0)).collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut plans: Vec<(Uuid, String, i32, Plan)> = vec![];

        for r in &rows {
            let (id, path, version): (Uuid, String, i32) = (r.get(0), r.get(1), r.get(2));
            let key = path.to_lowercase();
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
                plans.push((id, path, version, plan));
            } else {
                seen.insert(key.clone());
                let plan = if disk.contains_key(&key) { Plan::Reconcile(r.get(3)) } else { Plan::Write };
                plans.push((id, path, version, plan));
            }
        }

        // contenu en base des notes concernées, en une requête
        let need: Vec<Uuid> = plans.iter().filter(|p| !matches!(p.3, Plan::DeleteNote)).map(|p| p.0).collect();
        let mut contents: HashMap<Uuid, String> = HashMap::new();
        if !need.is_empty() {
            for r in self.conn().query("SELECT id, content FROM notes WHERE id = ANY($1)", &[&need]).await? {
                contents.insert(r.get(0), r.get(1));
            }
        }

        // notes supprimées dans Mnemo depuis le dernier passage
        let gone: Vec<Uuid> = v.known.keys().filter(|id| !db_ids.contains(id)).copied().collect();
        for id in gone {
            let k = v.known.remove(&id).expect("known");
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
        let t = tagged_tx(self.client.as_mut().expect("connected"), &tag).await?;
        // dossiers qui contiennent des notes : ils existent implicitement des deux côtés
        let mut note_dirs: HashSet<String> = HashSet::new();
        for r in &rows {
            ancestors(r.get::<_, &str>(1), &mut note_dirs);
        }

        for (id, path, version, plan) in plans {
            let abs = note_file(&root, &path);
            let db_text = contents.remove(&id).unwrap_or_default();
            let record = |v: &mut Vault, version: i32, mtime: Option<SystemTime>, text: &str| {
                v.known.insert(id, Known { path: path.clone(), version, mtime, hash: hash(text) });
            };
            let disk_file = disk.get(&path.to_lowercase());
            // la note garde en local l'identifiant qu'elle a dans la base
            self.shared.ids.lock().expect("ids").set(&path, id, true);
            match plan {
                Plan::Write => match write_file(&abs, &db_text) {
                    Ok(m) => {
                        stats.written += 1;
                        record(v, version, m, &db_text);
                    }
                    Err(e) => em.emit(Msg::Error(format!("Écriture de {} impossible : {e}", abs.display()))),
                },
                Plan::DeleteNote => {
                    t.execute("DELETE FROM notes WHERE id = $1", &[&id]).await?;
                    v.known.remove(&id);
                    stats.removed += 1;
                    continue;
                }
                Plan::Pull | Plan::Both | Plan::Reconcile(_) => {
                    let Some(f) = disk_file else { continue };
                    let Some(text) = read(f) else {
                        stats.skipped += 1;
                        continue;
                    };
                    let known_hash = v.known.get(&id).map(|k| k.hash);
                    let disk_newer = match plan {
                        Plan::Reconcile(updated) => {
                            f.mtime.map(chrono::DateTime::<chrono::Utc>::from).is_some_and(|m| m > updated)
                        }
                        _ => true,
                    };
                    if text == db_text || (matches!(plan, Plan::Pull) && known_hash == Some(hash(&text))) {
                        // rien à recopier (fichier simplement « touché »)
                        record(v, version, f.mtime, &text);
                    } else if matches!(plan, Plan::Both) {
                        // les deux côtés ont changé : la base l'emporte, le texte du disque est gardé à part
                        let copy = format!("{path} (conflit disque)");
                        t.execute(
                            "INSERT INTO notes (path, content) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                            &[&copy, &text],
                        )
                        .await?;
                        stats.imported += 1;
                        if let Ok(m) = write_file(&abs, &db_text) {
                            stats.written += 1;
                            record(v, version, m, &db_text);
                        }
                    } else if disk_newer {
                        let nv: Option<i32> = t
                            .query_opt("UPDATE notes SET content = $2 WHERE id = $1 RETURNING version", &[&id, &text])
                            .await?
                            .map(|r| r.get(0));
                        stats.pulled += 1;
                        record(v, nv.unwrap_or(version), f.mtime, &text);
                    } else if let Ok(m) = write_file(&abs, &db_text) {
                        stats.written += 1;
                        record(v, version, m, &db_text);
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
            let mut id = self.shared.ids.lock().expect("ids").get_or_create(&f.rel);
            if db_ids.contains(&id) {
                id = Uuid::new_v4();
                self.shared.ids.lock().expect("ids").set(&f.rel, id, true);
            }
            let row = t
                .query_opt(
                    "INSERT INTO notes (id, path, content) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING id, version",
                    &[&id, &f.rel, &text],
                )
                .await?;
            if let Some(row) = row {
                let id: Uuid = row.get(0);
                v.known
                    .insert(id, Known { path: f.rel.clone(), version: row.get(1), mtime: f.mtime, hash: hash(&text) });
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
            } else if v.folders.contains(lk) && !v.first {
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
            if v.folders.contains(lk) && !v.first {
                t.execute("DELETE FROM folders WHERE lower(path) = $1", &[lk]).await?; // supprimé sur le disque
                stats.removed += 1;
            } else if std::fs::create_dir_all(dir_path(&root, f)).is_ok() {
                kept.insert(lk.clone());
                stats.written += 1;
            }
        }
        // icônes et réglages des plugins : .mnemo/icons.json et plugins.json ↔ tables de la base
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
                let _guard = self.shared.state_lock.lock().expect("state lock");
                let local = crate::local::shared::read_state(&root, name);
                let (result, push) = merge_kv(v.kv_known.get(name), &local, &remote);
                if result != local {
                    crate::local::shared::write_state(&root, name, &result);
                    stats.written += 1;
                }
                v.kv_known.insert(name, result);
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
        t.commit().await?;
        v.folders = kept;

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
