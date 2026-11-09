//! Icônes des dossiers et des notes dans l'explorateur (comme le plugin Iconize d'Obsidian) :
//! choix dans toutes les icônes Lucide, stockage en base (synchronisé), import depuis Iconize.

use super::*;

/// Fenêtre de choix d'une icône pour un chemin (dossier ou note).
pub(super) struct IconPick {
    pub(super) path: String,
    pub(super) query: String,
    pub(super) focus: bool,
}

/// Nombre maximal d'icônes affichées à la fois (la recherche affine).
const MAX_SHOWN: usize = 420;

/// Nom d'icône Iconize (« LiRadioTower », « LiGamepad2 ») → nom Lucide (« radio-tower », « gamepad-2 »).
/// Les emoji sont gardés tels quels ; les autres jeux d'icônes (Font Awesome, Simple Icons…) sont ignorés.
pub(super) fn iconize_name(raw: &str) -> Option<String> {
    if !raw.is_ascii() {
        return Some(raw.to_string()); // emoji
    }
    let name = raw.strip_prefix("Li")?;
    let mut out = String::new();
    let mut prev_digit = false;
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_digit() {
            if i > 0 && !prev_digit {
                out.push('-');
            }
            out.push(c);
        } else {
            out.push(c);
        }
        prev_digit = c.is_ascii_digit();
    }
    // « ArrowDown01 » s'écrit « arrow-down-0-1 » chez Lucide
    if theme::lucide(&out).is_some() {
        return Some(out);
    }
    let split: String = out
        .chars()
        .enumerate()
        .flat_map(|(i, c)| {
            let sep = c.is_ascii_digit() && i > 0 && out.as_bytes()[i - 1].is_ascii_digit();
            sep.then_some('-').into_iter().chain(std::iter::once(c))
        })
        .collect();
    theme::lucide(&split).is_some().then_some(split)
}

impl MnemoApp {
    /// Icône d'un dossier / d'une note (chemin tel qu'affiché).
    pub(super) fn icon_of(&self, path: &str) -> Option<&str> {
        self.icons.get(&path.to_lowercase()).map(String::as_str)
    }

    pub(super) fn set_icon(&mut self, path: &str, icon: Option<String>) {
        match &icon {
            Some(i) => self.icons.insert(path.to_lowercase(), i.clone()),
            None => self.icons.remove(&path.to_lowercase()),
        };
        self.db.send(Cmd::SetIcon { path: path.to_string(), icon });
    }

    /// Reprend les icônes réglées avec le plugin Iconize d'un coffre Obsidian
    /// (`.obsidian/plugins/obsidian-icon-folder/data.json`). Renvoie (importées, ignorées).
    pub(super) fn import_iconize(&mut self, obsidian_dir: &std::path::Path) -> Option<(usize, usize)> {
        let file = obsidian_dir.join("plugins").join("obsidian-icon-folder").join("data.json");
        let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        let (mut done, mut skipped) = (0, 0);
        for (key, value) in data.as_object()? {
            if key == "settings" {
                continue;
            }
            let raw = value.as_str().or_else(|| value.get("iconName").and_then(|v| v.as_str()));
            let path = key.strip_suffix(".md").unwrap_or(key).trim_matches('/');
            match raw.and_then(iconize_name) {
                Some(icon) if !path.is_empty() => {
                    self.set_icon(path, Some(icon));
                    done += 1;
                }
                _ => skipped += 1,
            }
        }
        Some((done, skipped))
    }

    /// Bouton « Importer les icônes d'Obsidian » et import automatique à la liaison d'un coffre.
    pub(super) fn import_iconize_notice(&mut self, obsidian_dir: &std::path::Path, quiet: bool, now: f64) {
        match self.import_iconize(obsidian_dir) {
            Some((done, skipped)) => {
                let mut msg = format!("{done} icône(s) importée(s) depuis Iconize");
                if skipped > 0 {
                    msg.push_str(&format!(" ({skipped} ignorée(s) : jeu d'icônes autre que Lucide)"));
                }
                self.notice(msg, false, now);
            }
            None if !quiet => self.notice("Aucune icône Iconize trouvée dans ce coffre", false, now),
            None => {}
        }
    }

    pub(super) fn icon_picker(&mut self, ctx: &egui::Context) {
        let Some(mut pick) = self.icon_pick.take() else { return };
        let p = self.palette();
        let current = self.icon_of(&pick.path).map(str::to_string);
        let mut close = false;
        let mid = egui::Id::unique("icon-picker");
        let modal = egui::Modal::new(mid)
            .backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 110 } else { 50 }))
            .area(egui::Modal::default_area(mid).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0)))
            .frame(
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::same(16))
                    .shadow(ctx.global_style().visuals.window_shadow),
            );
        let resp = modal.show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Icône de").color(p.muted));
                ui.label(RichText::new(basename(&pick.path)).color(p.text).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if current.is_some() && ui.button("Retirer l'icône").clicked() {
                        self.set_icon(&pick.path, None);
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut pick.query)
                    .hint_text("Rechercher une icône (en anglais : folder, book, star…) ou coller un emoji")
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut pick.focus) {
                r.request_focus();
            }
            let q = pick.query.trim().to_lowercase();
            // un emoji (ou tout texte non ASCII) collé dans la recherche s'utilise directement
            if !q.is_empty() && !q.is_ascii() {
                ui.add_space(6.0);
                if ui.button(format!("Utiliser « {} »", pick.query.trim())).clicked() {
                    self.set_icon(&pick.path, Some(pick.query.trim().to_string()));
                    close = true;
                }
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                // Entrée : la première icône trouvée
                if let Some((name, _)) = theme::lucide_icons().iter().find(|(n, _)| q.is_empty() || n.contains(&q)) {
                    self.set_icon(&pick.path, Some(name.to_string()));
                    close = true;
                }
            }
            ui.add_space(8.0);
            let matches: Vec<&(&str, char)> =
                theme::lucide_icons().iter().filter(|(n, _)| q.is_empty() || n.contains(q.as_str())).collect();
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for (name, c) in matches.iter().take(MAX_SHOWN) {
                        let selected = current.as_deref() == Some(*name);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::click());
                        if selected || resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, if selected { p.active } else { p.hover });
                        }
                        let color = if selected || resp.hovered() { p.text } else { p.muted };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            *c,
                            FontId::new(18.0, theme::icon_family()),
                            color,
                        );
                        if resp.on_hover_text(*name).clicked() {
                            self.set_icon(&pick.path, Some(name.to_string()));
                            close = true;
                        }
                    }
                });
                if matches.is_empty() {
                    ui.label(RichText::new("Aucune icône trouvée.").color(p.faint));
                }
            });
            ui.add_space(6.0);
            let more = matches.len().saturating_sub(MAX_SHOWN);
            let info = if more > 0 {
                format!("{} icônes · affinez la recherche pour voir les {more} autres", matches.len())
            } else {
                format!("{} icône(s) · Lucide", matches.len())
            };
            ui.label(RichText::new(info).small().color(p.faint));
        });
        if resp.should_close() {
            close = true;
        }
        if !close {
            self.icon_pick = Some(pick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iconize_names() {
        assert_eq!(iconize_name("LiRadioTower").as_deref(), Some("radio-tower"));
        assert_eq!(iconize_name("LiListTodo").as_deref(), Some("list-todo"));
        assert_eq!(iconize_name("LiGamepad2").as_deref(), Some("gamepad-2"));
        assert_eq!(iconize_name("LiBook").as_deref(), Some("book"));
        assert_eq!(iconize_name("🚀").as_deref(), Some("🚀"));
        assert_eq!(iconize_name("SiDiscord"), None); // Simple Icons : pas dans Lucide
        assert_eq!(iconize_name("LiPasUneIcone"), None);
        assert!(theme::lucide("folder").is_some());
    }
}
