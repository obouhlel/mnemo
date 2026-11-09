//! Look & feel: fonts, colour palette, egui visuals, markdown highlighting for the editor
//! and a few custom widgets (icon buttons, file-tree rows, tabs).

use egui::text::LayoutJob;
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Response, Sense, Stroke, TextFormat, Ui,
    Vec2, vec2,
};
use std::sync::Arc;

pub const FONT_BOLD: &str = "inter-semibold";
pub const FONT_ITALIC: &str = "inter-italic";

pub fn bold_family() -> FontFamily {
    FontFamily::Name(FONT_BOLD.into())
}
pub fn italic_family() -> FontFamily {
    FontFamily::Name(FONT_ITALIC.into())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Main background (editor / reading area)
    pub bg: Color32,
    /// Sidebars, ribbon, tab header, status bar
    pub bg2: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub active: Color32,
    pub text: Color32,
    pub heading: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub link: Color32,
    pub code_bg: Color32,
    pub code_text: Color32,
    pub tag_bg: Color32,
}

/// Typography / layout values that CSS themes can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Editor / reading font size (`--font-text-size`), `None` = user setting
    pub font_size: Option<f32>,
    /// Readable line width in px (`--file-line-width`)
    pub line_width: Option<f32>,
    pub line_height: f32,
    pub h_scale: [f32; 6],
    pub h_color: [Option<Color32>; 6],
    pub bold: Option<Color32>,
    pub italic: Option<Color32>,
    pub checkbox: Option<Color32>,
    pub quote_bar: Option<Color32>,
    pub tag_color: Option<Color32>,
    pub selection: Option<Color32>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            font_size: None,
            line_width: None,
            line_height: 1.55,
            h_scale: [1.802, 1.602, 1.424, 1.266, 1.125, 1.0],
            h_color: [None; 6],
            bold: None,
            italic: None,
            checkbox: None,
            quote_bar: None,
            tag_color: None,
            selection: None,
        }
    }
}

/// Palette + metrics computed from user CSS (themes / snippets), for both schemes.
#[derive(Clone)]
pub struct Override {
    pub dark: (Palette, Metrics),
    pub light: (Palette, Metrics),
}

static OVERRIDE: std::sync::RwLock<Option<Override>> = std::sync::RwLock::new(None);
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn set_override(o: Option<Override>) {
    *OVERRIDE.write().unwrap() = o;
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Changes every time the CSS override changes (used to re-apply egui visuals).
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn palette(dark: bool) -> Palette {
    match OVERRIDE.read().unwrap().as_ref() {
        Some(o) => {
            if dark {
                o.dark.0
            } else {
                o.light.0
            }
        }
        None => builtin_palette(dark),
    }
}

pub fn metrics(dark: bool) -> Metrics {
    match OVERRIDE.read().unwrap().as_ref() {
        Some(o) => {
            if dark {
                o.dark.1
            } else {
                o.light.1
            }
        }
        None => Metrics::default(),
    }
}

pub fn builtin_palette(dark: bool) -> Palette {
    // Values of the classic default theme (CSS variables --background-primary, etc.)
    if dark {
        Palette {
            bg: Color32::from_rgb(0x1e, 0x1e, 0x1e),
            bg2: Color32::from_rgb(0x26, 0x26, 0x26),
            border: Color32::from_rgb(0x36, 0x36, 0x36),
            hover: Color32::from_rgb(0x2f, 0x2f, 0x2f),
            active: Color32::from_rgb(0x36, 0x36, 0x36),
            text: Color32::from_rgb(0xda, 0xda, 0xda),
            heading: Color32::from_rgb(0xda, 0xda, 0xda),
            muted: Color32::from_rgb(0xb3, 0xb3, 0xb3),
            faint: Color32::from_rgb(0x66, 0x66, 0x66),
            accent: Color32::from_rgb(0x8b, 0x6c, 0xef),
            link: Color32::from_rgb(0x94, 0x78, 0xf0),
            code_bg: Color32::from_rgb(0x2a, 0x2a, 0x2a),
            code_text: Color32::from_rgb(0xda, 0xda, 0xda),
            tag_bg: Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xef, 0x26),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(0xff, 0xff, 0xff),
            bg2: Color32::from_rgb(0xf6, 0xf6, 0xf6),
            border: Color32::from_rgb(0xe0, 0xe0, 0xe0),
            hover: Color32::from_rgb(0xea, 0xea, 0xea),
            active: Color32::from_rgb(0xe3, 0xe3, 0xe3),
            text: Color32::from_rgb(0x22, 0x22, 0x22),
            heading: Color32::from_rgb(0x22, 0x22, 0x22),
            muted: Color32::from_rgb(0x5c, 0x5c, 0x5c),
            faint: Color32::from_rgb(0xab, 0xab, 0xab),
            accent: Color32::from_rgb(0x8b, 0x6c, 0xef),
            link: Color32::from_rgb(0x70, 0x52, 0xe6),
            code_bg: Color32::from_rgb(0xf5, 0xf5, 0xf5),
            code_text: Color32::from_rgb(0x22, 0x22, 0x22),
            tag_bg: Color32::from_rgba_unmultiplied(0x8a, 0x6c, 0xef, 0x1a),
        }
    }
}

/// Lucide icon codepoints (open-source icon set, ISC licence).
pub mod icon {
    pub const FILES_TAB: &str = "\u{e334}"; // folder-closed
    pub const SEARCH: &str = "\u{e155}";
    pub const TAG: &str = "\u{e183}";
    pub const NEW_NOTE: &str = "\u{e176}"; // square-pen
    pub const NEW_FOLDER: &str = "\u{e0de}";
    pub const SORT: &str = "\u{e050}";
    pub const COLLAPSE: &str = "\u{e228}";
    pub const SETTINGS: &str = "\u{e158}";
    pub const HELP: &str = "\u{e082}";
    pub const GRAPH: &str = "\u{e28d}"; // git-fork
    pub const PALETTE: &str = "\u{e20a}"; // square-terminal
    pub const SWITCHER: &str = "\u{e0cf}"; // file-search
    pub const PANEL_LEFT: &str = "\u{e12e}";
    pub const PANEL_RIGHT: &str = "\u{e436}";
    pub const PLUS: &str = "\u{e141}";
    pub const BACK: &str = "\u{e04c}";
    pub const FORWARD: &str = "\u{e04d}";
    pub const READING: &str = "\u{e063}"; // book-open
    pub const EDITING: &str = "\u{e1f9}"; // pencil
    pub const SPLIT: &str = "\u{e09c}"; // columns-2
    pub const MORE: &str = "\u{e0bb}"; // ellipsis-vertical
    pub const LINK: &str = "\u{e108}";
    pub const LIST: &str = "\u{e10c}";
    pub const PUZZLE: &str = "\u{e29c}";
    pub const SYNC: &str = "\u{e149}"; // refresh-cw
    pub const OFFLINE: &str = "\u{e091}"; // cloud-off
    pub const ALERT: &str = "\u{e07b}";
    pub const CHEVRON_RIGHT: &str = "\u{e073}";
    pub const CHEVRON_DOWN: &str = "\u{e071}";
    pub const DASHBOARD: &str = "\u{e1c1}"; // layout-dashboard
    pub const TICKET: &str = "\u{e20f}";
}

/// Toutes les icônes Lucide de la police : (nom, caractère), triées par nom.
pub fn lucide_icons() -> &'static [(&'static str, char)] {
    static LIST: std::sync::OnceLock<Vec<(&'static str, char)>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        include_str!("../assets/fonts/lucide-names.txt")
            .lines()
            .filter_map(|l| {
                let (name, hex) = l.split_once(' ')?;
                Some((name, char::from_u32(u32::from_str_radix(hex, 16).ok()?)?))
            })
            .collect()
    })
}

/// Caractère d'une icône Lucide d'après son nom (« radio-tower »).
pub fn lucide(name: &str) -> Option<char> {
    let list = lucide_icons();
    list.binary_search_by(|(n, _)| (*n).cmp(name)).ok().map(|i| list[i].1)
}

/// Dessine l'icône d'un dossier / d'une note : nom Lucide, sinon le texte tel quel (emoji).
pub fn paint_entry_icon(painter: &egui::Painter, center: egui::Pos2, icon: &str, size: f32, color: Color32) {
    match lucide(icon) {
        Some(c) => {
            painter.text(center, egui::Align2::CENTER_CENTER, c, FontId::new(size, icon_family()), color);
        }
        None => {
            painter.text(center, egui::Align2::CENTER_CENTER, icon, FontId::proportional(size), color);
        }
    }
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name("lucide".into())
}

pub fn icon_text(icon: &str, size: f32) -> egui::RichText {
    egui::RichText::new(icon).font(FontId::new(size, icon_family()))
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf"))));
    fonts.font_data.insert(
        FONT_BOLD.into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.ttf"))),
    );
    fonts.font_data.insert(
        FONT_ITALIC.into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-Italic.ttf"))),
    );
    fonts
        .font_data
        .insert("lucide".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/lucide.ttf"))));
    fonts.families.insert(icon_family(), vec!["lucide".into()]);
    // keep egui's default fonts (emoji, symbols) as fallbacks
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "inter".into());
    for (name, family) in [(FONT_BOLD, bold_family()), (FONT_ITALIC, italic_family())] {
        let mut list = vec![name.to_string()];
        list.extend(fallbacks.iter().cloned());
        fonts.families.insert(family, list);
    }
    ctx.set_fonts(fonts);
}

/// `size` is the interface font size (the editor has its own size).
pub fn apply(ctx: &egui::Context, dark: bool, size: f32) {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg2;
    v.window_fill = p.bg2;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = p.bg;
    v.text_edit_bg_color = Some(p.bg);
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.code_bg;
    v.hyperlink_color = p.link;
    v.override_text_color = None;
    v.selection.bg_fill = metrics(dark).selection.unwrap_or(p.accent.gamma_multiply(0.35));
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = true;
    v.striped = false;
    let round = CornerRadius::same(5);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg2;
    w.noninteractive.weak_bg_fill = p.bg2;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = round;
    w.inactive.bg_fill = p.active;
    w.inactive.weak_bg_fill = p.active;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.muted);
    w.inactive.corner_radius = round;
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0, p.heading);
    w.hovered.corner_radius = round;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.active;
    w.active.weak_bg_fill = p.active;
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, p.heading);
    w.active.corner_radius = round;
    w.active.expansion = 0.0;
    w.open = w.active;
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 40 }),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 30 }),
    };
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(size));
        s.text_styles.insert(Button, FontId::proportional(size - 0.5));
        s.text_styles.insert(Small, FontId::proportional(size - 3.0));
        s.text_styles.insert(Monospace, FontId::monospace(size - 1.5));
        s.text_styles.insert(Heading, FontId::new(size * 1.6, bold_family()));
        s.spacing.item_spacing = vec2(8.0, 4.0);
        s.spacing.button_padding = vec2(8.0, 3.0);
        s.spacing.indent = 16.0;
        s.spacing.interact_size.y = 24.0;
        s.spacing.window_margin = egui::Margin::same(14);
        s.spacing.menu_margin = egui::Margin::same(6);
    });
}

// ---------------------------------------------------------------------------------- widgets

/// Flat icon button (ribbon, toolbars): muted, highlighted on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, active: bool) -> Response {
    icon_button_sized(ui, icon, tooltip, active, true, 18.0)
}

pub fn icon_button_sized(
    ui: &mut Ui,
    icon: &str,
    tooltip: &str,
    active: bool,
    enabled: bool,
    icon_size: f32,
) -> Response {
    let p = palette(ui.visuals().dark_mode);
    let size = vec2(icon_size + 12.0, icon_size + 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if enabled && (resp.hovered() || active) {
        ui.painter().rect_filled(rect, 5.0, if active { p.active } else { p.hover });
    }
    let color = if !enabled {
        p.faint.gamma_multiply(0.7)
    } else if resp.hovered() || active {
        p.text
    } else {
        p.muted
    };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::new(icon_size, icon_family()), color);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

/// One row of the file explorer: indentation guides, chevron for folders, hover/active background.
pub fn tree_row(ui: &mut Ui, label: &str, depth: usize, active: bool, folder: Option<bool>) -> Response {
    tree_row_icon(ui, label, depth, active, folder, None)
}

/// Comme `tree_row`, avec une icône (Lucide ou emoji) devant le nom.
pub fn tree_row_icon(
    ui: &mut Ui,
    label: &str,
    depth: usize,
    active: bool,
    folder: Option<bool>,
    icon: Option<&str>,
) -> Response {
    let p = palette(ui.visuals().dark_mode);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let painter = ui.painter();
    let indent = 16.0;
    let left = rect.left() + 6.0 + depth as f32 * indent;
    let row = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 2.0, rect.top() + 1.0),
        egui::pos2(rect.right() - 2.0, rect.bottom() - 1.0),
    );
    if active {
        painter.rect_filled(row, 5.0, p.active);
    } else if resp.hovered() {
        painter.rect_filled(row, 5.0, p.hover);
    }
    for d in 0..depth {
        let x = rect.left() + 13.0 + d as f32 * indent;
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(1.0, p.border));
    }
    let text_color = if active || resp.hovered() { p.text } else { p.muted };
    let mut x = left;
    if let Some(open) = folder {
        let c = egui::pos2(x + 6.0, rect.center().y);
        let ic = if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT };
        painter.text(c, egui::Align2::CENTER_CENTER, ic, FontId::new(13.0, icon_family()), p.faint);
        x += 16.0;
    } else {
        x += 4.0;
    }
    if let Some(ic) = icon {
        paint_entry_icon(painter, egui::pos2(x + 7.0, rect.center().y), ic, 14.0, text_color);
        x += 21.0;
    }
    // single line, truncated with "…" (full name in the tooltip)
    let mut job = LayoutJob::single_section(
        label.to_string(),
        TextFormat {
            font_id: FontId::proportional(ui.style().text_styles[&egui::TextStyle::Body].size - 1.0),
            color: text_color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x - 8.0).max(10.0));
    let galley = painter.layout_job(job);
    let elided = galley.elided;
    let y = rect.center().y - galley.size().y / 2.0;
    painter.with_clip_rect(rect).galley(egui::pos2(x, y), galley, text_color);
    if elided { resp.on_hover_text(label) } else { resp }
}

/// Ligne compacte d'un panneau latéral (tags…) : libellé à gauche, compteur discret à droite,
/// même hauteur et mêmes retraits que l'explorateur.
pub fn count_row(ui: &mut Ui, label: &str, count: &str) -> Response {
    let p = palette(ui.visuals().dark_mode);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let row = rect.shrink2(vec2(2.0, 1.0));
    if resp.hovered() {
        ui.painter().rect_filled(row, 5.0, p.hover);
    }
    let size = ui.style().text_styles[&egui::TextStyle::Body].size - 1.0;
    let color = if resp.hovered() { p.text } else { p.muted };
    let c = ui.painter().layout_no_wrap(count.to_string(), FontId::proportional(size - 1.5), p.faint);
    let cx = row.right() - 8.0 - c.size().x;
    let mut job = LayoutJob::single_section(
        label.to_string(),
        TextFormat { font_id: FontId::proportional(size), color, ..Default::default() },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((cx - row.left() - 16.0).max(10.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(row.left() + 8.0, rect.center().y - g.size().y / 2.0), g, color);
    ui.painter().galley(egui::pos2(cx, rect.center().y - c.size().y / 2.0), c, p.faint);
    resp
}

/// A suggestion row in the quick switcher / command palette.
pub fn suggestion_row(ui: &mut Ui, title: &str, detail: &str, selected: bool) -> Response {
    let p = palette(ui.visuals().dark_mode);
    let width = ui.available_width();
    let has_detail = !detail.is_empty();
    let h = 34.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    if selected || resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, if selected { p.active } else { p.hover });
    }
    let font = FontId::proportional(14.5);
    let g = ui.painter().layout_no_wrap(title.to_string(), font, p.text);
    let x = rect.left() + 12.0;
    ui.painter().with_clip_rect(rect).galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.text);
    if has_detail {
        let d = ui.painter().layout_no_wrap(detail.to_string(), FontId::proportional(12.5), p.faint);
        let dx = rect.right() - d.size().x - 12.0;
        ui.painter().with_clip_rect(rect).galley(egui::pos2(dx, rect.center().y - d.size().y / 2.0), d, p.faint);
    }
    resp
}

/// A tab in the tab header. Returns (tab response, close clicked).
pub fn tab(ui: &mut Ui, title: &str, active: bool, dirty: bool) -> (Response, bool) {
    let p = palette(ui.visuals().dark_mode);
    let font = FontId::proportional(ui.style().text_styles[&egui::TextStyle::Body].size - 1.5);
    let galley = ui.painter().layout_no_wrap(title.to_string(), font.clone(), p.text);
    let w = (galley.size().x + 48.0).clamp(110.0, 220.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
    let painter = ui.painter();
    let body = egui::Rect::from_min_max(rect.min + vec2(0.0, 4.0), rect.max);
    if active {
        painter.rect_filled(body, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.bg);
    } else if resp.hovered() {
        painter.rect_filled(body.shrink2(vec2(2.0, 3.0)), 6.0, p.hover);
    }
    let color = if active { p.text } else { p.muted };
    let text_w = w - 42.0;
    let galley = painter.layout(title.to_string(), font, color, text_w);
    let galley = if galley.rows.len() > 1 {
        painter.layout_no_wrap(truncate(title, 22), FontId::proportional(13.5), color)
    } else {
        galley
    };
    painter.with_clip_rect(body.shrink2(vec2(12.0, 0.0))).galley(
        egui::pos2(body.left() + 14.0, body.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    // close button / dirty dot
    let close_rect = egui::Rect::from_center_size(egui::pos2(body.right() - 16.0, body.center().y), Vec2::splat(18.0));
    let close_hover = ui.rect_contains_pointer(close_rect);
    let mut close = false;
    if active || resp.hovered() {
        if close_hover {
            painter.rect_filled(close_rect, 4.0, p.active);
        }
        let c = close_rect.center();
        let s = Stroke::new(1.4, if close_hover { p.heading } else { p.faint });
        painter.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], s);
        painter.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], s);
        if close_hover && resp.clicked() {
            close = true;
        }
    } else if dirty {
        painter.circle_filled(close_rect.center(), 3.0, p.faint);
    }
    if resp.middle_clicked() {
        close = true;
    }
    (resp, close)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n - 1).collect::<String>()) }
}

/// Small uppercase section header used in the side panels.
#[allow(dead_code)]
pub fn section_header(ui: &mut Ui, text: &str) {
    let p = palette(ui.visuals().dark_mode);
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).color(p.faint).strong());
    ui.add_space(2.0);
}

// ---------------------------------------------------------------------------------- editor highlighting

/// Builds a [`LayoutJob`] that styles markdown *source* (headings sized, bold/italic,
/// code, links, tags...) while keeping every character, so cursor positions stay exact.
/// `active` is the range of line indices containing the cursor/selection: markdown syntax is
/// shown there and hidden elsewhere ("live preview"); `None` hides it everywhere.
pub fn markdown_job(
    text: &str,
    p: &Palette,
    m: &Metrics,
    size: f32,
    wrap_width: f32,
    active: Option<(usize, usize)>,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let base = TextFormat {
        font_id: FontId::proportional(size),
        color: p.text,
        line_height: Some(size * m.line_height),
        ..Default::default()
    };
    let mono = TextFormat {
        font_id: FontId::monospace(size - 1.5),
        color: p.code_text,
        background: p.code_bg,
        line_height: Some(size * 1.5),
        ..Default::default()
    };
    let faint = TextFormat { color: p.faint, ..base.clone() };
    let mut fence = false;
    for (line_no, line) in text.split_inclusive('\n').enumerate() {
        let reveal = active.is_some_and(|(a, b)| line_no >= a && line_no <= b);
        let hidden =
            TextFormat { font_id: FontId::proportional(0.5), color: Color32::TRANSPARENT, ..Default::default() };
        let syntax = |f: &TextFormat| if reveal { TextFormat { color: p.faint, ..f.clone() } } else { hidden.clone() };
        let t = line.trim_start();
        let code_line = TextFormat { background: Color32::TRANSPARENT, ..mono.clone() };
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            let c = if reveal { p.faint } else { Color32::TRANSPARENT };
            job.append(line, 0.0, TextFormat { color: c, ..code_line });
            continue;
        }
        if fence {
            job.append(line, 0.0, TextFormat { color: p.text, ..code_line });
            continue;
        }
        let invisible = TextFormat { color: Color32::TRANSPARENT, ..base.clone() };
        let indent_len = line.len() - t.len();
        if indent_len > 0 {
            job.append(&line[..indent_len], 0.0, base.clone());
        }
        // headings
        let level = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && t[level..].starts_with(' ') {
            let hs = size * m.h_scale[level - 1];
            let hf = TextFormat {
                font_id: FontId::new(hs, bold_family()),
                color: m.h_color[level - 1].unwrap_or(p.heading),
                line_height: Some(hs * 1.45),
                ..Default::default()
            };
            job.append(&t[..level + 1], 0.0, syntax(&hf));
            inline(&mut job, &t[level + 1..], &hf, p, m, &mono, reveal);
            continue;
        }
        // horizontal rule
        let compact: String = t.trim_end().chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() >= 3
            && (compact.chars().all(|c| c == '-')
                || compact.chars().all(|c| c == '*')
                || compact.chars().all(|c| c == '_'))
        {
            job.append(t, 0.0, faint.clone());
            continue;
        }
        let mut rest = t;
        let mut fmt = base.clone();
        // blockquote / callout
        if rest.starts_with('>') {
            let n = rest.chars().take_while(|c| *c == '>' || *c == ' ').count();
            job.append(
                &rest[..n],
                0.0,
                if reveal { TextFormat { color: p.accent, ..base.clone() } } else { invisible.clone() },
            );
            rest = &rest[n..];
            fmt = TextFormat { color: p.muted, font_id: FontId::new(size, italic_family()), ..base.clone() };
        }
        // list markers & tasks
        let marker = if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
            2
        } else {
            let d = rest.chars().take_while(char::is_ascii_digit).count();
            if d > 0 && (rest[d..].starts_with(". ") || rest[d..].starts_with(") ")) { d + 2 } else { 0 }
        };
        if marker > 0 {
            let after = &rest[marker..];
            let is_task = after.len() >= 3
                && after.starts_with('[')
                && after.as_bytes()[2] == b']'
                && matches!(after.as_bytes()[1], b' ' | b'x' | b'X');
            let bullet = rest.starts_with(['-', '*', '+']);
            let marker_fmt = if reveal {
                TextFormat { color: p.faint, ..base.clone() }
            } else if bullet && is_task {
                syntax(&base) // hidden: the painted checkbox replaces it
            } else if bullet {
                invisible.clone() // keeps its width, a dot is painted over it
            } else {
                TextFormat { color: p.muted, ..base.clone() }
            };
            job.append(&rest[..marker], 0.0, marker_fmt);
            rest = &rest[marker..];
            if is_task {
                let done = rest.as_bytes()[1] != b' ';
                let f = if reveal {
                    TextFormat { color: if done { p.accent } else { p.faint }, ..mono.clone() }
                } else {
                    invisible.clone()
                };
                job.append(&rest[..3], 0.0, f);
                rest = &rest[3..];
                if done {
                    fmt = TextFormat { color: p.faint, strikethrough: Stroke::new(1.0, p.faint), ..fmt };
                }
            }
        }
        inline(&mut job, rest, &fmt, p, m, &mono, reveal);
    }
    job
}

/// Paints the live-preview decorations on top of / under the editor galley:
/// bullets, checkboxes, blockquote bars and code block backgrounds (into `bg_slot`).
/// Returns the clickable checkbox rects with the byte index of their check character.
#[allow(clippy::too_many_arguments)]
pub fn decorate_editor(
    content: &str,
    active: Option<(usize, usize)>,
    galley: &egui::Galley,
    origin: egui::Pos2,
    right: f32,
    p: &Palette,
    m: &Metrics,
    painter: &egui::Painter,
    bg_slot: egui::layers::ShapeIdx,
) -> Vec<(egui::Rect, usize)> {
    let check = m.checkbox.unwrap_or(p.accent);
    let bar = m.quote_bar.unwrap_or(p.accent);
    use egui::text::CCursor;
    let pos = |ci: usize| galley.pos_from_cursor(CCursor::new(ci)).translate(origin.to_vec2());
    let mut under = vec![];
    let mut boxes = vec![];
    let (mut ci, mut byte) = (0usize, 0usize);
    let mut fence_start: Option<usize> = None;
    let total_chars = content.chars().count();
    let code_rect = |a: usize, b: usize| {
        let (top, bottom) = (pos(a).top(), pos(b).bottom());
        egui::Shape::rect_filled(
            egui::Rect::from_min_max(egui::pos2(origin.x - 12.0, top - 3.0), egui::pos2(right + 12.0, bottom + 3.0)),
            4.0,
            p.code_bg,
        )
    };
    for (no, line) in content.split_inclusive('\n').enumerate() {
        let reveal = active.is_some_and(|(a, b)| no >= a && no <= b);
        let t = line.trim_start();
        let indent_chars = line.chars().count() - t.chars().count();
        let line_chars = line.chars().count();
        let last_char = (ci + line_chars).saturating_sub(1).max(ci);
        if t.starts_with("```") || t.starts_with("~~~") {
            match fence_start {
                None => fence_start = Some(ci),
                Some(start) => {
                    under.push(code_rect(start, last_char));
                    fence_start = None;
                }
            }
        } else if fence_start.is_none() && !reveal {
            let mut k = ci + indent_chars;
            let mut kb = byte + (line.len() - t.len());
            let mut rest = t;
            if rest.starts_with('>') {
                let r = pos(k);
                painter.line_segment(
                    [egui::pos2(r.left() + 1.0, r.top() + 2.0), egui::pos2(r.left() + 1.0, r.bottom() - 2.0)],
                    Stroke::new(2.0, bar),
                );
                let n = rest.chars().take_while(|c| *c == '>' || *c == ' ').count();
                k += n;
                kb += n;
                rest = &rest[n..];
            }
            if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
                let after = &rest[2..];
                let is_task = after.len() >= 3
                    && after.starts_with('[')
                    && after.as_bytes()[2] == b']'
                    && matches!(after.as_bytes()[1], b' ' | b'x' | b'X');
                if is_task {
                    let (a, b) = (pos(k + 2), pos(k + 5));
                    let c = egui::pos2((a.left() + b.left()) / 2.0, a.center().y);
                    let rect = egui::Rect::from_center_size(c, Vec2::splat(15.0));
                    if after.as_bytes()[1] == b' ' {
                        painter.rect_stroke(rect, 4.0, Stroke::new(1.3, p.faint), egui::StrokeKind::Inside);
                    } else {
                        painter.rect_filled(rect, 4.0, check);
                        let s = Stroke::new(1.8, Color32::WHITE);
                        painter.line_segment([c + vec2(-3.8, 0.2), c + vec2(-1.0, 3.0)], s);
                        painter.line_segment([c + vec2(-1.0, 3.0), c + vec2(4.0, -2.8)], s);
                    }
                    boxes.push((rect, kb + 3));
                } else {
                    let (a, b) = (pos(k), pos(k + 1));
                    painter.circle_filled(egui::pos2((a.left() + b.left()) / 2.0, a.center().y), 2.6, p.muted);
                }
            }
        }
        ci += line_chars;
        byte += line.len();
    }
    if let Some(start) = fence_start {
        under.push(code_rect(start, total_chars.saturating_sub(1).max(start)));
    }
    painter.set(bg_slot, egui::Shape::Vec(under));
    boxes
}

#[allow(clippy::too_many_arguments)]
fn inline(job: &mut LayoutJob, s: &str, base: &TextFormat, p: &Palette, m: &Metrics, mono: &TextFormat, reveal: bool) {
    let faint = if reveal {
        TextFormat { color: p.faint, ..base.clone() }
    } else {
        TextFormat { font_id: FontId::proportional(0.5), color: Color32::TRANSPARENT, ..Default::default() }
    };
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut plain_start = 0;
    let flush = |job: &mut LayoutJob, from: usize, to: usize| {
        if to > from {
            job.append(&s[from..to], 0.0, base.clone());
        }
    };
    while i < bytes.len() {
        let rest = &s[i..];
        // `code`
        if bytes[i] == b'`'
            && let Some(end) = rest[1..].find('`')
        {
            flush(job, plain_start, i);
            let mut m = mono.clone();
            m.font_id.size = base.font_id.size - 1.5;
            job.append("`", 0.0, faint.clone());
            job.append(&rest[1..end + 1], 0.0, m);
            job.append("`", 0.0, faint.clone());
            i += end + 2;
            plain_start = i;
            continue;
        }
        // [[wikilink]]
        if rest.starts_with("[[")
            && let Some(end) = rest.find("]]")
            && !rest[2..end].contains('\n')
        {
            flush(job, plain_start, i);
            let inner = &rest[2..end];
            job.append("[[", 0.0, faint.clone());
            match inner.find('|') {
                Some(bar) if !reveal => {
                    job.append(&inner[..=bar], 0.0, faint.clone());
                    job.append(&inner[bar + 1..], 0.0, TextFormat { color: p.link, ..base.clone() });
                }
                _ => job.append(inner, 0.0, TextFormat { color: p.link, ..base.clone() }),
            }
            job.append("]]", 0.0, faint.clone());
            i += end + 2;
            plain_start = i;
            continue;
        }
        // [text](url)
        if bytes[i] == b'['
            && let Some(close) = rest.find("](")
            && let Some(paren) = rest[close..].find(')')
            && !rest[..close].contains('\n')
        {
            flush(job, plain_start, i);
            job.append("[", 0.0, faint.clone());
            job.append(&rest[1..close], 0.0, TextFormat { color: p.link, ..base.clone() });
            job.append(&rest[close..close + paren + 1], 0.0, faint.clone());
            i += close + paren + 1;
            plain_start = i;
            continue;
        }
        // **bold** / __bold__
        if (rest.starts_with("**") || rest.starts_with("__"))
            && let Some(end) = rest[2..].find(&rest[..2])
            && end > 0
        {
            flush(job, plain_start, i);
            let b = TextFormat {
                font_id: FontId::new(base.font_id.size, bold_family()),
                color: m.bold.unwrap_or(p.heading),
                ..base.clone()
            };
            job.append(&rest[..2], 0.0, faint.clone());
            job.append(&rest[2..2 + end], 0.0, b);
            job.append(&rest[2 + end..4 + end], 0.0, faint.clone());
            i += end + 4;
            plain_start = i;
            continue;
        }
        // *italic* / _italic_ (not inside words for "_")
        if (bytes[i] == b'*' || (bytes[i] == b'_' && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())))
            && rest.len() > 2
            && !rest[1..].starts_with(' ')
            && let Some(end) = rest[1..].find(bytes[i] as char)
            && end > 0
        {
            flush(job, plain_start, i);
            let it = TextFormat {
                font_id: FontId::new(base.font_id.size, italic_family()),
                color: m.italic.unwrap_or(base.color),
                ..base.clone()
            };
            job.append(&rest[..1], 0.0, faint.clone());
            job.append(&rest[1..1 + end], 0.0, it);
            job.append(&rest[1 + end..2 + end], 0.0, faint.clone());
            i += end + 2;
            plain_start = i;
            continue;
        }
        // ==highlight==
        if rest.starts_with("==")
            && let Some(end) = rest[2..].find("==")
            && end > 0
        {
            flush(job, plain_start, i);
            job.append(&rest[..2], 0.0, faint.clone());
            job.append(
                &rest[2..2 + end],
                0.0,
                TextFormat { background: Color32::from_rgba_unmultiplied(255, 208, 0, 60), ..base.clone() },
            );
            job.append(&rest[2 + end..4 + end], 0.0, faint.clone());
            i += end + 4;
            plain_start = i;
            continue;
        }
        // #tag
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let tag_len: usize = rest[1..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                .map(char::len_utf8)
                .sum();
            if tag_len > 0 && rest[1..].chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
                flush(job, plain_start, i);
                job.append(
                    &rest[..1 + tag_len],
                    0.0,
                    TextFormat { color: m.tag_color.unwrap_or(p.link), background: p.tag_bg, ..base.clone() },
                );
                i += 1 + tag_len;
                plain_start = i;
                continue;
            }
        }
        i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
    }
    flush(job, plain_start, s.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_keeps_every_char() {
        let p = palette(true);
        let src = "# Titre **gras**\n\n- [x] fait *ok* `code` [[Lien|alias]] #tag ==hl==\n> [!note] cite\n```rs\nlet a = 1;\n```\n[a](b) é_t_é __x__\n";
        for active in [None, Some((0, 2)), Some((0, 100))] {
            let job = markdown_job(src, &p, &Metrics::default(), 15.0, 500.0, active);
            assert_eq!(job.text, src);
        }
    }
}
