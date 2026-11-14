//! Whiteboard à dessin libre dans un onglet : stylo, ligne, flèche, rectangle, ellipse, texte, gomme,
//! sélection et déplacement, couleurs, épaisseurs, annuler / rétablir, zoom et déplacement de la vue.
//! Le dessin est un fichier `.mnemo-board` du dossier des notes (voir `crate::sketch`), enregistré
//! automatiquement et synchronisé comme les autres pièces jointes.

use super::*;
use crate::sketch::{Element, Sketch, parse_color, simplify};

/// Délai entre le dernier trait et l'enregistrement.
const SAVE_DELAY: f64 = 0.8;

/// Couleur « encre » : noire sur fond clair, claire sur fond sombre (comme Excalidraw).
pub(super) const INK: &str = "#1e1e1e";
pub(super) const COLORS: [&str; 6] = [INK, "#e03131", "#1971c2", "#2f9e44", "#f08c00", "#9c36b5"];
const WIDTHS: [f32; 3] = [2.0, 4.0, 8.0];

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

/// Un whiteboard ouvert.
pub(super) struct BoardView {
    pub(super) sketch: Sketch,
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
    /// texte en cours de saisie : position (dessin), contenu, indice s'il remplace un texte existant
    text_edit: Option<([f32; 2], String, Option<usize>)>,
    focus_text: bool,
    /// modifié depuis le dernier enregistrement (moment de la dernière modification)
    dirty: Option<f64>,
    /// date du fichier au chargement / au dernier enregistrement (pour recharger un changement extérieur)
    pub(super) mtime: Option<std::time::SystemTime>,
    pub(super) error: Option<String>,
}

impl BoardView {
    pub(super) fn load(abs: &std::path::Path) -> Self {
        let (sketch, error) = match std::fs::read_to_string(abs) {
            Ok(s) => match Sketch::parse(&s) {
                Ok(sk) => (sk, None),
                Err(e) => (Sketch::default(), Some(e)),
            },
            Err(_) => (Sketch::default(), None),
        };
        Self {
            sketch,
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
            text_edit: None,
            focus_text: false,
            dirty: None,
            mtime: std::fs::metadata(abs).and_then(|m| m.modified()).ok(),
            error,
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty.is_some()
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
            self.selected = None;
            self.changed(now);
        }
    }

    fn redo(&mut self, now: f64) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.sketch.elements, next));
            self.selected = None;
            self.changed(now);
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
        self.checkpoint();
        match replace {
            Some(i) if i < self.sketch.elements.len() => {
                if text.is_empty() {
                    self.sketch.elements.remove(i);
                } else if let Element::Text { text: t, .. } = &mut self.sketch.elements[i] {
                    *t = text;
                }
            }
            _ if text.is_empty() => {
                self.undo.pop();
                return;
            }
            _ => self.sketch.elements.push(Element::Text {
                color: self.color.into(),
                size: 12.0 + self.width * 3.0,
                x: pos[0],
                y: pos[1],
                text,
            }),
        }
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
pub(super) fn paint_elements(
    painter: &egui::Painter,
    elements: &[Element],
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
        self.flush_save();
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
        self.boards.insert(rel.clone(), BoardView::load(std::path::Path::new("")));
        self.open_whiteboard(&rel);
        if let Some(b) = self.boards.get_mut(&rel) {
            b.mtime = None;
        }
    }

    /// Enregistre les whiteboards modifiés depuis `SAVE_DELAY` (ou tous si `force`).
    pub(super) fn tick_whiteboards(&mut self, now: f64, force: bool) {
        let due: Vec<String> = self
            .boards
            .iter()
            .filter(|(_, b)| b.dirty.is_some_and(|t| force || now - t >= SAVE_DELAY))
            .map(|(k, _)| k.clone())
            .collect();
        for rel in due {
            let b = self.boards.get_mut(&rel).expect("board");
            b.dirty = None;
            let bytes = b.sketch.to_json().into_bytes();
            self.db.send(Cmd::WriteFile { path: rel.clone(), bytes });
            // la date du fichier sera celle de notre écriture : ne pas le prendre pour un changement extérieur
            b.mtime = None;
        }
    }

    /// Un whiteboard ouvert a changé sur le disque (autre appareil) : recharge s'il n'est pas en cours d'édition.
    pub(super) fn reload_whiteboards(&mut self) {
        let root = self.cfg.vault_dir.clone();
        for (rel, b) in self.boards.iter_mut() {
            let Some(abs) = root.as_ref().map(|r| r.join(rel)) else { continue };
            let m = std::fs::metadata(&abs).and_then(|m| m.modified()).ok();
            if b.mtime.is_none() {
                b.mtime = m; // notre propre écriture
            } else if m != b.mtime && b.dirty.is_none() && b.drawing.is_none() {
                let fresh = BoardView::load(&abs);
                b.sketch = fresh.sketch;
                b.mtime = fresh.mtime;
                b.selected = None;
            }
        }
    }

    pub(super) fn whiteboard_ui(&mut self, ui: &mut egui::Ui, rel: &str, now: f64) {
        let p = self.palette();
        let Some(b) = self.boards.get_mut(rel) else {
            ui.label("Whiteboard introuvable.");
            return;
        };
        // ---- en-tête : nom du fichier
        let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        let title = basename(rel).trim_end_matches(&format!(".{}", crate::sketch::EXTENSION)).to_string();
        ui.painter().text(head.center(), egui::Align2::CENTER_CENTER, &title, FontId::proportional(14.0), p.text);
        if b.dirty.is_some() {
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

        // ---- clavier (si aucun champ texte n'a le focus)
        // raccourcis actifs dans tout l'onglet (pointeur sur la barre d'outils compris), sauf pendant une saisie
        let typing = b.text_edit.is_some() || ui.ctx().memory(|m| m.focused().is_some());
        if !typing {
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
                                let last = points[points.len() - 1];
                                if (w[0] - last[0]).hypot(w[1] - last[1]) * zoom > 1.5 {
                                    points.push(w);
                                }
                            }
                            Element::Line { points, .. } | Element::Arrow { points, .. } => points[1] = w,
                            Element::Rect { x, y, w: ww, h, .. } | Element::Ellipse { x, y, w: ww, h, .. } => {
                                *ww = w[0] - *x;
                                *h = w[1] - *y;
                            }
                            Element::Text { .. } => {}
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
                        if resp.drag_started() || resp.clicked() || b.drawing.is_none() {
                            b.checkpoint();
                            // marqueur de geste : un seul « annuler » pour tout le coup de gomme
                            b.drawing = Some(Element::Line { color: String::new(), width: 0.0, points: vec![] });
                        }
                        b.sketch.elements.remove(i);
                        b.changed(now);
                    }
                    if resp.drag_stopped() || resp.clicked() {
                        b.drawing = None;
                    }
                }
                Tool::Select => {
                    if resp.clicked() {
                        b.selected = world.and_then(|w| b.sketch.hit(w, tol));
                    }
                    if resp.double_clicked()
                        && let Some(i) = b.selected
                        && let Element::Text { x, y, text, .. } = &b.sketch.elements[i]
                    {
                        b.text_edit = Some(([*x, *y], text.clone(), Some(i)));
                        b.focus_text = true;
                    }
                    if resp.drag_started() {
                        b.selected = world.and_then(|w| b.sketch.hit(w, tol));
                        if b.selected.is_some() {
                            b.checkpoint();
                        }
                    }
                    if resp.dragged()
                        && let Some(i) = b.selected
                    {
                        let d = resp.drag_delta() / zoom;
                        b.sketch.elements[i].translate(d.x, d.y);
                        b.changed(now);
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

        // ---- dessin
        let editing = b.text_edit.as_ref().and_then(|t| t.2);
        let visible: Vec<Element> =
            b.sketch.elements.iter().enumerate().filter(|(i, _)| Some(*i) != editing).map(|(_, e)| e.clone()).collect();
        paint_elements(&painter, &visible, &to_screen, zoom, &p);
        if let Some(el) = &b.drawing
            && !el.color().is_empty()
        {
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
                                    b.tool = t;
                                    b.selected = None;
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
                                    if let Some(i) = b.selected
                                        && let Some(el) = b.sketch.elements.get_mut(i)
                                    {
                                        set_color(el, c);
                                        b.dirty = Some(now);
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
                                b.redo(now);
                            }
                            if theme::icon_button_sized(ui, &fit, "Tout afficher", false, true, 16.0).clicked() {
                                b.fit(rect);
                            }
                        });
                    });
            });
        if let Some(e) = &b.error {
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - 20.0),
                egui::Align2::CENTER_CENTER,
                e,
                FontId::proportional(12.0),
                Color32::from_rgb(0xfb, 0x46, 0x4c),
            );
        }
        if b.dirty.is_some() || b.drawing.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
}

fn set_color(el: &mut Element, c: &str) {
    match el {
        Element::Stroke { color, .. }
        | Element::Rect { color, .. }
        | Element::Ellipse { color, .. }
        | Element::Line { color, .. }
        | Element::Arrow { color, .. }
        | Element::Text { color, .. } => *color = c.to_string(),
    }
}
