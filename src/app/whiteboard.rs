//! Whiteboard à dessin libre dans un onglet : stylo, ligne, flèche, rectangle, ellipse, texte, gomme,
//! sélection et déplacement, couleurs, épaisseurs, annuler / rétablir, zoom et déplacement de la vue.
//! Le dessin est un fichier `.mnemo-board` du dossier des notes (voir `crate::sketch`), enregistré
//! automatiquement et synchronisé comme les autres pièces jointes.
//!
//! Le fichier peut changer sous nos pieds (autre appareil, synchronisation) : avant chaque enregistrement et à
//! chaque changement signalé, il est relu et fusionné élément par élément avec ce qui a été tracé ici
//! (`sketch::merge`, base = dernier état lu ou écrit). Un fichier illisible ou disparu met l'onglet en lecture
//! seule : rien n'est écrit tant que l'utilisateur n'a pas choisi.

use super::*;
use crate::sketch::{Element, Sketch, parse_color, simplify};
use std::path::Path;
use std::time::SystemTime;

/// Délai entre le dernier trait et l'enregistrement.
const SAVE_DELAY: f64 = 0.8;

/// Couleur « encre » : noire sur fond clair, claire sur fond sombre (comme Excalidraw).
pub(super) const INK: &str = "#1e1e1e";
pub(super) const COLORS: [&str; 6] = [INK, "#e03131", "#1971c2", "#2f9e44", "#f08c00", "#9c36b5"];
const WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];
const RED: Color32 = Color32::from_rgb(0xfb, 0x46, 0x4c);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    Select,
    Hand,
    Pen,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Eraser,
}

impl Tool {
    const ALL: [(Tool, &'static str, &'static str, Key); 9] = [
        (Tool::Select, "mouse-pointer-2", "Sélection et déplacement (V)", Key::V),
        (Tool::Hand, "hand", "Déplacer la vue (H, ou clic molette)", Key::H),
        (Tool::Pen, "pencil", "Stylo (P)", Key::P),
        (Tool::Line, "minus", "Ligne (L)", Key::L),
        (Tool::Arrow, "move-up-right", "Flèche (A)", Key::A),
        (Tool::Rect, "square", "Rectangle (R)", Key::R),
        (Tool::Ellipse, "circle", "Ellipse (O)", Key::O),
        (Tool::Text, "type", "Texte (T)", Key::T),
        (Tool::Eraser, "eraser", "Gomme (E)", Key::E),
    ];
}

/// Le fichier d'un whiteboard tel qu'il est sur le disque.
enum Disk {
    Missing,
    Unreadable(String),
    Ok(Vec<Element>, Option<SystemTime>),
}

fn file_mtime(abs: &Path) -> Option<SystemTime> {
    std::fs::metadata(abs).and_then(|m| m.modified()).ok()
}

fn read_disk(abs: &Path) -> Disk {
    match std::fs::read_to_string(abs) {
        Ok(s) => match Sketch::parse(&s) {
            Ok(sk) => Disk::Ok(sk.elements, file_mtime(abs)),
            Err(e) => Disk::Unreadable(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk::Missing,
        Err(e) => Disk::Unreadable(format!("Whiteboard illisible : {e}")),
    }
}

/// Écriture immédiate (à la fermeture de l'application), par fichier temporaire caché comme le stockage local.
fn write_now(abs: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let tmp = abs.with_file_name(format!(".{name}.mnemo-tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, abs)
}

/// Nouveau chemin de `path` quand le dossier `from` devient `to` (même règle que le stockage local : casse
/// ignorée, comptée en caractères). `None` si `path` n'est pas dans ce dossier.
fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    let skip = from.chars().count();
    let head: String = path.chars().take(skip).collect();
    let tail: String = path.chars().skip(skip).collect();
    (head.to_lowercase() == from.to_lowercase() && tail.starts_with('/')).then(|| format!("{to}{tail}"))
}

/// Un whiteboard ouvert.
pub(super) struct BoardView {
    pub(super) sketch: Sketch,
    /// éléments tels qu'ils ont été lus ou écrits la dernière fois sur le disque : base de la fusion à 3 voies
    base: Vec<Element>,
    /// base d'avant notre écriture en route : tant que le disque la montre, l'écriture n'est pas encore faite
    sent_from: Option<Vec<Element>>,
    undo: Vec<Vec<Element>>,
    redo: Vec<Vec<Element>>,
    /// déplacement de la vue (coordonnées du dessin) et zoom
    pan: egui::Vec2,
    zoom: f32,
    fitted: bool,
    tool: Tool,
    color: &'static str,
    width: f32,
    selected: Option<usize>,
    /// élément en cours de tracé
    drawing: Option<Element>,
    /// coup de gomme en cours (un seul « annuler » pour tout le geste)
    erasing: bool,
    /// déplacement d'un élément en cours
    moving: bool,
    /// texte en cours de saisie : position (dessin), contenu, indice s'il remplace un texte existant
    text_edit: Option<([f32; 2], String, Option<usize>)>,
    focus_text: bool,
    /// modifié depuis le dernier enregistrement (moment de la dernière modification)
    dirty: Option<f64>,
    /// date du fichier à sa dernière lecture ; `None` : notre écriture est en route (ou rien n'est encore lu)
    mtime: Option<SystemTime>,
    /// le fichier a déjà été vu sur le disque (faux : dessin tout neuf, ou dossier en cours de déplacement)
    seen: bool,
    /// le fichier a changé pendant un geste : à relire dès qu'il est fini
    stale: bool,
    /// fichier illisible : lecture seule, rien n'est enregistré (le vrai fichier ne doit pas être écrasé)
    error: Option<String>,
    /// fichier déplacé ou supprimé hors de Mnemo : lecture seule
    gone: bool,
}

impl BoardView {
    /// Dessin vide dont le fichier n'existe pas encore (ou pas encore à cet endroit).
    pub(super) fn empty() -> Self {
        Self {
            sketch: Sketch::default(),
            base: vec![],
            sent_from: None,
            undo: vec![],
            redo: vec![],
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            fitted: false,
            tool: Tool::Pen,
            color: INK,
            width: WIDTHS[0],
            selected: None,
            drawing: None,
            erasing: false,
            moving: false,
            text_edit: None,
            focus_text: false,
            dirty: None,
            mtime: None,
            seen: false,
            stale: false,
            error: None,
            gone: false,
        }
    }

    pub(super) fn load(abs: &Path) -> Self {
        let mut b = Self::empty();
        b.seen = true;
        match read_disk(abs) {
            Disk::Ok(elements, m) => {
                b.base = elements.clone();
                b.sketch.elements = elements;
                b.mtime = m;
            }
            Disk::Unreadable(e) => b.error = Some(e),
            Disk::Missing => b.gone = true,
        }
        b
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty.is_some()
    }

    /// Lecture seule : le fichier est illisible ou a disparu.
    fn read_only(&self) -> bool {
        self.error.is_some() || self.gone
    }

    /// Un geste est en cours (tracé, gomme, déplacement, saisie de texte) : ni relecture ni fusion maintenant.
    fn busy(&self) -> bool {
        self.drawing.is_some() || self.erasing || self.moving || self.text_edit.is_some()
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.sketch.elements.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn changed(&mut self, now: f64) {
        self.dirty = Some(now);
    }

    fn undo(&mut self, now: f64) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.sketch.elements, prev));
            self.after_replace();
            self.changed(now);
        }
    }

    fn redo(&mut self, now: f64) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.sketch.elements, next));
            self.after_replace();
            self.changed(now);
        }
    }

    /// La liste des éléments vient d'être remplacée : les indices gardés ne désignent plus rien.
    fn after_replace(&mut self) {
        self.selected = None;
        self.text_edit = None;
        self.drawing = None;
        self.erasing = false;
        self.moving = false;
    }

    /// Le disque montre `remote`, différent de notre base : un autre appareil a écrit. Fusion avec ce qui a été
    /// fait ici, et l'historique est oublié (un « annuler » ferait revenir l'état d'avant, sans leurs traits).
    fn absorb(&mut self, remote: Vec<Element>) {
        self.sketch.elements = if self.dirty.is_some() || self.sketch.elements != self.base {
            crate::sketch::merge(&self.base, &self.sketch.elements, &remote)
        } else {
            remote.clone()
        };
        self.base = remote;
        self.sent_from = None;
        self.undo.clear();
        self.redo.clear();
        self.after_replace();
    }

    /// Compare avec le fichier (après un changement signalé dans le dossier) et s'aligne dessus.
    fn sync_disk(&mut self, abs: &Path) {
        if self.busy() {
            self.stale = true;
            return;
        }
        self.stale = false;
        if !self.read_only() && self.mtime.is_some() && file_mtime(abs) == self.mtime {
            return;
        }
        match read_disk(abs) {
            // jamais vu : notre première écriture (ou le déplacement du dossier) n'est pas encore faite
            Disk::Missing => self.gone = self.seen,
            Disk::Unreadable(e) => {
                // des traits pas encore enregistrés : on les garde à l'écran, l'enregistrement attendra
                if self.dirty.is_none() {
                    self.error = Some(e);
                    self.gone = false;
                }
            }
            Disk::Ok(remote, m) => {
                self.error = None;
                self.gone = false;
                self.seen = true;
                if remote == self.base {
                    self.sent_from = None; // notre écriture est arrivée
                    self.mtime = m;
                } else if self.sent_from.as_ref() != Some(&remote) {
                    self.absorb(remote);
                    self.mtime = m;
                }
            }
        }
    }

    /// Cadre la vue sur tout le dessin.
    fn fit(&mut self, rect: egui::Rect) {
        match self.sketch.bounds() {
            Some(b) => {
                let (w, h) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
                self.zoom = ((rect.width() - 80.0) / w).min((rect.height() - 120.0) / h).clamp(0.1, 2.0);
                let center = egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                self.pan = rect.size() / (2.0 * self.zoom) - center;
            }
            None => {
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
            }
        }
    }

    fn commit_text(&mut self, now: f64) {
        let Some((pos, text, replace)) = self.text_edit.take() else { return };
        let text = text.trim_end().to_string();
        // l'indice ne vaut que s'il désigne encore un texte (la liste a pu changer depuis le double-clic)
        let old = replace.and_then(|i| match self.sketch.elements.get(i) {
            Some(Element::Text { text, .. }) => Some((i, text.clone())),
            _ => None,
        });
        match &old {
            Some((_, t)) if *t == text => return,
            None if text.is_empty() => return,
            _ => {}
        }
        self.checkpoint();
        match old {
            Some((i, _)) if text.is_empty() => {
                self.sketch.elements.remove(i);
            }
            Some((i, _)) => {
                if let Element::Text { text: t, .. } = &mut self.sketch.elements[i] {
                    *t = text;
                }
            }
            None => self.sketch.elements.push(Element::Text {
                color: self.color.into(),
                size: 12.0 + self.width * 3.0,
                x: pos[0],
                y: pos[1],
                text,
            }),
        }
        self.selected = None;
        self.changed(now);
    }
}

/// Couleur d'affichage d'un élément (l'encre suit le thème).
pub(super) fn display_color(c: &str, p: &theme::Palette) -> Color32 {
    if c.eq_ignore_ascii_case(INK) {
        return p.text;
    }
    let [r, g, b] = parse_color(c);
    Color32::from_rgb(r, g, b)
}

/// Dessine des éléments ; `to_screen` place un point du dessin, `scale` = taille d'une unité à l'écran.
pub(super) fn paint_elements<'a>(
    painter: &egui::Painter,
    elements: impl IntoIterator<Item = &'a Element>,
    to_screen: &dyn Fn([f32; 2]) -> egui::Pos2,
    scale: f32,
    p: &theme::Palette,
) {
    for el in elements {
        let color = display_color(el.color(), p);
        match el {
            Element::Stroke { points, width, .. } | Element::Line { points, width, .. } => {
                let stroke = egui::Stroke::new((width * scale).max(0.5), color);
                let pts: Vec<egui::Pos2> = points.iter().map(|q| to_screen(*q)).collect();
                if pts.len() == 1 {
                    painter.circle_filled(pts[0], stroke.width / 2.0, color);
                } else {
                    painter.add(egui::Shape::line(pts, stroke));
                }
            }
            Element::Arrow { points, width, .. } => {
                let stroke = egui::Stroke::new((width * scale).max(0.5), color);
                if points.len() >= 2 {
                    let (a, b) = (to_screen(points[0]), to_screen(points[points.len() - 1]));
                    painter.line_segment([a, b], stroke);
                    let dir = (b - a).normalized();
                    let head = (10.0 + width * 2.0) * scale;
                    let n = egui::vec2(-dir.y, dir.x);
                    for side in [1.0, -1.0] {
                        painter.line_segment([b, b - dir * head + n * head * 0.5 * side], stroke);
                    }
                }
            }
            Element::Rect { x, y, w, h, width, .. } => {
                let r = egui::Rect::from_two_pos(to_screen([*x, *y]), to_screen([x + w, y + h]));
                painter.rect_stroke(
                    r,
                    2.0,
                    egui::Stroke::new((width * scale).max(0.5), color),
                    egui::StrokeKind::Middle,
                );
            }
            Element::Ellipse { x, y, w, h, width, .. } => {
                let r = egui::Rect::from_two_pos(to_screen([*x, *y]), to_screen([x + w, y + h]));
                painter.add(egui::Shape::ellipse_stroke(
                    r.center(),
                    r.size() / 2.0,
                    egui::Stroke::new((width * scale).max(0.5), color),
                ));
            }
            Element::Text { x, y, text, size, .. } => {
                let galley = painter.layout_no_wrap(text.clone(), FontId::proportional((size * scale).max(1.0)), color);
                painter.galley(to_screen([*x, *y]), galley, color);
            }
            // élément d'un format plus récent : gardé dans le fichier, pas dessiné
            Element::Unknown(_) => {}
        }
    }
}

/// Aperçu d'un dessin entier dans un rectangle (intégration dans une note).
pub(super) fn paint_preview(painter: &egui::Painter, sketch: &Sketch, rect: egui::Rect, p: &theme::Palette) {
    let Some(b) = sketch.bounds() else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Whiteboard vide",
            FontId::proportional(13.0),
            p.faint,
        );
        return;
    };
    let (w, h) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
    let inner = rect.shrink(12.0);
    let scale = (inner.width() / w).min(inner.height() / h).min(2.0);
    let offset = inner.center() - egui::vec2((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0) * scale;
    let to_screen = |q: [f32; 2]| egui::pos2(q[0] * scale + offset.x, q[1] * scale + offset.y);
    paint_elements(&painter.with_clip_rect(rect), &sketch.elements, &to_screen, scale, p);
}

impl MnemoApp {
    /// Ouvre un whiteboard dans un onglet (réutilise l'onglet s'il est déjà ouvert).
    pub(super) fn open_whiteboard(&mut self, rel: &str) {
        self.leave_note();
        match self.tabs.iter().position(|t| t.board.as_deref() == Some(rel)) {
            Some(i) => self.active_tab = i,
            None => {
                self.tabs.push(Tab { board: Some(rel.to_string()), ..Default::default() });
                self.active_tab = self.tabs.len() - 1;
            }
        }
        self.open = None;
        if !self.boards.contains_key(rel)
            && let Some(abs) = self.file_abs(rel)
        {
            self.boards.insert(rel.to_string(), BoardView::load(&abs));
        }
    }

    /// Crée un whiteboard vide dans `folder` et l'ouvre.
    pub(super) fn new_whiteboard(&mut self, folder: &str) {
        let base = format!("Whiteboard {}", chrono::Local::now().format("%Y-%m-%d %H%M"));
        let mut n = 0;
        let rel = loop {
            let name = if n == 0 { base.clone() } else { format!("{base} {n}") };
            let rel = if folder.is_empty() {
                format!("{name}.{}", crate::sketch::EXTENSION)
            } else {
                format!("{folder}/{name}.{}", crate::sketch::EXTENSION)
            };
            if !self.files.iter().any(|f| f.eq_ignore_ascii_case(&rel)) {
                break rel;
            }
            n += 1;
        };
        self.db.send(Cmd::WriteFile { path: rel.clone(), bytes: Sketch::default().to_json().into_bytes() });
        self.files.push(rel.clone());
        self.boards.insert(rel.clone(), BoardView::empty());
        self.open_whiteboard(&rel);
    }

    /// Enregistre les whiteboards modifiés depuis `SAVE_DELAY`. Avec `force` (fermeture de l'application) :
    /// tous, tout de suite, texte en cours de saisie compris, et l'écriture est faite avant de rendre la main.
    pub(super) fn tick_whiteboards(&mut self, now: f64, force: bool) {
        let Some(root) = self.cfg.vault_dir.clone() else { return };
        let shown = self.tabs.get(self.active_tab).and_then(|t| t.board.clone());
        for (rel, b) in self.boards.iter_mut() {
            let abs = root.join(rel);
            // onglet caché (ou fermeture) : aucun geste ne peut continuer, le texte en cours de saisie est validé
            if force || shown.as_deref() != Some(rel.as_str()) {
                b.commit_text(now);
                b.after_replace();
            }
            if b.busy() {
                continue;
            }
            if b.stale {
                b.sync_disk(&abs);
            }
            if b.error.is_some() || !b.dirty.is_some_and(|t| force || now - t >= SAVE_DELAY) {
                continue;
            }
            // fichier disparu : pas recréé à l'ancien endroit sans l'accord de l'utilisateur (sauf à la
            // fermeture, pour ne pas perdre les derniers traits)
            if b.gone && !force {
                continue;
            }
            // le fichier a-t-il changé depuis notre dernière lecture ? (autre appareil, synchronisation)
            if b.mtime.is_none() || file_mtime(&abs) != b.mtime {
                match read_disk(&abs) {
                    Disk::Ok(remote, _) => {
                        b.seen = true;
                        if remote != b.base && b.sent_from.as_ref() != Some(&remote) {
                            b.absorb(remote);
                        }
                    }
                    Disk::Missing if b.seen && !force => {
                        b.gone = true;
                        continue;
                    }
                    // illisible pour l'instant (écriture en cours ailleurs, fichier verrouillé) : on réessaiera
                    Disk::Unreadable(_) if !force => {
                        b.dirty = Some(now);
                        continue;
                    }
                    _ => {}
                }
            }
            let bytes = b.sketch.to_json().into_bytes();
            b.dirty = None;
            b.sent_from = Some(std::mem::replace(&mut b.base, b.sketch.elements.clone()));
            // la date du fichier sera celle de notre écriture : `sync_disk` la reprendra en la voyant arrivée
            b.mtime = None;
            if force && write_now(&abs, &bytes).is_ok() {
                b.sent_from = None;
                b.mtime = file_mtime(&abs);
                b.gone = false;
            } else {
                self.db.send(Cmd::WriteFile { path: rel.clone(), bytes });
            }
        }
    }

    /// Un whiteboard ouvert attend un enregistrement ou une relecture (pour redemander une frame même si son
    /// onglet n'est pas affiché).
    pub(super) fn boards_pending(&self) -> bool {
        self.boards.values().any(|b| (b.dirty.is_some() && !b.read_only()) || b.stale)
    }

    /// Les pièces jointes ont changé sur le disque (autre appareil, synchronisation, notre propre écriture) :
    /// chaque whiteboard ouvert s'aligne sur son fichier (rechargé, fusionné s'il a des traits non enregistrés,
    /// en lecture seule s'il est devenu illisible ou a disparu).
    pub(super) fn reload_whiteboards(&mut self) {
        let Some(root) = self.cfg.vault_dir.clone() else { return };
        for (rel, b) in self.boards.iter_mut() {
            b.sync_disk(&root.join(rel));
        }
    }

    /// Le dossier `from` est renommé ou déplacé en `to` depuis l'interface : les whiteboards ouverts qu'il
    /// contient suivent (onglets et dessins), au lieu de recréer l'ancien chemin au prochain trait.
    pub(super) fn boards_moved(&mut self, from: &str, to: &str) {
        // mêmes refus que le stockage local : le dossier ne bougera pas
        let exists = self.file_abs(to).is_some_and(|p| p.exists());
        if exists || portable_name(to).is_err() {
            return;
        }
        for t in &mut self.tabs {
            if let Some(new) = t.board.as_deref().and_then(|b| moved_path(b, from, to)) {
                t.board = Some(new);
            }
        }
        let keys: Vec<String> = self.boards.keys().cloned().collect();
        for k in keys {
            if let Some(new) = moved_path(&k, from, to)
                && let Some(mut b) = self.boards.remove(&k)
            {
                // le fichier n'est pas encore à sa nouvelle place : ne pas le croire supprimé
                b.seen = false;
                b.mtime = None;
                self.boards.insert(new, b);
            }
        }
    }

    /// Un fichier (ou tout un dossier, `folder`) est supprimé depuis l'interface : les onglets des whiteboards
    /// concernés sont fermés, sans rien enregistrer.
    pub(super) fn boards_deleted(&mut self, path: &str, folder: bool) {
        let hit = |rel: &str| {
            if folder { moved_path(rel, path, "").is_some() } else { rel.eq_ignore_ascii_case(path) }
        };
        self.boards.retain(|rel, _| !hit(rel));
        for i in (0..self.tabs.len()).rev() {
            if self.tabs[i].board.as_deref().is_some_and(&hit) {
                self.close_tab(i);
            }
        }
    }

    pub(super) fn whiteboard_ui(&mut self, ui: &mut egui::Ui, rel: &str, now: f64) {
        let p = self.palette();
        let abs = self.file_abs(rel);
        let Some(b) = self.boards.get_mut(rel) else {
            ui.label("Whiteboard introuvable.");
            return;
        };
        // ---- en-tête : nom du fichier
        let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        let title = basename(rel).trim_end_matches(&format!(".{}", crate::sketch::EXTENSION)).to_string();
        ui.painter().text(head.center(), egui::Align2::CENTER_CENTER, &title, FontId::proportional(14.0), p.text);
        if b.dirty.is_some() && !b.read_only() {
            ui.painter().text(
                egui::pos2(head.right() - 16.0, head.center().y),
                egui::Align2::RIGHT_CENTER,
                "enregistrement…",
                FontId::proportional(12.0),
                p.faint,
            );
        }

        // ---- surface
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, p.bg);
        if !b.fitted {
            b.fit(rect);
            b.fitted = true;
        }
        let (pan, zoom) = (b.pan, b.zoom);
        let to_screen = |q: [f32; 2]| rect.min + (egui::vec2(q[0], q[1]) + pan) * zoom;
        let to_world = |s: egui::Pos2| {
            let v = (s - rect.min) / zoom - pan;
            [v.x, v.y]
        };
        // grille de points
        let step = 24.0 * zoom;
        if step > 6.0 {
            let start = rect.min + egui::vec2((pan.x * zoom).rem_euclid(step), (pan.y * zoom).rem_euclid(step));
            let dot = p.border;
            let mut y = start.y;
            while y < rect.bottom() {
                let mut x = start.x;
                while x < rect.right() {
                    painter.circle_filled(egui::pos2(x, y), 1.0, dot);
                    x += step;
                }
                y += step;
            }
        }

        // ---- fichier illisible ou disparu : lecture seule, rien n'est enregistré
        if b.read_only() {
            paint_elements(&painter, &b.sketch.elements, &to_screen, zoom, &p);
            let (msg, detail, action) = match &b.error {
                Some(e) => (
                    "Ce whiteboard ne peut pas être lu. Il est en lecture seule : son fichier n'est pas modifié.",
                    e.clone(),
                    "Recharger",
                ),
                None => (
                    "Le fichier de ce whiteboard a été déplacé ou supprimé en dehors de Mnemo.",
                    rel.to_string(),
                    "Recréer le fichier ici",
                ),
            };
            let (mut retry, mut close) = (false, false);
            let area = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width().min(560.0) - 32.0, 150.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add(egui::Label::new(RichText::new(msg).color(p.text)).wrap());
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(detail).size(12.0).color(RED)).wrap());
                            ui.add_space(10.0);
                            retry = ui.button(action).clicked();
                            ui.add_space(4.0);
                            close = ui.button("Fermer l'onglet").clicked();
                        });
                    });
            });
            if retry {
                if b.error.is_some() {
                    if let Some(abs) = &abs {
                        *b = BoardView::load(abs);
                    }
                } else {
                    // l'utilisateur veut le dessin à cet endroit : il y sera écrit au prochain enregistrement
                    b.gone = false;
                    b.seen = false;
                    b.mtime = None;
                    b.dirty = Some(now - SAVE_DELAY);
                }
            }
            if close {
                self.boards.remove(rel);
                self.actions.push(UiAction::CloseTab(self.active_tab));
            }
            return;
        }

        // ---- clavier (si aucun champ texte n'a le focus)
        // raccourcis actifs dans tout l'onglet (pointeur sur la barre d'outils compris), sauf pendant une saisie
        // ou un geste : changer d'outil ou annuler au milieu d'un tracé laisserait un geste à moitié fait
        let typing = b.text_edit.is_some() || ui.ctx().memory(|m| m.focused().is_some());
        if !typing && !b.busy() && !resp.is_pointer_button_down_on() {
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y)
                {
                    b.redo(now);
                } else if i.consume_key(Modifiers::COMMAND, Key::Z) {
                    b.undo(now);
                }
                if let Some(s) = b.selected
                    && (i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace))
                    && s < b.sketch.elements.len()
                {
                    b.checkpoint();
                    b.sketch.elements.remove(s);
                    b.selected = None;
                    b.changed(now);
                }
                for (t, _, _, k) in Tool::ALL {
                    if i.consume_key(Modifiers::NONE, k) {
                        b.tool = t;
                        b.selected = None;
                    }
                }
            });
        }

        // ---- vue : molette = défilement, Ctrl+molette ou pincement = zoom (autour du pointeur)
        if resp.hovered() {
            let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if zoom_delta != 1.0
                && let Some(ptr) = resp.hover_pos()
            {
                let before = to_world(ptr);
                b.zoom = (b.zoom * zoom_delta).clamp(0.05, 8.0);
                b.pan = (ptr - rect.min) / b.zoom - egui::vec2(before[0], before[1]);
            } else if scroll != egui::Vec2::ZERO {
                b.pan += scroll / b.zoom;
            }
        }
        let panning =
            b.tool == Tool::Hand || ui.input(|i| i.pointer.middle_down() || i.key_down(Key::Space)) && resp.dragged();
        if panning && resp.dragged() {
            b.pan += resp.drag_delta() / b.zoom;
        }

        // ---- outils
        let tol = 6.0 / b.zoom;
        if !panning {
            let world = resp.interact_pointer_pos().map(to_world);
            match b.tool {
                Tool::Pen | Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse => {
                    if resp.drag_started()
                        && let Some(w) = world
                    {
                        let (color, width) = (b.color.to_string(), b.width);
                        b.drawing = Some(match b.tool {
                            Tool::Pen => Element::Stroke { color, width, points: vec![w] },
                            Tool::Line => Element::Line { color, width, points: vec![w, w] },
                            Tool::Arrow => Element::Arrow { color, width, points: vec![w, w] },
                            Tool::Rect => Element::Rect { color, width, x: w[0], y: w[1], w: 0.0, h: 0.0 },
                            _ => Element::Ellipse { color, width, x: w[0], y: w[1], w: 0.0, h: 0.0 },
                        });
                    } else if resp.dragged()
                        && let (Some(w), Some(el)) = (world, b.drawing.as_mut())
                    {
                        match el {
                            Element::Stroke { points, .. } => {
                                let far = points.last().is_none_or(|l| (w[0] - l[0]).hypot(w[1] - l[1]) * zoom > 1.5);
                                if far {
                                    points.push(w);
                                }
                            }
                            Element::Line { points, .. } | Element::Arrow { points, .. } => {
                                if let Some(end) = points.get_mut(1) {
                                    *end = w;
                                }
                            }
                            Element::Rect { x, y, w: ww, h, .. } | Element::Ellipse { x, y, w: ww, h, .. } => {
                                *ww = w[0] - *x;
                                *h = w[1] - *y;
                            }
                            Element::Text { .. } | Element::Unknown(_) => {}
                        }
                    }
                    if resp.drag_stopped()
                        && let Some(mut el) = b.drawing.take()
                    {
                        if let Element::Stroke { points, .. } = &mut el {
                            *points = simplify(points, 0.6 / zoom);
                        }
                        let b2 = el.bounds();
                        let tiny =
                            (b2[2] - b2[0]).max(b2[3] - b2[1]) * zoom < 3.0 && !matches!(el, Element::Stroke { .. });
                        if !tiny {
                            b.checkpoint();
                            b.sketch.elements.push(el);
                            b.changed(now);
                        }
                    }
                    // simple clic au stylo : un point
                    if resp.clicked()
                        && b.tool == Tool::Pen
                        && let Some(w) = world
                    {
                        b.checkpoint();
                        b.sketch.elements.push(Element::Stroke {
                            color: b.color.into(),
                            width: b.width,
                            points: vec![w],
                        });
                        b.changed(now);
                    }
                }
                Tool::Eraser => {
                    if (resp.dragged() || resp.clicked())
                        && let Some(w) = world
                        && let Some(i) = b.sketch.hit(w, tol)
                    {
                        // un seul « annuler » pour tout le coup de gomme
                        if !b.erasing {
                            b.checkpoint();
                            b.erasing = true;
                        }
                        b.sketch.elements.remove(i);
                        b.changed(now);
                    }
                }
                Tool::Select => {
                    if resp.clicked() {
                        b.selected = world.and_then(|w| b.sketch.hit(w, tol));
                    }
                    if resp.double_clicked()
                        && let Some(i) = b.selected
                        && let Some(Element::Text { x, y, text, .. }) = b.sketch.elements.get(i)
                    {
                        b.text_edit = Some(([*x, *y], text.clone(), Some(i)));
                        b.focus_text = true;
                    }
                    if resp.drag_started() {
                        b.selected = world.and_then(|w| b.sketch.hit(w, tol));
                        if b.selected.is_some() {
                            b.checkpoint();
                            b.moving = true;
                        }
                    }
                    if resp.dragged()
                        && b.moving
                        && let Some(el) = b.selected.and_then(|i| b.sketch.elements.get_mut(i))
                    {
                        let d = resp.drag_delta() / zoom;
                        el.translate(d.x, d.y);
                        b.dirty = Some(now);
                    }
                }
                Tool::Text => {
                    if resp.clicked()
                        && let Some(w) = world
                    {
                        b.commit_text(now);
                        b.text_edit = Some((w, String::new(), None));
                        b.focus_text = true;
                    }
                }
                Tool::Hand => {}
            }
        }
        // plus de bouton enfoncé sur la surface : aucun geste ne reste « en cours »
        if !resp.dragged() {
            b.drawing = None;
            b.erasing = false;
            b.moving = false;
        }

        // ---- dessin
        let editing = b.text_edit.as_ref().and_then(|t| t.2);
        let visible = b.sketch.elements.iter().enumerate().filter(|(i, _)| Some(*i) != editing).map(|(_, e)| e);
        paint_elements(&painter, visible, &to_screen, zoom, &p);
        if let Some(el) = &b.drawing {
            paint_elements(&painter, std::slice::from_ref(el), &to_screen, zoom, &p);
        }
        if let Some(i) = b.selected
            && let Some(el) = b.sketch.elements.get(i)
        {
            let bb = el.bounds();
            let r = egui::Rect::from_two_pos(to_screen([bb[0], bb[1]]), to_screen([bb[2], bb[3]])).expand(4.0);
            painter.rect_stroke(r, 3.0, egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Outside);
        }
        if b.tool == Tool::Eraser
            && let Some(ptr) = resp.hover_pos()
        {
            painter.circle_stroke(ptr, 6.0, egui::Stroke::new(1.0, p.muted));
        }

        // ---- saisie de texte, à l'endroit cliqué
        if let Some((pos, text, _)) = b.text_edit.as_mut() {
            let size = (12.0 + b.width * 3.0) * zoom;
            let color = display_color(b.color, &p);
            let r = egui::Area::new(egui::Id::unique("board-text"))
                .fixed_pos(to_screen(*pos))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .font(FontId::proportional(size.max(8.0)))
                            .text_color(color)
                            .frame(egui::Frame::NONE)
                            .background_color(Color32::TRANSPARENT)
                            .desired_rows(1)
                            .desired_width(400.0)
                            .hint_text("Texte…"),
                    )
                })
                .inner;
            if std::mem::take(&mut b.focus_text) {
                r.request_focus();
            }
            let done = ui.input(|i| i.key_pressed(Key::Escape))
                || (r.lost_focus() && !ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.shift));
            if done {
                b.commit_text(now);
            }
        }

        // ---- barre d'outils flottante (en haut, centrée)
        let bar_pos = egui::pos2(rect.center().x, rect.top() + 12.0);
        egui::Area::new(egui::Id::unique(("board-toolbar", rel)))
            .fixed_pos(bar_pos)
            .pivot(egui::Align2::CENTER_TOP)
            .order(egui::Order::Middle)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 2],
                        blur: 8,
                        spread: 0,
                        color: Color32::from_black_alpha(40),
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (t, ic, tip, _) in Tool::ALL {
                                let glyph = theme::lucide(ic).map(String::from).unwrap_or_default();
                                if theme::icon_button_sized(ui, &glyph, tip, b.tool == t, true, 16.0).clicked() {
                                    b.commit_text(now);
                                    b.after_replace();
                                    b.tool = t;
                                }
                            }
                            ui.add_space(6.0);
                            for c in COLORS {
                                let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 28.0), egui::Sense::click());
                                let col = display_color(c, &p);
                                ui.painter().circle_filled(r.center(), 7.0, col);
                                if b.color == c {
                                    ui.painter().circle_stroke(r.center(), 10.0, egui::Stroke::new(1.5, p.accent));
                                }
                                if resp.on_hover_text("Couleur").clicked() {
                                    b.color = c;
                                    // recolorer la sélection est une modification comme une autre : annulable
                                    let recolor = b.selected.filter(|i| {
                                        b.sketch.elements.get(*i).is_some_and(|el| {
                                            !matches!(el, Element::Unknown(_)) && !el.color().eq_ignore_ascii_case(c)
                                        })
                                    });
                                    if let Some(i) = recolor {
                                        b.checkpoint();
                                        b.sketch.elements[i].set_color(c);
                                        b.changed(now);
                                    }
                                }
                            }
                            ui.add_space(6.0);
                            for w in WIDTHS {
                                let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 28.0), egui::Sense::click());
                                if b.width == w || resp.hovered() {
                                    ui.painter().rect_filled(
                                        r.shrink(2.0),
                                        5.0,
                                        if b.width == w { p.active } else { p.hover },
                                    );
                                }
                                ui.painter().line_segment(
                                    [r.center() - egui::vec2(7.0, 0.0), r.center() + egui::vec2(7.0, 0.0)],
                                    egui::Stroke::new(w * 0.75, p.text),
                                );
                                if resp.on_hover_text("Épaisseur / taille du texte").clicked() {
                                    b.width = w;
                                }
                            }
                            ui.add_space(6.0);
                            let undo = theme::lucide("undo-2").map(String::from).unwrap_or_default();
                            let redo = theme::lucide("redo-2").map(String::from).unwrap_or_default();
                            let fit = theme::lucide("scan").map(String::from).unwrap_or_default();
                            if theme::icon_button_sized(ui, &undo, "Annuler (Ctrl+Z)", false, !b.undo.is_empty(), 16.0)
                                .clicked()
                            {
                                // le texte en cours de saisie est d'abord validé (puis annulé avec le reste)
                                b.commit_text(now);
                                b.undo(now);
                            }
                            if theme::icon_button_sized(
                                ui,
                                &redo,
                                "Rétablir (Ctrl+Maj+Z)",
                                false,
                                !b.redo.is_empty(),
                                16.0,
                            )
                            .clicked()
                            {
                                b.commit_text(now);
                                b.redo(now);
                            }
                            if theme::icon_button_sized(ui, &fit, "Tout afficher", false, true, 16.0).clicked() {
                                b.fit(rect);
                            }
                        });
                    });
            });
        if b.dirty.is_some() || b.busy() || b.stale {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(x: f32) -> Element {
        Element::Stroke { color: INK.into(), width: 2.0, points: vec![[x, 0.0], [x, 5.0]] }
    }

    /// Chemin d'un fichier de test dans un dossier temporaire neuf.
    fn tmp_board() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mnemo-board-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("b.mnemo-board")
    }

    fn write(path: &Path, elements: &[Element]) {
        std::fs::write(path, Sketch { version: 1, elements: elements.to_vec() }.to_json()).unwrap();
    }

    #[test]
    fn folder_move_remaps_paths() {
        let moved = |p: &str, from: &str, to: &str| moved_path(p, from, to);
        assert_eq!(moved("Projets/a.mnemo-board", "projets", "Archive/P").as_deref(), Some("Archive/P/a.mnemo-board"));
        assert_eq!(moved("Projets/Sous/a.mnemo-board", "Projets", "X").as_deref(), Some("X/Sous/a.mnemo-board"));
        assert_eq!(moved("Projets2/a.mnemo-board", "Projets", "X"), None);
        assert_eq!(moved("Projets", "Projets", "X"), None);
        assert_eq!(moved("Été/b.mnemo-board", "été", "X").as_deref(), Some("X/b.mnemo-board"));
    }

    #[test]
    fn unreadable_board_is_read_only_and_kept() {
        let abs = tmp_board();
        std::fs::write(&abs, "{ pas du json").unwrap();
        let mut b = BoardView::load(&abs);
        assert!(b.error.is_some() && b.read_only());
        // un changement signalé ne fait pas oublier l'erreur tant que le fichier reste illisible
        b.sync_disk(&abs);
        assert!(b.error.is_some());
        // réparé ailleurs : le dessin revient
        write(&abs, &[stroke(1.0)]);
        b.sync_disk(&abs);
        assert!(b.error.is_none() && !b.read_only());
        assert_eq!(b.sketch.elements, vec![stroke(1.0)]);
        // fichier supprimé hors de Mnemo : lecture seule, pas d'erreur de lecture
        std::fs::remove_file(&abs).unwrap();
        b.sync_disk(&abs);
        assert!(b.gone && b.read_only());
        assert!(BoardView::load(&abs).gone);
    }

    #[test]
    fn remote_change_is_merged_and_history_dropped() {
        let abs = tmp_board();
        write(&abs, &[stroke(1.0)]);
        let mut b = BoardView::load(&abs);
        // un trait ici, pas encore enregistré
        b.checkpoint();
        b.sketch.elements.push(stroke(2.0));
        b.changed(0.0);
        // pendant ce temps un autre appareil ajoute le sien
        write(&abs, &[stroke(1.0), stroke(3.0)]);
        b.mtime = None; // (la date du fichier peut ne pas avoir bougé dans la même seconde)
        b.sync_disk(&abs);
        assert_eq!(b.sketch.elements, vec![stroke(1.0), stroke(3.0), stroke(2.0)]);
        assert!(b.undo.is_empty() && b.redo.is_empty() && b.is_dirty());
        // sans rien de local : simple rechargement
        b.dirty = None;
        b.base = b.sketch.elements.clone();
        write(&abs, &[stroke(3.0)]);
        b.mtime = None;
        b.sync_disk(&abs);
        assert_eq!(b.sketch.elements, vec![stroke(3.0)]);
        // pendant un geste : rien ne bouge, relecture remise à plus tard
        b.erasing = true;
        write(&abs, &[]);
        b.mtime = None;
        b.sync_disk(&abs);
        assert!(b.stale && b.sketch.elements == vec![stroke(3.0)]);
    }

    #[test]
    fn own_write_in_flight_is_not_a_remote_change() {
        let abs = tmp_board();
        write(&abs, &[stroke(1.0)]);
        let mut b = BoardView::load(&abs);
        // enregistrement envoyé (pas encore écrit par le stockage local)
        b.sketch.elements.push(stroke(2.0));
        b.sent_from = Some(std::mem::replace(&mut b.base, b.sketch.elements.clone()));
        b.mtime = None;
        b.sync_disk(&abs);
        assert_eq!(b.sketch.elements, vec![stroke(1.0), stroke(2.0)]);
        assert!(b.mtime.is_none());
        // l'écriture arrive
        write(&abs, &[stroke(1.0), stroke(2.0)]);
        b.sync_disk(&abs);
        assert!(b.mtime.is_some() && b.sent_from.is_none());
    }

    #[test]
    fn stale_text_index_never_overwrites_another_element() {
        let mut b = BoardView::empty();
        b.sketch.elements = vec![stroke(1.0)];
        // l'indice désignait un texte qui n'est plus là : le texte saisi devient un nouvel élément
        b.text_edit = Some(([0.0, 0.0], "bonjour".into(), Some(0)));
        b.commit_text(1.0);
        assert_eq!(b.sketch.elements.len(), 2);
        assert_eq!(b.sketch.elements[0], stroke(1.0));
        assert!(matches!(&b.sketch.elements[1], Element::Text { text, .. } if text == "bonjour"));
        // annuler oublie la saisie en cours
        b.text_edit = Some(([0.0, 0.0], "x".into(), Some(1)));
        b.undo(2.0);
        assert!(b.text_edit.is_none() && b.sketch.elements == vec![stroke(1.0)]);
        // texte vide sans cible : aucun point d'annulation, « rétablir » reste possible
        b.text_edit = Some(([0.0, 0.0], "  ".into(), None));
        b.commit_text(3.0);
        assert_eq!(b.redo.len(), 1);
    }
}
