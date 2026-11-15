//! Dossier du coffre sur le disque (Obsidian) : liaison (la synchronisation périodique tourne dans le
//! worker, voir `db::Worker::run`), affichage du dernier passage, onglet des paramètres, et import de fichiers / dossiers déposés depuis le système.

use super::settings::{PickTarget, path_label, setting_row, settings_note};
use super::*;
use crate::db::VaultStats;
use std::path::PathBuf;

/// Le dossier choisi : `.obsidian` désigne son coffre (on remonte d'un niveau).
pub(super) fn notes_dir_of(p: PathBuf) -> PathBuf {
    if p.file_name().is_some_and(|n| n == ".obsidian")
        && let Some(parent) = p.parent()
    {
        return parent.to_path_buf();
    }
    p
}

fn summary(s: &VaultStats) -> String {
    let mut parts = vec![];
    for (n, one, many) in [
        (s.imported, "note importée", "notes importées"),
        (s.pulled, "note mise à jour depuis le disque", "notes mises à jour depuis le disque"),
        (s.written, "fichier écrit", "fichiers écrits"),
        (s.removed, "suppression", "suppressions"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {}", if n > 1 { many } else { one }));
        }
    }
    if parts.is_empty() { "déjà à jour".into() } else { parts.join(", ") }
}

impl MnemoApp {
    /// Synchronisation dossier ↔ base : à la connexion et quand le dossier change.
    pub(super) fn send_vault(&mut self) {
        self.db.send(Cmd::SetVault(self.cfg.vault_dir.clone()));
        if self.cfg.vault_dir.is_some() {
            self.db.send(Cmd::VaultScan);
        }
    }

    /// Change le dossier des notes (la référence de Mnemo) ; la synchro avec la base suit si elle est active.
    pub(super) fn set_notes_dir(&mut self, dir: PathBuf, now: f64) {
        if !dir.is_dir() {
            self.notice(format!("Dossier introuvable : {}", dir.display()), true, now);
            return;
        }
        let was_vault = self.vault_obsidian_dir().is_some();
        let is_vault = dir.join(".obsidian").is_dir();
        // un coffre Obsidian : ses thèmes / snippets, ses réglages et ses icônes Iconize sont repris
        if is_vault {
            self.cfg.obsidian_dir = Some(dir.join(".obsidian"));
        }
        self.cfg.vault_dir = Some(dir.clone());
        if was_vault || is_vault {
            self.reload_css();
        }
        self.vault_last = None;
        self.cfg.save();
        self.leave_note();
        self.open = None;
        self.tabs = vec![Tab::default()];
        self.active_tab = 0;
        self.db.send(Cmd::SetRoot(dir.clone()));
        if dir.join(".obsidian").is_dir() {
            self.import_iconize_notice(&dir.join(".obsidian"), true, now);
        }
        // la synchro suit le nouveau dossier (tout de suite si la base est connectée, sinon à la connexion)
        self.send_vault();
    }

    pub(super) fn on_vault_synced(&mut self, stats: VaultStats, first: bool, now: f64) {
        if first && stats.changed() {
            self.notice(format!("Synchronisé avec la base : {}", summary(&stats)), false, now);
        }
        self.vault_last = Some((stats, chrono::Local::now()));
    }

    pub(super) fn vault_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        settings_note(
            ui,
            "Vos notes sont des fichiers Markdown (.md) dans ce dossier : Mnemo fonctionne sans base de données. \
             Choisissez le dossier d'un coffre Obsidian pour retrouver toutes vos notes ; les changements faits \
             par Obsidian ou un autre éditeur apparaissent dans Mnemo en une seconde. Les icônes et réglages \
             de Mnemo sont rangés dans le sous-dossier .mnemo.",
        );
        let mut browse = false;
        let current = self.cfg.vault_dir.clone();
        let busy = self.folder_pick.is_some();
        setting_row(
            ui,
            "Dossier des notes",
            "Un dossier de fichiers .md, par exemple un coffre Obsidian. Le changement s'applique tout de suite.",
            |ui| {
                if ui.add_enabled(!busy, egui::Button::new("Parcourir…")).clicked() {
                    browse = true;
                }
                if let Some(dir) = &current {
                    path_label(ui, dir);
                }
            },
        );
        if browse {
            self.pick_folder(ui.ctx(), PickTarget::Notes, current);
        }
        let state = match (&self.conn, &self.vault_last) {
            (ConnState::Connected, Some((s, at))) => {
                format!("Synchronisé avec la base à {} : {}.", at.format("%H:%M:%S"), summary(s))
            }
            (ConnState::Connected, None) => "Première synchronisation avec la base en cours…".into(),
            _ if self.cfg.database_url.trim().is_empty() => {
                "Local uniquement. Pour retrouver vos notes sur plusieurs appareils, connectez une base PostgreSQL \
                 dans « Synchronisation »."
                    .into()
            }
            _ => "Base injoignable : tout est gardé en local et sera synchronisé au retour de la connexion.".into(),
        };
        let mut rescan = false;
        let mut iconize = false;
        let connected = self.conn == ConnState::Connected;
        setting_row(ui, "Synchronisation", &state, |ui| {
            if connected && ui.button("Synchroniser maintenant").clicked() {
                rescan = true;
            }
        });
        if let Some(dir) = self.cfg.vault_dir.clone() {
            setting_row(
                ui,
                "Icônes",
                "Reprend les icônes de dossiers et de notes réglées avec le plugin Iconize d'Obsidian.",
                |ui| {
                    if ui.button("Importer les icônes d'Obsidian").clicked() {
                        iconize = true;
                    }
                },
            );
            if iconize {
                self.import_iconize_notice(&dir.join(".obsidian"), false, now);
            }
        }
        if rescan {
            self.db.send(Cmd::VaultScan);
        }
        settings_note(
            ui,
            "Les notes supprimées vont dans le dossier .trash. Les pièces jointes (images, PDF…) restent sur le disque.",
        );
    }

    /// Dossier .obsidian du dossier des notes, si c'est un coffre Obsidian.
    pub(super) fn vault_obsidian_dir(&self) -> Option<PathBuf> {
        self.cfg.vault_dir.as_ref().map(|d| d.join(".obsidian")).filter(|d| d.is_dir())
    }

    /// Dossier .obsidian dont l'apparence est reprise : celui du coffre ouvert, sinon celui choisi
    /// dans les options avancées.
    pub(super) fn obsidian_dir(&self) -> Option<PathBuf> {
        self.vault_obsidian_dir().or_else(|| self.cfg.obsidian_dir.clone())
    }

    /// Dossier des notes par défaut (premier lancement) : ~/Documents/Mnemo.
    pub(super) fn default_notes_dir() -> PathBuf {
        directories::UserDirs::new()
            .and_then(|u| u.document_dir().map(|d| d.to_path_buf()))
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Mnemo")
    }

    /// Fichiers / dossiers glissés depuis le système vers la fenêtre : importés dans `target`
    /// (dossier de l'explorateur survolé, ou la racine).
    pub(super) fn import_dropped(&mut self, files: Vec<PathBuf>, target: &str, now: f64) {
        let mut notes: Vec<(String, PathBuf)> = vec![];
        let mut folders: Vec<String> = vec![];
        let mut skipped = 0;
        let mut copied = 0;
        let join = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { format!("{a}/{b}") };
        for f in files {
            let Some(name) = f.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            if f.is_dir() {
                let base = join(target, &name);
                folders.push(base.clone());
                let (found, dirs, sk) = crate::db::walk_markdown(&f);
                skipped += sk;
                folders.extend(dirs.into_iter().map(|d| join(&base, &d)));
                notes.extend(found.into_iter().map(|(rel, abs)| (join(&base, &rel), abs)));
            } else if let Some(stem) = name.strip_suffix(".md").or_else(|| name.strip_suffix(".markdown")) {
                notes.push((join(target, stem), f.clone()));
            } else if let Ok(bytes) = std::fs::read(&f) {
                // image, PDF, whiteboard… : copié tel quel dans le dossier visé
                self.db.send(Cmd::SaveFile { path: join(target, &name), bytes, insert: false });
                copied += 1;
            } else {
                skipped += 1;
            }
        }
        let mut count = 0;
        for (path, abs) in notes {
            let (Ok(path), Ok(content)) = (normalize_path(&path), std::fs::read_to_string(&abs)) else {
                skipped += 1;
                continue;
            };
            self.db.send(Cmd::Create { path, content, open: false, if_missing: true });
            count += 1;
        }
        for f in folders {
            if let Ok(f) = normalize_path(&f) {
                self.db.send(Cmd::CreateFolder(f));
            }
        }
        if !target.is_empty() {
            self.expanded.insert(target.to_lowercase());
        }
        self.need.notes = true;
        self.need.folders = true;
        let mut msg = format!("{count} note(s) importée(s)");
        if copied > 0 {
            msg.push_str(&format!(", {copied} fichier(s) copié(s)"));
        }
        if skipped > 0 {
            msg.push_str(&format!(", {skipped} fichier(s) illisible(s)"));
        }
        self.notice(msg, skipped > 0 && count == 0, now);
    }
}
