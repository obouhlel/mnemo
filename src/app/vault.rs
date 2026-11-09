//! Dossier du coffre sur le disque (Obsidian) : liaison, synchronisation périodique,
//! onglet des paramètres, et import de fichiers / dossiers déposés depuis le système.

use super::*;
use crate::db::VaultStats;
use std::path::PathBuf;

/// Intervalle entre deux comparaisons dossier ↔ base.
const VAULT_SCAN_EVERY: f64 = 2.0;

/// `~/x` → dossier personnel ; accepte aussi le dossier `.obsidian` (on remonte au coffre).
pub(super) fn expand_dir(raw: &str) -> PathBuf {
    let raw = raw.trim();
    let p = match raw.strip_prefix("~/") {
        Some(rest) => directories::BaseDirs::new().map(|b| b.home_dir().join(rest)).unwrap_or_else(|| raw.into()),
        None => PathBuf::from(raw),
    };
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
    /// Envoie la liaison au worker (à la connexion et quand le dossier change).
    pub(super) fn send_vault(&mut self) {
        self.db.send(Cmd::SetVault(self.cfg.vault_dir.clone()));
        if self.cfg.vault_dir.is_some() {
            self.db.send(Cmd::VaultScan);
        }
        self.vault_next_scan = 0.0;
    }

    pub(super) fn link_vault(&mut self, dir: Option<PathBuf>, now: f64) {
        if let Some(d) = &dir {
            if !d.is_dir() {
                self.notice(format!("Dossier introuvable : {}", d.display()), true, now);
                return;
            }
            // réutilise aussi les thèmes / snippets du coffre s'il n'y en a pas déjà
            if self.cfg.obsidian_dir.is_none() && d.join(".obsidian").is_dir() {
                self.cfg.obsidian_dir = Some(d.join(".obsidian"));
                self.css.obsidian_input = d.join(".obsidian").display().to_string();
                self.reload_css();
            }
            // et les icônes de dossiers / notes réglées avec Iconize
            if d.join(".obsidian").is_dir() {
                self.import_iconize_notice(&d.join(".obsidian"), true, now);
            }
        }
        self.vault_input = dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        self.cfg.vault_dir = dir;
        self.vault_last = None;
        self.cfg.save();
        self.send_vault();
    }

    /// Une comparaison toutes les `VAULT_SCAN_EVERY` secondes tant qu'un dossier est lié.
    pub(super) fn vault_tick(&mut self, ctx: &egui::Context, now: f64) {
        if self.cfg.vault_dir.is_none() || self.conn != ConnState::Connected {
            return;
        }
        if now >= self.vault_next_scan {
            self.vault_next_scan = now + VAULT_SCAN_EVERY;
            self.db.send(Cmd::VaultScan);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((self.vault_next_scan - now).max(0.1)));
    }

    pub(super) fn on_vault_synced(&mut self, stats: VaultStats, first: bool, now: f64) {
        if first {
            let mut msg = format!("Coffre synchronisé : {}", summary(&stats));
            if stats.skipped > 0 {
                msg.push_str(&format!(
                    " ({} fichier(s) ignoré(s) : nom ou encodage non pris en charge)",
                    stats.skipped
                ));
            }
            self.notice(msg, false, now);
        }
        self.vault_last = Some((stats, chrono::Local::now()));
        self.need.notes = true;
        self.need.folders = true;
    }

    pub(super) fn vault_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.palette();
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Liez le dossier d'un coffre Obsidian : toutes ses notes (.md) sont importées, et chaque note créée, \
                     modifiée, renommée ou supprimée dans Mnemo est aussi enregistrée dans ce dossier. Les changements \
                     faits dans le dossier (par Obsidian ou un autre éditeur) reviennent dans Mnemo en quelques secondes.",
                )
                .size(12.5)
                .color(p.muted),
            )
            .wrap(),
        );
        ui.add_space(14.0);
        let mut input = self.vault_input.clone();
        let mut link = false;
        settings_row(
            ui,
            &p,
            "Dossier du coffre",
            "Le dossier qui contient vos notes (et le dossier .obsidian).",
            |ui| {
                if ui.button("Lier").clicked() {
                    link = true;
                }
                let r = ui.add(egui::TextEdit::singleline(&mut input).hint_text("~/MonCoffre").desired_width(240.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    link = true;
                }
            },
        );
        self.vault_input = input;
        if link {
            let raw = self.vault_input.trim().to_string();
            self.link_vault(if raw.is_empty() { None } else { Some(expand_dir(&raw)) }, now);
        }
        if let Some(dir) = self.cfg.vault_dir.clone() {
            let state = match &self.vault_last {
                Some((s, at)) => format!("Dernière synchronisation à {} : {}.", at.format("%H:%M:%S"), summary(s)),
                None => "Première synchronisation en cours…".into(),
            };
            let mut unlink = false;
            let mut rescan = false;
            let mut iconize = false;
            settings_row(ui, &p, "Synchronisation", &state, |ui| {
                if ui.button("Délier").clicked() {
                    unlink = true;
                }
                if ui.button("Synchroniser maintenant").clicked() {
                    rescan = true;
                }
            });
            settings_row(
                ui,
                &p,
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
            if rescan {
                self.db.send(Cmd::VaultScan);
            }
            if unlink {
                self.link_vault(None, now);
                self.notice(format!("Dossier délié : {}", dir.display()), false, now);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Les fichiers supprimés depuis Mnemo sont déplacés dans le dossier .trash du coffre. \
                         Les pièces jointes (images, PDF…) restent sur le disque et ne sont pas importées.",
                    )
                    .size(12.5)
                    .color(p.faint),
                )
                .wrap(),
            );
        }
    }

    /// Fichiers / dossiers glissés depuis le système vers la fenêtre : importés dans `target`
    /// (dossier de l'explorateur survolé, ou la racine).
    pub(super) fn import_dropped(&mut self, files: Vec<PathBuf>, target: &str, now: f64) {
        let mut notes: Vec<(String, PathBuf)> = vec![];
        let mut folders: Vec<String> = vec![];
        let mut skipped = 0;
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
        if skipped > 0 {
            msg.push_str(&format!(", {skipped} fichier(s) ignoré(s) (seuls les .md sont importés)"));
        }
        self.notice(msg, skipped > 0 && count == 0, now);
    }
}

/// Une ligne de réglage : titre et description à gauche, contrôles alignés à droite, séparateur.
pub(super) fn settings_row(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    title: &str,
    desc: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 280.0).max(200.0));
            ui.label(RichText::new(title).color(p.text));
            if !desc.is_empty() {
                ui.add(egui::Label::new(RichText::new(desc).size(12.5).color(p.muted)).wrap());
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, p.border));
    ui.add_space(10.0);
}
