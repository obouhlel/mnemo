//! Interactive graph of notes and links (force-directed layout painted with egui).

use crate::model::GraphData;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use std::collections::HashMap;

#[derive(Default)]
pub struct GraphView {
    pub open: bool,
    ids: Vec<String>,
    labels: Vec<String>,
    paths: Vec<String>,
    ghost: Vec<bool>,
    degree: Vec<usize>,
    pos: Vec<Vec2>,
    vel: Vec<Vec2>,
    edges: Vec<(usize, usize)>,
    alpha: f32,
    pan: Vec2,
    zoom: f32,
    pub show_ghosts: bool,
    pub loaded: bool,
    /// keep fitting the view to the layout until the user pans/zooms
    auto_fit: bool,
}

impl GraphView {
    pub fn set_data(&mut self, data: GraphData) {
        // keep previous positions for nodes that still exist (smooth live updates)
        let old: HashMap<String, Vec2> = self.ids.iter().cloned().zip(self.pos.iter().copied()).collect();
        let n = data.nodes.len();
        self.ids = data.nodes.iter().map(|n| n.0.clone()).collect();
        self.labels = data.nodes.iter().map(|n| crate::model::basename(&n.1).to_string()).collect();
        self.paths = data.nodes.iter().map(|n| n.1.clone()).collect();
        self.ghost = data.nodes.iter().map(|n| n.2).collect();
        let index: HashMap<&str, usize> = self.ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        self.edges = data
            .edges
            .iter()
            .filter_map(|(a, b)| Some((*index.get(a.as_str())?, *index.get(b.as_str())?)))
            .collect();
        self.degree = vec![0; n];
        for (a, b) in &self.edges {
            self.degree[*a] += 1;
            self.degree[*b] += 1;
        }
        let mut fresh = false;
        self.pos = self
            .ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                old.get(id).copied().unwrap_or_else(|| {
                    fresh = true;
                    // golden-angle spiral initial placement (deterministic)
                    let a = i as f32 * 2.399_963;
                    let r = 12.0 * (i as f32 + 1.0).sqrt();
                    vec2(r * a.cos(), r * a.sin())
                })
            })
            .collect();
        self.vel = vec![Vec2::ZERO; n];
        self.alpha = if fresh || old.is_empty() { 1.0 } else { self.alpha.max(0.3) };
        if self.zoom == 0.0 {
            self.zoom = 1.0;
            self.auto_fit = true;
        }
        self.loaded = true;
    }

    fn step(&mut self) {
        let n = self.pos.len();
        if n == 0 || self.alpha < 0.005 {
            return;
        }
        let visible: Vec<bool> = (0..n).map(|i| self.show_ghosts || !self.ghost[i]).collect();
        let mut force = vec![Vec2::ZERO; n];
        // repulsion (O(n²), fine for a few thousand notes)
        for i in 0..n {
            if !visible[i] {
                continue;
            }
            for j in (i + 1)..n {
                if !visible[j] {
                    continue;
                }
                let d = self.pos[i] - self.pos[j];
                let dist2 = d.length_sq().max(25.0);
                let f = d * (260.0 / dist2);
                force[i] += f;
                force[j] -= f;
            }
        }
        // springs
        for &(a, b) in &self.edges {
            if !visible[a] || !visible[b] {
                continue;
            }
            let d = self.pos[b] - self.pos[a];
            let len = d.length().max(0.01);
            let f = d / len * (len - 60.0) * 0.05;
            force[a] += f;
            force[b] -= f;
        }
        for (i, f) in force.iter_mut().enumerate() {
            *f -= self.pos[i] * 0.02; // gravity to center
            self.vel[i] = (self.vel[i] + *f * self.alpha) * 0.6;
            let v = self.vel[i];
            self.pos[i] += if v.length() > 30.0 { v.normalized() * 30.0 } else { v };
        }
        self.alpha *= 0.985;
    }

    fn fit(&mut self, rect: Rect) {
        let pts: Vec<Vec2> = (0..self.pos.len()).filter(|&i| self.show_ghosts || !self.ghost[i]).map(|i| self.pos[i]).collect();
        if pts.is_empty() {
            return;
        }
        let (mut min, mut max) = (pts[0], pts[0]);
        for p in &pts {
            min = min.min(*p);
            max = max.max(*p);
        }
        let size = (max - min) + vec2(120.0, 120.0);
        self.zoom = (rect.width() / size.x).min(rect.height() / size.y).clamp(0.15, 2.0);
        self.pan = -(min + max) / 2.0;
    }

    /// Returns the id of the clicked node, if any.
    pub fn ui(&mut self, ui: &mut egui::Ui, current: Option<&str>) -> Option<String> {
        self.step();
        if self.alpha >= 0.005 {
            ui.ctx().request_repaint();
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_ghosts, "Notes inexistantes");
            if ui.button("Recentrer").clicked() {
                self.auto_fit = true;
            }
            if ui.button("Réorganiser").clicked() {
                self.alpha = 1.0;
            }
            let links = self.edges.iter().filter(|(a, b)| self.show_ghosts || (!self.ghost[*a] && !self.ghost[*b])).count();
            ui.weak(format!("{} notes · {links} liens", self.ghost.iter().filter(|g| !**g).count()));
        });
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);

        if resp.dragged() {
            self.pan += resp.drag_delta() / self.zoom;
            self.auto_fit = false;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.1, 6.0);
                self.auto_fit = false;
            }
        }
        if self.auto_fit {
            self.fit(rect);
        }
        let center = rect.center().to_vec2();
        let to_screen = |p: Vec2| -> Pos2 { ((p + self.pan) * self.zoom + center).to_pos2() };
        let visible = |i: usize| self.show_ghosts || !self.ghost[i];

        let hover_pos = resp.hover_pos();
        let mut hovered: Option<usize> = None;
        let mut best = f32::MAX;
        let radius = |i: usize| (3.5 + (self.degree[i] as f32).sqrt() * 1.6) * self.zoom.sqrt();
        if let Some(hp) = hover_pos {
            for i in 0..self.pos.len() {
                if !visible(i) {
                    continue;
                }
                let d = to_screen(self.pos[i]).distance(hp);
                if d < radius(i) + 5.0 && d < best {
                    best = d;
                    hovered = Some(i);
                }
            }
        }
        let accent = ui.visuals().selection.bg_fill;
        let text_color = ui.visuals().text_color();
        let edge_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let clip = Rect::from_min_max(rect.min, rect.max);
        let painter = painter.with_clip_rect(clip);

        for &(a, b) in &self.edges {
            if !visible(a) || !visible(b) {
                continue;
            }
            let lit = hovered.is_some_and(|h| h == a || h == b);
            painter.line_segment(
                [to_screen(self.pos[a]), to_screen(self.pos[b])],
                Stroke::new(if lit { 1.8 } else { 1.0 }, if lit { accent } else { edge_color }),
            );
        }
        for i in 0..self.pos.len() {
            if !visible(i) {
                continue;
            }
            let p = to_screen(self.pos[i]);
            if !rect.expand(50.0).contains(p) {
                continue;
            }
            let is_current = current == Some(self.ids[i].as_str());
            let color = if is_current {
                Color32::from_rgb(240, 180, 70)
            } else if self.ghost[i] {
                Color32::from_gray(110)
            } else if hovered == Some(i) {
                accent
            } else {
                Color32::from_rgb(130, 160, 190)
            };
            painter.circle_filled(p, radius(i), color);
            if self.zoom > 0.7 || hovered == Some(i) || is_current {
                painter.text(
                    p + vec2(0.0, radius(i) + 2.0),
                    egui::Align2::CENTER_TOP,
                    &self.labels[i],
                    egui::FontId::proportional(11.0 * self.zoom.sqrt().clamp(0.8, 1.4)),
                    if self.ghost[i] { Color32::from_gray(130) } else { text_color },
                );
            }
        }
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked()
            && let Some(h) = hovered
        {
            return Some(if self.ghost[h] { format!("ghost:{}", self.paths[h]) } else { self.ids[h].clone() });
        }
        None
    }
}
