//! Pièces jointes : images (et whiteboards) intégrées dans les notes, collées depuis le presse-papiers ou
//! déposées depuis le système, rangées dans le dossier des pièces jointes, affichées dans l'explorateur.

use super::*;
use crate::markdown::{Embed, is_image};

impl MnemoApp {
    /// Chemin absolu d'une pièce jointe (chemin relatif au dossier des notes).
    pub(super) fn file_abs(&self, rel: &str) -> Option<std::path::PathBuf> {
        self.cfg.vault_dir.as_ref().map(|r| r.join(rel))
    }

    /// Pièce jointe désignée par `![[cible]]` ou `![](cible)` depuis la note `note_path` : chemin relatif au
    /// dossier de la note, chemin complet, ou nom de fichier seul (le plus court si plusieurs, comme Obsidian).
    pub(super) fn find_file(&self, target: &str, note_path: &str) -> Option<String> {
        let t = target.trim().trim_start_matches("./").to_lowercase();
        if t.is_empty() {
            return None;
        }
        if let Some(dir) = parent(note_path) {
            let local = format!("{}/{t}", dir.to_lowercase());
            if let Some(f) = self.files.iter().find(|f| f.to_lowercase() == local) {
                return Some(f.clone());
            }
        }
        self.files
            .iter()
            .filter(|f| f.to_lowercase() == t || basename(f).to_lowercase() == t)
            .min_by_key(|f| (f.to_lowercase() != t, f.len()))
            .cloned()
    }

    /// Résolution des intégrations pour l'aperçu Markdown.
    pub(super) fn resolve_embed(&self, target: &str, note_path: &str) -> Option<Embed> {
        let rel = self.find_file(target, note_path)?;
        if rel.to_lowercase().ends_with(".mnemo-board") {
            return Some(Embed::Board(rel));
        }
        if !is_image(&rel) {
            return None;
        }
        let abs = self.file_abs(&rel)?;
        Some(Embed::Image(format!("file://{}", abs.display())))
    }

    /// Dossier des pièces jointes pour une note : réglage d'Obsidian (`attachmentFolderPath` de
    /// `.obsidian/app.json` : « / », « ./ », « ./sous-dossier » ou un dossier), sinon « Pièces jointes ».
    pub(super) fn attachment_dir(&self, note_path: &str) -> String {
        let setting = self
            .cfg
            .obsidian_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("app.json")).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("attachmentFolderPath").and_then(|p| p.as_str()).map(str::to_string));
        let note_dir = parent(note_path).unwrap_or("").to_string();
        let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
            (true, _) => b.to_string(),
            (_, true) => a.to_string(),
            _ => format!("{a}/{b}"),
        };
        match setting.as_deref().map(str::trim) {
            Some("/") | Some("") => String::new(),
            Some(p) if p == "./" || p == "." => note_dir,
            Some(p) if p.starts_with("./") => join(&note_dir, p.trim_start_matches("./").trim_matches('/')),
            Some(p) => p.trim_matches('/').to_string(),
            None => "Pièces jointes".into(),
        }
    }

    /// Texte d'intégration d'une pièce jointe : nom seul s'il est unique, sinon chemin complet.
    pub(super) fn embed_text(&self, rel: &str) -> String {
        let name = basename(rel);
        let unique = self.files.iter().filter(|f| basename(f).eq_ignore_ascii_case(name)).count() <= 1;
        format!("![[{}]]", if unique { name } else { rel })
    }

    /// Enregistre une pièce jointe pour la note ouverte puis l'y intègre au curseur.
    pub(super) fn attach_to_open_note(&mut self, file_name: &str, bytes: Vec<u8>) {
        let Some(note) = &self.open else { return };
        let dir = self.attachment_dir(&note.path);
        let path = if dir.is_empty() { file_name.to_string() } else { format!("{dir}/{file_name}") };
        self.db.send(Cmd::SaveFile { path, bytes, insert: true });
    }

    /// Ctrl+V avec une image dans le presse-papiers (capture d'écran…) : egui la transmet en `PasteImage` ;
    /// elle devient une pièce jointe PNG intégrée au curseur. Renvoie vrai si une image a été collée.
    pub(super) fn paste_image(&mut self, ctx: &egui::Context) -> bool {
        if self.open.is_none() {
            return false;
        }
        let images: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PasteImage(img) => Some(img.clone()),
                    _ => None,
                })
                .collect()
        });
        let mut pasted = false;
        for img in images {
            let [w, h] = img.size;
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else { continue };
            let mut png = std::io::Cursor::new(Vec::new());
            if buf.write_to(&mut png, image::ImageFormat::Png).is_err() {
                continue;
            }
            let name = format!("Image {}.png", chrono::Local::now().format("%Y%m%d%H%M%S"));
            self.attach_to_open_note(&name, png.into_inner());
            pasted = true;
        }
        pasted
    }

    /// Fenêtre d'aperçu d'une image cliquée dans l'explorateur.
    pub(super) fn image_viewer(&mut self, ctx: &egui::Context) {
        let Some(rel) = self.image_view.clone() else { return };
        let p = self.palette();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("image-viewer")).frame(
            egui::Frame::new()
                .fill(p.bg2)
                .stroke(egui::Stroke::new(1.0, p.border))
                .corner_radius(10.0)
                .inner_margin(16.0),
        );
        let screen = ctx.content_rect();
        let resp = modal.show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&rel).color(p.text).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button_sized(ui, "\u{e1b2}", "Fermer", false, true, 16.0).clicked() {
                        close = true;
                    }
                    if ui.button("Copier l'intégration").clicked() {
                        ui.ctx().copy_text(self.embed_text(&rel));
                    }
                });
            });
            ui.add_space(8.0);
            if let Some(abs) = self.file_abs(&rel) {
                ui.add(
                    egui::Image::from_uri(format!("file://{}", abs.display()))
                        .max_size(egui::vec2(screen.width() - 160.0, screen.height() - 180.0))
                        .fit_to_original_size(1.0),
                );
            }
        });
        if resp.should_close() || close {
            self.image_view = None;
        }
    }
}

/// Icône Lucide d'une pièce jointe dans l'explorateur.
pub(super) fn file_icon(rel: &str) -> &'static str {
    let l = rel.to_lowercase();
    if l.ends_with(".mnemo-board") {
        "pen-tool"
    } else if is_image(&l) {
        "image"
    } else if l.ends_with(".pdf") {
        "file-text"
    } else {
        "file"
    }
}
