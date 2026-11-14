//! Tests de bout en bout de l'écriture simultanée : le stockage local et, avec une vraie base PostgreSQL,
//! deux « appareils » (chacun son dossier, son stockage local et son worker) qui modifient la même note.
//!
//! Les tests à deux appareils ne tournent que si `MNEMO_TEST_PG` contient l'adresse d'un serveur PostgreSQL
//! où créer des bases, par exemple `postgres://postgres:postgres@localhost:5432` (`make db`).

use crate::db::{Cmd, DbHandle, LoadPurpose, Msg};
use crate::model::Note;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use uuid::Uuid;

const BASE: &str = "ligne 1\nligne 2\nligne 3\nligne 4\nligne 5\nligne 6\n";

struct Device {
    db: DbHandle,
    rx: Receiver<Msg>,
    root: PathBuf,
    client_id: String,
}

impl Device {
    /// Lance un appareil sur un dossier neuf ; `url` vide : sans base.
    fn start(name: &str, url: &str) -> Self {
        let root = std::env::temp_dir().join(format!("mnemo-sync-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self::open(root, url)
    }

    /// (Re)lance un appareil sur un dossier existant, comme au redémarrage de Mnemo.
    fn open(root: PathBuf, url: &str) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let client_id = Uuid::new_v4().to_string();
        let db = crate::db::spawn(client_id.clone(), tx, || {}, crate::local::shared::new_shared());
        db.send(Cmd::SetRoot(root.clone()));
        db.send(Cmd::SetVault(Some(root.clone())));
        let d = Self { db, rx, root, client_id };
        d.wait("dossier indexé", |m| matches!(m, Msg::LocalReady).then_some(()));
        if !url.is_empty() {
            d.db.send(Cmd::Connect(url.to_string()));
            d.wait("première synchronisation", |m| matches!(m, Msg::VaultSynced { .. }).then_some(()));
        }
        d
    }

    /// Attend un message ; comme l'interface, demande un passage de synchronisation à chaque événement
    /// venu d'un autre appareil.
    fn wait<T>(&self, what: &str, mut f: impl FnMut(&Msg) -> Option<T>) -> T {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let left = end.checked_duration_since(Instant::now()).unwrap_or_else(|| panic!("délai dépassé : {what}"));
            let Ok(m) = self.rx.recv_timeout(left) else { panic!("délai dépassé : {what}") };
            self.relay(&m);
            if let Some(v) = f(&m) {
                return v;
            }
        }
    }

    fn relay(&self, m: &Msg) {
        if let Msg::Event(ev) = m
            && !ev.updated_by.as_deref().is_some_and(|u| u.starts_with(&self.client_id))
        {
            self.db.send(Cmd::VaultScan);
        }
    }

    /// Traite les messages en attente pendant `d`.
    fn pump(&self, d: Duration) {
        let end = Instant::now() + d;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if let Ok(m) = self.rx.recv_timeout(left) {
                self.relay(&m);
            }
        }
    }

    fn file(&self, path: &str) -> PathBuf {
        crate::db::note_file(&self.root, path)
    }

    fn text(&self, path: &str) -> String {
        std::fs::read_to_string(self.file(path)).unwrap_or_default()
    }

    /// Notes du dossier (chemins sans extension), triées.
    fn notes(&self) -> Vec<String> {
        let mut v: Vec<String> = crate::db::walk_markdown(&self.root).0.into_iter().map(|(p, _)| p).collect();
        v.sort();
        v
    }

    /// La note telle que l'éditeur l'ouvrirait.
    fn load(&self, path: &str) -> Note {
        // le stockage local relit le disque chaque seconde : la note peut ne pas être encore indexée
        let end = Instant::now() + Duration::from_secs(10);
        let id = loop {
            self.db.send(Cmd::ListNotes);
            let id = self.wait("liste des notes", |m| match m {
                Msg::Notes(n) => Some(n.iter().find(|n| n.path == path).map(|n| n.id)),
                _ => None,
            });
            match id {
                Some(id) => break id,
                None => assert!(Instant::now() < end, "note « {path} » absente"),
            }
            self.pump(Duration::from_millis(200));
        };
        self.db.send(Cmd::Load { id, purpose: LoadPurpose::Open });
        self.wait("note chargée", |m| match m {
            Msg::Loaded { note, .. } if note.id == id => Some(note.clone()),
            _ => None,
        })
    }

    /// Enregistre comme l'éditeur ; renvoie `true` si c'est accepté, `false` si un conflit est signalé.
    fn save(&self, note: &Note, content: &str) -> bool {
        self.db.send(Cmd::Save { id: note.id, content: content.to_string(), base_version: note.version });
        self.wait("enregistrement", |m| match m {
            Msg::Saved(n) if n.id == note.id => Some(true),
            Msg::Conflict(n) if n.id == note.id => Some(false),
            _ => None,
        })
    }

    /// Modifie une ligne de la note, comme une frappe suivie de l'enregistrement automatique.
    fn edit(&self, path: &str, from: &str, to: &str) {
        // un conflit signalé (la note vient de changer sur le disque) : l'éditeur fusionne et réenregistre
        for _ in 0..20 {
            let note = self.load(path);
            assert!(note.content.contains(from), "« {from} » absent de {:?}", note.content);
            if self.save(&note, &note.content.replace(from, to)) {
                return;
            }
        }
        panic!("enregistrement toujours refusé");
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Attend que les deux appareils aient le même texte, qui vérifie `ok`.
fn converge(a: &Device, b: &Device, path: &str, ok: impl Fn(&str) -> bool) -> String {
    let end = Instant::now() + Duration::from_secs(25);
    loop {
        a.pump(Duration::from_millis(150));
        b.pump(Duration::from_millis(150));
        let (ta, tb) = (a.text(path), b.text(path));
        if ta == tb && ok(&ta) {
            return ta;
        }
        assert!(Instant::now() < end, "pas de convergence sur « {path} »\n--- A\n{ta}\n--- B\n{tb}");
    }
}

/// Crée une base vide pour un test ; `None` si `MNEMO_TEST_PG` n'est pas défini.
fn test_db() -> Option<String> {
    let server = std::env::var("MNEMO_TEST_PG").ok().filter(|s| !s.trim().is_empty())?;
    let server = server.trim_end_matches('/').to_string();
    let name = format!("mnemo_test_{}", Uuid::new_v4().simple());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (client, conn) =
            tokio_postgres::connect(&format!("{server}/postgres"), tokio_postgres::NoTls).await.expect("MNEMO_TEST_PG");
        tokio::spawn(conn);
        client.batch_execute(&format!("CREATE DATABASE {name}")).await.expect("create database");
    });
    Some(format!("{server}/{name}"))
}

/// Deux appareils synchronisés qui ont la même note « Partage ».
fn pair(url: &str) -> (Device, Device) {
    let a = Device::start("a", "");
    std::fs::write(a.file("Partage"), BASE).unwrap();
    let a = Device::open(a.root.clone(), url).forget_drop(a);
    let b = Device::start("b", url);
    converge(&a, &b, "Partage", |t| t == BASE);
    (a, b)
}

impl Device {
    /// Remplace `old` (même dossier) sans que sa destruction efface le dossier.
    fn forget_drop(self, old: Device) -> Device {
        std::mem::forget(old);
        self
    }
}

#[test]
fn save_does_not_overwrite_a_change_made_outside() {
    // un autre éditeur (ou la synchronisation) écrit le fichier, puis l'éditeur de Mnemo enregistre avant
    // que le stockage local ait relu le disque : le changement extérieur ne doit pas être écrasé
    let d = Device::start("local", "");
    std::fs::write(d.file("Note"), BASE).unwrap();
    d.wait("note vue", |m| matches!(m, Msg::LocalEvent(_)).then_some(()));
    let note = d.load("Note");
    let outside = BASE.replace("ligne 2", "ligne 2 modifiée par un autre éditeur");
    std::fs::write(d.file("Note"), &outside).unwrap();
    let accepted = d.save(&note, &BASE.replace("ligne 5", "ligne 5 modifiée dans Mnemo"));
    assert!(!accepted, "l'enregistrement aurait dû signaler un conflit");
    assert_eq!(d.text("Note"), outside, "le changement extérieur a été écrasé");
}

#[test]
fn create_does_not_overwrite_a_file_made_outside() {
    let d = Device::start("local", "");
    std::fs::write(d.file("Idée"), "écrite ailleurs").unwrap();
    d.db.send(Cmd::Create { path: "Idée".into(), content: String::new(), open: false, if_missing: true });
    d.pump(Duration::from_millis(300));
    assert_eq!(d.text("Idée"), "écrite ailleurs");
}

#[test]
fn simultaneous_edits_on_different_lines_are_merged() {
    let Some(url) = test_db() else { return };
    let (a, b) = pair(&url);
    // les deux enregistrent avant d'avoir vu le changement de l'autre
    a.edit("Partage", "ligne 2", "ligne 2 depuis A");
    b.edit("Partage", "ligne 5", "ligne 5 depuis B");
    let text = converge(&a, &b, "Partage", |t| t.contains("depuis A") && t.contains("depuis B"));
    assert_eq!(text, BASE.replace("ligne 2", "ligne 2 depuis A").replace("ligne 5", "ligne 5 depuis B"));
    a.pump(Duration::from_secs(3));
    b.pump(Duration::from_secs(3));
    assert_eq!(a.notes(), vec!["Partage"], "aucune note de conflit ne doit apparaître");
    assert_eq!(b.notes(), vec!["Partage"]);
}

#[test]
fn repeated_simultaneous_edits_lose_nothing() {
    let Some(url) = test_db() else { return };
    let (a, b) = pair(&url);
    for i in 1..=6 {
        a.edit("Partage", "ligne 1", &format!("ligne 1 a{i}"));
        b.edit("Partage", "ligne 6", &format!("ligne 6 b{i}"));
        converge(&a, &b, "Partage", |t| t.contains(&format!(" a{i}")) && t.contains(&format!(" b{i}")));
    }
    let text = a.text("Partage");
    for i in 1..=6 {
        assert!(text.contains(&format!(" a{i}")) && text.contains(&format!(" b{i}")), "frappe perdue : {text}");
    }
    assert_eq!(b.notes(), vec!["Partage"]);
}

#[test]
fn simultaneous_edits_of_the_same_line_keep_both_texts() {
    let Some(url) = test_db() else { return };
    let (a, b) = pair(&url);
    // deux fois de suite : la seconde copie de conflit ne doit pas remplacer ni perdre la première
    for round in ["premier", "second"] {
        a.edit("Partage", "ligne 3", &format!("ligne 3 {round} texte de A"));
        b.edit("Partage", "ligne 3", &format!("ligne 3 {round} texte de B"));
        converge(&a, &b, "Partage", |t| t.contains(round));
        // la copie de conflit arrive sur les deux appareils
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            a.pump(Duration::from_millis(200));
            b.pump(Duration::from_millis(200));
            let all = |d: &Device| d.notes().iter().map(|n| d.text(n)).collect::<String>();
            let (ta, tb) = (all(&a), all(&b));
            let ok = |t: &str| t.contains(&format!("{round} texte de A")) && t.contains(&format!("{round} texte de B"));
            if ok(&ta) && ok(&tb) {
                break;
            }
            assert!(Instant::now() < end, "un des deux textes « {round} » a disparu\n--- A\n{ta}\n--- B\n{tb}");
        }
        // repartir d'une ligne 3 commune
        let cur = a.text("Partage");
        let line = cur.lines().nth(2).unwrap().to_string();
        a.edit("Partage", &line, "ligne 3");
        converge(&a, &b, "Partage", |t| t.lines().nth(2) == Some("ligne 3"));
    }
}

#[test]
fn offline_edits_on_both_devices_are_merged_after_restart() {
    let Some(url) = test_db() else { return };
    let (a, b) = pair(&url);
    // B est éteint ; sa note est modifiée (Obsidian, ou Mnemo sans réseau)
    let root_b = b.root.clone();
    std::mem::forget(b);
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(crate::db::note_file(&root_b, "Partage"), BASE.replace("ligne 6", "ligne 6 écrite hors ligne"))
        .unwrap();
    // plus tard, A modifie la même note et la synchronise
    std::thread::sleep(Duration::from_millis(1100));
    a.edit("Partage", "ligne 1", "ligne 1 écrite ensuite sur A");
    a.pump(Duration::from_secs(4));
    // B redémarre
    let b = Device::open(root_b, &url);
    let text = converge(&a, &b, "Partage", |t| t.contains("hors ligne") && t.contains("ensuite sur A"));
    assert_eq!(
        text,
        BASE.replace("ligne 6", "ligne 6 écrite hors ligne").replace("ligne 1", "ligne 1 écrite ensuite sur A")
    );
}

#[test]
fn note_deleted_elsewhere_while_device_was_off_stays_deleted() {
    let Some(url) = test_db() else { return };
    let (a, b) = pair(&url);
    let root_b = b.root.clone();
    std::mem::forget(b);
    let note = a.load("Partage");
    a.db.send(Cmd::Delete(note.id));
    a.pump(Duration::from_secs(4));
    let b = Device::open(root_b, &url);
    b.pump(Duration::from_secs(4));
    a.pump(Duration::from_secs(1));
    assert!(b.notes().is_empty(), "la note supprimée est revenue sur B : {:?}", b.notes());
    assert!(a.notes().is_empty(), "la note supprimée est revenue sur A : {:?}", a.notes());
}
