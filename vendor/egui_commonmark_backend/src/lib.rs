//! Shared code for egui_commonmark and egui_commonmark_macro. Breaking changes will happen and
//! should ideally not be relied upon. Only items that can been seen in this documentation
//! can be safely used directly.

#[doc(hidden)]
pub mod alerts;
#[doc(hidden)]
pub mod elements;
#[doc(hidden)]
pub mod misc;
#[doc(hidden)]
pub mod pulldown;
#[doc(hidden)]
pub mod search;

#[cfg(feature = "embedded_image")]
mod data_url_loader;

// For ease of use in proc macros
#[doc(hidden)]
pub use {
    alerts::{Alert, AlertBundle, alert_ui},
    // Pretty much every single element in this module is used by the proc macros
    elements::*,
    misc::{CodeBlock, CommonMarkOptions, Image, Link, prepare_show},
};

// The only structs that may be used directly. (If one does not need egui_commonmark)
pub use misc::CommonMarkCache;
#[cfg(feature = "regex")]
pub use misc::SearchOptions;

#[cfg(feature = "better_syntax_highlighting")]
pub use syntect;

/// Patch Mnemo : couleurs et polices du thème (CSS Obsidian) pour le mode lecture. L'application le pose
/// dans `ctx.data` avec [`MdTheme::set`] ; sans thème posé, le rendu reste celui d'origine.
#[derive(Clone, Default, Debug)]
pub struct MdTheme {
    /// couleur de chaque niveau de titre (`--h1-color`…)
    pub heading: [Option<egui::Color32>; 6],
    /// code en ligne (`--code-normal`)
    pub code: Option<egui::Color32>,
    /// barre des citations (`--blockquote-border-color`)
    pub quote_bar: Option<egui::Color32>,
    /// texte des citations ; `None` : texte atténué d'origine
    pub quote_text: Option<egui::Color32>,
    pub bold: Option<egui::Color32>,
    pub italic: Option<egui::Color32>,
    /// bordures des tableaux
    pub table_border: Option<egui::Color32>,
    /// fond de la ligne d'en-tête des tableaux
    pub table_header: Option<egui::Color32>,
    /// familles de police du texte de la note (normal est porté par `TextStyle::Body`)
    pub bold_family: Option<egui::FontFamily>,
    pub italic_family: Option<egui::FontFamily>,
    /// police d'icônes et nom d'icône → caractère (icônes Lucide des callouts)
    pub icon_family: Option<egui::FontFamily>,
    pub icon_glyph: Option<fn(&str) -> Option<char>>,
    /// interligne du texte, en multiple de la taille de police (`--line-height-normal`, 1.5 dans Obsidian) ;
    /// `None` : celui de la police
    pub line_height: Option<f32>,
    /// couleurs du code par type de jeton (`--code-keyword`…, comme Obsidian) et fond des blocs de code
    pub code_colors: Option<CodeColors>,
    pub code_bg: Option<egui::Color32>,
}

/// Patch Mnemo : couleurs des blocs de code, variables `--code-*` d'Obsidian.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodeColors {
    pub normal: egui::Color32,
    pub comment: egui::Color32,
    pub function: egui::Color32,
    pub important: egui::Color32,
    pub keyword: egui::Color32,
    pub operator: egui::Color32,
    pub property: egui::Color32,
    pub punctuation: egui::Color32,
    pub string: egui::Color32,
    pub tag: egui::Color32,
    pub value: egui::Color32,
}

impl MdTheme {
    fn id() -> egui::Id {
        egui::Id::unique("mnemo-md-theme")
    }

    pub fn set(ctx: &egui::Context, theme: Self) {
        ctx.data_mut(|d| d.insert_temp(Self::id(), theme));
    }

    pub fn get(ctx: &egui::Context) -> Option<Self> {
        ctx.data(|d| d.get_temp(Self::id()))
    }
}

/// Takes [`egui::Ui`], the math text to be rendered and whether it is inline
pub type RenderMathFn = dyn Fn(&mut egui::Ui, &str, bool);
/// Takes [`egui::Ui`] and the html text to be rendered/used
pub type RenderHtmlFn = dyn Fn(&mut egui::Ui, &str);
