//! Reads Obsidian-style CSS (themes and snippets) and turns their CSS custom properties
//! into Mnemo's palette and metrics.
//!
//! egui is not a browser: we don't apply arbitrary selectors. Obsidian themes, however, are
//! almost entirely driven by CSS variables declared on `body`, `:root`, `.theme-dark` and
//! `.theme-light`; those are parsed (with `var()`, `calc()`, `hsl()`, `rgb()`, hex…) and mapped.

use crate::theme::{Metrics, Palette};
use egui::Color32;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Both,
    Dark,
    Light,
}

#[derive(Debug, Default, Clone)]
pub struct Parsed {
    /// Variable declarations in source order
    pub vars: Vec<(Scope, String, String)>,
    /// Rules whose selector targets specific elements (not supported)
    pub ignored_rules: usize,
}

// ------------------------------------------------------------------------------- parsing

/// Avance dans du CSS en suivant les chaînes : `"…"` / `'…'` avec échappements `\x` (une chaîne non fermée
/// s'arrête à la fin de la ligne, comme dans un navigateur). `quote` est la chaîne en cours ; renvoie vrai si
/// l'octet `i` est du CSS « actif » (hors chaîne et non échappé), et met `i` à jour pour sauter un échappement.
fn active(bytes: &[u8], i: &mut usize, quote: &mut Option<u8>) -> bool {
    let c = bytes[*i];
    if c == b'\\' {
        // l'octet suivant est pris tel quel (un saut de ligne échappé continue la chaîne)
        *i += 1;
        return false;
    }
    match *quote {
        Some(q) => {
            if c == q || c == b'\n' {
                *quote = None;
            }
            false
        }
        None if c == b'"' || c == b'\'' => {
            *quote = Some(c);
            false
        }
        None => true,
    }
}

/// Retire les commentaires `/* … */`, sauf dans les chaînes et les `url(…)` sans guillemets.
fn strip_comments(css: &str) -> String {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let (mut i, mut copied, mut quote) = (0, 0, None);
    while i < b.len() {
        if active(b, &mut i, &mut quote) {
            if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                out.push_str(&css[copied..i]);
                out.push(' ');
                match css[i + 2..].find("*/") {
                    Some(end) => i += 2 + end + 2,
                    None => return out,
                }
                copied = i;
                continue;
            }
            // url(http://x/*y) : pas un commentaire
            if b[i] == b'(' && i >= 3 && b[i - 3..i].eq_ignore_ascii_case(b"url") {
                let arg = css[i + 1..].trim_start();
                if !arg.starts_with(['"', '\'']) {
                    i += css[i..].find(')').unwrap_or(css.len() - i - 1);
                }
            }
        }
        i += 1;
    }
    out.push_str(&css[copied.min(css.len())..]);
    out
}

/// Position du premier octet `what` actif (hors chaîne) à partir de `from`.
fn find_active(s: &str, from: usize, what: u8) -> Option<usize> {
    let b = s.as_bytes();
    let (mut i, mut quote) = (from, None);
    while i < b.len() {
        if active(b, &mut i, &mut quote) && b[i] == what {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Index of the `}` matching the `{` at `open` (byte index), honouring nesting, strings and escapes.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (mut i, mut quote, mut depth) = (open, None, 0i32);
    while i < b.len() {
        if active(b, &mut i, &mut quote) {
            match b[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn classify(selectors: &str) -> Option<Scope> {
    let mut scope: Option<Scope> = None;
    for sel in selectors.split(',') {
        let sel = sel.trim().to_lowercase();
        let s = if sel.contains("theme-dark") {
            Scope::Dark
        } else if sel.contains("theme-light") {
            Scope::Light
        } else {
            Scope::Both
        };
        // what remains once the "global" parts are removed must be empty
        let rest = sel
            .replace(".theme-dark", "")
            .replace(".theme-light", "")
            .replace(":root", "")
            .replace("html", "")
            .replace("body", "")
            .replace(".app-container", "");
        if !rest.trim().is_empty() {
            continue;
        }
        scope = Some(match (scope, s) {
            (None, s) => s,
            (Some(a), b) if a == b => a,
            _ => Scope::Both,
        });
    }
    scope
}

fn declarations(block: &str, scope: Scope, out: &mut Parsed) {
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = block.as_bytes();
    let mut push = |decl: &str| {
        if let Some((name, value)) = decl.split_once(':') {
            let name = name.trim();
            if name.starts_with("--") {
                let value = value.trim().trim_end_matches("!important").trim();
                out.vars.push((scope, name.to_string(), value.to_string()));
            }
        }
    };
    let (mut i, mut quote) = (0, None);
    while i < bytes.len() {
        if active(bytes, &mut i, &mut quote) {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth <= 0 => {
                    push(&block[start..i]);
                    start = i + 1;
                    depth = 0;
                }
                _ => {}
            }
        }
        i += 1;
    }
    push(&block[start.min(block.len())..]);
}

/// Fenêtre de référence pour les requêtes `min-width` / `max-width` (Mnemo est une application de bureau).
const VIEWPORT: (f32, f32) = (1280.0, 800.0);

/// Une requête `@media` s'applique-t-elle à l'écran ? `None` : non ; `Some(scope)` : oui, pour ce mode.
/// `print`, `speech`… sont ignorés ; `prefers-color-scheme` limite le bloc au mode sombre ou clair ;
/// `min/max-width/height` sont comparés à une fenêtre de bureau ; les autres conditions sont tenues pour vraies.
fn media_query(query: &str) -> Option<Scope> {
    let q = query.trim().to_lowercase();
    let (negated, q) = match q.strip_prefix("not ") {
        Some(rest) => (true, rest.trim().to_string()),
        None => (false, q.strip_prefix("only ").map(|r| r.trim().to_string()).unwrap_or(q)),
    };
    let mut scope = Some(Scope::Both);
    for (n, part) in q.split(" and ").map(str::trim).enumerate() {
        let part_scope = match part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
            // type de média (uniquement en tête)
            None if n == 0 => matches!(part, "" | "screen" | "all").then_some(Scope::Both),
            None => None,
            Some(feature) => {
                let (name, value) = feature.split_once(':').map(|(a, b)| (a.trim(), b.trim())).unwrap_or((feature, ""));
                let px = || parse_px(value);
                match name {
                    "prefers-color-scheme" => match value {
                        "dark" => Some(Scope::Dark),
                        "light" => Some(Scope::Light),
                        _ => None,
                    },
                    "min-width" => px().is_some_and(|v| v <= VIEWPORT.0).then_some(Scope::Both),
                    "max-width" => px().is_some_and(|v| v >= VIEWPORT.0).then_some(Scope::Both),
                    "min-height" => px().is_some_and(|v| v <= VIEWPORT.1).then_some(Scope::Both),
                    "max-height" => px().is_some_and(|v| v >= VIEWPORT.1).then_some(Scope::Both),
                    _ => Some(Scope::Both),
                }
            }
        };
        scope = match (scope, part_scope) {
            (Some(a), Some(b)) => narrow(a, b),
            _ => None,
        };
    }
    match (negated, scope) {
        (false, s) => s,
        (true, None) => Some(Scope::Both),
        (true, Some(Scope::Dark)) => Some(Scope::Light),
        (true, Some(Scope::Light)) => Some(Scope::Dark),
        (true, Some(Scope::Both)) => None,
    }
}

/// Intersection de deux portées (`None` : contradictoires, sombre et clair).
fn narrow(a: Scope, b: Scope) -> Option<Scope> {
    match (a, b) {
        (Scope::Both, s) | (s, Scope::Both) => Some(s),
        (a, b) if a == b => Some(a),
        _ => None,
    }
}

/// Liste de requêtes séparées par des virgules : réunion des portées de celles qui s'appliquent.
fn media_scope(queries: &str) -> Option<Scope> {
    queries.split(',').filter_map(media_query).reduce(|a, b| if a == b { a } else { Scope::Both })
}

/// Profondeur maximale des blocs imbriqués (`@media { @supports { … } }`) : au-delà, le bloc est ignoré.
const MAX_NESTING: usize = 32;

/// `limit` : portée imposée par les `@media` englobants.
fn parse_rules(css: &str, limit: Scope, depth: usize, out: &mut Parsed) {
    let mut i = 0;
    while i < css.len() {
        let Some(open) = find_active(css, i, b'{') else { break };
        let prelude_start = css[i..open].rfind(';').map(|p| i + p + 1).unwrap_or(i);
        let prelude = css[prelude_start..open].trim();
        let Some(close) = matching_brace(css, open) else { break };
        let body = &css[open + 1..close];
        if let Some(at) = prelude.strip_prefix('@') {
            let kw = at.split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_lowercase();
            let inner = match kw.as_str() {
                "media" => media_scope(&at[kw.len()..]).and_then(|s| narrow(limit, s)),
                "supports" | "layer" | "container" | "document" => Some(limit),
                _ => None,
            };
            match inner {
                Some(scope) if depth < MAX_NESTING => parse_rules(body, scope, depth + 1, out),
                _ => {}
            }
        } else {
            // top-level declarations of the rule (nested rules removed)
            let mut flat = String::new();
            let mut j = 0;
            while j < body.len() {
                match find_active(body, j, b'{') {
                    Some(o) => {
                        let decl_end = body[j..o].rfind(';').map(|p| j + p + 1).unwrap_or(j);
                        flat.push_str(&body[j..decl_end]);
                        flat.push(';');
                        let c = matching_brace(body, o).unwrap_or(body.len() - 1);
                        j = c + 1;
                    }
                    None => {
                        flat.push_str(&body[j..]);
                        break;
                    }
                }
            }
            match classify(prelude) {
                // `.theme-light` dans un bloc réservé au mode sombre : jamais appliqué
                Some(scope) => {
                    if let Some(scope) = narrow(limit, scope) {
                        declarations(&flat, scope, out)
                    }
                }
                None => out.ignored_rules += 1,
            }
        }
        i = close + 1;
    }
}

pub fn parse(css: &str, out: &mut Parsed) {
    parse_rules(&strip_comments(css), Scope::Both, 0, out);
}

// ------------------------------------------------------------------------------- evaluation

/// Taille maximale d'une valeur résolue, et nombre maximal de `var()` développés pour une valeur.
const MAX_RESOLVED: usize = 16 * 1024;
const MAX_EXPANSIONS: usize = 2000;

struct Resolver<'a> {
    vars: &'a HashMap<String, String>,
    /// variables en cours de résolution (détection des cycles)
    stack: Vec<&'a str>,
    expansions: usize,
    /// cycle détecté : position dans `stack` de la variable qui le ferme (toutes celles du cycle sont invalides)
    cycle: Option<usize>,
    /// budget dépassé : toute la résolution échoue, sans essayer les valeurs de repli
    exhausted: bool,
}

impl<'a> Resolver<'a> {
    fn value(&mut self, value: &'a str) -> Option<String> {
        let mut out = String::new();
        let mut rest = value;
        while let Some(pos) = rest.find("var(") {
            out.push_str(&rest[..pos]);
            let inner_start = pos + 4;
            let mut d = 1;
            let mut end = None;
            for (k, c) in rest[inner_start..].char_indices() {
                match c {
                    '(' => d += 1,
                    ')' => {
                        d -= 1;
                        if d == 0 {
                            end = Some(inner_start + k);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = end?;
            let inner = &rest[inner_start..end];
            let (name, fallback) = match inner.split_once(',') {
                Some((n, f)) => (n.trim(), Some(f.trim())),
                None => (inner.trim(), None),
            };
            let mut v = self.var(name);
            if self.cycle.is_some() {
                return None; // la variable en cours fait partie d'un cycle : pas de repli
            }
            if v.is_none() && !self.exhausted {
                v = fallback.and_then(|f| self.value(f));
            }
            out.push_str(&v?);
            if out.len() > MAX_RESOLVED {
                self.exhausted = true;
            }
            if self.exhausted {
                return None;
            }
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// Valeur d'une variable ; `None` si elle est inconnue ou se référence elle-même (directement ou non) :
    /// comme en CSS, elle est alors invalide et le `var()` qui la lit prend sa valeur de repli.
    fn var(&mut self, name: &str) -> Option<String> {
        let (name, value) = self.vars.get_key_value(name)?;
        self.expansions += 1;
        if self.expansions > MAX_EXPANSIONS {
            self.exhausted = true;
        }
        if self.exhausted {
            return None;
        }
        if let Some(at) = self.stack.iter().position(|n| *n == name) {
            self.cycle = Some(self.cycle.map_or(at, |c| c.min(at)));
            return None;
        }
        let at = self.stack.len();
        self.stack.push(name);
        let v = self.value(value);
        self.stack.pop();
        if let Some(c) = self.cycle {
            if c >= at {
                self.cycle = None;
            }
            return None;
        }
        v
    }
}

/// Resolves every `var(--x[, fallback])` in `value`. Les cycles et les valeurs démesurées (plus de
/// `MAX_RESOLVED` octets ou de `MAX_EXPANSIONS` variables développées) donnent `None`.
pub fn resolve(value: &str, vars: &HashMap<String, String>) -> Option<String> {
    Resolver { vars, stack: vec![], expansions: 0, cycle: None, exhausted: false }.value(value)
}

/// Valeur résolue de la variable `name`.
fn resolve_var(name: &str, vars: &HashMap<String, String>) -> Option<String> {
    Resolver { vars, stack: vec![], expansions: 0, cycle: None, exhausted: false }.var(name)
}

/// Evaluates a number possibly wrapped in calc(); units are ignored (68% -> 68, 16px -> 16).
pub fn eval_number(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_prefix("calc").map(str::trim).unwrap_or(s);
    let tokens = tokenize(s)?;
    let mut pos = 0;
    let v = expr(&tokens, &mut pos, 0)?;
    (pos == tokens.len()).then_some(v)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32),
    Op(char),
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let mut out = vec![];
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || c == '.'
            || (c == '-'
                && matches!(out.last(), None | Some(Tok::Op(_)))
                && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == '.'))
        {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let n: f32 = chars[start..i].iter().collect::<String>().parse().ok()?;
            while i < chars.len() && (chars[i].is_alphabetic() || chars[i] == '%') {
                i += 1; // unit
            }
            out.push(Tok::Num(n));
        } else if "+-*/()".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

/// Profondeur maximale des parenthèses et des `-` unaires d'un calc() (l'analyse est récursive).
const MAX_CALC_DEPTH: usize = 64;

fn expr(t: &[Tok], p: &mut usize, depth: usize) -> Option<f32> {
    let mut v = term(t, p, depth)?;
    while let Some(Tok::Op(c @ ('+' | '-'))) = t.get(*p) {
        *p += 1;
        let r = term(t, p, depth)?;
        v = if *c == '+' { v + r } else { v - r };
    }
    Some(v)
}

fn term(t: &[Tok], p: &mut usize, depth: usize) -> Option<f32> {
    let mut v = factor(t, p, depth)?;
    while let Some(Tok::Op(c @ ('*' | '/'))) = t.get(*p) {
        *p += 1;
        let r = factor(t, p, depth)?;
        v = if *c == '*' { v * r } else { v / r };
    }
    Some(v)
}

fn factor(t: &[Tok], p: &mut usize, depth: usize) -> Option<f32> {
    if depth > MAX_CALC_DEPTH {
        return None;
    }
    match t.get(*p)? {
        Tok::Num(n) => {
            *p += 1;
            Some(*n)
        }
        Tok::Op('(') => {
            *p += 1;
            let v = expr(t, p, depth + 1)?;
            (t.get(*p) == Some(&Tok::Op(')'))).then(|| *p += 1)?;
            Some(v)
        }
        Tok::Op('-') => {
            *p += 1;
            factor(t, p, depth + 1).map(|v| -v)
        }
        _ => None,
    }
}

/// Splits function arguments on commas / spaces / slashes at depth 0.
fn args(inner: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut depth = 0;
    for c in inner.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c)
            }
            ')' => {
                depth -= 1;
                cur.push(c)
            }
            ',' | '/' | ' ' if depth == 0 => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

fn alpha_of(s: Option<&String>) -> Option<f32> {
    match s {
        None => Some(1.0),
        Some(a) if a.trim_end().ends_with('%') => eval_number(a).map(|v| v / 100.0),
        Some(a) => eval_number(a),
    }
}

pub fn parse_color(s: &str) -> Option<Color32> {
    let s = s.trim().to_lowercase();
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgba = |r: f32, g: f32, b: f32, a: f32| Color32::from_rgba_unmultiplied(to8(r), to8(g), to8(b), to8(a));
    if let Some(hex) = s.strip_prefix('#') {
        let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let full = match hex.len() {
            3 | 4 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => hex,
            _ => return None,
        };
        let v = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
        let a = if full.len() == 8 { v(6)? } else { 255 };
        return Some(Color32::from_rgba_unmultiplied(v(0)?, v(2)?, v(4)?, a));
    }
    if let Some(open) = s.find('(') {
        let name = &s[..open];
        let inner = s[open + 1..].strip_suffix(')')?;
        let a = args(inner);
        return match name {
            "rgb" | "rgba" if a.len() >= 3 => {
                let ch = |x: &String| -> Option<f32> {
                    let v = eval_number(x)?;
                    Some(if x.contains('%') { v / 100.0 } else { v / 255.0 })
                };
                Some(rgba(ch(&a[0])?, ch(&a[1])?, ch(&a[2])?, alpha_of(a.get(3))?))
            }
            "hsl" | "hsla" if a.len() >= 3 => {
                let (r, g, b) =
                    hsl_to_rgb(eval_number(&a[0])?, eval_number(&a[1])? / 100.0, eval_number(&a[2])? / 100.0);
                Some(rgba(r, g, b, alpha_of(a.get(3))?))
            }
            _ => None, // color-mix(), lab(), … not supported
        };
    }
    Some(match s.as_str() {
        "transparent" => Color32::TRANSPARENT,
        "white" => Color32::WHITE,
        "black" => Color32::BLACK,
        "red" => Color32::from_rgb(255, 0, 0),
        "green" => Color32::from_rgb(0, 128, 0),
        "blue" => Color32::from_rgb(0, 0, 255),
        "gray" | "grey" => Color32::from_rgb(128, 128, 128),
        "orange" => Color32::from_rgb(255, 165, 0),
        "purple" => Color32::from_rgb(128, 0, 128),
        _ => return None,
    })
}

/// Length in px (`16px`, `1.2em`/`rem` relative to 16px, plain numbers, calc()).
pub fn parse_px(s: &str) -> Option<f32> {
    let t = s.trim().to_lowercase();
    let v = eval_number(&t)?;
    Some(if t.contains("em") { v * 16.0 } else { v })
}

// ------------------------------------------------------------------------------- defaults

/// The default variable chain (documented defaults of the base theme), so that themes which only
/// override e.g. `--color-base-00` or `--accent-h` work as expected.
fn defaults(dark: bool) -> Vec<(&'static str, &'static str)> {
    let mut v = vec![
        ("--accent-h", "254"),
        ("--accent-s", "80%"),
        ("--accent-l", "68%"),
        ("--color-accent", "hsl(var(--accent-h), var(--accent-s), var(--accent-l))"),
        ("--interactive-accent", "var(--color-accent)"),
        ("--link-color", "var(--text-accent)"),
        ("--background-primary", "var(--color-base-00)"),
        ("--background-primary-alt", "var(--color-base-10)"),
        ("--background-secondary", "var(--color-base-20)"),
        ("--background-modifier-border", "var(--color-base-30)"),
        ("--background-modifier-hover", "rgba(var(--mono-rgb-100), 0.075)"),
        ("--background-modifier-active-hover", "rgba(var(--mono-rgb-100), 0.1)"),
        ("--nav-item-background-hover", "var(--background-modifier-hover)"),
        ("--nav-item-background-active", "var(--background-modifier-active-hover)"),
        ("--text-normal", "var(--color-base-100)"),
        ("--text-muted", "var(--color-base-70)"),
        ("--text-faint", "var(--color-base-50)"),
        ("--code-background", "var(--background-primary-alt)"),
        ("--code-normal", "var(--text-normal)"),
        ("--tag-background", "hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.1)"),
        ("--tag-color", "var(--text-accent)"),
        ("--checkbox-color", "var(--interactive-accent)"),
        ("--blockquote-border-color", "var(--interactive-accent)"),
        ("--h1-size", "1.802em"),
        ("--h2-size", "1.602em"),
        ("--h3-size", "1.424em"),
        ("--h4-size", "1.266em"),
        ("--h5-size", "1.125em"),
        ("--h6-size", "1em"),
    ];
    if dark {
        v.extend([
            ("--mono-rgb-100", "255, 255, 255"),
            ("--color-base-00", "#1e1e1e"),
            ("--color-base-10", "#242424"),
            ("--color-base-20", "#262626"),
            ("--color-base-30", "#363636"),
            ("--color-base-50", "#666666"),
            ("--color-base-70", "#b3b3b3"),
            ("--color-base-100", "#dadada"),
            ("--text-accent", "hsl(var(--accent-h), var(--accent-s), calc(var(--accent-l) + 2.5%))"),
        ]);
    } else {
        v.extend([
            ("--mono-rgb-100", "0, 0, 0"),
            ("--color-base-00", "#ffffff"),
            ("--color-base-10", "#fafafa"),
            ("--color-base-20", "#f6f6f6"),
            ("--color-base-30", "#e0e0e0"),
            ("--color-base-50", "#ababab"),
            ("--color-base-70", "#5c5c5c"),
            ("--color-base-100", "#222222"),
            ("--text-accent", "hsl(var(--accent-h), var(--accent-s), calc(var(--accent-l) - 3.8%))"),
        ]);
    }
    v
}

/// Result of applying CSS to one colour scheme.
pub struct Applied {
    pub palette: Palette,
    pub metrics: Metrics,
    pub used: Vec<String>,
}

const KNOWN: &[&str] = &[
    "--background-primary",
    "--background-primary-alt",
    "--background-secondary",
    "--background-modifier-border",
    "--background-modifier-hover",
    "--background-modifier-active-hover",
    "--nav-item-background-hover",
    "--nav-item-background-active",
    "--text-normal",
    "--text-muted",
    "--text-faint",
    "--interactive-accent",
    "--color-accent",
    "--text-accent",
    "--link-color",
    "--code-background",
    "--code-normal",
    "--tag-background",
    "--tag-color",
    "--checkbox-color",
    "--blockquote-border-color",
    "--bold-color",
    "--italic-color",
    "--font-text-size",
    "--file-line-width",
    "--line-height-normal",
    "--h1-size",
    "--h2-size",
    "--h3-size",
    "--h4-size",
    "--h5-size",
    "--h6-size",
    "--h1-color",
    "--h2-color",
    "--h3-color",
    "--h4-color",
    "--h5-color",
    "--h6-color",
    "--text-selection",
    "--accent-h",
    "--accent-s",
    "--accent-l",
    "--mono-rgb-100",
    "--color-base-00",
    "--color-base-10",
    "--color-base-20",
    "--color-base-30",
    "--color-base-50",
    "--color-base-70",
    "--color-base-100",
];

pub fn apply(parsed: &Parsed, extra: &[(String, String)], dark: bool, base: Palette, base_metrics: Metrics) -> Applied {
    let mut vars: HashMap<String, String> =
        defaults(dark).into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut user = std::collections::HashSet::new();
    let scope = if dark { Scope::Dark } else { Scope::Light };
    for (s, k, v) in &parsed.vars {
        if *s == Scope::Both || *s == scope {
            vars.insert(k.clone(), v.clone());
            user.insert(k.clone());
        }
    }
    for (k, v) in extra {
        vars.insert(k.clone(), v.clone());
        user.insert(k.clone());
    }
    let get = |name: &str| resolve_var(name, &vars);
    let color = |names: &[&str], fallback: Color32| {
        names.iter().find_map(|n| get(n).and_then(|v| parse_color(&v))).unwrap_or(fallback)
    };
    let opt_color = |name: &str| if user.contains(name) { get(name).and_then(|v| parse_color(&v)) } else { None };
    let palette = Palette {
        bg: color(&["--background-primary"], base.bg),
        bg2: color(&["--background-secondary"], base.bg2),
        border: color(&["--background-modifier-border"], base.border),
        hover: color(&["--nav-item-background-hover", "--background-modifier-hover"], base.hover),
        active: color(&["--nav-item-background-active", "--background-modifier-active-hover"], base.active),
        text: color(&["--text-normal"], base.text),
        heading: color(&["--text-normal"], base.heading),
        muted: color(&["--text-muted"], base.muted),
        faint: color(&["--text-faint"], base.faint),
        accent: color(&["--interactive-accent", "--color-accent"], base.accent),
        link: color(&["--link-color", "--text-accent"], base.link),
        code_bg: color(&["--code-background"], base.code_bg),
        code_text: color(&["--code-normal"], base.code_text),
        tag_bg: color(&["--tag-background"], base.tag_bg),
    };
    let mut m = base_metrics;
    if user.contains("--font-text-size") {
        m.font_size = get("--font-text-size").and_then(|v| parse_px(&v)).filter(|v| (8.0..=48.0).contains(v));
    }
    if user.contains("--file-line-width") {
        m.line_width = get("--file-line-width").and_then(|v| parse_px(&v)).filter(|v| (200.0..=4000.0).contains(v));
    }
    if let Some(lh) = get("--line-height-normal").and_then(|v| eval_number(&v)).filter(|v| (1.0..=3.0).contains(v)) {
        m.line_height = lh;
    }
    for (i, n) in ["--h1-size", "--h2-size", "--h3-size", "--h4-size", "--h5-size", "--h6-size"].iter().enumerate() {
        if let Some(v) = get(n) {
            let t = v.trim().to_lowercase();
            if let Some(x) = eval_number(&t) {
                m.h_scale[i] = if t.contains("px") { x / 16.0 } else { x };
            }
        }
        m.h_color[i] = opt_color(&format!("--h{}-color", i + 1));
    }
    m.bold = opt_color("--bold-color");
    m.italic = opt_color("--italic-color");
    m.checkbox = opt_color("--checkbox-color");
    m.quote_bar = opt_color("--blockquote-border-color");
    m.tag_color = opt_color("--tag-color");
    m.selection = opt_color("--text-selection");
    let mut used: Vec<String> = user.into_iter().collect();
    used.sort();
    Applied { palette, metrics: m, used }
}

pub fn is_known(name: &str) -> bool {
    KNOWN.contains(&name)
}

// ------------------------------------------------------------------------------- files

/// (name, path) of themes and snippets found in the given config folders
/// (Obsidian layout: `themes/<Name>/theme.css`, `themes/<Name>.css`, `snippets/*.css`).
/// (name, file) pairs
pub type Named = Vec<(String, PathBuf)>;

pub fn discover(dirs: &[PathBuf]) -> (Named, Named) {
    let mut themes: Vec<(String, PathBuf)> = vec![];
    let mut snippets: Vec<(String, PathBuf)> = vec![];
    let list = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> =
            std::fs::read_dir(d).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        v
    };
    for dir in dirs {
        for p in list(&dir.join("themes")) {
            let (name, file) = if p.is_dir() {
                (p.file_name().map(|n| n.to_string_lossy().to_string()), p.join("theme.css"))
            } else if p.extension().is_some_and(|e| e == "css") {
                (p.file_stem().map(|n| n.to_string_lossy().to_string()), p.clone())
            } else {
                continue;
            };
            if let Some(name) = name
                && file.is_file()
                && !themes.iter().any(|(n, _)| n == &name)
            {
                themes.push((name, file));
            }
        }
        for p in list(&dir.join("snippets")) {
            if p.extension().is_some_and(|e| e == "css")
                && let Some(name) = p.file_stem().map(|n| n.to_string_lossy().to_string())
                && !snippets.iter().any(|(n, _)| n == &name)
            {
                snippets.push((name, p));
            }
        }
    }
    (themes, snippets)
}

/// Subset of `.obsidian/appearance.json`
#[derive(Debug, Default, Clone)]
pub struct Appearance {
    pub css_theme: Option<String>,
    pub snippets: Vec<String>,
    pub base_font_size: Option<f32>,
    pub dark: Option<bool>,
    pub accent: Option<String>,
}

pub fn read_appearance(obsidian_dir: &Path) -> Option<Appearance> {
    let s = std::fs::read_to_string(obsidian_dir.join("appearance.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    Some(Appearance {
        css_theme: v.get("cssTheme").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
        snippets: v
            .get("enabledCssSnippets")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        base_font_size: v.get("baseFontSize").and_then(|x| x.as_f64()).map(|f| f as f32),
        dark: v.get("theme").and_then(|x| x.as_str()).and_then(|t| match t {
            "obsidian" => Some(true),
            "moonstone" => Some(false),
            _ => None,
        }),
        accent: v.get("accentColor").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#1e1e1e"), Some(Color32::from_rgb(0x1e, 0x1e, 0x1e)));
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some(Color32::from_rgb(10, 20, 30)));
        assert_eq!(parse_color("rgb(10 20 30 / 50%)").map(|c| c.a()), Some(128));
        let c = parse_color("hsl(254, 80%, calc(68% + 0%))").unwrap();
        assert_eq!((c.r(), c.g(), c.b()), (0x8b, 0x6c, 0xef));
        assert_eq!(parse_color("color-mix(in srgb, red, blue)"), None);
        assert_eq!(eval_number("calc(68% - 3.8%)"), Some(64.2));
        assert_eq!(eval_number("calc((2 + 3) * 4px)"), Some(20.0));
    }

    #[test]
    fn theme_file() {
        let css = r#"
            /* @settings ... */
            @import url("x.css");
            body { --font-text-size: 18px; --file-line-width: 50rem; }
            .theme-dark { --color-base-00: #101010; --accent-h: 200; --text-normal: var(--missing, #eeeeee); }
            .theme-light, .theme-dark .something { --background-primary: #fafafa !important; }
            @media (min-width: 400px) { .theme-dark { --text-muted: red; } }
            @media print { body { --background-primary: white; --text-muted: blue; } }
            .nav-file-title { color: red; }
            .theme-dark { .nested { color: blue } --h1-color: #ff0000; }
        "#;
        let mut p = Parsed::default();
        parse(css, &mut p);
        assert_eq!(p.ignored_rules, 1);
        let base = theme::builtin_palette(true);
        let a = apply(&p, &[], true, base, Metrics::default());
        assert_eq!(a.palette.bg, Color32::from_rgb(0x10, 0x10, 0x10));
        assert_eq!(a.palette.text, Color32::from_rgb(0xee, 0xee, 0xee));
        assert_eq!(a.palette.muted, Color32::from_rgb(255, 0, 0));
        assert_eq!(a.metrics.font_size, Some(18.0));
        assert_eq!(a.metrics.line_width, Some(800.0));
        assert_eq!(a.metrics.h_color[0], Some(Color32::from_rgb(255, 0, 0)));
        assert!(a.palette.accent.b() > a.palette.accent.r()); // hue 200 = blue-ish
        // the light scheme gets its own value
        let l = apply(&p, &[], false, theme::builtin_palette(false), Metrics::default());
        assert_eq!(l.palette.bg, Color32::from_rgb(0xfa, 0xfa, 0xfa));
    }

    fn vars(css: &str, dark: bool) -> HashMap<String, String> {
        let mut p = Parsed::default();
        parse(css, &mut p);
        let scope = if dark { Scope::Dark } else { Scope::Light };
        p.vars.into_iter().filter(|(s, _, _)| *s == Scope::Both || *s == scope).map(|(_, k, v)| (k, v)).collect()
    }

    #[test]
    fn media_queries() {
        let css = r#"
            body { --a: 1; --b: 1; --c: 1; --d: 1; --e: 1; --f: 1; --g: 1 }
            @media print { body { --a: 2 } }
            @media screen and (prefers-color-scheme: dark) { body { --b: 2 } .theme-light { --c: 2 } }
            @media (prefers-color-scheme: light) { :root { --c: 3 } }
            @media not print { body { --d: 2 } }
            @media (max-width: 600px) { body { --e: 2 } }
            @media print, (min-width: 600px) { body { --f: 2 } }
            @media only screen { @media speech { body { --g: 2 } } }
            @MEDIA ALL { body { --h: 2 } }
        "#;
        let get = |dark, k: &str| vars(css, dark).get(k).cloned().unwrap_or_default();
        assert_eq!((get(true, "--a"), get(false, "--a")), ("1".into(), "1".into()));
        assert_eq!((get(true, "--b"), get(false, "--b")), ("2".into(), "1".into()));
        assert_eq!((get(true, "--c"), get(false, "--c")), ("1".into(), "3".into()));
        assert_eq!((get(true, "--d"), get(true, "--e"), get(true, "--f")), ("2".into(), "1".into(), "2".into()));
        assert_eq!((get(true, "--g"), get(true, "--h")), ("1".into(), "2".into()));
    }

    #[test]
    fn strings_and_comments() {
        // guillemet échappé : la suite du thème est lue
        let css = r#"
            .x::before { content: "\""; }
            body { --a: "};/* pas un commentaire */"; --b: url(http://x/*y); --c: 3; /* c */ --d: '\'{' }
            .y { content: "non fermée
            }
            body { --e: 5 } /* fin
        "#;
        let v = vars(css, true);
        assert_eq!(v["--a"], r#""};/* pas un commentaire */""#);
        assert_eq!(v["--b"], "url(http://x/*y)");
        assert_eq!((v["--c"].as_str(), v["--d"].as_str(), v["--e"].as_str()), ("3", r"'\'{'", "5"));
    }

    #[test]
    fn hostile_values() {
        let map = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        // auto-référence : la variable est invalide, le repli du var() qui la lit s'applique
        let v = map(&[("--t", "var(--t, x) var(--t, x)"), ("--a", "var(--b)"), ("--b", "var(--a, 1)")]);
        assert_eq!(resolve("var(--t, red)", &v).as_deref(), Some("red"));
        assert_eq!(resolve("var(--t)", &v), None);
        assert_eq!(resolve("var(--a, blue)", &v).as_deref(), Some("blue"));
        assert_eq!(resolve("var(--b, blue)", &v).as_deref(), Some("blue"));
        // éventail sans cycle (2^40 copies) : abandon immédiat au lieu d'une chaîne géante
        let names: Vec<String> = (0..=40).map(|i| format!("--v{i}")).collect();
        let mut fan: HashMap<String, String> =
            (0..40).map(|i| (names[i].clone(), format!("var({0}) var({0})", names[i + 1]))).collect();
        fan.insert(names[40].clone(), "x".into());
        let t = std::time::Instant::now();
        assert_eq!(resolve("var(--v0, repli)", &fan), None);
        assert!(t.elapsed().as_secs() < 2);
        // un usage normal répété reste permis
        assert_eq!(resolve("var(--v38)", &fan).as_deref(), Some("x x x x"));
        // calc() et blocs imbriqués à l'excès : pas de débordement de pile
        assert_eq!(eval_number(&format!("calc{}", "(".repeat(200_000))), None);
        assert_eq!(eval_number(&format!("calc({}1)", "-".repeat(200_000))), None);
        assert_eq!(eval_number(&format!("calc({}1{})", "(".repeat(40), ")".repeat(40))), Some(1.0));
        let deep = format!("{}body {{ --a: 1 }}{}", "@media screen {".repeat(100_000), "}".repeat(100_000));
        assert!(vars(&deep, true).is_empty());
        let a = apply(
            &Parsed::default(),
            &[("--text-normal".into(), "var(--text-normal, x) var(--text-normal, x)".into())],
            true,
            theme::builtin_palette(true),
            Metrics::default(),
        );
        assert_eq!(a.palette.text, theme::builtin_palette(true).text);
    }

    #[test]
    fn defaults_match_builtin() {
        for dark in [true, false] {
            let a = apply(&Parsed::default(), &[], dark, theme::builtin_palette(dark), Metrics::default());
            let b = theme::builtin_palette(dark);
            assert_eq!(a.palette.bg, b.bg);
            assert_eq!(a.palette.bg2, b.bg2);
            assert_eq!(a.palette.text, b.text);
            assert_eq!(a.palette.accent, b.accent);
        }
    }
}
