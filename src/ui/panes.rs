//! Volets des panneaux latéraux (comme les onglets des barres latérales d'Obsidian) : chaque volet se glisse
//! d'un panneau à l'autre ou change de place dans sa rangée ; l'organisation est gardée dans la configuration.
//! Graphe local de la note ouverte en bas du panneau de gauche.

use super::sidebar::{HEADER_H, SIDE_PAD};
use super::*;

/// Hauteur de la barre de titre du graphe local.
const GRAPH_BAR_H: f32 = 30.0;

/// Volets des plugins absents des deux panneaux : ajoutés à droite, comme avant. Les volets d'un plugin
/// désactivé restent à leur place (ils reviennent au même endroit quand il est réactivé), simplement masqués.
pub(super) fn reconcile_panes(left: &mut Vec<Pane>, right: &mut Vec<Pane>, plugins: &[String]) -> bool {
    let mut changed = false;
    // un volet en double (configuration modifiée à la main) n'est gardé qu'une fois
    let mut seen: Vec<Pane> = vec![];
    for list in [&mut *left, &mut *right] {
        let before = list.len();
        list.retain(|p| {
            let dup = seen.contains(p);
            if !dup {
                seen.push(p.clone());
            }
            !dup
        });
        changed |= list.len() != before;
    }
    for core in [Pane::Files, Pane::Search, Pane::Tags, Pane::Bookmarks] {
        if !seen.contains(&core) {
            left.push(core);
            changed = true;
        }
    }
    for core in [Pane::Backlinks, Pane::Tickets] {
        if !seen.contains(&core) {
            right.push(core);
            changed = true;
        }
    }
    for name in plugins {
        let pane = Pane::Plugin(name.clone());
        if !left.contains(&pane) && !right.contains(&pane) {
            right.push(pane);
            changed = true;
        }
    }
    changed
}

/// Déplace `pane` dans `to` à la position `index` (après l'avoir retiré de sa liste d'origine).
pub(super) fn move_pane(left: &mut Vec<Pane>, right: &mut Vec<Pane>, pane: &Pane, to: Side, index: usize) {
    let (from_left, from_idx) = match (left.iter().position(|p| p == pane), right.iter().position(|p| p == pane)) {
        (Some(i), _) => (true, i),
        (None, Some(i)) => (false, i),
        (None, None) => return,
    };
    let mut index = index;
    if from_left == (to == Side::Left) && from_idx < index {
        index -= 1; // la place libérée décale la cible
    }
    if from_left {
        left.remove(from_idx);
    } else {
        right.remove(from_idx);
    }
    let dest = if to == Side::Left { left } else { right };
    dest.insert(index.min(dest.len()), pane.clone());
}

impl MnemoApp {
    /// Icône et nom d'un volet.
    fn pane_meta(&self, pane: &Pane) -> (&'static str, String) {
        match pane {
            Pane::Files => (icon::FILES_TAB, "Fichiers".into()),
            Pane::Search => (icon::SEARCH, "Rechercher".into()),
            Pane::Tags => (icon::TAG, "Tags".into()),
            Pane::Bookmarks => (icon::BOOKMARK, "Signets".into()),
            Pane::Backlinks => (icon::LINK, "Rétroliens".into()),
            Pane::Tickets => (icon::TICKET, "Tickets liés".into()),
            Pane::Plugin(name) => (if name == "Plan" { icon::LIST } else { icon::PUZZLE }, name.clone()),
        }
    }

    /// Volet affichable : un volet de plugin n'est montré que si le plugin est actif.
    fn pane_available(&self, pane: &Pane) -> bool {
        match pane {
            Pane::Plugin(name) => self.plugin_panels.iter().any(|(n, _)| n == name),
            _ => true,
        }
    }

    /// Montre un volet, dans le panneau où il se trouve (le panneau est ouvert s'il était masqué).
    pub(super) fn show_pane(&mut self, pane: Pane) {
        if self.cfg.right_panes.contains(&pane) {
            self.right_tab = pane.clone();
            if !self.cfg.show_right {
                self.cfg.show_right = true;
                self.cfg.save();
            }
        } else {
            self.left_tab = pane.clone();
            if !self.cfg.show_left {
                self.cfg.show_left = true;
                self.cfg.save();
            }
        }
        if pane == Pane::Search {
            self.focus_search = true;
        }
    }

    /// Onglets d'un panneau : clic pour afficher, glisser pour réordonner ou changer de panneau.
    pub(super) fn pane_tabs(&mut self, ui: &mut egui::Ui, side: Side, _now: f64) {
        let plugins: Vec<String> = self.plugin_panels.iter().map(|(n, _)| n.clone()).collect();
        if reconcile_panes(&mut self.cfg.left_panes, &mut self.cfg.right_panes, &plugins) {
            self.cfg.save();
        }
        let panes: Vec<Pane> = match side {
            Side::Left => self.cfg.left_panes.clone(),
            Side::Right => self.cfg.right_panes.clone(),
        };
        let shown: Vec<Pane> = panes.iter().filter(|p| self.pane_available(p)).cloned().collect();
        // volet actif disparu (déplacé de l'autre côté, plugin désactivé) : le premier de la rangée
        let active = match side {
            Side::Left => &mut self.left_tab,
            Side::Right => &mut self.right_tab,
        };
        if !shown.contains(active)
            && let Some(first) = shown.first()
        {
            *active = first.clone();
        }
        let active = active.clone();
        let ctx = ui.ctx().clone();
        let dragging = egui::DragAndDrop::has_payload_of_type::<PaneDrag>(&ctx);
        let accent = self.palette().accent;
        let header = ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), HEADER_H),
            // en bas de la rangée, alignées sur le titre des onglets
            egui::Layout::left_to_right(egui::Align::Max),
            |ui| {
                ui.add_space(SIDE_PAD);
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut rects: Vec<egui::Rect> = vec![];
                for pane in &shown {
                    let (ic, name) = self.pane_meta(pane);
                    let r = theme::icon_button_sized(ui, ic, &name, active == *pane, true, 16.0);
                    let r = r.interact(egui::Sense::drag());
                    r.dnd_set_drag_payload(PaneDrag(pane.clone()));
                    if r.clicked() {
                        self.show_pane(pane.clone());
                    }
                    rects.push(r.rect);
                }
                if side == Side::Right {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                        ui.add_space(SIDE_PAD);
                        if theme::icon_button_sized(
                            ui,
                            icon::PANEL_RIGHT,
                            "Masquer le panneau de droite",
                            false,
                            true,
                            16.0,
                        )
                        .clicked()
                        {
                            self.actions.push(UiAction::Core(CoreCmd::ToggleRight));
                        }
                    });
                }
                rects
            },
        );
        let rect = header.response.rect;
        let rects = header.inner;
        if !dragging {
            return;
        }
        let Some(pointer) = ctx.pointer_hover_pos().filter(|p| rect.contains(*p)) else { return };
        // place visée : avant le premier onglet dont le centre est à droite du pointeur
        let pos_shown = rects.iter().position(|r| r.center().x > pointer.x).unwrap_or(rects.len());
        let x = match rects.get(pos_shown) {
            Some(r) => r.left() - 2.0,
            None => rects.last().map_or(rect.left() + SIDE_PAD, |r| r.right() + 2.0),
        };
        ui.painter().vline(x, rect.y_range().shrink(8.0), egui::Stroke::new(2.0, accent));
        if ctx.input(|i| i.pointer.any_released())
            && let Some(drag) = egui::DragAndDrop::take_payload::<PaneDrag>(&ctx)
        {
            // position dans la liste complète (volets masqués compris)
            let index = match shown.get(pos_shown) {
                Some(p) => panes.iter().position(|q| q == p).unwrap_or(panes.len()),
                None => panes.len(),
            };
            move_pane(&mut self.cfg.left_panes, &mut self.cfg.right_panes, &drag.0, side, index);
            match side {
                Side::Left => self.left_tab = drag.0.clone(),
                Side::Right => self.right_tab = drag.0.clone(),
            }
            self.cfg.save();
        }
    }

    /// Graphe local en bas du panneau de gauche : la note ouverte, ses liens et rétroliens. Sa bordure du
    /// haut se glisse pour changer sa hauteur ; la barre de titre le replie.
    pub(super) fn local_graph_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let full = ui.available_rect_before_wrap();
        // bordure du haut : poignée de redimensionnement
        let handle = egui::Rect::from_min_size(full.left_top() - egui::vec2(0.0, 3.0), egui::vec2(full.width(), 6.0));
        let resp = ui.interact(handle, egui::Id::unique("local-graph-resize"), egui::Sense::drag());
        if self.cfg.local_graph && (resp.hovered() || resp.dragged()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if self.cfg.local_graph && resp.dragged() {
            self.cfg.local_graph_height = (self.cfg.local_graph_height - resp.drag_delta().y).clamp(120.0, 900.0);
        }
        if resp.drag_stopped() {
            self.cfg.save();
        }
        ui.painter().hline(full.x_range(), full.top(), egui::Stroke::new(1.0, p.border));
        let bar = ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), GRAPH_BAR_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(SIDE_PAD + 2.0);
                let arrow = if self.cfg.local_graph { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT };
                ui.label(theme::icon_text(arrow, 12.0).color(p.faint));
                ui.label(RichText::new("Graphe local").color(p.muted).size(12.5));
            },
        );
        let bar = ui.interact(bar.response.rect, egui::Id::unique("local-graph-bar"), egui::Sense::click());
        if bar.clicked() {
            self.cfg.local_graph = !self.cfg.local_graph;
            self.cfg.save();
        }
        if !self.cfg.local_graph {
            return;
        }
        let Some(open) = self.open.as_ref().map(|n| n.id) else {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(SIDE_PAD + 4.0);
                ui.label(RichText::new("Aucun fichier actif.").color(p.faint));
            });
            return;
        };
        // données du graphe : demandées une fois, puis tenues à jour comme celles de la vue graphe
        let Some(data) = &self.graph_data else {
            if !std::mem::replace(&mut self.graph_requested, true) {
                self.db.send(Cmd::Graph);
            }
            ui.centered_and_justified(|ui| ui.spinner());
            return;
        };
        if self.local_graph_for != Some((open, self.graph_gen)) {
            self.local_graph.set_data(local_subgraph(data, &open.to_string(), 1));
            self.local_graph.compact = true;
            self.local_graph_for = Some((open, self.graph_gen));
        }
        let mut clicked = None;
        egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 6, top: 0, bottom: 6 }).show(ui, |ui| {
            clicked = self.local_graph.ui(ui, Some(&open.to_string()));
        });
        if let Some(c) = clicked {
            self.actions.push(UiAction::OpenLink(c));
        }
    }
}

/// Sous-graphe autour de `center` : les nœuds à `depth` liens au plus (dans les deux sens), et leurs liens.
pub(super) fn local_subgraph(data: &GraphData, center: &str, depth: usize) -> GraphData {
    let mut keep: std::collections::HashSet<&str> = std::iter::once(center).collect();
    for _ in 0..depth {
        let next: Vec<&str> = data
            .edges
            .iter()
            .filter_map(|(a, b)| match (keep.contains(a.as_str()), keep.contains(b.as_str())) {
                (true, false) => Some(b.as_str()),
                (false, true) => Some(a.as_str()),
                _ => None,
            })
            .collect();
        keep.extend(next);
    }
    GraphData {
        nodes: data.nodes.iter().filter(|n| keep.contains(n.0.as_str())).cloned().collect(),
        edges: data
            .edges
            .iter()
            .filter(|(a, b)| keep.contains(a.as_str()) && keep.contains(b.as_str()))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(l: &[Pane], r: &[Pane]) -> (Vec<Pane>, Vec<Pane>) {
        (l.to_vec(), r.to_vec())
    }

    #[test]
    fn moving_panes() {
        let (mut l, mut r) =
            panes(&[Pane::Files, Pane::Search, Pane::Tags, Pane::Bookmarks], &[Pane::Backlinks, Pane::Tickets]);
        // tags vers la droite, en premier
        move_pane(&mut l, &mut r, &Pane::Tags, Side::Right, 0);
        assert_eq!(l, [Pane::Files, Pane::Search, Pane::Bookmarks]);
        assert_eq!(r, [Pane::Tags, Pane::Backlinks, Pane::Tickets]);
        // réordonner dans la même rangée : avant « Tickets » (index 2 avant retrait)
        move_pane(&mut l, &mut r, &Pane::Tags, Side::Right, 2);
        assert_eq!(r, [Pane::Backlinks, Pane::Tags, Pane::Tickets]);
        // retour à gauche, à la fin
        move_pane(&mut l, &mut r, &Pane::Tags, Side::Left, 99);
        assert_eq!(l, [Pane::Files, Pane::Search, Pane::Bookmarks, Pane::Tags]);
    }

    #[test]
    fn reconcile() {
        let (mut l, mut r) = panes(&[Pane::Tags, Pane::Files, Pane::Tags], &[Pane::Search]);
        assert!(reconcile_panes(&mut l, &mut r, &["Plan".into()]));
        assert_eq!(l, [Pane::Tags, Pane::Files, Pane::Bookmarks]);
        assert_eq!(r, [Pane::Search, Pane::Backlinks, Pane::Tickets, Pane::Plugin("Plan".into())]);
        assert!(!reconcile_panes(&mut l, &mut r, &["Plan".into()]));
    }

    #[test]
    fn local_graph_keeps_neighbours() {
        let g = GraphData {
            nodes: ["a", "b", "c", "d"].iter().map(|n| (n.to_string(), n.to_string(), false)).collect(),
            edges: vec![("a".into(), "b".into()), ("c".into(), "a".into()), ("c".into(), "d".into())],
        };
        let sub = local_subgraph(&g, "a", 1);
        let ids: Vec<&str> = sub.nodes.iter().map(|n| n.0.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(sub.edges.len(), 2);
        assert_eq!(local_subgraph(&g, "a", 2).nodes.len(), 4);
    }
}
