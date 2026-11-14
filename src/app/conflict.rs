//! Fenêtre de résolution des conflits : quand la note a changé dans la base pendant
//! qu'on l'éditait, fusion à 3 voies bloc par bloc (onglet « Résoudre ») ou diff unifié (onglet « Diff »).

use super::*;
use crate::merge::{Block, Choice, Origin};

const DEL_BG: Color32 = Color32::from_rgba_premultiplied(60, 16, 16, 90);
const INS_BG: Color32 = Color32::from_rgba_premultiplied(14, 52, 24, 90);
const THEIRS_BG: Color32 = Color32::from_rgba_premultiplied(16, 36, 66, 90);

fn code_block(ui: &mut egui::Ui, text: &str, bg: Option<Color32>) {
    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 4))
        .corner_radius(4.0)
        .fill(bg.unwrap_or(ui.visuals().extreme_bg_color));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if text.is_empty() {
            ui.label(RichText::new("(vide : supprimé de ce côté)").italics().weak());
        } else {
            ui.add(egui::Label::new(RichText::new(text.trim_end_matches('\n')).monospace()).wrap());
        }
    });
}

fn resolve_ui(ui: &mut egui::Ui, blocks: &mut [Block]) {
    let mut k = 0;
    for b in blocks.iter_mut() {
        match b {
            Block::Text { text, origin } => {
                let lines: Vec<&str> = text.lines().collect();
                match origin {
                    Origin::Unchanged => {
                        if lines.len() <= 6 {
                            code_block(ui, text, None);
                        } else {
                            code_block(ui, &lines[..2].join("\n"), None);
                            ui.weak(format!("     ⋯ {} lignes identiques ⋯", lines.len() - 4));
                            code_block(ui, &lines[lines.len() - 2..].join("\n"), None);
                        }
                    }
                    o => {
                        let (label, bg) = match o {
                            Origin::Mine => ("Modifié par vous : fusionné automatiquement", INS_BG),
                            Origin::Theirs => ("Modifié à distance : fusionné automatiquement", THEIRS_BG),
                            _ => ("Modification identique des deux côtés", INS_BG),
                        };
                        ui.label(RichText::new(label).small().strong());
                        code_block(ui, text, Some(bg));
                    }
                }
            }
            Block::Conflict { base, mine, theirs, choice, custom } => {
                k += 1;
                let resolved = *choice != Choice::Unresolved;
                let stroke = if resolved { Color32::from_rgb(70, 170, 100) } else { Color32::from_rgb(230, 140, 40) };
                egui::Frame::new().stroke(egui::Stroke::new(1.5, stroke)).corner_radius(6.0).inner_margin(8.0).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Conflit {k}")).strong());
                            ui.label(
                                RichText::new(if resolved { "✔ résolu" } else { "à résoudre" }).color(stroke).small(),
                            );
                        });
                        ui.columns(2, |cols| {
                            cols[0]
                                .label(RichText::new("Votre version").strong().color(Color32::from_rgb(90, 200, 120)));
                            code_block(&mut cols[0], mine, Some(INS_BG));
                            cols[1].label(
                                RichText::new("Version distante").strong().color(Color32::from_rgb(110, 160, 240)),
                            );
                            code_block(&mut cols[1], theirs, Some(THEIRS_BG));
                        });
                        if !base.is_empty() {
                            egui::CollapsingHeader::new(
                                RichText::new("Version d'origine (avant les deux modifications)").small(),
                            )
                            .id_salt(("base", k))
                            .show(ui, |ui| code_block(ui, base, None));
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(choice, Choice::Mine, "Garder la mienne");
                            ui.selectable_value(choice, Choice::Theirs, "Garder la distante");
                            ui.selectable_value(choice, Choice::MineThenTheirs, "Les deux (la mienne d'abord)");
                            ui.selectable_value(choice, Choice::TheirsThenMine, "Les deux (distante d'abord)");
                            if ui.selectable_label(*choice == Choice::Custom, "Éditer à la main…").clicked()
                                && *choice != Choice::Custom
                            {
                                *custom = merge::resolved_text(mine, theirs, choice, custom)
                                    .unwrap_or_else(|| merge::concat(mine, theirs));
                                *choice = Choice::Custom;
                            }
                            if resolved && ui.small_button("Annuler le choix").clicked() {
                                *choice = Choice::Unresolved;
                            }
                        });
                        if *choice == Choice::Custom {
                            ui.add(
                                egui::TextEdit::multiline(custom)
                                    .code_editor()
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                        } else if let Some(r) = merge::resolved_text(mine, theirs, choice, custom) {
                            ui.label(RichText::new("Résultat :").small());
                            code_block(ui, &r, None);
                        }
                    },
                );
            }
        }
        ui.add_space(4.0);
    }
}

fn diff_ui(ui: &mut egui::Ui, old: &str, new: &str, new_label: &str) {
    let hunks = merge::unified(old, new, 3);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("− version distante").monospace().background_color(DEL_BG));
        ui.label(RichText::new(format!("+ {new_label}")).monospace().background_color(INS_BG));
    });
    ui.add_space(4.0);
    if hunks.is_empty() {
        ui.weak("Aucune différence.");
        return;
    }
    for hunk in hunks {
        egui::Frame::group(ui.style()).inner_margin(0.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let first = &hunk[0];
            ui.label(
                RichText::new(format!(" @@ -{} +{} @@", first.old_no.unwrap_or(0), first.new_no.unwrap_or(0)))
                    .monospace()
                    .weak(),
            );
            for line in &hunk {
                let (bg, sign) = match line.tag {
                    similar::ChangeTag::Delete => (DEL_BG, "-"),
                    similar::ChangeTag::Insert => (INS_BG, "+"),
                    similar::ChangeTag::Equal => (Color32::TRANSPARENT, " "),
                };
                egui::Frame::new().fill(bg).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let num = |n: Option<usize>| n.map(|n| format!("{n:>4}")).unwrap_or_else(|| "    ".into());
                        ui.label(
                            RichText::new(format!("{} {} {sign} ", num(line.old_no), num(line.new_no)))
                                .monospace()
                                .weak(),
                        );
                        ui.add(egui::Label::new(RichText::new(&line.text).monospace()).wrap());
                    });
                });
            }
        });
        ui.add_space(6.0);
    }
}

impl Conflict {
    /// Recalcule les blocs pour le texte actuel de l'éditeur (`mine`), en gardant les choix déjà faits sur les
    /// conflits qui n'ont pas changé. Sans effet si le texte est celui du dernier calcul.
    pub(super) fn rebuild(&mut self, saved: &str, mine: &str) {
        if self.mine == mine {
            return;
        }
        let old = std::mem::replace(&mut self.blocks, merge::build(saved, mine, &self.remote.content));
        for b in &mut self.blocks {
            if let Block::Conflict { base, mine, theirs, choice, custom } = b {
                let same = old.iter().find_map(|o| match o {
                    Block::Conflict { base: b2, mine: m2, theirs: t2, choice: c2, custom: u2 }
                        if b2 == base && m2 == mine && t2 == theirs =>
                    {
                        Some((c2.clone(), u2.clone()))
                    }
                    _ => None,
                });
                if let Some((c2, u2)) = same {
                    *choice = c2;
                    *custom = u2;
                }
            }
        }
        self.mine = mine.to_string();
    }
}

impl MnemoApp {
    pub(super) fn conflict_window(&mut self, ctx: &egui::Context, now: f64) {
        let Some(n) = self.open.as_mut() else { return };
        let path = n.path.clone();
        let mine_full = n.content.clone();
        let Some(c) = n.conflict.as_mut() else { return };
        if !c.window_open {
            return;
        }
        // l'éditeur reste utilisable derrière la fenêtre : ce qui y est tapé entre dans la fusion
        c.rebuild(&n.saved, &n.content);
        let mut open = true;
        let mut apply: Option<String> = None;
        let mut close = false;
        egui::Window::new("Résoudre le conflit").open(&mut open).default_size([980.0, 640.0]).collapsible(false).show(
            ctx,
            |ui| {
                let (total, left) = merge::conflict_count(&c.blocks);
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut c.tab,
                        ConflictTab::Resolve,
                        format!("Résolution ({}/{total})", total - left),
                    );
                    ui.selectable_value(&mut c.tab, ConflictTab::Diff, "Diff complet");
                });
                ui.weak(format!(
                    "« {path} » a été modifiée ailleurs (version {}) pendant que vous l'éditiez. \
                 Les changements qui ne se chevauchent pas sont déjà fusionnés ; choisissez pour chaque conflit.",
                    c.remote.version
                ));
                ui.separator();
                let h = (ui.available_height() - 44.0).max(160.0);
                egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| match c.tab {
                    ConflictTab::Resolve => resolve_ui(ui, &mut c.blocks),
                    ConflictTab::Diff => {
                        ui.checkbox(
                            &mut c.diff_against_result,
                            "Comparer au résultat de la fusion (au lieu de votre version)",
                        );
                        let merged = merge::assemble(&c.blocks);
                        let (new, label) = match (&merged, c.diff_against_result) {
                            (Some(m), true) => (m.clone(), "résultat de la fusion"),
                            _ => (mine_full.clone(), "votre version"),
                        };
                        diff_ui(ui, &c.remote.content, &new, label);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let merged = merge::assemble(&c.blocks);
                    let btn = egui::Button::new(RichText::new("✔ Appliquer la fusion").strong());
                    if ui
                        .add_enabled(merged.is_some(), btn)
                        .on_disabled_hover_text(format!("{left} conflit(s) restant(s)"))
                        .clicked()
                    {
                        apply = merged;
                    }
                    let set_all = |c: &mut Conflict, ch: Choice| {
                        for b in &mut c.blocks {
                            if let Block::Conflict { choice, .. } = b {
                                *choice = ch.clone();
                            }
                        }
                    };
                    if ui.button("Tout : la mienne").clicked() {
                        set_all(c, Choice::Mine);
                    }
                    if ui.button("Tout : la distante").clicked() {
                        set_all(c, Choice::Theirs);
                    }
                    if ui.button("Résoudre plus tard").clicked() {
                        close = true;
                    }
                });
            },
        );
        if !open || close {
            c.window_open = false;
        }
        if let Some(text) = apply {
            let remote = n.conflict.take().unwrap().remote;
            self.cursor_fix.get_or_insert_with(|| (n.id, n.content.clone()));
            n.content = text;
            n.saved = remote.content;
            n.version = remote.version;
            n.last_edit = Some(now - AUTOSAVE_DELAY);
            self.content_gen += 1;
            self.notice("Fusion appliquée et enregistrée", false, now);
        }
    }
}
