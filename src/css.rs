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

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Index of the `}` matching the `{` at `open` (byte index), honouring nesting and strings.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in s[open..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '{') => depth += 1,
            (None, '}') => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
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
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 0 => {
                push(&block[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&block[start..]);
}

fn parse_rules(css: &str, out: &mut Parsed) {
    let mut i = 0;
    while i < css.len() {
        let Some(rel) = css[i..].find('{') else { break };
        let open = i + rel;
        let prelude_start = css[i..open].rfind(';').map(|p| i + p + 1).unwrap_or(i);
        let prelude = css[prelude_start..open].trim();
        let Some(close) = matching_brace(css, open) else { break };
        let body = &css[open + 1..close];
        if let Some(at) = prelude.strip_prefix('@') {
            let kw = at.split_whitespace().next().unwrap_or("").to_lowercase();
            if matches!(kw.as_str(), "media" | "supports" | "layer" | "container" | "document") {
                parse_rules(body, out);
            }
        } else {
            // top-level declarations of the rule (nested rules removed)
            let mut flat = String::new();
            let mut j = 0;
            while j < body.len() {
                match body[j..].find('{') {
                    Some(r) => {
                        let o = j + r;
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
                Some(scope) => declarations(&flat, scope, out),
                None => out.ignored_rules += 1,
            }
        }
        i = close + 1;
    }
}

pub fn parse(css: &str, out: &mut Parsed) {
    parse_rules(&strip_comments(css), out);
}

// ------------------------------------------------------------------------------- evaluation

/// Resolves every `var(--x[, fallback])` in `value`.
pub fn resolve(value: &str, vars: &HashMap<String, String>, depth: usize) -> Option<String> {
    if depth > 24 {
        return None;
    }
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
        let v = match vars.get(name) {
            Some(v) => resolve(v, vars, depth + 1),
            None => None,
        }
        .or_else(|| fallback.and_then(|f| resolve(f, vars, depth + 1)))?;
        out.push_str(&v);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Evaluates a number possibly wrapped in calc(); units are ignored (68% -> 68, 16px -> 16).
pub fn eval_number(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_prefix("calc").map(str::trim).unwrap_or(s);
    let tokens = tokenize(s)?;
    let mut pos = 0;
    let v = expr(&tokens, &mut pos)?;
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

fn expr(t: &[Tok], p: &mut usize) -> Option<f32> {
    let mut v = term(t, p)?;
    while let Some(Tok::Op(c @ ('+' | '-'))) = t.get(*p) {
        *p += 1;
        let r = term(t, p)?;
        v = if *c == '+' { v + r } else { v - r };
    }
    Some(v)
}

fn term(t: &[Tok], p: &mut usize) -> Option<f32> {
    let mut v = factor(t, p)?;
    while let Some(Tok::Op(c @ ('*' | '/'))) = t.get(*p) {
        *p += 1;
        let r = factor(t, p)?;
        v = if *c == '*' { v * r } else { v / r };
    }
    Some(v)
}

fn factor(t: &[Tok], p: &mut usize) -> Option<f32> {
    match t.get(*p)? {
        Tok::Num(n) => {
            *p += 1;
            Some(*n)
        }
        Tok::Op('(') => {
            *p += 1;
            let v = expr(t, p)?;
            (t.get(*p) == Some(&Tok::Op(')'))).then(|| *p += 1)?;
            Some(v)
        }
        Tok::Op('-') => {
            *p += 1;
            factor(t, p).map(|v| -v)
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
    let get = |name: &str| resolve(vars.get(name)?, &vars, 0);
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
            @media (max-width: 400px) { .theme-dark { --text-muted: red; } }
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
