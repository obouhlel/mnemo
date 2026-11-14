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
        // adresse non encodée : `markdown::preprocess_with` encode lui-même ce que l'aperçu exige
        Some(Embed::Image(file_uri_plain(&self.file_abs(&rel)?)))
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
        // la réponse arrive plus tard : on retient pour quelle note c'était (voir `file_saved`)
        self.embed_targets.push((path.clone(), note.id));
        if self.embed_targets.len() > 32 {
            self.embed_targets.remove(0); // enregistrements restés sans réponse (erreur d'écriture)
        }
        self.db.send(Cmd::SaveFile { path, bytes, insert: true });
    }

    /// Une pièce jointe vient d'être enregistrée (`Msg::FileSaved`). Si elle était destinée à une note
    /// (`insert`), `![[…]]` n'y est inséré que si cette note est encore celle affichée et n'a pas de conflit
    /// en attente ; sinon le fichier est simplement gardé.
    pub(super) fn file_saved(&mut self, path: String, insert: bool, now: f64) {
        if !self.files.contains(&path) {
            self.files.push(path.clone());
        }
        if !insert {
            return;
        }
        let target = self.embed_targets.iter().position(|(asked, _)| saved_as(asked, &path));
        let target = target.map(|i| self.embed_targets.remove(i).1);
        let why = match (&self.open, target) {
            (Some(n), Some(t)) if n.id == t && n.conflict.is_none() => {
                self.actions.push(UiAction::InsertEmbed(path));
                return;
            }
            (Some(n), Some(t)) if n.id == t => "sa note a un conflit à résoudre",
            _ => "sa note n'est plus affichée",
        };
        self.notice(
            format!("« {path} » est enregistrée mais pas intégrée ({why}). Elle reste dans l'explorateur."),
            false,
            now,
        );
    }

    /// Ctrl+V avec une image dans le presse-papiers (capture d'écran…) : egui la transmet en `PasteImage` ;
    /// elle devient une pièce jointe PNG intégrée au curseur. Seulement quand on écrit dans la note : un
    /// collage dans la recherche, le titre ou un réglage ne la concerne pas. Renvoie vrai si une image a été
    /// collée.
    pub(super) fn paste_image(&mut self, ctx: &egui::Context) -> bool {
        let Some(n) = &self.open else { return false };
        if !ctx.memory(|m| m.has_focus(editor_id(n.id))) {
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
                    egui::Image::from_uri(file_uri(&abs))
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

/// `saved` est-il le nom donné par le stockage local au fichier demandé sous `asked` ? (le même, ou
/// « photo 1.png », « photo 2.png »… si le nom était pris)
fn saved_as(asked: &str, saved: &str) -> bool {
    if asked == saved {
        return true;
    }
    let (stem, ext) = match asked.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.contains('/') => (s, format!(".{e}")),
        _ => (asked, String::new()),
    };
    saved
        .strip_prefix(stem)
        .and_then(|r| r.strip_suffix(ext.as_str()))
        .and_then(|n| n.strip_prefix(' '))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Adresse `file://` d'un fichier, sans encodage : `file:///home/moi/a b.png`, `file:///C:/Users/moi/a b.png`
/// (jamais `file://C:\…`), `file://serveur/partage/a.png` pour un chemin réseau Windows.
fn plain_uri(path: &str, windows: bool) -> String {
    if !windows {
        return format!("file://{}{path}", if path.starts_with('/') { "" } else { "/" });
    }
    let p = path.replace('\\', "/");
    // chemins « verbatim » de Windows : \\?\C:\… et \\?\UNC\serveur\partage\…
    let p = match p.strip_prefix("//?/") {
        Some(rest) => match rest.strip_prefix("UNC/") {
            Some(unc) => format!("//{unc}"),
            None => rest.to_string(),
        },
        None => p,
    };
    match p.strip_prefix("//") {
        Some(unc) => format!("file://{unc}"),
        None => format!("file:///{}", p.trim_start_matches('/')),
    }
}

/// Encode en `%XX` tout ce qui n'est pas sûr dans une adresse (espace, `#`, `?`, `%`, accents…).
fn percent_encode(uri: &str) -> String {
    let (scheme, rest) = uri.split_at(uri.find("//").map_or(0, |i| i + 2));
    let mut out = String::from(scheme);
    for b in rest.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Adresse `file://` d'un fichier, sur tous les systèmes, encodée (`a b#1.png` → `a%20b%231.png`) : pour
/// `egui::Image::from_uri` et pour ouvrir le fichier avec l'application du système. Le chargeur d'egui_extras
/// essaie le chemin tel quel puis le chemin décodé une fois : il ne faut donc encoder qu'une fois (ne pas
/// passer cette adresse à `markdown::preprocess_with`, qui encode déjà : voir `file_uri_plain`).
pub(super) fn file_uri(path: &std::path::Path) -> String {
    percent_encode(&file_uri_plain(path))
}

/// La même adresse sans encodage, pour l'aperçu Markdown (qui encode lui-même `%`, `<` et `>`).
pub(super) fn file_uri_plain(path: &std::path::Path) -> String {
    plain_uri(&path.to_string_lossy(), cfg!(windows))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris() {
        let uri = |p: &str, win: bool| percent_encode(&plain_uri(p, win));
        // Linux / macOS
        assert_eq!(uri("/home/moi/Notes/img/photo.png", false), "file:///home/moi/Notes/img/photo.png");
        assert_eq!(uri("/home/moi/Mes notes/a#1 50%?.png", false), "file:///home/moi/Mes%20notes/a%231%2050%25%3F.png");
        assert_eq!(uri("/n/été.png", false), "file:///n/%C3%A9t%C3%A9.png");
        assert_eq!(uri("/n/a\\b.png", false), "file:///n/a%5Cb.png"); // « \ » est un caractère comme un autre
        // Windows : barres obliques, trois barres avant la lettre du lecteur
        assert_eq!(uri("C:\\Users\\moi\\Notes\\photo.png", true), "file:///C:/Users/moi/Notes/photo.png");
        assert_eq!(
            uri("C:\\Users\\moi\\Mes notes/img/a b.png", true),
            "file:///C:/Users/moi/Mes%20notes/img/a%20b.png"
        );
        assert_eq!(uri("\\\\?\\C:\\Notes\\a.png", true), "file:///C:/Notes/a.png");
        assert_eq!(uri("\\\\nas\\partage\\Notes\\a.png", true), "file://nas/partage/Notes/a.png");
        assert_eq!(uri("\\\\?\\UNC\\nas\\partage\\a.png", true), "file://nas/partage/a.png");
        // sans encodage (aperçu Markdown) : seule la forme change
        assert_eq!(plain_uri("C:\\Mes notes\\a 50%.png", true), "file:///C:/Mes notes/a 50%.png");
        assert_eq!(plain_uri("/n/a b#1.png", false), "file:///n/a b#1.png");
        // un « % » du nom est encodé une fois : décoder une fois redonne le chemin
        assert_eq!(uri("/n/a%20b.png", false), "file:///n/a%2520b.png");
    }

    #[test]
    fn saved_file_matches_its_request() {
        assert!(saved_as("Pièces jointes/Image 1.png", "Pièces jointes/Image 1.png"));
        assert!(saved_as("Pièces jointes/photo.png", "Pièces jointes/photo 2.png"));
        assert!(saved_as("dossier.v2/notes", "dossier.v2/notes 1"));
        assert!(!saved_as("Pièces jointes/photo.png", "Pièces jointes/photo bis.png"));
        assert!(!saved_as("Pièces jointes/photo.png", "Pièces jointes/autre.png"));
        assert!(!saved_as("a/photo.png", "b/photo 1.png"));
    }
}
