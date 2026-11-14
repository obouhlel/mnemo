//! Whiteboard (dessin libre) : modèle du fichier `.mnemo-board` et géométrie, sans interface.
//!
//! Le fichier est un JSON lisible, partagé avec l'application Android :
//! ```json
//! { "version": 1, "elements": [
//!   { "type": "stroke",  "color": "#e03131", "width": 3, "points": [[0, 0], [10, 4]] },
//!   { "type": "rect",    "color": "#1971c2", "width": 2, "x": 0, "y": 0, "w": 100, "h": 60 },
//!   { "type": "ellipse", "color": "#2f9e44", "width": 2, "x": 0, "y": 0, "w": 80, "h": 80 },
//!   { "type": "line",    "color": "#000000", "width": 2, "points": [[0, 0], [50, 50]] },
//!   { "type": "arrow",   "color": "#000000", "width": 2, "points": [[0, 0], [50, 50]] },
//!   { "type": "text",    "color": "#000000", "size": 20, "x": 10, "y": 10, "text": "Bonjour" }
//! ] }
//! ```
//! Les coordonnées sont celles du dessin (pas de l'écran) ; l'origine est libre, la surface infinie.
//!
//! Un élément de type inconnu (format plus récent) est gardé tel quel et réécrit sans y toucher, comme sur
//! Android. Le format n'a pas d'identifiant d'élément : deux éléments sont « le même » s'ils sont identiques.

use serde::{Deserialize, Serialize};

pub const EXTENSION: &str = "mnemo-board";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Element {
    Stroke {
        color: String,
        width: f32,
        points: Vec<[f32; 2]>,
    },
    Rect {
        color: String,
        width: f32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Ellipse {
        color: String,
        width: f32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Line {
        color: String,
        width: f32,
        points: Vec<[f32; 2]>,
    },
    Arrow {
        color: String,
        width: f32,
        points: Vec<[f32; 2]>,
    },
    Text {
        color: String,
        size: f32,
        x: f32,
        y: f32,
        text: String,
    },
    /// élément que cette version ne connaît pas (ou incomplet) : ni affiché ni modifié, réécrit tel quel
    #[serde(untagged)]
    Unknown(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub elements: Vec<Element>,
}

fn one() -> u32 {
    1
}

impl Default for Sketch {
    fn default() -> Self {
        Self { version: 1, elements: vec![] }
    }
}

impl Sketch {
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(s).map_err(|e| format!("Whiteboard illisible : {e}"))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{\"version\":1,\"elements\":[]}".into())
    }

    /// Rectangle englobant de tout le dessin (min x, min y, max x, max y), `None` s'il est vide.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        self.elements
            .iter()
            .filter(|e| !matches!(e, Element::Unknown(_)))
            .map(Element::bounds)
            .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
    }

    /// Indice de l'élément le plus haut (dernier dessiné) sous le point, avec une tolérance.
    pub fn hit(&self, p: [f32; 2], tolerance: f32) -> Option<usize> {
        (0..self.elements.len()).rev().find(|&i| self.elements[i].hit(p, tolerance))
    }
}

/// Couleur `#rrggbb` → (r, g, b) ; gris si illisible.
pub fn parse_color(s: &str) -> [u8; 3] {
    let h = s.trim_start_matches('#');
    match u32::from_str_radix(h, 16) {
        Ok(v) if h.len() == 6 => [(v >> 16) as u8, (v >> 8) as u8, v as u8],
        _ => [128, 128, 128],
    }
}

fn dist_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

/// Largeur approximative d'un texte (le rendu exact dépend de la police) : pour la sélection.
fn text_size(text: &str, size: f32) -> (f32, f32) {
    let lines: Vec<&str> = text.lines().collect();
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f32 * size * 0.55;
    (w.max(size), lines.len().max(1) as f32 * size * 1.25)
}

impl Element {
    pub fn color(&self) -> &str {
        match self {
            Element::Stroke { color, .. }
            | Element::Rect { color, .. }
            | Element::Ellipse { color, .. }
            | Element::Line { color, .. }
            | Element::Arrow { color, .. }
            | Element::Text { color, .. } => color,
            Element::Unknown(_) => "#888888",
        }
    }

    pub fn set_color(&mut self, c: &str) {
        match self {
            Element::Stroke { color, .. }
            | Element::Rect { color, .. }
            | Element::Ellipse { color, .. }
            | Element::Line { color, .. }
            | Element::Arrow { color, .. }
            | Element::Text { color, .. } => *color = c.to_string(),
            Element::Unknown(_) => {}
        }
    }

    /// (min x, min y, max x, max y)
    pub fn bounds(&self) -> [f32; 4] {
        match self {
            Element::Stroke { points, width, .. }
            | Element::Line { points, width, .. }
            | Element::Arrow { points, width, .. } => {
                let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for p in points {
                    b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
                }
                if points.is_empty() {
                    b = [0.0; 4];
                }
                let m = width / 2.0;
                [b[0] - m, b[1] - m, b[2] + m, b[3] + m]
            }
            Element::Rect { x, y, w, h, .. } | Element::Ellipse { x, y, w, h, .. } => {
                [x.min(x + w), y.min(y + h), x.max(x + w), y.max(y + h)]
            }
            Element::Text { x, y, text, size, .. } => {
                let (w, h) = text_size(text, *size);
                [*x, *y, x + w, y + h]
            }
            Element::Unknown(_) => [0.0; 4],
        }
    }

    /// Le point est-il sur l'élément (trait, contour ou intérieur d'une forme, texte) ?
    pub fn hit(&self, p: [f32; 2], tol: f32) -> bool {
        match self {
            Element::Stroke { points, width, .. }
            | Element::Line { points, width, .. }
            | Element::Arrow { points, width, .. } => {
                let reach = width / 2.0 + tol;
                match points.len() {
                    0 => false,
                    1 => dist_to_segment(p, points[0], points[0]) <= reach,
                    _ => points.windows(2).any(|s| dist_to_segment(p, s[0], s[1]) <= reach),
                }
            }
            Element::Rect { .. } | Element::Text { .. } => {
                let b = self.bounds();
                p[0] >= b[0] - tol && p[0] <= b[2] + tol && p[1] >= b[1] - tol && p[1] <= b[3] + tol
            }
            Element::Ellipse { x, y, w, h, .. } => {
                let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, (w / 2.0).abs() + tol, (h / 2.0).abs() + tol);
                ((p[0] - cx) / rx).powi(2) + ((p[1] - cy) / ry).powi(2) <= 1.0
            }
            Element::Unknown(_) => false,
        }
    }

    /// Déplace l'élément.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Element::Stroke { points, .. } | Element::Line { points, .. } | Element::Arrow { points, .. } => {
                for p in points {
                    p[0] += dx;
                    p[1] += dy;
                }
            }
            Element::Rect { x, y, .. } | Element::Ellipse { x, y, .. } | Element::Text { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Element::Unknown(_) => {}
        }
    }
}

/// Fusion à 3 voies de deux versions d'un dessin parties de la même `base` (dernier état lu ou écrit sur le
/// disque) : on part de `remote` (le fichier tel qu'un autre appareil l'a laissé), on en retire ce qui a été
/// effacé ici et on ajoute par-dessus ce qui a été tracé ici. Un élément modifié des deux côtés (déplacé ici,
/// recoloré là-bas) est gardé dans ses deux versions : rien n'est perdu.
pub fn merge(base: &[Element], local: &[Element], remote: &[Element]) -> Vec<Element> {
    let key = |e: &Element| serde_json::to_string(e).unwrap_or_default();
    // éléments de la base pas encore retrouvés ici (les doublons comptent)
    let mut left: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in base {
        *left.entry(key(e)).or_default() += 1;
    }
    let mut added = vec![];
    for e in local {
        match left.get_mut(&key(e)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added.push(e.clone()),
        }
    }
    // ce qui reste dans `left` a été effacé (ou modifié) ici
    let mut out: Vec<Element> = remote
        .iter()
        .filter(|e| match left.get_mut(&key(e)) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
        .cloned()
        .collect();
    out.extend(added);
    out
}

/// Simplifie un trait à main levée (Ramer-Douglas-Peucker) : fichiers plus légers, rendu identique.
pub fn simplify(points: &[[f32; 2]], epsilon: f32) -> Vec<[f32; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (mut idx, mut dmax) = (0, 0.0);
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = dist_to_segment(*p, a, b);
        if d > dmax {
            idx = i;
            dmax = d;
        }
    }
    if dmax > epsilon {
        let mut left = simplify(&points[..=idx], epsilon);
        let right = simplify(&points[idx..], epsilon);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![a, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let s = Sketch {
            version: 1,
            elements: vec![
                Element::Stroke { color: "#e03131".into(), width: 3.0, points: vec![[0.0, 0.0], [10.0, 4.0]] },
                Element::Text { color: "#000000".into(), size: 20.0, x: 1.0, y: 2.0, text: "Été".into() },
            ],
        };
        let back = Sketch::parse(&s.to_json()).unwrap();
        assert_eq!(back, s);
        assert!(s.to_json().contains("\"type\": \"stroke\""));
        assert_eq!(Sketch::parse("").unwrap(), Sketch::default());
        assert!(Sketch::parse("{pas du json").is_err());
    }

    #[test]
    fn unknown_elements_are_kept() {
        let src = r##"{ "version": 1, "elements": [
            { "type": "stroke", "color": "#e03131", "width": 3, "points": [[0, 0], [10, 4]] },
            { "type": "image", "src": "photo.png", "x": 5, "y": 6, "extra": { "a": [1, 2] } },
            { "type": "rect", "color": "#000000" },
            42
        ] }"##;
        let s = Sketch::parse(src).unwrap();
        assert_eq!(s.elements.len(), 4);
        assert!(matches!(s.elements[0], Element::Stroke { .. }));
        // type inconnu, élément connu mais incomplet, valeur qui n'est pas un objet : gardés tels quels
        assert!(s.elements[1..].iter().all(|e| matches!(e, Element::Unknown(_))));
        let back: serde_json::Value = serde_json::from_str(&s.to_json()).unwrap();
        let orig: serde_json::Value = serde_json::from_str(src).unwrap();
        assert_eq!(back["elements"][1], orig["elements"][1]);
        assert_eq!(back["elements"][2], orig["elements"][2]);
        assert_eq!(back["elements"][3], orig["elements"][3]);
        assert_eq!(back["elements"][0]["type"], "stroke");
        // ils ne comptent ni dans le cadre ni dans la sélection
        assert_eq!(s.bounds().unwrap(), [-1.5, -1.5, 11.5, 5.5]);
        assert_eq!(s.hit([5.0, 6.0], 2.0), None);
        assert_eq!(Sketch::parse(&s.to_json()).unwrap(), s);
    }

    #[test]
    fn three_way_merge() {
        let st = |x: f32| Element::Stroke { color: "#000000".into(), width: 2.0, points: vec![[x, 0.0], [x, 9.0]] };
        let (a, b, c, d, e) = (st(1.0), st(2.0), st(3.0), st(4.0), st(5.0));
        let base = vec![a.clone(), b.clone(), c.clone()];
        // rien de changé d'un côté : l'autre gagne
        assert_eq!(merge(&base, &base, std::slice::from_ref(&a)), vec![a.clone()]);
        assert_eq!(merge(&base, &[a.clone(), d.clone()], &base), vec![a.clone(), d.clone()]);
        // ici : b effacé, d tracé ; là-bas : c effacé, e tracé
        let local = vec![a.clone(), c.clone(), d.clone()];
        let remote = vec![a.clone(), b.clone(), e.clone()];
        assert_eq!(merge(&base, &local, &remote), vec![a.clone(), e.clone(), d.clone()]);
        // même élément déplacé ici et effacé là-bas : la version déplacée reste
        let mut moved = b.clone();
        moved.translate(10.0, 0.0);
        assert_eq!(merge(&base, &[a.clone(), moved.clone()], std::slice::from_ref(&a)), vec![a.clone(), moved]);
        // doublons : un seul des deux traits identiques effacé ici
        let twice = vec![a.clone(), a.clone()];
        assert_eq!(
            merge(&twice, std::slice::from_ref(&a), &[a.clone(), a.clone(), e.clone()]),
            vec![a.clone(), e.clone()]
        );
        // les éléments inconnus suivent les mêmes règles
        let u = Element::Unknown(serde_json::json!({ "type": "image", "src": "p.png" }));
        assert_eq!(merge(&[], std::slice::from_ref(&d), std::slice::from_ref(&u)), vec![u, d]);
    }

    #[test]
    fn geometry() {
        let line = Element::Line { color: "#000000".into(), width: 2.0, points: vec![[0.0, 0.0], [100.0, 0.0]] };
        assert!(line.hit([50.0, 2.0], 1.0));
        assert!(!line.hit([50.0, 10.0], 1.0));
        let ell = Element::Ellipse { color: "#000000".into(), width: 2.0, x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        assert!(ell.hit([50.0, 25.0], 0.0) && !ell.hit([2.0, 2.0], 0.0));
        let mut r = Element::Rect { color: "#000000".into(), width: 2.0, x: 10.0, y: 10.0, w: -10.0, h: 20.0 };
        assert_eq!(r.bounds(), [0.0, 10.0, 10.0, 30.0]); // largeur négative : tracé vers la gauche
        r.translate(5.0, 5.0);
        assert_eq!(r.bounds(), [5.0, 15.0, 15.0, 35.0]);
        let s = Sketch { version: 1, elements: vec![line, ell] };
        assert_eq!(s.hit([50.0, 25.0], 1.0), Some(1));
        assert_eq!(s.bounds().unwrap(), [-1.0, -1.0, 101.0, 50.0]);
        assert_eq!(parse_color("#1971c2"), [0x19, 0x71, 0xc2]);
    }

    #[test]
    fn simplification() {
        let pts: Vec<[f32; 2]> = (0..=100).map(|i| [i as f32, 0.0]).collect();
        assert_eq!(simplify(&pts, 0.5), vec![[0.0, 0.0], [100.0, 0.0]]);
        let corner = vec![[0.0, 0.0], [5.0, 0.1], [10.0, 0.0], [10.0, 10.0]];
        assert_eq!(simplify(&corner, 0.5).len(), 3);
    }
}
